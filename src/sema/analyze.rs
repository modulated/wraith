//! Semantic Analysis Logic
//!
//! Traverses the AST to populate the symbol table and perform type checking.

use crate::ast::{Expr, Function, Item, PrimitiveType, SourceFile, Spanned, Stmt, TypeExpr};
use crate::codegen::memory_layout::MemoryLayout;
use crate::sema::const_eval::{eval_const_expr, eval_const_expr_with_env, ConstEnv, ConstValue};
use crate::sema::table::{SymbolInfo, SymbolKind, SymbolLocation, SymbolTable};
use crate::sema::type_defs::TypeRegistry;
use crate::sema::types::Type;
use crate::sema::{FunctionMetadata, ProgramInfo, SemaError};

use crate::ast::Span;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

pub struct SemanticAnalyzer {
    pub table: SymbolTable,
    pub errors: Vec<SemaError>,
    current_return_type: Option<Type>,
    resolved_symbols: HashMap<Span, SymbolInfo>,
    function_metadata: HashMap<String, FunctionMetadata>,
    folded_constants: HashMap<Span, ConstValue>,
    type_registry: TypeRegistry,
    base_path: Option<PathBuf>,
    imported_files: HashSet<PathBuf>,
    zp_allocator: ZeroPageAllocator,
    const_env: ConstEnv,
    loop_depth: usize,
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
            current_return_type: None,
            resolved_symbols: HashMap::new(),
            function_metadata: HashMap::new(),
            folded_constants: HashMap::new(),
            type_registry: TypeRegistry::new(),
            base_path: None,
            imported_files: HashSet::new(),
            zp_allocator: ZeroPageAllocator::new(),
            const_env: ConstEnv::new(),
            loop_depth: 0,
        }
    }

    pub fn with_base_path(base_path: PathBuf) -> Self {
        Self {
            table: SymbolTable::new(),
            errors: Vec::new(),
            current_return_type: None,
            resolved_symbols: HashMap::new(),
            function_metadata: HashMap::new(),
            folded_constants: HashMap::new(),
            type_registry: TypeRegistry::new(),
            base_path: Some(base_path),
            imported_files: HashSet::new(),
            zp_allocator: ZeroPageAllocator::new(),
            const_env: ConstEnv::new(),
            loop_depth: 0,
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

        Ok(ProgramInfo {
            table: self.table.clone(),
            resolved_symbols: self.resolved_symbols.clone(),
            function_metadata: self.function_metadata.clone(),
            folded_constants: self.folded_constants.clone(),
            type_registry: self.type_registry.clone(),
        })
    }

    fn register_item(&mut self, item: &Spanned<Item>) -> Result<(), SemaError> {
        match &item.node {
            Item::Function(func) => {
                let name = func.name.node.clone();

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

                // Check if function is inline
                let is_inline = func.is_inline || func.attributes.iter().any(|attr| {
                    matches!(attr, crate::ast::FnAttribute::Inline)
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
                    },
                );
            }
            Item::Static(stat) => {
                let name = stat.name.node.clone();

                // Check for duplicate static definition
                if self.table.defined_in_current_scope(&name) {
                    return Err(SemaError::DuplicateSymbol {
                        name: name.clone(),
                        span: stat.name.span,
                        previous_span: None,
                    });
                }

                // If it's a non-mutable static (const), evaluate it and add to const_env
                if !stat.mutable {
                    match eval_const_expr_with_env(&stat.init, &self.const_env) {
                        Ok(val) => {
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
                    ty: self.resolve_type(&stat.ty.node)?,
                    location: SymbolLocation::Absolute(0),
                    mutable: stat.mutable,
                };
                self.table.insert(name, info);
            }
            Item::Address(addr) => {
                let name = addr.name.node.clone();

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

                // Add the resolved address value to const_env for future references
                self.const_env.insert(name.clone(), ConstValue::Integer(address as i64));

                let info = SymbolInfo {
                    name: name.clone(),
                    kind: SymbolKind::Variable,
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

        // Import the requested symbols into our table
        for symbol_name in &import.symbols {
            let name = &symbol_name.node;
            if let Some(symbol) = imported_info.table.lookup(name) {
                self.table.insert(name.clone(), symbol.clone());
            } else {
                return Err(SemaError::ImportError {
                    path: import.path.node.clone(),
                    reason: format!("symbol '{}' not found in imported file", name),
                    span: symbol_name.span,
                });
            }
        }

        // Merge the imported files set
        self.imported_files.extend(imported_analyzer.imported_files);

        Ok(())
    }

    fn register_struct(&mut self, struct_def: &crate::ast::Struct) -> Result<(), SemaError> {
        use crate::sema::type_defs::{FieldInfo, StructDef};

        let name = struct_def.name.node.clone();
        let mut fields = Vec::new();
        let mut offset = 0;

        // Calculate field offsets
        for field in &struct_def.fields {
            let field_type = self.resolve_type(&field.ty.node)?;
            let size = self.type_size(&field_type);

            fields.push(FieldInfo {
                name: field.name.node.clone(),
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
        let mut variants = Vec::new();
        let mut next_tag: u8 = 0;

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
            self.table.enter_scope();

            // Set current return type for checking return statements
            let return_type = if let Some(ret) = &func.return_type {
                self.resolve_type(&ret.node)?
            } else {
                Type::Void
            };
            self.current_return_type = Some(return_type);

            // Register parameters
            // Allocate parameters in zero page using allocator
            for param in &func.params {
                let name = param.name.node.clone();

                // Check for duplicate parameter names
                if self.table.defined_in_current_scope(&name) {
                    return Err(SemaError::DuplicateSymbol {
                        name: name.clone(),
                        span: param.name.span,
                        previous_span: None,
                    });
                }

                let addr = self.zp_allocator.allocate()?;
                let location = SymbolLocation::ZeroPage(addr);
                let info = SymbolInfo {
                    name: name.clone(),
                    kind: SymbolKind::Variable,
                    ty: self.resolve_type(&param.ty.node)?,
                    location,
                    mutable: false,
                };
                self.table.insert(name, info.clone());
                // Add to resolved_symbols so codegen (especially inline asm) can find it
                self.resolved_symbols.insert(param.name.span, info);
            }

            // Analyze body
            self.analyze_stmt(&func.body)?;

            self.current_return_type = None;
            self.table.exit_scope();
        }
        Ok(())
    }

    fn analyze_stmt(&mut self, stmt: &Spanned<Stmt>) -> Result<(), SemaError> {
        match &stmt.node {
            Stmt::Block(stmts) => {
                self.table.enter_scope();
                for s in stmts {
                    self.analyze_stmt(s)?;
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
                let types_compatible = declared_ty == init_ty
                    || (matches!(declared_ty, Type::Primitive(crate::ast::PrimitiveType::U8))
                        && matches!(init_ty, Type::Primitive(crate::ast::PrimitiveType::Bool)));

                if !types_compatible {
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

                // Allocate in zero page using allocator
                let addr = self.zp_allocator.allocate()?;
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
            }
            Stmt::Assign { target, value } => {
                let target_ty = self.check_expr(target)?;
                let value_ty = self.check_expr(value)?;

                // Allow Bool to U8 assignment (booleans are 0/1 bytes in 6502)
                let types_compatible = target_ty == value_ty
                    || (matches!(target_ty, Type::Primitive(crate::ast::PrimitiveType::U8))
                        && matches!(value_ty, Type::Primitive(crate::ast::PrimitiveType::Bool)));

                if !types_compatible {
                    return Err(SemaError::TypeMismatch {
                        expected: target_ty.display_name(),
                        found: value_ty.display_name(),
                        span: value.span,
                    });
                }

                // Check mutability
                if let Expr::Variable(name) = &target.node {
                    if let Some(info) = self.table.lookup(name) {
                        if !info.mutable {
                            return Err(SemaError::ImmutableAssignment {
                                symbol: name.clone(),
                                span: target.span,
                            });
                        }
                    }
                }
            }
            Stmt::Return(expr) => {
                let expr_ty = if let Some(e) = expr {
                    self.check_expr(e)?
                } else {
                    Type::Void
                };

                if let Some(ret_ty) = &self.current_return_type {
                    if &expr_ty != ret_ty {
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
            _ => {}
        }
        Ok(())
    }

    fn check_expr(&mut self, expr: &Spanned<Expr>) -> Result<Type, SemaError> {
        // Try to fold the expression if it's constant
        if let Ok(const_val) = eval_const_expr(expr) {
            self.folded_constants.insert(expr.span, const_val);
        }

        match &expr.node {
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
                            return Err(SemaError::Custom {
                                message: format!("integer literal {} is too large (max 65535 for u16)", val),
                                span: expr.span,
                            });
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

                // Standard case: both operands must have the same type
                if left_ty != right_ty {
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
                // TODO: Check function signature
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
                    if &arg_ty != param_ty {
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
            _ => Ok(Type::Void), // TODO: Handle other expressions
        }
    }

    fn resolve_type(&self, ty: &TypeExpr) -> Result<Type, SemaError> {
        match ty {
            TypeExpr::Primitive(p) => Ok(Type::Primitive(*p)),
            TypeExpr::Named(name) => {
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
