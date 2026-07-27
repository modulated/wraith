//! Registration Pass
//!
//! First pass of semantic analysis that registers all global items
//! (functions, statics, structs, enums, imports) before analyzing bodies.

use rustc_hash::FxHashSet as HashSet;
use std::path::PathBuf;

use crate::ast::{EnumVariant, Import, Item, PrimitiveType, Span, Spanned};
use crate::sema::const_eval::{ConstValue, eval_const_expr_with_env};
use crate::sema::table::{SymbolInfo, SymbolKind, SymbolLocation};
use crate::sema::type_defs::{EnumDef, FieldInfo, StructDef, VariantData, VariantInfo};
use crate::sema::types::Type;
use crate::sema::{FunctionMetadata, SemaError, Warning};

use super::SemanticAnalyzer;

/// The tag an implicit enum variant takes, or an error when the range is spent.
///
/// Discriminants are one byte: they are what a value carries at runtime and what
/// the match jump table indexes by. Following an explicit `= 0xFF` with another
/// variant leaves nothing to assign, which used to wrap to 0 and collide.
fn next_tag_or_err(
    next_tag: u16,
    enum_name: &str,
    variant: &Spanned<String>,
) -> Result<u8, SemaError> {
    u8::try_from(next_tag).map_err(|_| SemaError::Custom {
        message: format!(
            "enum '{}' has run out of discriminants at '{}': the previous variant used 255",
            enum_name, variant.node
        ),
        span: variant.span,
    })
}

