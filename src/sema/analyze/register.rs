//! Registration Pass
//!
//! First pass of semantic analysis that registers all global items
//! (functions, statics, structs, enums, imports) before analyzing bodies.

use std::collections::HashSet;
use std::path::PathBuf;

use crate::ast::{EnumVariant, Import, Item, PrimitiveType, Spanned};
use crate::sema::const_eval::{eval_const_expr_with_env, ConstValue};
use crate::sema::table::{SymbolInfo, SymbolKind, SymbolLocation};
use crate::sema::type_defs::{EnumDef, FieldInfo, StructDef, VariantData, VariantInfo};
use crate::sema::types::Type;
use crate::sema::{FunctionMetadata, SemaError, Warning};

use super::SemanticAnalyzer;

/// Check if a name is all uppercase (allowing underscores and digits)
/// Used to enforce constant naming conventions
pub(super) fn is_uppercase_name(name: &str) -> bool {
    name.chars().all(|c| c.is_uppercase() || c == '_' || c.is_ascii_digit())
}

impl SemanticAnalyzer {
    pub(super) fn register_item(&mut self, item: &Spanned<Item>) -> Result<(), SemaError> {
        match &item.node {
            Item::Function(func) => {
                self.register_function(func)?;
            }
            Item::Static(stat) => {
                self.register_static(stat)?;
            }
            Item::Address(addr) => {
                self.register_address(addr)?;
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

    fn register_function(&mut self, func: &crate::ast::Function) -> Result<(), SemaError> {
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
            access_mode: None,
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

        // Calculate total bytes used by parameters
        let param_bytes_used: u8 = func.params.iter()
            .map(|p| {
                if let Ok(ty) = self.resolve_type(&p.ty.node) {
                    self.type_size(&ty) as u8
                } else {
                    1 // Default to 1 byte if type resolution fails
                }
            })
            .sum();

        // Warn if parameters exceed available space (64 bytes: $80-$BF)
        let param_space = self.memory_layout.param_space();
        if param_bytes_used > param_space {
            self.warnings.push(Warning::ParameterOverflow {
                function_name: name.clone(),
                bytes_used: param_bytes_used,
                bytes_available: param_space,
                span: func.name.span,
            });
        }

        self.function_metadata.insert(
            name.clone(),
            FunctionMetadata {
                org_address,
                section,
                is_inline,
                inline_body,
                inline_params,
                inline_param_symbols: None, // Will be populated in second pass
                has_tail_recursion: false,  // Will be populated by tail call analysis
                param_bytes_used,
                struct_param_locals: std::collections::HashMap::new(), // Will be populated during second pass
            },
        );

        // Track function declarations for unused function detection
        // Skip special functions that should never be warned about:
        // - reset (main/entry point)
        // - irq (interrupt handler)
        // - nmi (NMI handler)
        // - inline (may be called from other modules)
        let is_special = func.attributes.iter().any(|attr| {
            matches!(attr,
                crate::ast::FnAttribute::Reset |
                crate::ast::FnAttribute::Irq |
                crate::ast::FnAttribute::Nmi |
                crate::ast::FnAttribute::Inline
            )
        });

        if !is_special {
            self.declared_functions.push((name, func.name.span));
        }

        Ok(())
    }

    fn register_static(&mut self, stat: &crate::ast::Static) -> Result<(), SemaError> {
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

        // Warn if constant name is not all uppercase (per language spec)
        if !stat.mutable && !is_uppercase_name(&name) {
            self.warnings.push(Warning::NonUppercaseConstant {
                name: name.clone(),
                span: stat.name.span,
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
                            Type::Primitive(PrimitiveType::U8) => {
                                (0..=255).contains(&int_val)
                            }
                            Type::Primitive(PrimitiveType::I8) => {
                                (-128..=127).contains(&int_val)
                            }
                            Type::Primitive(PrimitiveType::U16) => {
                                (0..=65535).contains(&int_val)
                            }
                            Type::Primitive(PrimitiveType::I16) => {
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
            access_mode: None,
        };
        self.table.insert(name, info);

        Ok(())
    }

    fn register_address(&mut self, addr: &crate::ast::AddressDecl) -> Result<(), SemaError> {
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

        // Check for overlap with compiler-managed memory sections
        for section in &self.memory_config.sections {
            if section.contains(address) {
                self.warnings.push(Warning::AddressOverlap {
                    name: name.clone(),
                    address,
                    section_name: section.name.clone(),
                    section_start: section.start,
                    section_end: section.end,
                    span: addr.address.span,
                });
                break; // Only warn once per address
            }
        }

        let info = SymbolInfo {
            name: name.clone(),
            kind: SymbolKind::Address,
            ty: Type::Primitive(PrimitiveType::U8),
            location: SymbolLocation::Absolute(address),
            // Write and ReadWrite can be written to; Read cannot
            mutable: matches!(addr.access, crate::ast::AccessMode::Write | crate::ast::AccessMode::ReadWrite),
            access_mode: Some(addr.access),
        };
        self.table.insert(name, info);

        Ok(())
    }

    pub(super) fn process_import(&mut self, import: &Import) -> Result<(), SemaError> {
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
            if matches!(symbol.kind, SymbolKind::Constant | SymbolKind::Address)
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

    pub(super) fn register_struct(&mut self, struct_def: &crate::ast::Struct) -> Result<(), SemaError> {
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

            // Check for invalid addr usage in struct fields
            if matches!(field_type, Type::Primitive(PrimitiveType::Addr)) {
                return Err(SemaError::InvalidAddrUsage {
                    context: "struct fields".to_string(),
                    span: field.ty.span,
                });
            }

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
                access_mode: None,
            },
        );

        Ok(())
    }

    pub(super) fn register_enum(&mut self, enum_def: &crate::ast::Enum) -> Result<(), SemaError> {
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
                    let mut types = Vec::new();
                    for ty in field_types {
                        let resolved_ty = self.resolve_type(&ty.node)?;

                        // Check for invalid addr usage in enum tuple variant fields
                        if matches!(resolved_ty, Type::Primitive(PrimitiveType::Addr)) {
                            return Err(SemaError::InvalidAddrUsage {
                                context: "enum variant fields".to_string(),
                                span: ty.span,
                            });
                        }

                        types.push(resolved_ty);
                    }
                    let tag = next_tag;
                    next_tag += 1;
                    (var_name.node.clone(), VariantData::Tuple(types), tag)
                }
                EnumVariant::Struct { name: var_name, fields } => {
                    let mut variant_fields = Vec::new();
                    let mut field_offset = 0;

                    for field in fields {
                        let field_type = self.resolve_type(&field.ty.node)?;

                        // Check for invalid addr usage in enum struct variant fields
                        if matches!(field_type, Type::Primitive(PrimitiveType::Addr)) {
                            return Err(SemaError::InvalidAddrUsage {
                                context: "enum variant fields".to_string(),
                                span: field.ty.span,
                            });
                        }

                        let size = self.type_size(&field_type);

                        variant_fields.push(FieldInfo {
                            name: field.name.node.clone(),
                            ty: field_type,
                            offset: field_offset,
                        });

                        field_offset += size;
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
                access_mode: None,
            },
        );

        Ok(())
    }
}
