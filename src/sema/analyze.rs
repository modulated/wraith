//! Semantic Analysis Logic
//!
//! Traverses the AST to populate the symbol table and perform type checking.

use crate::ast::{Expr, Function, Item, PrimitiveType, SourceFile, Spanned, Stmt, TypeExpr};
use crate::codegen::memory_layout::MemoryLayout;
use crate::sema::const_eval::{eval_const_expr_with_env, ConstEnv, ConstValue};
use crate::sema::table::{SymbolInfo, SymbolKind, SymbolLocation, SymbolTable};
use crate::sema::type_defs::TypeRegistry;
use crate::sema::types::Type;
use crate::sema::{FunctionMetadata, ProgramInfo, SemaError, Warning};

use crate::ast::Span;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

pub struct SemanticAnalyzer {
    pub table: SymbolTable,
    pub errors: Vec<SemaError>,
    pub warnings: Vec<Warning>,
    current_return_type: Option<Type>,
    resolved_symbols: HashMap<Span, SymbolInfo>,
    function_metadata: HashMap<String, FunctionMetadata>,
    folded_constants: HashMap<Span, ConstValue>,
    resolved_types: HashMap<Span, Type>,
    type_registry: TypeRegistry,
    imported_items: Vec<Spanned<Item>>,
    base_path: Option<PathBuf>,
    imported_files: HashSet<PathBuf>,
    zp_allocator: ZeroPageAllocator,
    const_env: ConstEnv,
    loop_depth: usize,
    /// Track variable usage for unused variable warnings (per-function, cleared after each function)
    used_variables: HashSet<String>,
    /// Track ALL symbol usage across entire file (never cleared, for import checking)
    all_used_symbols: HashSet<String>,
    /// Track declared variables in current scope (name -> span) for unused variable detection
    declared_variables: Vec<(String, Span)>,
    /// Track function parameters (name -> span) for unused parameter detection
    declared_parameters: Vec<(String, Span)>,
    /// Track imported symbols (name -> span) for unused import detection
    imported_symbols: Vec<(String, Span)>,
}

/// Zero page memory allocator
/// Manages allocation of zero page addresses ($00-$FF)
#[allow(dead_code)]
struct ZeroPageAllocator {
    /// Next available address
    next_addr: u8,
    /// Reserved ranges (start, end) that cannot be allocated
    reserved: Vec<(u8, u8)>,
}

impl ZeroPageAllocator {
    fn new() -> Self {
        let layout = MemoryLayout::new();
        Self {
            next_addr: layout.variable_alloc_start,
            reserved: layout.get_reserved_regions(),
        }
    }

    /// Allocate a single byte in zero page
    fn allocate(&mut self) -> Result<u8, SemaError> {
        // Find next available address
        loop {
            let addr = self.next_addr;

            // Check if this address is reserved
            let is_reserved = self.reserved.iter().any(|(start, end)| addr >= *start && addr <= *end);

            if !is_reserved && addr != 0xFF {
                self.next_addr = addr + 1;
                return Ok(addr);
            }

            // Try next address
            self.next_addr += 1;

            if self.next_addr == 0 {
                // Wrapped around - out of zero page
                return Err(SemaError::OutOfZeroPage {
                    span: Span { start: 0, end: 0 }, // No span context in allocator
                });
            }
        }
    }

    /// Allocate multiple consecutive bytes
    #[allow(dead_code)]
    fn allocate_range(&mut self, count: u8) -> Result<u8, SemaError> {
        let start = self.next_addr;

        // Check if we have enough space
        if start as usize + count as usize > 0x100 {
            return Err(SemaError::OutOfZeroPage {
                span: Span { start: 0, end: 0 }, // No span context in allocator
            });
        }

        // Allocate each byte
        for _ in 0..count {
            self.allocate()?;
        }

        Ok(start)
    }

    /// Reset allocator (for new scope/function)
    #[allow(dead_code)]
    fn reset(&mut self) {
        let layout = MemoryLayout::new();
        self.next_addr = layout.variable_alloc_start;
    }
}

impl Default for SemanticAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

impl SemanticAnalyzer {
    pub fn new() -> Self {
        Self {
            table: SymbolTable::new(),
            errors: Vec::new(),
            warnings: Vec::new(),
            current_return_type: None,
            resolved_symbols: HashMap::new(),
            function_metadata: HashMap::new(),
            folded_constants: HashMap::new(),
            resolved_types: HashMap::new(),
            type_registry: TypeRegistry::new(),
            imported_items: Vec::new(),
            base_path: None,
            imported_files: HashSet::new(),
            zp_allocator: ZeroPageAllocator::new(),
            const_env: ConstEnv::new(),
            loop_depth: 0,
            used_variables: HashSet::new(),
            all_used_symbols: HashSet::new(),
            declared_variables: Vec::new(),
            declared_parameters: Vec::new(),
            imported_symbols: Vec::new(),
        }
    }

    pub fn with_base_path(base_path: PathBuf) -> Self {
        Self {
            table: SymbolTable::new(),
            errors: Vec::new(),
            warnings: Vec::new(),
            current_return_type: None,
            resolved_symbols: HashMap::new(),
            function_metadata: HashMap::new(),
            folded_constants: HashMap::new(),
            resolved_types: HashMap::new(),
            type_registry: TypeRegistry::new(),
            imported_items: Vec::new(),
            base_path: Some(base_path),
            imported_files: HashSet::new(),
            zp_allocator: ZeroPageAllocator::new(),
            const_env: ConstEnv::new(),
            loop_depth: 0,
            used_variables: HashSet::new(),
            all_used_symbols: HashSet::new(),
            declared_variables: Vec::new(),
            declared_parameters: Vec::new(),
            imported_symbols: Vec::new(),
        }
    }

