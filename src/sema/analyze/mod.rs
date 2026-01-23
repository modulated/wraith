//! Semantic Analysis Logic
//!
//! Traverses the AST to populate the symbol table and perform type checking.

mod expr;
mod register;
mod stmt;
mod tail_call;
mod unused;
mod zp_alloc;

use crate::ast::{Function, Item, PrimitiveType, SourceFile, Spanned, TypeExpr};
use crate::codegen::memory_layout::MemoryLayout;
use crate::sema::const_eval::ConstEnv;
use crate::sema::table::{SymbolInfo, SymbolKind, SymbolLocation, SymbolTable};
use crate::sema::type_defs::TypeRegistry;
use crate::sema::types::Type;
use crate::sema::{FunctionMetadata, ProgramInfo, SemaError, Warning};

use crate::ast::Span;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use zp_alloc::ZeroPageAllocator;

pub struct SemanticAnalyzer {
    pub table: SymbolTable,
    pub errors: Vec<SemaError>,
    pub warnings: Vec<Warning>,
    pub(super) current_return_type: Option<Type>,
    pub(super) resolved_symbols: HashMap<Span, SymbolInfo>,
    pub(super) function_metadata: HashMap<String, FunctionMetadata>,
    pub(super) folded_constants: HashMap<Span, crate::sema::const_eval::ConstValue>,
    pub(super) resolved_types: HashMap<Span, Type>,
    pub(super) type_registry: TypeRegistry,
    pub(super) imported_items: Vec<Spanned<Item>>,
    pub(super) base_path: Option<PathBuf>,
    pub(super) imported_files: HashSet<PathBuf>,
    zp_allocator: ZeroPageAllocator,
    pub(super) const_env: ConstEnv,
    pub(super) loop_depth: usize,
    /// Track variable usage for unused variable warnings (per-function, cleared after each function)
    pub(super) used_variables: HashSet<String>,
    /// Track ALL symbol usage across entire file (never cleared, for import checking)
    pub(super) all_used_symbols: HashSet<String>,
    /// Track declared variables in current scope (name -> span) for unused variable detection
    pub(super) declared_variables: Vec<(String, Span)>,
    /// Track function parameters (name -> span) for unused parameter detection
    pub(super) declared_parameters: Vec<(String, Span)>,
    /// Track imported symbols (name -> span) for unused import detection
    pub(super) imported_symbols: Vec<(String, Span)>,
    /// Track declared functions (name -> span) for unused function detection
    pub(super) declared_functions: Vec<(String, Span)>,
    /// Track function calls for unused function detection
    pub(super) called_functions: HashSet<String>,
    /// Track unreachable statements for dead code elimination
    pub(super) unreachable_stmts: HashSet<Span>,
    /// Memory layout configuration for parameter space checking
    pub(super) memory_layout: MemoryLayout,
    /// True when checking an assignment target (not reading a value)
    pub(super) checking_assignment_target: bool,
    /// Expected type for type inference (e.g., for anonymous struct literals)
    pub(super) expected_type: Option<Type>,
    /// Map from span to resolved struct name for anonymous struct inits
    pub(super) resolved_struct_names: HashMap<Span, String>,
    /// Memory configuration from wraith.toml for overlap checking
    pub(super) memory_config: crate::config::MemoryConfig,
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
            declared_functions: Vec::new(),
            called_functions: HashSet::new(),
            unreachable_stmts: HashSet::new(),
            memory_layout: MemoryLayout::new(),
            checking_assignment_target: false,
            expected_type: None,
            resolved_struct_names: HashMap::new(),
            memory_config: crate::config::MemoryConfig::load_or_default(),
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
            declared_functions: Vec::new(),
            called_functions: HashSet::new(),
            unreachable_stmts: HashSet::new(),
            memory_layout: MemoryLayout::new(),
            checking_assignment_target: false,
            expected_type: None,
            resolved_struct_names: HashMap::new(),
            memory_config: crate::config::MemoryConfig::load_or_default(),
        }
    }

    /// Get the standard library path
    /// Checks WRAITH_STD_PATH environment variable, falls back to ./std
    pub(super) fn get_std_lib_path() -> PathBuf {
        std::env::var("WRAITH_STD_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("std"))
    }

    /// Compute the size of a type, looking up named types in the registry
    pub(super) fn type_size(&self, ty: &Type) -> usize {
        match ty {
            Type::Primitive(prim) => prim.size_bytes(),
            Type::Array(element_ty, len) => self.type_size(element_ty) * len,
            Type::Slice(_) => 4, // Fat pointer: 2 bytes base address + 2 bytes length
            Type::String => 2,   // String is represented as a pointer
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

        // Check for unused imports and functions after all analysis is complete
        self.check_unused_imports();
        self.check_unused_functions();

        // Analyze tail calls after all other analysis is complete
        let tail_call_info = self.analyze_tail_calls(source);

        Ok(ProgramInfo {
            table: self.table.clone(),
            resolved_symbols: self.resolved_symbols.clone(),
            function_metadata: self.function_metadata.clone(),
            folded_constants: self.folded_constants.clone(),
            type_registry: self.type_registry.clone(),
            resolved_types: self.resolved_types.clone(),
            imported_items: self.imported_items.clone(),
            warnings: self.warnings.clone(),
            unreachable_stmts: self.unreachable_stmts.clone(),
            tail_call_info,
            resolved_struct_names: self.resolved_struct_names.clone(),
        })
    }

    fn analyze_item(&mut self, item: &Spanned<Item>) -> Result<(), SemaError> {
        if let Item::Function(func) = &item.node {
            let func_name = func.name.node.clone();

            // Check if this is an inline function
            let is_inline = func
                .attributes
                .iter()
                .any(|attr| matches!(attr, crate::ast::FnAttribute::Inline));

            self.table.enter_scope();

            // Set current return type for checking return statements
            let return_type = if let Some(ret) = &func.return_type {
                let ty = self.resolve_type(&ret.node)?;

                // Check for invalid addr usage in function return types
                if matches!(ty, Type::Primitive(PrimitiveType::Addr)) {
                    return Err(SemaError::InvalidAddrUsage {
                        context: "function return types".to_string(),
                        span: ret.span,
                    });
                }

                ty
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
            let mut struct_param_locals: std::collections::HashMap<String, u8> =
                std::collections::HashMap::new();

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

                // Check for invalid addr usage in function parameters
                if matches!(param_type, Type::Primitive(PrimitiveType::Addr)) {
                    return Err(SemaError::InvalidAddrUsage {
                        context: "function parameters".to_string(),
                        span: param.ty.span,
                    });
                }

                // Struct and enum parameters are passed by reference (2-byte pointer)
                // Other types are passed by value
                let is_struct_param = matches!(param_type, Type::Named(_))
                    && self
                        .type_registry
                        .get_struct(if let Type::Named(n) = &param_type {
                            n
                        } else {
                            ""
                        })
                        .is_some();

                // Enum parameters are also passed as 2-byte pointers
                let is_enum_param = matches!(param_type, Type::Named(_))
                    && self
                        .type_registry
                        .get_enum(if let Type::Named(n) = &param_type {
                            n
                        } else {
                            ""
                        })
                        .is_some();

                let param_size = if is_struct_param || is_enum_param {
                    2 // Pointer size for pass-by-reference
                } else {
                    param_type.size()
                };

                // For struct parameters, allocate local storage to copy the pointer
                // This prevents nested calls from clobbering the struct pointer in param space
                if is_struct_param {
                    let local_addr = self.zp_allocator.allocate_range(2)?;
                    struct_param_locals.insert(name.clone(), local_addr);
                }

                let info = SymbolInfo {
                    name: name.clone(),
                    kind: SymbolKind::Variable,
                    ty: param_type,
                    location,
                    mutable: false,
                    access_mode: None,
                    is_pub: false, // Function parameters are never public
                };
                self.table.insert(name.clone(), info.clone());
                // Add to resolved_symbols so codegen (especially inline asm) can find it
                self.resolved_symbols.insert(param.name.span, info.clone());

                // Track parameter for unused parameter detection
                self.declared_parameters.push((name, param.name.span));

                // Advance byte offset by parameter size (16-bit params take 2 bytes)
                byte_offset += param_size as u8;
            }

            // Store struct param locals mapping in function metadata
            if !struct_param_locals.is_empty()
                && let Some(metadata) = self.function_metadata.get_mut(&func_name)
            {
                metadata.struct_param_locals = struct_param_locals;
            }

            // Analyze body
            self.analyze_stmt(&func.body)?;

            // For inline functions, capture all symbols that were added during body analysis
            // This includes both parameter definitions and all references to them
            if is_inline && let Some(before) = resolved_before {
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

    pub(super) fn resolve_type(&self, ty: &TypeExpr) -> Result<Type, SemaError> {
        match ty {
            TypeExpr::Primitive(p) => Ok(Type::Primitive(*p)),
            TypeExpr::Named(name) => {
                // Special case: "str" maps to Type::String
                if name == "str" {
                    return Ok(Type::String);
                }

                // Check if it's a known type (struct or enum)
                if self.type_registry.structs.contains_key(name)
                    || self.type_registry.enums.contains_key(name)
                {
                    Ok(Type::Named(name.clone()))
                } else {
                    // For now, allow unknown named types
                    // They'll be caught later if they're actually used
                    Ok(Type::Named(name.clone()))
                }
            }
            TypeExpr::Array { element, size } => {
                let element_type = self.resolve_type(&element.node)?;
                Ok(Type::Array(Box::new(element_type), *size))
            }
            TypeExpr::Slice {
                element,
                mutable: _,
            } => {
                // Slice is a fat pointer with base address and length
                let element_type = self.resolve_type(&element.node)?;
                Ok(Type::Slice(Box::new(element_type)))
            }
        }
    }

    pub(super) fn resolve_function_type(&self, func: &Function) -> Result<Type, SemaError> {
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