/// Check if a name is all uppercase (allowing underscores and digits)
/// Used to enforce constant naming conventions
pub(super) fn is_uppercase_name(name: &str) -> bool {
    name.chars()
        .all(|c| c.is_uppercase() || c == '_' || c.is_ascii_digit())
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
        let is_inline = func
            .attributes
            .iter()
            .any(|attr| matches!(attr, crate::ast::FnAttribute::Inline));

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
            is_pub: func.is_pub,
            containing_function: None, // Functions are global
            is_param: false,
        };
        self.function_signatures
            .insert(name.clone(), info.ty.clone());
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
        let param_bytes_used: u8 = func
            .params
            .iter()
            .map(|p| {
                if let Ok(ty) = self.resolve_type(&p.ty.node) {
                    self.type_size(&ty) as u8
                } else {
                    1 // Default to 1 byte if type resolution fails
                }
            })
            .sum();

        // Parameters now live in the function frame; overall zero-page capacity is
        // enforced by finalize_frames (SemaError::FrameRegionOverflow), so the old
        // per-function $80-$BF parameter-space warning no longer applies.

        let is_interrupt_handler = func.attributes.iter().any(|attr| {
            matches!(
                attr,
                crate::ast::FnAttribute::Interrupt
                    | crate::ast::FnAttribute::Nmi
                    | crate::ast::FnAttribute::Irq
            )
        });

        self.function_metadata.insert(
            name.clone(),
            FunctionMetadata {
                org_address,
                section,
                is_inline,
                is_interrupt_handler,
                inline_body,
                inline_params,
                inline_param_symbols: None, // Will be populated in second pass
                has_tail_recursion: false,  // Will be populated by tail call analysis
                param_bytes_used,
            },
        );

        // Track function declarations for unused function detection
        // Skip special functions that should never be warned about:
        // - reset (main/entry point)
        // - irq (interrupt handler)
        // - nmi (NMI handler)
        // - inline (may be called from other modules)
        let is_special = func.attributes.iter().any(|attr| {
            matches!(
                attr,
                crate::ast::FnAttribute::Reset
                    | crate::ast::FnAttribute::Irq
                    | crate::ast::FnAttribute::Nmi
                    | crate::ast::FnAttribute::Inline
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
                            Type::Primitive(PrimitiveType::U8) => (0..=255).contains(&int_val),
                            Type::Primitive(PrimitiveType::I8) => (-128..=127).contains(&int_val),
                            Type::Primitive(PrimitiveType::U16) => (0..=65535).contains(&int_val),
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

        // A mutable `static` needs writable storage, so give it a real address in
        // the BSS (RAM) section. Immutable consts stay at Absolute(0): they are
        // emitted as ROM data referenced by label, not by a computed address.
        let (kind, location) = if stat.mutable {
            let size = self.type_size(&declared_ty).max(1) as u16;
            let addr = self.bss_alloc(size, stat.name.span)?;
            // Record the startup value so the reset handler can write it: RAM
            // holds garbage at power-on and cannot be pre-loaded from ROM.
            let bytes = self.static_init_bytes(&stat.init, &declared_ty, size as usize)?;
            // A function named in a static's initializer (e.g. a device vtable)
            // has its address taken, even though the initializer is never
            // checked as an expression. Record it so codegen gives that function
            // the indirect-arg staging prologue; otherwise an indirect call
            // through the table would find its parameters unset.
            for b in &bytes {
                if let crate::sema::InitByte::FnLow(f) = b {
                    self.address_taken_functions.insert(f.clone());
                    // Installing a function in a vtable is a use of it, even if
                    // it is never named at a call site; otherwise every driver
                    // entry point would be reported as unused.
                    self.called_functions.insert(f.clone());
                    self.all_used_symbols.insert(f.clone());
                }
            }
            // `static P: &T = &OTHER;` is a use of OTHER, even though a
            // static's initializer is never checked as an expression. Record
            // the edge so dead-code elimination keeps OTHER's own startup
            // write; without it the pointer would be correct and the bytes it
            // names would never be initialised.
            if let crate::ast::Expr::Unary {
                op: crate::ast::UnaryOp::AddrOf,
                operand,
            } = &stat.init.node
            {
                let mut target = &**operand;
                while let crate::ast::Expr::Paren(inner) = &target.node {
                    target = inner;
                }
                if let crate::ast::Expr::Variable(referent) = &target.node {
                    self.symbol_refs
                        .entry(Some(name.clone()))
                        .or_default()
                        .insert(referent.clone());
                    self.all_used_symbols.insert(referent.clone());
                }
            }
            self.static_inits.push(crate::sema::StaticInit {
                name: name.clone(),
                addr,
                bytes,
            });
            (SymbolKind::Variable, SymbolLocation::Absolute(addr))
        } else {
            (SymbolKind::Constant, SymbolLocation::Absolute(0))
        };

        let info = SymbolInfo {
            name: name.clone(),
            kind,
            ty: declared_ty,
            location,
            mutable: stat.mutable,
            access_mode: None,
            is_pub: stat.is_pub,
            containing_function: None, // Globals are not scoped to a function
            is_param: false,
        };
        self.table.insert(name, info);

        Ok(())
    }

    /// Flatten a mutable static's initializer into the exact bytes to write at
    /// startup. Integers are little-endian across the type's width, arrays and
    /// fills expand element-wise. Anything not constant-evaluable becomes zeros
    /// (BSS semantics), which is also the common `= 0` / `[0; N]` case.
    fn static_init_bytes(
        &self,
        init: &Spanned<crate::ast::Expr>,
        ty: &Type,
        size: usize,
    ) -> Result<Vec<crate::sema::InitByte>, SemaError> {
        use crate::ast::{Expr, Literal};
        use crate::sema::InitByte;

        let elem_width = match ty {
            Type::Array(elem, _) => self.type_size(elem).max(1),
            _ => size,
        };
        let push_int = |out: &mut Vec<InitByte>, v: i64, width: usize| {
            for i in 0..width {
                out.push(InitByte::Byte(((v >> (i * 8)) & 0xFF) as u8));
            }
        };
        // A bare name whose symbol is a function contributes that function's
        // address; the label is resolved by the assembler.
        let fn_ref = |out: &mut Vec<InitByte>, e: &Spanned<Expr>| -> bool {
            if let Expr::Variable(n) = &e.node
                && self
                    .table
                    .lookup(n)
                    .is_some_and(|s| matches!(s.ty, Type::Function(..)))
            {
                out.push(InitByte::FnLow(n.clone()));
                out.push(InitByte::FnHigh(n.clone()));
                return true;
            }
            false
        };

        let mut bytes: Vec<InitByte> = Vec::with_capacity(size);
        match &init.node {
            Expr::Literal(Literal::Integer(v)) => push_int(&mut bytes, *v, size),
            Expr::Literal(Literal::Bool(b)) => bytes.push(InitByte::Byte(*b as u8)),
            Expr::Literal(Literal::Array(elems)) => {
                for e in elems {
                    match &e.node {
                        Expr::Literal(Literal::Integer(v)) => push_int(&mut bytes, *v, elem_width),
                        Expr::Literal(Literal::Bool(b)) => bytes.push(InitByte::Byte(*b as u8)),
                        _ if fn_ref(&mut bytes, e) => {}
                        _ => break,
                    }
                }
            }
            Expr::Literal(Literal::ArrayFill { value, count }) => {
                if let Expr::Literal(Literal::Integer(v)) = &value.node {
                    for _ in 0..*count {
                        push_int(&mut bytes, *v, elem_width);
                    }
                }
            }
            // Struct literal: lay fields out in declaration order. Function
            // fields become label references (a device vtable).
            Expr::StructInit { name, fields } => {
                if let Some(def) = self.type_registry.get_struct(&name.node) {
                    for f in &def.fields {
                        let Some(init_f) = fields.iter().find(|x| x.name.node == f.name) else {
                            break;
                        };
                        let w = self.type_size(&f.ty).max(1);
                        match &init_f.value.node {
                            Expr::Literal(Literal::Integer(v)) => push_int(&mut bytes, *v, w),
                            Expr::Literal(Literal::Bool(b)) => bytes.push(InitByte::Byte(*b as u8)),
                            _ if fn_ref(&mut bytes, &init_f.value) => {}
                            _ => break,
                        }
                    }
                }
            }
            // `static P: &T = &OTHER;` — the address of another static. A
            // static's BSS address is known during analysis, so this needs no
            // label indirection, unlike a function reference.
            Expr::Unary {
                op: crate::ast::UnaryOp::AddrOf,
                operand,
            } => {
                let addr = self.static_address_for_init(operand)?;
                push_int(&mut bytes, addr as i64, 2);
            }
            _ if fn_ref(&mut bytes, init) => {}
            // Fall back to constant folding (e.g. `1 + 2`, a const reference).
            _ => {
                if let Ok(val) = eval_const_expr_with_env(init, &self.const_env)
                    && let Some(v) = val.as_integer()
                {
                    push_int(&mut bytes, v, size);
                }
            }
        }
        bytes.resize(size, InitByte::Byte(0));
        Ok(bytes)
    }

    /// Resolve `&NAME` in a static's initializer to a fixed address.
    ///
    /// Statics are allocated in declaration order, so a name that has not been
    /// allocated yet is not merely unknown — it is a *forward reference*, and
    /// silently falling back to zeros would leave a null pointer that only
    /// misbehaves at run time. Say which case it is.
    fn static_address_for_init(
        &self,
        operand: &Spanned<crate::ast::Expr>,
    ) -> Result<u16, SemaError> {
        use crate::ast::Expr;

        let operand = match &operand.node {
            Expr::Paren(inner) => inner,
            _ => operand,
        };
        let Expr::Variable(name) = &operand.node else {
            return Err(SemaError::Custom {
                message: "a static's initializer can only take the address of another static"
                    .to_string(),
                span: operand.span,
            });
        };

        match self.table.lookup(name) {
            // An immutable const lives in ROM and is referenced by label, so
            // `Absolute(0)` is a sentinel rather than an address — the same
            // reason `&CONST` is rejected in ordinary code.
            Some(sym) if sym.kind == SymbolKind::Constant => Err(SemaError::Custom {
                message: format!(
                    "cannot take the address of the constant '{}'; constants live in ROM \
                     and are referenced by label, not by address",
                    name
                ),
                span: operand.span,
            }),
            Some(sym) => match sym.location {
                SymbolLocation::Absolute(addr) => Ok(addr),
                _ => Err(SemaError::Custom {
                    message: format!(
                        "cannot take the address of '{}' here; only a static has a fixed \
                         address at startup",
                        name
                    ),
                    span: operand.span,
                }),
            },
            None => Err(SemaError::Custom {
                message: format!(
                    "'{}' is not declared yet; statics are laid out in declaration order, \
                     so a static's initializer can only name one declared above it",
                    name
                ),
                span: operand.span,
            }),
        }
    }

    /// Reserve `size` bytes of BSS (RAM) for a mutable global, returning its
    /// address. Statics are allocated in declaration order from the start of the
    /// BSS section; unlike function frames they are never reused or colored,
    /// because they are live for the whole program and shared with interrupts.
    fn bss_alloc(&mut self, size: u16, span: crate::ast::Span) -> Result<u16, SemaError> {
        // Fall back to a built-in RAM range when the project's wraith.toml
        // predates the BSS section, so existing configs keep working. The
        // default sits above the zero page, the hardware stack, and the
        // compiler's software-stack page ($0200).
        const DEFAULT_BSS: (u16, u16) = (0x0400, 0x07FF);
        let (start, end) = self
            .memory_config
            .get_section("BSS")
            .map(|s| (s.start, s.end))
            .unwrap_or(DEFAULT_BSS);
        let base = self.bss_cursor.unwrap_or(start);
        let last = base as u32 + size as u32 - 1;
        if last > end as u32 {
            return Err(SemaError::Custom {
                message: format!(
                    "BSS section overflow: mutable globals exceed ${:04X}-${:04X}",
                    start, end
                ),
                span,
            });
        }
        self.bss_cursor = Some(base + size);
        Ok(base)
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
        self.const_env
            .insert(name.clone(), ConstValue::Integer(address as i64));

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
            mutable: matches!(
                addr.access,
                crate::ast::AccessMode::Write | crate::ast::AccessMode::ReadWrite
            ),
            access_mode: Some(addr.access),
            is_pub: addr.is_pub,
            containing_function: None, // Addresses are global
            is_param: false,
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
        let source = std::fs::read_to_string(&import_path).map_err(|e| SemaError::ImportError {
            path: import.path.node.clone(),
            // The formatter already prefixes "failed to import '<path>':".
            reason: e.to_string(),
            span: import.path.span,
        })?;

        // Everything that can go wrong from here on is a fault *in the imported
        // module*, and its spans index that module's text — which the driver
        // never reads. Render each against the source we are holding right now,
        // and carry the finished diagnostic upward rather than a `{:?}` dump of
        // the error struct.
        let module_path = import_path.to_string_lossy().to_string();
        let module_file = crate::ast::file_id(&module_path);

        let tokens = crate::lex(&source).map_err(|e| {
            let where_ = crate::ast::Span::in_file(e.span.start, e.span.end, module_file);
            SemaError::InModule {
                path: module_path.clone(),
                rendered: format!(
                    "error: {}\n{}",
                    e.message,
                    where_.format_error_context_of(
                        &source,
                        Some(&module_path),
                        &e.message,
                        module_file,
                    )
                ),
                trail: Vec::new(),
                import_span: import.path.span,
            }
        })?;

        // Stamp the module's spans with an id derived from its path, so its
        // symbol tables can be merged into ours without offsets from two files
        // colliding on the same map key.
        let ast = crate::Parser::parse_in_file(&tokens, module_file).map_err(|e| {
            SemaError::InModule {
                path: module_path.clone(),
                rendered: e.format_with_source_of_file(&source, Some(&module_path), module_file),
                trail: Vec::new(),
                import_span: import.path.span,
            }
        })?;

        // Analyze the imported file WITHOUT finalizing frames: frame bases must
        // be assigned once, by the root analyzer, over the merged program. The
        // child leaves its symbols at FrameOffset and exposes its call graph and
        // frame sizes, which we merge below so the root's finalize_frames colors
        // imported functions together with this module (fixing the historical
        // collision where a child allocator also started at $40).
        let mut imported_analyzer = SemanticAnalyzer::with_base_path(import_path.clone());
        imported_analyzer.imported_files = self.imported_files.clone();
        imported_analyzer
            .analyze_module(&ast)
            .map_err(|e| match e {
                // The failure is deeper in the chain. Its diagnostic is already
                // rendered; what this level can add is the hop *through* this
                // module, because we are the ones holding its source.
                SemaError::InModule {
                    path,
                    rendered,
                    mut trail,
                    import_span: inner_span,
                } => {
                    trail.push(format!(
                        "note: imported by '{}'\n{}",
                        module_path,
                        inner_span.format_error_context_of(
                            &source,
                            Some(&module_path),
                            "",
                            module_file,
                        )
                    ));
                    SemaError::InModule {
                        path,
                        rendered,
                        trail,
                        import_span: import.path.span,
                    }
                }
                e => SemaError::InModule {
                    path: module_path.clone(),
                    rendered: e.format_with_source_of_file(
                        &source,
                        Some(&module_path),
                        module_file,
                    ),
                    trail: Vec::new(),
                    import_span: import.path.span,
                },
            })?;

        // Collect all items from the imported file for codegen
        // We collect ALL items, not just the imported symbols, because functions
        // may depend on other functions in the same module
        self.imported_items.extend(ast.items.clone());

        // Also collect items from transitively imported modules
        self.imported_items
            .extend(imported_analyzer.imported_items.clone());

        // Work out which names this import brings in. A glob takes every `pub`
        // item in the module (private ones stay private, exactly as a named
        // import of one would be rejected below); named imports take their list.
        // Both forms can appear together, so dedupe.
        let mut requested: Vec<(String, Span)> = Vec::new();
        if let Some(glob_span) = import.glob {
            for (name, symbol) in imported_analyzer.table.module_symbols() {
                if symbol.is_pub {
                    requested.push((name.clone(), glob_span));
                }
            }
        }
        for symbol_name in &import.symbols {
            if !requested.iter().any(|(n, _)| n == &symbol_name.node) {
                requested.push((symbol_name.node.clone(), symbol_name.span));
            }
        }

        // Import the requested symbols into our table
        for (name, name_span) in &requested {
            let name_span = *name_span;
            if let Some(symbol) = imported_analyzer.table.lookup(name) {
                // Check if the symbol is public
                if !symbol.is_pub {
                    return Err(SemaError::ImportError {
                        path: import.path.node.clone(),
                        reason: format!("symbol '{}' is private and cannot be imported", name),
                        span: name_span,
                    });
                }

                self.table.insert(name.clone(), symbol.clone());

                // Track imported symbol for unused import detection. A glob
                // deliberately brings in names the file may not use, so it is
                // not reported: that is what the wildcard asked for, and the
                // unused ones are dropped from the output anyway.
                if import.glob.is_none() {
                    self.imported_symbols.push((name.clone(), name_span));
                }

                // Also import function metadata if this is a function
                if let Some(metadata) = imported_analyzer.function_metadata.get(name) {
                    self.function_metadata
                        .insert(name.clone(), metadata.clone());
                }

                // Also import type definitions (struct/enum) if this is a type
                if symbol.kind == SymbolKind::Type {
                    if let Some(struct_info) = imported_analyzer.type_registry.get_struct(name) {
                        self.type_registry.add_struct(struct_info.clone());
                    } else if let Some(enum_info) = imported_analyzer.type_registry.get_enum(name) {
                        self.type_registry.add_enum(enum_info.clone());
                    }
                }
            } else {
                return Err(SemaError::ImportError {
                    path: import.path.node.clone(),
                    reason: format!("symbol '{}' not found in imported file", name),
                    span: name_span,
                });
            }
        }

        // Record every function defined in the imported module in a signature
        // side-table (not the symbol table, to avoid duplicate-symbol collisions
        // and visibility leaks). An imported function may call sibling functions
        // in the same module that were never named here (e.g. str_copy calls
        // memcpy); codegen looks up the callee's signature by name to marshal
        // arguments, and without it would treat every argument as a single byte
        // and corrupt the call.
        for item in ast
            .items
            .iter()
            .chain(imported_analyzer.imported_items.iter())
        {
            if let crate::ast::Item::Function(f) = &item.node {
                let fname = &f.name.node;
                if let Some(sym) = imported_analyzer.table.lookup(fname) {
                    self.function_signatures
                        .entry(fname.clone())
                        .or_insert_with(|| sym.ty.clone());
                }
            }
        }

        // Merge ALL resolved_symbols from the imported module
        // This is necessary because when we emit imported functions during codegen,
        // they reference symbols (variables, constants, addresses) using their original spans
        for (span, symbol) in &imported_analyzer.resolved_symbols {
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
        for (span, value) in &imported_analyzer.folded_constants {
            self.folded_constants.insert(*span, value.clone());
        }

        // Merge loop-bound slots so for-loops in imported functions keep their
        // hidden frame slots when emitted from this module. They stay at
        // FrameOffset until the root finalize pass rewrites them.
        for (span, info) in &imported_analyzer.loop_bound_slots {
            self.loop_bound_slots.insert(*span, info.clone());
        }

        // Merge resolved_types so type information from imported modules is available
        for (span, ty) in &imported_analyzer.resolved_types {
            self.resolved_types.insert(*span, ty.clone());
        }

        // Merge function_metadata (already done above in the loop, but ensure transitives)
        for (name, metadata) in &imported_analyzer.function_metadata {
            if !self.function_metadata.contains_key(name) {
                self.function_metadata
                    .insert(name.clone(), metadata.clone());
            }
        }

        // Merge frame sizes and call-graph edges so the root finalize_frames
        // colors imported functions together with this module. Symbols stay at
        // FrameOffset until that single finalize pass rewrites them.
        for (name, size) in &imported_analyzer.frame_sizes {
            self.frame_sizes.entry(name.clone()).or_insert(*size);
        }
        for (caller, callees) in &imported_analyzer.call_edges {
            self.call_edges
                .entry(caller.clone())
                .or_default()
                .extend(callees.iter().cloned());
        }
        self.address_taken_functions
            .extend(imported_analyzer.address_taken_functions.iter().cloned());

        // Merge the imported module's reference graph. Liveness is computed over
        // the whole program at the root, so an imported function that calls a
        // sibling keeps that sibling alive without the root ever naming it.
        for (owner, refs) in &imported_analyzer.symbol_refs {
            self.symbol_refs
                .entry(owner.clone())
                .or_default()
                .extend(refs.iter().cloned());
        }

        // Merge the imported files set
        self.imported_files.extend(imported_analyzer.imported_files);

        Ok(())
    }

    pub(super) fn register_struct(
        &mut self,
        struct_def: &crate::ast::Struct,
    ) -> Result<(), SemaError> {
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
        let mut seen_fields = HashSet::default();

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

        let struct_info = StructDef {
            name: name.clone(),
            fields,
            total_size: offset,
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
                is_pub: struct_def.is_pub,
                containing_function: None, // Types are global
                is_param: false,
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
        // The tag an implicit variant would take. Held as u16 so running off the
        // end of the range is detectable rather than wrapping: `A = 0xFF, B`
        // would otherwise give B the tag 0, silently colliding with whatever
        // already holds it.
        let mut next_tag: u16 = 0;
        let mut seen_variants = HashSet::default();
        // Discriminant -> variant name, to reject duplicates. A tag identifies a
        // variant at runtime and indexes the match jump table, so two variants
        // sharing one makes the second unreachable.
        let mut tags_used: Vec<(u8, String)> = Vec::new();

        // Process each variant
        for variant in &enum_def.variants {
            let (variant_name, variant_data, tag) = match variant {
                EnumVariant::Unit {
                    name: var_name,
                    value,
                } => {
                    let tag = match value {
                        Some(v) => u8::try_from(*v).map_err(|_| SemaError::Custom {
                            message: format!(
                                "enum discriminant {} is out of range for '{}::{}' (0-255)",
                                v, name, var_name.node
                            ),
                            span: var_name.span,
                        })?,
                        None => next_tag_or_err(next_tag, &name, var_name)?,
                    };
                    next_tag = tag as u16 + 1;
                    (var_name.node.clone(), VariantData::Unit, tag)
                }
                EnumVariant::Tuple {
                    name: var_name,
                    fields: field_types,
                } => {
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
                    let tag = next_tag_or_err(next_tag, &name, var_name)?;
                    next_tag = tag as u16 + 1;
                    (var_name.node.clone(), VariantData::Tuple(types), tag)
                }
                EnumVariant::Struct {
                    name: var_name,
                    fields,
                } => {
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

                    let tag = next_tag_or_err(next_tag, &name, var_name)?;
                    next_tag = tag as u16 + 1;
                    (
                        var_name.node.clone(),
                        VariantData::Struct(variant_fields),
                        tag,
                    )
                }
            };

            // Check for duplicate variant
            let variant_span = match variant {
                EnumVariant::Unit { name, .. } => name.span,
                EnumVariant::Tuple { name, .. } => name.span,
                EnumVariant::Struct { name, .. } => name.span,
            };

            if !seen_variants.insert(variant_name.clone()) {
                return Err(SemaError::DuplicateSymbol {
                    name: variant_name,
                    span: variant_span,
                    previous_span: None,
                });
            }

            // Two variants sharing a discriminant are indistinguishable at
            // runtime: the tag is the whole value for a unit variant, and it is
            // what the match jump table indexes by, so the second arm would be
            // unreachable.
            if let Some((_, first)) = tags_used.iter().find(|(t, _)| *t == tag) {
                return Err(SemaError::Custom {
                    message: format!(
                        "enum discriminant {} is used by both '{}::{}' and '{}::{}'",
                        tag, name, first, name, variant_name
                    ),
                    span: variant_span,
                });
            }
            tags_used.push((tag, variant_name.clone()));

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
                    fields
                        .last()
                        .map(|f| f.offset + self.type_size(&f.ty))
                        .unwrap_or(0)
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
                is_pub: enum_def.is_pub,
                containing_function: None, // Types are global
                is_param: false,
            },
        );

        Ok(())
    }
}