    /// Get the standard library path
    /// Checks WRAITH_STD_PATH environment variable, falls back to ./std
    fn get_std_lib_path() -> PathBuf {
        std::env::var("WRAITH_STD_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("std"))
    }

    /// Compute the size of a type, looking up named types in the registry
    fn type_size(&self, ty: &Type) -> usize {
        match ty {
            Type::Primitive(prim) => prim.size_bytes(),
            Type::Pointer(_, _) => 2, // Pointers are 16-bit
            Type::Array(element_ty, len) => self.type_size(element_ty) * len,
            Type::String => 2, // String is represented as a pointer
            Type::Function(_, _) => 2, // Function pointer is 16-bit
            Type::Void => 0,
            Type::Named(name) => {
                // Look up in struct registry
                if let Some(struct_def) = self.type_registry.structs.get(name) {
                    return struct_def.total_size;
                }
                // Look up in enum registry
                if let Some(enum_def) = self.type_registry.enums.get(name) {
                    return enum_def.total_size;
                }
                // Unknown type - return 0 as fallback
                0
            }
        }
    }

    pub fn analyze(&mut self, source: &SourceFile) -> Result<ProgramInfo, SemaError> {
        // First pass: Register all global items (functions, statics, structs)
        for item in &source.items {
            self.register_item(item)?;
        }

        // Second pass: Analyze function bodies
        for item in &source.items {
            self.analyze_item(item)?;
        }

        if !self.errors.is_empty() {
            return Err(self.errors[0].clone());
        }

        // Check for unused imports after all analysis is complete
        self.check_unused_imports();

        Ok(ProgramInfo {
            table: self.table.clone(),
            resolved_symbols: self.resolved_symbols.clone(),
            function_metadata: self.function_metadata.clone(),
            folded_constants: self.folded_constants.clone(),
            type_registry: self.type_registry.clone(),
            resolved_types: self.resolved_types.clone(),
            imported_items: self.imported_items.clone(),
            warnings: self.warnings.clone(),
        })
    }

    fn register_item(&mut self, item: &Spanned<Item>) -> Result<(), SemaError> {
        match &item.node {
            Item::Function(func) => {
                let name = func.name.node.clone();

                // Check for instruction conflict
                // Check if function has inline attribute
                let is_inline = func.attributes.iter().any(|attr| {
                    matches!(attr, crate::ast::FnAttribute::Inline)
                });

                // Exception: inline functions (intrinsics) are allowed to use instruction names
                // because they're meant to be direct wrappers for CPU instructions
                if !is_inline && crate::sema::is_instruction_conflict(&name) {
                    return Err(SemaError::InstructionConflict {
                        name: name.clone(),
                        span: func.name.span,
                    });
                }

                // Check for duplicate function definition
                if self.table.defined_in_current_scope(&name) {
                    return Err(SemaError::DuplicateSymbol {
                        name: name.clone(),
                        span: func.name.span,
                        previous_span: None, // Could track this if we store spans
                    });
                }

                let info = SymbolInfo {
                    name: name.clone(),
                    kind: SymbolKind::Function,
                    ty: self.resolve_function_type(func)?,
                    location: SymbolLocation::Absolute(0),
                    mutable: false,
                };
                self.table.insert(name.clone(), info);

                // Extract org and section attributes if present
                let org_address = func.attributes.iter().find_map(|attr| {
                    if let crate::ast::FnAttribute::Org(addr) = attr {
                        Some(*addr)
                    } else {
                        None
                    }
                });

                let section = func.attributes.iter().find_map(|attr| {
                    if let crate::ast::FnAttribute::Section(s) = attr {
                        Some(s.clone())
                    } else {
                        None
                    }
                });

                // For inline functions, store body and parameters for later expansion
                let (inline_body, inline_params) = if is_inline {
                    (Some(func.body.clone()), Some(func.params.clone()))
                } else {
                    (None, None)
                };

                self.function_metadata.insert(
                    name,
                    FunctionMetadata {
                        org_address,
                        section,
                        is_inline,
                        inline_body,
                        inline_params,
                        inline_param_symbols: None, // Will be populated in second pass
                    },
                );
            }
            Item::Static(stat) => {
                let name = stat.name.node.clone();

                // Check for instruction conflict
                if crate::sema::is_instruction_conflict(&name) {
                    return Err(SemaError::InstructionConflict {
                        name: name.clone(),
                        span: stat.name.span,
                    });
                }

                // Check for duplicate static definition
                if self.table.defined_in_current_scope(&name) {
                    return Err(SemaError::DuplicateSymbol {
                        name: name.clone(),
                        span: stat.name.span,
                        previous_span: None,
                    });
                }

                // Check for duplicate static definition
                if self.table.defined_in_current_scope(&name) {
                    return Err(SemaError::DuplicateSymbol {
                        name: name.clone(),
                        span: stat.name.span,
                        previous_span: None,
                    });
                }

                // Resolve the type first so we can check bounds
                let declared_ty = self.resolve_type(&stat.ty.node)?;

                // If it's a non-mutable static (const), evaluate it and add to const_env
                if !stat.mutable {
                    match eval_const_expr_with_env(&stat.init, &self.const_env) {
                        Ok(val) => {
                            // Check that the constant value fits within the declared type
                            if let Some(int_val) = val.as_integer() {
                                // Check overflow based on type
                                let fits = match &declared_ty {
                                    Type::Primitive(crate::ast::PrimitiveType::U8) => {
                                        (0..=255).contains(&int_val)
                                    }
                                    Type::Primitive(crate::ast::PrimitiveType::I8) => {
                                        (-128..=127).contains(&int_val)
                                    }
                                    Type::Primitive(crate::ast::PrimitiveType::U16) => {
                                        (0..=65535).contains(&int_val)
                                    }
                                    Type::Primitive(crate::ast::PrimitiveType::I16) => {
                                        (-32768..=32767).contains(&int_val)
                                    }
                                    _ => true, // For non-primitive types, don't check
                                };

                                if !fits {
                                    return Err(SemaError::ConstantOverflow {
                                        value: int_val,
                                        ty: declared_ty.display_name(),
                                        span: stat.init.span,
                                    });
                                }
                            }
                            self.const_env.insert(name.clone(), val);
                        }
                        Err(_) => {
                            // If it's not a constant expression, that's okay - just don't add to const_env
                        }
                    }
                }

                let info = SymbolInfo {
                    name: name.clone(),
                    kind: SymbolKind::Constant,
                    ty: declared_ty,
                    location: SymbolLocation::Absolute(0),
                    mutable: stat.mutable,
                };
                self.table.insert(name, info);
            }
            Item::Address(addr) => {
                let name = addr.name.node.clone();

                // Check for instruction conflict
                if crate::sema::is_instruction_conflict(&name) {
                    return Err(SemaError::InstructionConflict {
                        name: name.clone(),
                        span: addr.name.span,
                    });
                }

                // Check for duplicate address definition
                if self.table.defined_in_current_scope(&name) {
                    return Err(SemaError::DuplicateSymbol {
                        name: name.clone(),
                        span: addr.name.span,
                        previous_span: None,
                    });
                }

                // Check for duplicate address definition
                if self.table.defined_in_current_scope(&name) {
                    return Err(SemaError::DuplicateSymbol {
                        name: name.clone(),
                        span: addr.name.span,
                        previous_span: None,
                    });
                }

                // Evaluate the address expression as a constant, using the const environment
                let address = match eval_const_expr_with_env(&addr.address, &self.const_env) {
                    Ok(ConstValue::Integer(val)) => {
                        if !(0..=0xFFFF).contains(&val) {
                            return Err(SemaError::Custom {
                                message: format!("address value {} out of range (must be 0-65535)", val),
                                span: addr.address.span,
                            });
                        }
                        val as u16
                    }
                    Ok(_) => {
                        return Err(SemaError::Custom {
                            message: "address must evaluate to an integer".to_string(),
                            span: addr.address.span,
                        });
                    }
                    Err(e) => return Err(e),
                };

                // Add address to const_env so it can be used in other addr declarations
                // (e.g., addr SCREEN = BASE + 0x100)
                self.const_env.insert(name.clone(), ConstValue::Integer(address as i64));

                let info = SymbolInfo {
                    name: name.clone(),
                    kind: SymbolKind::Address,
                    ty: Type::Primitive(crate::ast::PrimitiveType::U8),
                    location: SymbolLocation::Absolute(address),
                    mutable: true,
                };
                self.table.insert(name, info);
            }
            Item::Import(import) => {
                self.process_import(import)?;
            }
            Item::Struct(struct_def) => {
                self.register_struct(struct_def)?;
            }
            Item::Enum(enum_def) => {
                self.register_enum(enum_def)?;
            }
        }
        Ok(())
    }

    fn process_import(&mut self, import: &crate::ast::Import) -> Result<(), SemaError> {
        // Resolve the import path
        let import_str = &import.path.node;
        let import_path = if import_str.starts_with("./") || import_str.starts_with("../") {
            // Relative import - resolve relative to the current file's directory
            if let Some(base) = &self.base_path {
                base.parent().unwrap_or(base).join(import_str)
            } else {
                PathBuf::from(import_str)
            }
        } else {
            // Non-relative import - search in standard library directory first
            let std_path = Self::get_std_lib_path().join(import_str);
            if std_path.exists() {
                std_path
            } else {
                // Fall back to current directory or relative to base path
                if let Some(base) = &self.base_path {
                    base.parent().unwrap_or(base).join(import_str)
                } else {
                    PathBuf::from(import_str)
                }
            }
        };

        // Check if we've already imported this file to avoid circular imports
        if self.imported_files.contains(&import_path) {
            return Ok(());
        }
        self.imported_files.insert(import_path.clone());

        // Load and parse the imported file
        let source = std::fs::read_to_string(&import_path)
            .map_err(|e| SemaError::ImportError {
                path: import.path.node.clone(),
                reason: format!("failed to import '{}': {}", import.path.node, e),
                span: import.path.span,
            })?;

        let tokens = crate::lex(&source)
            .map_err(|e| SemaError::ImportError {
                path: import.path.node.clone(),
                reason: format!("lexer error: {:?}", e),
                span: import.path.span,
            })?;

        let ast = crate::Parser::parse(&tokens)
            .map_err(|e| SemaError::ImportError {
                path: import.path.node.clone(),
                reason: format!("parser error: {:?}", e),
                span: import.path.span,
            })?;

        // Analyze the imported file
        let mut imported_analyzer = SemanticAnalyzer::with_base_path(import_path.clone());
        imported_analyzer.imported_files = self.imported_files.clone();
        let imported_info = imported_analyzer.analyze(&ast)?;

        // Collect all items from the imported file for codegen
        // We collect ALL items, not just the imported symbols, because functions
        // may depend on other functions in the same module
        self.imported_items.extend(ast.items.clone());

        // Also collect items from transitively imported modules
        self.imported_items.extend(imported_info.imported_items.clone());

        // Import the requested symbols into our table
        for symbol_name in &import.symbols {
            let name = &symbol_name.node;
            if let Some(symbol) = imported_info.table.lookup(name) {
                self.table.insert(name.clone(), symbol.clone());

                // Track imported symbol for unused import detection
                self.imported_symbols.push((name.clone(), symbol_name.span));

                // Also import function metadata if this is a function
                if let Some(metadata) = imported_info.function_metadata.get(name) {
                    self.function_metadata.insert(name.clone(), metadata.clone());
                }
            } else {
                return Err(SemaError::ImportError {
                    path: import.path.node.clone(),
                    reason: format!("symbol '{}' not found in imported file", name),
                    span: symbol_name.span,
                });
            }
        }

        // Merge ALL resolved_symbols from the imported module
        // This is necessary because when we emit imported functions during codegen,
        // they reference symbols (variables, constants, addresses) using their original spans
        for (span, symbol) in &imported_info.resolved_symbols {
            self.resolved_symbols.insert(*span, symbol.clone());

            // Also add constants and addresses to the symbol table so they're visible
            // to code in this module
            if matches!(symbol.kind, crate::sema::table::SymbolKind::Constant | crate::sema::table::SymbolKind::Address)
                && self.table.lookup(&symbol.name).is_none()
            {
                self.table.insert(symbol.name.clone(), symbol.clone());
            }
        }

        // Merge folded_constants so constant expressions from imported modules are available
        for (span, value) in &imported_info.folded_constants {
            self.folded_constants.insert(*span, value.clone());
        }

        // Merge resolved_types so type information from imported modules is available
        for (span, ty) in &imported_info.resolved_types {
            self.resolved_types.insert(*span, ty.clone());
        }

        // Merge function_metadata (already done above in the loop, but ensure transitives)
        for (name, metadata) in &imported_info.function_metadata {
            if !self.function_metadata.contains_key(name) {
                self.function_metadata.insert(name.clone(), metadata.clone());
            }
        }

        // Merge the imported files set
        self.imported_files.extend(imported_analyzer.imported_files);

        Ok(())
    }

    fn register_struct(&mut self, struct_def: &crate::ast::Struct) -> Result<(), SemaError> {
        use crate::sema::type_defs::{FieldInfo, StructDef};

        let name = struct_def.name.node.clone();

        // Check for instruction conflict
        if crate::sema::is_instruction_conflict(&name) {
            return Err(SemaError::InstructionConflict {
                name: name.clone(),
                span: struct_def.name.span,
            });
        }

        // Check for duplicate struct definition
        if self.type_registry.get_struct(&name).is_some() {
            return Err(SemaError::DuplicateSymbol {
                name: name.clone(),
                span: struct_def.name.span,
                previous_span: None,
            });
        }

        let mut fields = Vec::new();
        let mut offset = 0;
        let mut seen_fields = HashSet::new();

        // Calculate field offsets
        for field in &struct_def.fields {
            let field_name = field.name.node.clone();

            // Check for duplicate field
            if !seen_fields.insert(field_name.clone()) {
                return Err(SemaError::DuplicateSymbol {
                    name: field_name,
                    span: field.name.span,
                    previous_span: None,
                });
            }

            let field_type = self.resolve_type(&field.ty.node)?;
            let size = self.type_size(&field_type);

            fields.push(FieldInfo {
                name: field_name,
                ty: field_type,
                offset,
            });

            offset += size;
        }

        // Check if struct should be in zero page
        let zero_page = struct_def.attributes.iter().any(|attr| {
            matches!(attr, crate::ast::StructAttribute::ZpSection)
        });

        let struct_info = StructDef {
            name: name.clone(),
            fields,
            total_size: offset,
            zero_page,
        };

        self.type_registry.add_struct(struct_info);

        // Add the struct type to the symbol table as a type name
        self.table.insert(
            name.clone(),
            SymbolInfo {
                name: name.clone(),
                kind: SymbolKind::Type,
                ty: Type::Named(name),
                location: SymbolLocation::None,
                mutable: false,
            },
        );

        Ok(())
    }

    fn register_enum(&mut self, enum_def: &crate::ast::Enum) -> Result<(), SemaError> {
        use crate::sema::type_defs::{EnumDef, FieldInfo, VariantData, VariantInfo};
        use crate::ast::EnumVariant;

        let name = enum_def.name.node.clone();

        // Check for instruction conflict
        if crate::sema::is_instruction_conflict(&name) {
            return Err(SemaError::InstructionConflict {
                name: name.clone(),
                span: enum_def.name.span,
            });
        }

        // Check for duplicate enum definition
        if self.type_registry.get_enum(&name).is_some() {
            return Err(SemaError::DuplicateSymbol {
                name: name.clone(),
                span: enum_def.name.span,
                previous_span: None,
            });
        }

        let mut variants = Vec::new();
        let mut next_tag: u8 = 0;
        let mut seen_variants = HashSet::new();

        // Process each variant
        for variant in &enum_def.variants {
            let (variant_name, variant_data, tag) = match variant {
                EnumVariant::Unit { name: var_name, value } => {
                    let tag = value.map(|v| v as u8).unwrap_or(next_tag);
                    next_tag = tag + 1;
                    (var_name.node.clone(), VariantData::Unit, tag)
                }
                EnumVariant::Tuple { name: var_name, fields: field_types } => {
                    let types: Result<Vec<Type>, SemaError> = field_types
                        .iter()
                        .map(|ty| self.resolve_type(&ty.node))
                        .collect();
                    let tag = next_tag;
                    next_tag += 1;
                    (var_name.node.clone(), VariantData::Tuple(types?), tag)
                }
                EnumVariant::Struct { name: var_name, fields } => {
                    let mut variant_fields = Vec::new();
                    let mut offset = 0;

                    for field in fields {
                        let field_type = self.resolve_type(&field.ty.node)?;
                        let size = self.type_size(&field_type);

                        variant_fields.push(FieldInfo {
                            name: field.name.node.clone(),
                            ty: field_type,
                            offset,
                        });

                        offset += size;
                    }

                    let tag = next_tag;
                    next_tag += 1;
                    (var_name.node.clone(), VariantData::Struct(variant_fields), tag)
                }
            };

            // Check for duplicate variant
            if !seen_variants.insert(variant_name.clone()) {
                // Get the span from the variant
                let variant_span = match variant {
                    EnumVariant::Unit { name, .. } => name.span,
                    EnumVariant::Tuple { name, .. } => name.span,
                    EnumVariant::Struct { name, .. } => name.span,
                };
                return Err(SemaError::DuplicateSymbol {
                    name: variant_name,
                    span: variant_span,
                    previous_span: None,
                });
            }

            variants.push(VariantInfo {
                name: variant_name,
                tag,
                data: variant_data,
            });
        }

        // Calculate enum size: 1 byte tag + max variant data size
        let max_data_size = variants
            .iter()
            .map(|v| match &v.data {
                VariantData::Unit => 0,
                VariantData::Tuple(types) => types.iter().map(|t| self.type_size(t)).sum(),
                VariantData::Struct(fields) => {
                    // Use the last field's offset + size, or 0 if no fields
                    fields.last().map(|f| f.offset + self.type_size(&f.ty)).unwrap_or(0)
                }
            })
            .max()
            .unwrap_or(0);

        let total_size = 1 + max_data_size;

        let enum_info = EnumDef {
            name: name.clone(),
            variants,
            total_size,
        };

        self.type_registry.add_enum(enum_info);

        // Add the enum type to the symbol table as a type name
        self.table.insert(
            name.clone(),
            SymbolInfo {
                name: name.clone(),
                kind: SymbolKind::Type,
                ty: Type::Named(name),
                location: SymbolLocation::None,
                mutable: false,
            },
        );

        Ok(())
    }

    fn analyze_item(&mut self, item: &Spanned<Item>) -> Result<(), SemaError> {
        if let Item::Function(func) = &item.node {
            let func_name = func.name.node.clone();

            // Check if this is an inline function
            let is_inline = func.attributes.iter().any(|attr| {
                matches!(attr, crate::ast::FnAttribute::Inline)
            });

            self.table.enter_scope();

            // Set current return type for checking return statements
            let return_type = if let Some(ret) = &func.return_type {
                self.resolve_type(&ret.node)?
            } else {
                Type::Void
            };
            self.current_return_type = Some(return_type);

            // For inline functions, track symbols before body analysis
            let resolved_before = if is_inline {
                Some(self.resolved_symbols.clone())
            } else {
                None
            };

            // Register parameters
            // Parameters are passed via the param region ($80+), not regular variable space
            // Each parameter gets sequential bytes (16-bit params take 2 bytes)
            let layout = MemoryLayout::new();
            let mut byte_offset = 0u8;
            for param in func.params.iter() {
                let name = param.name.node.clone();

                // Check for duplicate parameter names
                if self.table.defined_in_current_scope(&name) {
                    return Err(SemaError::DuplicateSymbol {
                        name: name.clone(),
                        span: param.name.span,
                        previous_span: None,
                    });
                }

                // Allocate parameter in the param region ($80 + byte_offset)
                let addr = layout.param_base + byte_offset;
                if addr > layout.param_end {
                    return Err(SemaError::OutOfZeroPage {
                        span: param.name.span,
                    });
                }

                let location = SymbolLocation::ZeroPage(addr);
                let param_type = self.resolve_type(&param.ty.node)?;
                let param_size = param_type.size();

                let info = SymbolInfo {
                    name: name.clone(),
                    kind: SymbolKind::Variable,
                    ty: param_type,
                    location,
                    mutable: false,
                };
                self.table.insert(name.clone(), info.clone());
                // Add to resolved_symbols so codegen (especially inline asm) can find it
                self.resolved_symbols.insert(param.name.span, info.clone());

                // Track parameter for unused parameter detection
                self.declared_parameters.push((name, param.name.span));

                // Advance byte offset by parameter size (16-bit params take 2 bytes)
                byte_offset += param_size as u8;
            }

            // Analyze body
            self.analyze_stmt(&func.body)?;

            // For inline functions, capture all symbols that were added during body analysis
            // This includes both parameter definitions and all references to them
            if is_inline
                && let Some(before) = resolved_before {
                    // Collect all NEW symbols that were added during parameter registration and body analysis
                    let mut inline_symbols = std::collections::HashMap::new();
                    for (span, info) in &self.resolved_symbols {
                        if !before.contains_key(span) {
                            inline_symbols.insert(*span, info.clone());
                        }
                    }

                    if let Some(metadata) = self.function_metadata.get_mut(&func_name) {
                        metadata.inline_param_symbols = Some(inline_symbols);
                    }
                }

            // Check for unused variables and parameters
            self.check_unused_variables();

            self.current_return_type = None;
            self.table.exit_scope();
        }
        Ok(())
    }

    /// Check for unused variables and parameters, generate warnings
    fn check_unused_variables(&mut self) {
        // Check unused local variables
        for (var_name, var_span) in &self.declared_variables {
            if !self.used_variables.contains(var_name) {
                self.warnings.push(Warning::UnusedVariable {
                    name: var_name.clone(),
                    span: *var_span,
                });
            }
        }

        // Check unused function parameters
        // Skip parameters starting with _ (convention for intentionally unused)
        for (param_name, param_span) in &self.declared_parameters {
            if !param_name.starts_with('_') && !self.used_variables.contains(param_name) {
                self.warnings.push(Warning::UnusedParameter {
                    name: param_name.clone(),
                    span: *param_span,
                });
            }
        }

        // Clear for next function/scope
        self.declared_variables.clear();
        self.declared_parameters.clear();
        self.used_variables.clear();
    }

    /// Extract variable references from inline assembly template strings
    /// Variables are referenced as {var_name} or {struct.field}
    fn extract_asm_variables(&mut self, instruction: &str) {
        let mut chars = instruction.chars().peekable();

        while let Some(ch) = chars.next() {
            if ch == '{' {
                // Extract variable name between { and }
                let mut var_name = String::new();

                while let Some(&next_ch) = chars.peek() {
                    if next_ch == '}' {
                        chars.next(); // Consume the '}'
                        break;
                    }
                    var_name.push(next_ch);
                    chars.next();
                }

                // Handle struct field access: {struct.field}
                // Mark the base variable (before the dot) as used
                let base_var = if let Some(dot_pos) = var_name.find('.') {
                    &var_name[..dot_pos]
                } else {
                    &var_name
                };

                if !base_var.is_empty() {
                    // Mark variable as used
                    self.used_variables.insert(base_var.to_string());
                    self.all_used_symbols.insert(base_var.to_string());
                }
            }
        }
    }

    /// Check for unused imports and generate warnings
    /// This should be called at the end of file analysis, after all symbols have been used
    fn check_unused_imports(&mut self) {
        // all_used_symbols tracks usage across entire file
        // Check which imported symbols were never used
        for (import_name, import_span) in &self.imported_symbols {
            if !self.all_used_symbols.contains(import_name) {
                self.warnings.push(Warning::UnusedImport {
                    name: import_name.clone(),
                    span: *import_span,
                });
            }
        }
    }

    /// Check if a match statement exhaustively covers all enum variants
    fn check_match_exhaustiveness(
        &mut self,
        enum_name: &str,
        arms: &[crate::ast::MatchArm],
        match_span: Span,
    ) -> Result<(), SemaError> {
        use crate::ast::Pattern;

        // Get the enum definition
        let enum_def = if let Some(def) = self.type_registry.get_enum(enum_name) {
            def.clone()
        } else {
            // Not an enum or not found - skip exhaustiveness check
            return Ok(());
        };

        // Check if there's a wildcard pattern
        let has_wildcard = arms.iter().any(|arm| {
            matches!(arm.pattern.node, Pattern::Wildcard)
        });

        if has_wildcard {
            // Wildcard covers everything - match is exhaustive
            return Ok(());
        }

        // Collect covered variants
        let mut covered_variants = std::collections::HashSet::new();
        for arm in arms {
            if let Pattern::EnumVariant { variant, .. } = &arm.pattern.node {
                covered_variants.insert(variant.node.clone());
            }
        }

        // Find missing variants
        let all_variants: Vec<String> = enum_def.variants.iter().map(|v| v.name.clone()).collect();
        let missing_variants: Vec<String> = all_variants
            .iter()
            .filter(|v| !covered_variants.contains(*v))
            .cloned()
            .collect();

        if !missing_variants.is_empty() {
            // Generate warning for non-exhaustive match
            self.warnings.push(Warning::NonExhaustiveMatch {
                missing_patterns: missing_variants,
                span: match_span,
            });
        }

        Ok(())
    }

    /// Add pattern bindings to the current scope
    fn add_pattern_bindings(
        &mut self,
        pattern: &crate::ast::Pattern,
        match_ty: &Type,
    ) -> Result<(), SemaError> {
        use crate::ast::Pattern;

        match pattern {
            Pattern::EnumVariant { enum_name, variant, bindings } => {
                use crate::sema::type_defs::VariantData;

                // Get enum definition to find variant field types
                if let Some(enum_def) = self.type_registry.get_enum(&enum_name.node)
                    && let Some(variant_def) = enum_def.variants.iter().find(|v| v.name == variant.node) {
                        // Add bindings for tuple variant fields
                        match &variant_def.data {
                            VariantData::Tuple(field_types) => {
                                for (i, binding) in bindings.iter().enumerate() {
                                    if let Some(field_ty) = field_types.get(i) {
                                        let addr = self.zp_allocator.allocate()?;
                                        let info = SymbolInfo {
                                            name: binding.name.node.clone(),
                                            kind: SymbolKind::Variable,
                                            ty: field_ty.clone(),
                                            location: SymbolLocation::ZeroPage(addr),
                                            mutable: false,
                                        };
                                        self.table.insert(binding.name.node.clone(), info);
                                    }
                                }
                            }
                            _ => {
                                // Unit and Struct variants don't have tuple-style bindings
                            }
                        }
                    }
            }
            Pattern::Variable(name) => {
                // Bind the entire matched value
                let addr = self.zp_allocator.allocate()?;
                let info = SymbolInfo {
                    name: name.clone(),
                    kind: SymbolKind::Variable,
                    ty: match_ty.clone(),
                    location: SymbolLocation::ZeroPage(addr),
                    mutable: false,
                };
                self.table.insert(name.clone(), info);
            }
            Pattern::Wildcard => {
                // No bindings for wildcard
            }
            _ => {
                // Literal and Range patterns don't create bindings
            }
        }

        Ok(())
    }

    fn analyze_stmt(&mut self, stmt: &Spanned<Stmt>) -> Result<(), SemaError> {
        match &stmt.node {
            Stmt::Block(stmts) => {
                self.table.enter_scope();
                let mut found_terminator = false;
                for s in stmts.iter() {
                    if found_terminator {
                        // Warn about unreachable code after return/break/continue
                        self.warnings.push(Warning::UnreachableCode {
                            span: s.span,
                        });
                        // Continue analyzing (don't skip) to find other errors/warnings
                    }

                    self.analyze_stmt(s)?;

                    // Check if this statement terminates control flow
                    if matches!(s.node, Stmt::Return(_) | Stmt::Break | Stmt::Continue) {
                        found_terminator = true;
                    }
                }
                self.table.exit_scope();
            }
            Stmt::VarDecl {
                name,
                ty,
                init,
                mutable,
                zero_page: _,
            } => {
                let declared_ty = self.resolve_type(&ty.node)?;
                let init_ty = self.check_expr(init)?;

                // Allow Bool to U8 assignment (booleans are 0/1 bytes in 6502)
                // Check if initializer type can be implicitly converted to declared type
                if !init_ty.is_implicitly_convertible_to(&declared_ty) {
                    return Err(SemaError::TypeMismatch {
                        expected: declared_ty.display_name(),
                        found: init_ty.display_name(),
                        span: init.span,
                    });
                }

                // Check for duplicate variable in current scope
                if self.table.defined_in_current_scope(&name.node) {
                    return Err(SemaError::DuplicateSymbol {
                        name: name.node.clone(),
                        span: name.span,
                        previous_span: None,
                    });
                }

                // Check for duplicate variable in current scope
                if self.table.defined_in_current_scope(&name.node) {
                    return Err(SemaError::DuplicateSymbol {
                        name: name.node.clone(),
                        span: name.span,
                        previous_span: None,
                    });
                }

                // Allocate in zero page using allocator
                // Arrays (pointers) and u16/i16/b16 types need 2 bytes
                let addr = if matches!(declared_ty,
                    Type::Array(_, _) |
                    Type::Primitive(crate::ast::PrimitiveType::U16) |
                    Type::Primitive(crate::ast::PrimitiveType::I16) |
                    Type::Primitive(crate::ast::PrimitiveType::B16)
                ) {
                    self.zp_allocator.allocate_range(2)?
                } else {
                    self.zp_allocator.allocate()?
                };
                let location = SymbolLocation::ZeroPage(addr);

                let info = SymbolInfo {
                    name: name.node.clone(),
                    kind: SymbolKind::Variable,
                    ty: declared_ty,
                    location,
                    mutable: *mutable,
                };
                self.table.insert(name.node.clone(), info.clone());
                // Also add to resolved_symbols so codegen can find it
                self.resolved_symbols.insert(name.span, info);

                // Track declared variable for unused variable warnings
                self.declared_variables.push((name.node.clone(), name.span));
            }
            Stmt::Assign { target, value } => {
                let target_ty = self.check_expr(target)?;
                let value_ty = self.check_expr(value)?;

                // Allow Bool to U8 assignment (booleans are 0/1 bytes in 6502)
                // Check if value type can be implicitly converted to target type
                if !value_ty.is_implicitly_convertible_to(&target_ty) {
                    return Err(SemaError::TypeMismatch {
                        expected: target_ty.display_name(),
                        found: value_ty.display_name(),
                        span: value.span,
                    });
                }

                // Check mutability
                if let Expr::Variable(name) = &target.node
                    && let Some(info) = self.table.lookup(name)
                        && !info.mutable {
                            return Err(SemaError::ImmutableAssignment {
                                symbol: name.clone(),
                                span: target.span,
                            });
                        }
            }
            Stmt::Return(expr) => {
                let expr_ty = if let Some(e) = expr {
                    self.check_expr(e)?
                } else {
                    Type::Void
                };

                if let Some(ret_ty) = &self.current_return_type {
                    // Check if return expression type can be implicitly converted to return type
                    if !expr_ty.is_implicitly_convertible_to(ret_ty) {
                        return Err(SemaError::ReturnTypeMismatch {
                            expected: ret_ty.display_name(),
                            found: expr_ty.display_name(),
                            span: expr.as_ref().map(|e| e.span).unwrap_or(stmt.span),
                        });
                    }
                }
            }
            Stmt::If {
                condition,
                then_branch,
                else_branch,
            } => {
                let cond_ty = self.check_expr(condition)?;
                if cond_ty != Type::Primitive(crate::ast::PrimitiveType::Bool) {
                    return Err(SemaError::TypeMismatch {
                        expected: "bool".to_string(),
                        found: cond_ty.display_name(),
                        span: condition.span,
                    });
                }
                self.analyze_stmt(then_branch)?;
                if let Some(else_b) = else_branch {
                    self.analyze_stmt(else_b)?;
                }
            }
            Stmt::While { condition, body } => {
                let cond_ty = self.check_expr(condition)?;
                if cond_ty != Type::Primitive(crate::ast::PrimitiveType::Bool) {
                    return Err(SemaError::TypeMismatch {
                        expected: "bool".to_string(),
                        found: cond_ty.display_name(),
                        span: condition.span,
                    });
                }
                self.loop_depth += 1;
                self.analyze_stmt(body)?;
                self.loop_depth -= 1;
            }
            Stmt::For {
                var_name,
                var_type,
                range,
                body,
            } => {
                // Create a new scope for the loop variable
                self.table.enter_scope();

                // Determine loop variable type (explicit or inferred)
                let var_ty = if let Some(ty) = var_type {
                    self.resolve_type(&ty.node)?
                } else {
                    // Infer type from range bounds
                    let start_ty = self.check_expr(&range.start)?;
                    let end_ty = self.check_expr(&range.end)?;

                    // Use the larger of the two types
                    match (start_ty, end_ty) {
                        (Type::Primitive(PrimitiveType::U16), _) | (_, Type::Primitive(PrimitiveType::U16)) => {
                            Type::Primitive(PrimitiveType::U16)
                        }
                        (Type::Primitive(PrimitiveType::I16), _) | (_, Type::Primitive(PrimitiveType::I16)) => {
                            Type::Primitive(PrimitiveType::I16)
                        }
                        _ => Type::Primitive(PrimitiveType::U8) // Default to u8
                    }
                };

                // Check for duplicate loop variable (shouldn't happen in new scope, but check anyway)
                if self.table.defined_in_current_scope(&var_name.node) {
                    return Err(SemaError::DuplicateSymbol {
                        name: var_name.node.clone(),
                        span: var_name.span,
                        previous_span: None,
                    });
                }

                // Check for duplicate loop variable (shouldn't happen in new scope, but check anyway)
                if self.table.defined_in_current_scope(&var_name.node) {
                    return Err(SemaError::DuplicateSymbol {
                        name: var_name.node.clone(),
                        span: var_name.span,
                        previous_span: None,
                    });
                }

                let addr = self.zp_allocator.allocate()?;
                let info = SymbolInfo {
                    name: var_name.node.clone(),
                    kind: SymbolKind::Variable,
                    ty: var_ty,
                    location: SymbolLocation::ZeroPage(addr),
                    mutable: true,
                };
                self.table.insert(var_name.node.clone(), info);

                // Check range bounds if not already checked
                if var_type.is_some() {
                    self.check_expr(&range.start)?;
                    self.check_expr(&range.end)?;
                }

                // Analyze body
                self.loop_depth += 1;
                self.analyze_stmt(body)?;
                self.loop_depth -= 1;

                self.table.exit_scope();
            }
            Stmt::ForEach {
                var_name,
                var_type,
                iterable,
                body,
            } => {
                // Create a new scope for the loop variable
                self.table.enter_scope();

                // Check the iterable expression (should be an array)
                let iterable_ty = self.check_expr(iterable)?;

                // Extract element type from array type
                let element_ty = match &iterable_ty {
                    Type::Array(elem_ty, _size) => (**elem_ty).clone(),
                    _ => {
                        return Err(SemaError::TypeMismatch {
                            expected: "array".to_string(),
                            found: iterable_ty.display_name(),
                            span: iterable.span,
                        });
                    }
                };

                // Determine loop variable type (explicit or inferred from array element type)
                let var_ty = if let Some(ty) = var_type {
                    let declared_ty = self.resolve_type(&ty.node)?;
                    // Check that declared type matches element type
                    if declared_ty != element_ty {
                        return Err(SemaError::TypeMismatch {
                            expected: element_ty.display_name(),
                            found: declared_ty.display_name(),
                            span: ty.span,
                        });
                    }
                    declared_ty
                } else {
                    element_ty
                };

                // Allocate storage for loop variable
                // u16/i16 types need 2 bytes
                let addr = if matches!(var_ty,
                    Type::Primitive(crate::ast::PrimitiveType::U16) |
                    Type::Primitive(crate::ast::PrimitiveType::I16)
                ) {
                    self.zp_allocator.allocate_range(2)?
                } else {
                    self.zp_allocator.allocate()?
                };
                let info = SymbolInfo {
                    name: var_name.node.clone(),
                    kind: SymbolKind::Variable,
                    ty: var_ty,
                    location: SymbolLocation::ZeroPage(addr),
                    mutable: true,
                };
                self.table.insert(var_name.node.clone(), info.clone());
                // Add to resolved_symbols so codegen can find it
                self.resolved_symbols.insert(var_name.span, info);

                // Analyze body
                self.loop_depth += 1;
                self.analyze_stmt(body)?;
                self.loop_depth -= 1;

                self.table.exit_scope();
            }
            Stmt::Loop { body } => {
                self.loop_depth += 1;
                self.analyze_stmt(body)?;
                self.loop_depth -= 1;
            }
            Stmt::Expr(expr) => {
                self.check_expr(expr)?;
            }
            Stmt::Break => {
                if self.loop_depth == 0 {
                    return Err(SemaError::BreakOutsideLoop { span: stmt.span });
                }
            }
            Stmt::Continue => {
                if self.loop_depth == 0 {
                    return Err(SemaError::BreakOutsideLoop { span: stmt.span });
                }
            }
            Stmt::Match { expr, arms } => {
                // Check the matched expression type
                let match_ty = self.check_expr(expr)?;

                // Check exhaustiveness for enum types
                if let Type::Named(enum_name) = &match_ty {
                    self.check_match_exhaustiveness(enum_name, arms, stmt.span)?;
                }

                // Analyze each arm
                for arm in arms {
                    // Enter new scope for pattern bindings
                    self.table.enter_scope();

                    // Add pattern bindings to scope
                    self.add_pattern_bindings(&arm.pattern.node, &match_ty)?;

                    // Analyze arm body
                    self.analyze_stmt(&arm.body)?;

                    self.table.exit_scope();
                }
            }
            Stmt::Asm { lines } => {
                // Parse inline assembly to extract variable references
                // Variables are referenced as {var_name} or {struct.field}
                for line in lines {
                    self.extract_asm_variables(&line.instruction);
                }
            }
        }
        Ok(())
    }

    /// Check if an expression contains any references to addr symbols (runtime values)
    fn contains_addr_reference(&self, expr: &Spanned<Expr>) -> bool {
        match &expr.node {
            Expr::Variable(name) => {
                // Check if this variable is an addr
                if let Some(sym) = self.table.lookup(name) {
                    sym.kind == SymbolKind::Address
                } else {
                    false
                }
            }
            Expr::Binary { left, right, .. } => {
                self.contains_addr_reference(left) || self.contains_addr_reference(right)
            }
            Expr::Unary { operand, .. } => self.contains_addr_reference(operand),
            Expr::Paren(inner) => self.contains_addr_reference(inner),
            _ => false,
        }
    }

    fn check_expr(&mut self, expr: &Spanned<Expr>) -> Result<Type, SemaError> {
        // Try to fold the expression if it's constant
        // Use const_env so we can fold references to const variables
        // BUT: don't fold if the expression contains references to addr (runtime values)
        let contains_addr_ref = self.contains_addr_reference(expr);
        if !contains_addr_ref
            && let Ok(const_val) = eval_const_expr_with_env(expr, &self.const_env) {
                self.folded_constants.insert(expr.span, const_val);
            }

        let result_ty = match &expr.node {
            Expr::Literal(lit) => match lit {
                crate::ast::Literal::Integer(val) => {
                    // Infer type based on value range
                    if *val < 0 {
                        // Negative values
                        if *val >= -128 {
                            Ok(Type::Primitive(crate::ast::PrimitiveType::I8))
                        } else {
                            Ok(Type::Primitive(crate::ast::PrimitiveType::I16))
                        }
                    } else {
                        // Positive values
                        if *val <= 255 {
                            Ok(Type::Primitive(crate::ast::PrimitiveType::U8))
                        } else if *val <= 65535 {
                            Ok(Type::Primitive(crate::ast::PrimitiveType::U16))
                        } else {
                            // Value too large for any type
                            Err(SemaError::Custom {
                                message: format!("integer literal {} is too large (max 65535 for u16)", val),
                                span: expr.span,
                            })
                        }
                    }
                }
                crate::ast::Literal::Bool(_) => {
                    Ok(Type::Primitive(crate::ast::PrimitiveType::Bool))
                }
                crate::ast::Literal::String(_) => {
                    Ok(Type::String)
                }
                crate::ast::Literal::Array(elements) => {
                    if elements.is_empty() {
                        // Empty array - need type context to determine element type
                        // For now, default to [u8; 0]
                        return Ok(Type::Array(
                            Box::new(Type::Primitive(crate::ast::PrimitiveType::U8)),
                            0,
                        ));
                    }

                    // Infer element type from first element
                    let element_ty = self.check_expr(&elements[0])?;

                    // Check that all elements have the same type
                    for elem in &elements[1..] {
                        let elem_ty = self.check_expr(elem)?;
                        if elem_ty != element_ty {
                            return Err(SemaError::TypeMismatch {
                                expected: element_ty.display_name(),
                                found: elem_ty.display_name(),
                                span: elem.span,
                            });
                        }
                    }

                    Ok(Type::Array(Box::new(element_ty), elements.len()))
                }
                crate::ast::Literal::ArrayFill { value, count } => {
                    let element_ty = self.check_expr(value)?;
                    Ok(Type::Array(Box::new(element_ty), *count))
                }
            },
            Expr::Variable(name) => {
                let info = if let Some(info) = self.table.lookup(name) {
                    info.clone()
                } else {
                    return Err(SemaError::UndefinedSymbol {
                        name: name.clone(),
                        span: expr.span,
                    });
                };

                self.resolved_symbols.insert(expr.span, info.clone());

                // Mark variable as used (for unused variable/parameter warnings)
                self.used_variables.insert(name.clone());
                // Also track in all_used_symbols (for unused import warnings)
                self.all_used_symbols.insert(name.clone());

                Ok(info.ty)
            }
            Expr::Binary { left, op, right } => {
                let left_ty = self.check_expr(left)?;
                let right_ty = self.check_expr(right)?;

                use crate::ast::BinaryOp;

                // Special handling for pointer arithmetic
                match (&left_ty, op, &right_ty) {
                    // Pointer + Integer or Integer + Pointer
                    (Type::Pointer(..), BinaryOp::Add, Type::Primitive(_))
                    | (Type::Primitive(_), BinaryOp::Add, Type::Pointer(..)) => {
                        // Return the pointer type
                        if matches!(left_ty, Type::Pointer(..)) {
                            return Ok(left_ty);
                        } else {
                            return Ok(right_ty);
                        }
                    }
                    // Pointer - Integer
                    (Type::Pointer(..), BinaryOp::Sub, Type::Primitive(_)) => {
                        return Ok(left_ty);
                    }
                    // Pointer - Pointer (returns offset as integer)
                    (Type::Pointer(..), BinaryOp::Sub, Type::Pointer(..)) => {
                        return Ok(Type::Primitive(crate::ast::PrimitiveType::U16));
                    }
                    _ => {}
                }

                // BCD type validation
                if let (Type::Primitive(left_prim), Type::Primitive(right_prim)) = (&left_ty, &right_ty)
                    && (left_prim.is_bcd() || right_prim.is_bcd()) {
                        // Rule: Both operands must be same BCD type
                        if left_prim != right_prim {
                            return Err(SemaError::InvalidBinaryOp {
                                op: format!("{:?}", op),
                                left_ty: left_ty.display_name(),
                                right_ty: right_ty.display_name(),
                                span: expr.span,
                            });
                        }

                        // Only allow Add, Sub, comparisons on BCD
                        match op {
                            BinaryOp::Add | BinaryOp::Sub => {}  // Hardware supported
                            BinaryOp::Eq | BinaryOp::Ne | BinaryOp::Lt |
                            BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge => {}  // Comparisons work

                            BinaryOp::Mul | BinaryOp::Div | BinaryOp::Mod => {
                                return Err(SemaError::InvalidBinaryOp {
                                    op: format!("{:?} (not supported on BCD, convert to binary first)", op),
                                    left_ty: left_ty.display_name(),
                                    right_ty: right_ty.display_name(),
                                    span: expr.span,
                                });
                            }

                            BinaryOp::BitAnd | BinaryOp::BitOr | BinaryOp::BitXor |
                            BinaryOp::Shl | BinaryOp::Shr => {
                                return Err(SemaError::InvalidBinaryOp {
                                    op: format!("{:?} (bitwise ops not allowed on BCD)", op),
                                    left_ty: left_ty.display_name(),
                                    right_ty: right_ty.display_name(),
                                    span: expr.span,
                                });
                            }

                            _ => {
                                return Err(SemaError::InvalidBinaryOp {
                                    op: format!("{:?}", op),
                                    left_ty: left_ty.display_name(),
                                    right_ty: right_ty.display_name(),
                                    span: expr.span,
                                });
                            }
                        }
                    }

                // Special handling for shift operations: allow u16 to be shifted by u8
                // (shift amounts realistically never exceed 255)
                let types_compatible = if matches!(op, BinaryOp::Shl | BinaryOp::Shr) {
                    // Allow same-type shifts (u8 >> u8, u16 >> u16, etc.)
                    // Or allow larger type to be shifted by u8 (u16 >> u8)
                    left_ty == right_ty
                        || (matches!(left_ty, Type::Primitive(crate::ast::PrimitiveType::U16))
                            && matches!(right_ty, Type::Primitive(crate::ast::PrimitiveType::U8)))
                } else {
                    // For all other operations, types must match
                    left_ty == right_ty
                };

                if !types_compatible {
                    return Err(SemaError::InvalidBinaryOp {
                        op: format!("{:?}", op),
                        left_ty: left_ty.display_name(),
                        right_ty: right_ty.display_name(),
                        span: expr.span,
                    });
                }

                // Comparison and logical operators return Bool
                match op {
                    BinaryOp::Eq | BinaryOp::Ne | BinaryOp::Lt | BinaryOp::Le
                    | BinaryOp::Gt | BinaryOp::Ge | BinaryOp::And | BinaryOp::Or => {
                        Ok(Type::Primitive(crate::ast::PrimitiveType::Bool))
                    }
                    // Arithmetic and bitwise operators return the operand type
                    _ => Ok(left_ty),
                }
            }
            Expr::Call { function, args } => {
                // Mark function as used (for unused variable/import warnings)
                self.used_variables.insert(function.node.clone());
                self.all_used_symbols.insert(function.node.clone());

                // Verify function signature: check that it's a function and get param/return types
                let (param_types, ret_type) = if let Some(info) = self.table.lookup(&function.node)
                {
                    if let Type::Function(param_types, ret_type) = &info.ty {
                        (param_types.clone(), ret_type.clone())
                    } else {
                        return Err(SemaError::TypeMismatch {
                            expected: "function".to_string(),
                            found: info.ty.display_name(),
                            span: function.span,
                        });
                    }
                } else {
                    return Err(SemaError::UndefinedSymbol {
                        name: function.node.clone(),
                        span: function.span,
                    });
                };

                if args.len() != param_types.len() {
                    return Err(SemaError::ArityMismatch {
                        expected: param_types.len(),
                        found: args.len(),
                        span: expr.span,
                    });
                }
                for (arg, param_ty) in args.iter().zip(param_types.iter()) {
                    let arg_ty = self.check_expr(arg)?;
                    // Check if argument type can be implicitly converted to parameter type
                    if !arg_ty.is_implicitly_convertible_to(param_ty) {
                        return Err(SemaError::TypeMismatch {
                            expected: param_ty.display_name(),
                            found: arg_ty.display_name(),
                            span: arg.span,
                        });
                    }
                }
                Ok(*ret_type)
            }
            Expr::Unary { op, operand } => {
                let operand_ty = self.check_expr(operand)?;

                // Check type compatibility with the operator
                match op {
                    crate::ast::UnaryOp::Neg => {
                        // Negation works on numeric types
                        if operand_ty.is_primitive() {
                            Ok(operand_ty)
                        } else {
                            Err(SemaError::InvalidUnaryOp {
                                op: "-".to_string(),
                                operand_ty: operand_ty.display_name(),
                                span: expr.span,
                            })
                        }
                    }
                    crate::ast::UnaryOp::BitNot => {
                        // Bitwise NOT works on integer types
                        if operand_ty.is_primitive() {
                            Ok(operand_ty)
                        } else {
                            Err(SemaError::InvalidUnaryOp {
                                op: "~".to_string(),
                                operand_ty: operand_ty.display_name(),
                                span: expr.span,
                            })
                        }
                    }
                    crate::ast::UnaryOp::Not => {
                        // Logical NOT returns bool
                        Ok(Type::Primitive(crate::ast::PrimitiveType::Bool))
                    }
                    _ => Ok(operand_ty), // For other operators, preserve type
                }
            }
            Expr::Paren(inner) => self.check_expr(inner),
            Expr::Cast { expr: inner, target_type } => {
                // Check that the inner expression is valid
                self.check_expr(inner)?;
                // Return the target type
                self.resolve_type(&target_type.node)
            }
            Expr::StructInit { name, fields } => {
                // Look up the struct definition
                if !self.type_registry.structs.contains_key(&name.node) {
                    return Err(SemaError::UndefinedSymbol {
                        name: name.node.clone(),
                        span: name.span,
                    });
                }

                // Type check each field value
                // For now, just verify the struct exists and return its type
                for field in fields {
                    self.check_expr(&field.value)?;
                }

                Ok(Type::Named(name.node.clone()))
            }
            Expr::EnumVariant { enum_name, variant, data } => {
                // Look up the enum definition
                let enum_def = self.type_registry.get_enum(&enum_name.node)
                    .ok_or_else(|| SemaError::UndefinedSymbol {
                        name: enum_name.node.clone(),
                        span: enum_name.span,
                    })?;

                // Verify the variant exists
                let variant_info = enum_def.get_variant(&variant.node)
                    .ok_or_else(|| SemaError::Custom {
                        message: format!("variant '{}' not found in enum '{}'", variant.node, enum_name.node),
                        span: variant.span,
                    })?;

                // Type check the variant data
                use crate::ast::VariantData;
                use crate::sema::type_defs::VariantData as TypeDefVariantData;

                match (&variant_info.data, data) {
                    (TypeDefVariantData::Unit, VariantData::Unit) => {
                        // Unit variant - ok
                    }
                    (TypeDefVariantData::Tuple(field_types), VariantData::Tuple(values)) => {
                        // Type check each tuple field
                        if values.len() != field_types.len() {
                            return Err(SemaError::Custom {
                                message: format!(
                                    "variant '{}' expects {} fields, got {}",
                                    variant.node,
                                    field_types.len(),
                                    values.len()
                                ),
                                span: expr.span,
                            });
                        }

                        // Clone field types to avoid borrowing issues
                        let expected_types = field_types.clone();
                        for (value_expr, expected_ty) in values.iter().zip(expected_types.iter()) {
                            let value_ty = self.check_expr(value_expr)?;
                            if &value_ty != expected_ty {
                                return Err(SemaError::TypeMismatch {
                                    expected: expected_ty.display_name(),
                                    found: value_ty.display_name(),
                                    span: value_expr.span,
                                });
                            }
                        }
                    }
                    (TypeDefVariantData::Struct(field_infos), VariantData::Struct(field_inits)) => {
                        // Clone field infos to avoid borrowing issues
                        let field_info_vec = field_infos.clone();

                        // Type check struct variant fields
                        for field_init in field_inits {
                            let value_ty = self.check_expr(&field_init.value)?;

                            // Find the expected type for this field
                            let field_info = field_info_vec.iter()
                                .find(|f| f.name == field_init.name.node)
                                .ok_or_else(|| SemaError::FieldNotFound {
                                    struct_name: enum_name.node.clone(),
                                    field_name: field_init.name.node.clone(),
                                    span: field_init.name.span,
                                })?;

                            if value_ty != field_info.ty {
                                return Err(SemaError::TypeMismatch {
                                    expected: field_info.ty.display_name(),
                                    found: value_ty.display_name(),
                                    span: field_init.value.span,
                                });
                            }
                        }
                    }
                    _ => {
                        return Err(SemaError::Custom {
                            message: format!("variant data mismatch for '{}'", variant.node),
                            span: expr.span,
                        });
                    }
                }

                // Return the enum type
                Ok(Type::Named(enum_name.node.clone()))
            }
            Expr::Field { object, field } => {
                // Get the type of the object
                let object_ty = self.check_expr(object)?;

                // Extract struct name from the type
                let struct_name = match &object_ty {
                    Type::Named(name) => name,
                    _ => {
                        return Err(SemaError::TypeMismatch {
                            expected: "struct".to_string(),
                            found: object_ty.display_name(),
                            span: object.span,
                        });
                    }
                };

                // Look up the struct definition
                let struct_def = self.type_registry.get_struct(struct_name)
                    .ok_or_else(|| SemaError::Custom {
                        message: format!("struct '{}' not found", struct_name),
                        span: object.span,
                    })?;

                // Find the field and return its type
                let field_info = struct_def.get_field(&field.node)
                    .ok_or_else(|| SemaError::FieldNotFound {
                        struct_name: struct_name.clone(),
                        field_name: field.node.clone(),
                        span: field.span,
                    })?;

                Ok(field_info.ty.clone())
            }
            Expr::Index { object, index } => {
                // Type check the index expression (should be integer)
                let index_ty = self.check_expr(index)?;
                if !matches!(index_ty, Type::Primitive(crate::ast::PrimitiveType::U8 | crate::ast::PrimitiveType::I8)) {
                    return Err(SemaError::TypeMismatch {
                        expected: "u8 or i8".to_string(),
                        found: index_ty.display_name(),
                        span: index.span,
                    });
                }

                // Type check the object being indexed
                let object_ty = self.check_expr(object)?;

                // Extract element type from array or string type
                match &object_ty {
                    Type::Array(element_ty, _size) => {
                        // Return the element type
                        Ok((**element_ty).clone())
                    }
                    Type::String => {
                        // String indexing returns u8 (a single byte)
                        Ok(Type::Primitive(crate::ast::PrimitiveType::U8))
                    }
                    _ => {
                        Err(SemaError::TypeMismatch {
                            expected: "array or string".to_string(),
                            found: object_ty.display_name(),
                            span: object.span,
                        })
                    }
                }
            }
            Expr::SliceLen(slice_expr) => {
                // Verify the expression is actually a slice, array, or string
                let slice_ty = self.check_expr(slice_expr)?;

                // Check if it's a type that has a length
                match &slice_ty {
                    Type::Pointer(..) | Type::Array(_, _) | Type::String => {
                        // Slice/array/string length is always u16 on 6502 (our usize equivalent)
                        Ok(Type::Primitive(crate::ast::PrimitiveType::U16))
                    }
                    _ => Err(SemaError::TypeMismatch {
                        expected: "slice, array, or string".to_string(),
                        found: slice_ty.display_name(),
                        span: slice_expr.span,
                    }),
                }
            }

            Expr::U16Low(operand) => {
                let operand_ty = self.check_expr(operand)?;
                match &operand_ty {
                    Type::Primitive(crate::ast::PrimitiveType::U16)
                    | Type::Primitive(crate::ast::PrimitiveType::I16) => {
                        Ok(Type::Primitive(crate::ast::PrimitiveType::U8))
                    }
                    _ => Err(SemaError::TypeMismatch {
                        expected: "u16 or i16".to_string(),
                        found: operand_ty.display_name(),
                        span: operand.span,
                    }),
                }
            }

            Expr::U16High(operand) => {
                let operand_ty = self.check_expr(operand)?;
                match &operand_ty {
                    Type::Primitive(crate::ast::PrimitiveType::U16)
                    | Type::Primitive(crate::ast::PrimitiveType::I16) => {
                        Ok(Type::Primitive(crate::ast::PrimitiveType::U8))
                    }
                    _ => Err(SemaError::TypeMismatch {
                        expected: "u16 or i16".to_string(),
                        found: operand_ty.display_name(),
                        span: operand.span,
                    }),
                }
            }

            // CPU status flags - all return bool
            Expr::CpuFlagCarry | Expr::CpuFlagZero | Expr::CpuFlagOverflow | Expr::CpuFlagNegative => {
                Ok(Type::Primitive(crate::ast::PrimitiveType::Bool))
            }
        };

        // Store the resolved type for this expression so codegen can access it
        if let Ok(ref ty) = result_ty {
            self.resolved_types.insert(expr.span, ty.clone());
        }

        result_ty
    }

    fn resolve_type(&self, ty: &TypeExpr) -> Result<Type, SemaError> {
        match ty {
            TypeExpr::Primitive(p) => Ok(Type::Primitive(*p)),
            TypeExpr::Named(name) => {
                // Special case: "str" maps to Type::String
                if name == "str" {
                    return Ok(Type::String);
                }

                // Check if it's a known type (struct or enum)
                if self.type_registry.structs.contains_key(name) || self.type_registry.enums.contains_key(name) {
                    Ok(Type::Named(name.clone()))
                } else {
                    // For now, allow unknown named types
                    // They'll be caught later if they're actually used
                    Ok(Type::Named(name.clone()))
                }
            }
            TypeExpr::Pointer { pointee, mutable } => {
                let pointee_type = self.resolve_type(&pointee.node)?;
                Ok(Type::Pointer(Box::new(pointee_type), *mutable))
            }
            TypeExpr::Array { element, size } => {
                let element_type = self.resolve_type(&element.node)?;
                Ok(Type::Array(Box::new(element_type), *size))
            }
            TypeExpr::Slice { element, mutable: _ } => {
                // For now, treat slices as pointers to their element type
                // Full slice support would require length tracking
                let element_type = self.resolve_type(&element.node)?;
                Ok(Type::Pointer(Box::new(element_type), false))
            }
        }
    }

    fn resolve_function_type(&self, func: &Function) -> Result<Type, SemaError> {
        let mut param_types = Vec::new();
        for param in &func.params {
            param_types.push(self.resolve_type(&param.ty.node)?);
        }

        let return_type = if let Some(ret) = &func.return_type {
            self.resolve_type(&ret.node)?
        } else {
            Type::Void
        };

        Ok(Type::Function(param_types, Box::new(return_type)))
    }
}
