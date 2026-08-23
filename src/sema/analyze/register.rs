//! Registration Pass
//!
//! First pass of semantic analysis that registers all global items
//! (functions, statics, structs, enums, imports) before analyzing bodies.

use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};
use std::path::PathBuf;
use std::rc::Rc;

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
            decl_span: Some(func.name.span),
        };
        self.function_signatures
            .insert(name.clone(), info.ty.clone());
        self.table.insert(name.clone(), info);

        // `#[interrupt]` is a silent trap: the function gets a full handler
        // prologue and RTI, but nothing installs it in any vector, so it
        // never runs. Reject it until a generic vector exists; the real
        // attributes are `#[irq]` and `#[nmi]`.
        if func
            .attributes
            .iter()
            .any(|attr| matches!(attr, crate::ast::FnAttribute::Interrupt))
        {
            return Err(SemaError::Custom {
                message: format!(
                    "'{}': #[interrupt] is not supported — the handler would never be \
                     installed in a vector and never run; use #[irq] or #[nmi]",
                    name
                ),
                span: func.name.span,
            });
        }

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

        // A function the codegen auto-inliner may expand: not already `#[inline]`,
        // not an entry point (reset/irq/nmi never inline — they need a vector
        // address), not explicitly placed (`#[org]`/`#[section]` express an intent
        // to keep the body where it is), and returning a scalar or void (the
        // inline-expansion path is only exercised for those; aggregate returns are
        // left out for safety). Address-taken and recursive functions are excluded
        // later, in codegen, from data the register pass does not yet have.
        let is_entry = func.attributes.iter().any(|attr| {
            matches!(
                attr,
                crate::ast::FnAttribute::Reset
                    | crate::ast::FnAttribute::Irq
                    | crate::ast::FnAttribute::Nmi
                    | crate::ast::FnAttribute::Interrupt
            )
        });
        // Only functions with a purely scalar signature are inlined. Aggregate
        // and pointer parameters are passed by reference, and the inline
        // arg-store path does not set that up correctly (it would read the field
        // through the wrong location), so restrict to Primitive params and a
        // Primitive/void return.
        let signature_is_scalar = matches!(
            self.function_signatures.get(&name),
            Some(crate::sema::types::Type::Function(params, ret))
                if matches!(**ret, crate::sema::types::Type::Primitive(_) | crate::sema::types::Type::Void)
                    && params.iter().all(|p| matches!(p, crate::sema::types::Type::Primitive(_)))
        );
        let inline_candidate = !is_inline
            && !is_entry
            && org_address.is_none()
            && section.is_none()
            && signature_is_scalar;

        // Capture the body and parameters for `#[inline]` and auto-inline
        // candidates alike, so codegen can expand either.
        let (inline_body, inline_params) = if is_inline || inline_candidate {
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
                inline_candidate,
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

        // A static's initializer is never checked as an expression, so nothing
        // else records the names inside it. A function in a dispatch table and
        // a `&OTHER` pointer are both uses, and dead-code elimination drops
        // what it cannot see — for a `const` table that means the assembler
        // meets a `.WORD` naming a label nobody emitted.
        self.record_initializer_refs(&name, &stat.init);

        // A struct or enum `const` has no home: it cannot fold into const_env,
        // and codegen emits ROM data only for const arrays and strings, so the
        // declaration would exist but its bytes never would — every field read
        // hits the Absolute(0) sentinel. Say so, rather than emitting nothing.
        if !stat.mutable
            && let Type::Named(name) = &declared_ty
        {
            return Err(SemaError::Custom {
                message: format!(
                    "a const cannot have struct or enum type '{}'; only scalars, arrays and \
                     strings are supported. Use a `static` (kept in RAM) or a const array",
                    name
                ),
                span: stat.ty.span,
            });
        }

        // `#[soa]` decides the array's layout, so it has to be settled before
        // anything lays bytes down or resolves an address.
        if let Some(at) = stat.soa {
            self.register_soa(&name, &declared_ty, at, stat.ty.span)?;
        }

        // A generated table is folded here, not in the type checker: a `const`
        // or `static` array's initialiser is flattened during registration and
        // never reaches `check_expr`, so the declaration this feature exists
        // for — a table in ROM — would otherwise never be folded at all.
        if let crate::ast::Expr::Literal(crate::ast::Literal::ArrayGen { param, body }) =
            &stat.init.node
        {
            let Type::Array(elem, len) = &declared_ty else {
                return Err(SemaError::Custom {
                    message: format!(
                        "a generated table needs an array type, not {}",
                        declared_ty.display_name()
                    ),
                    span: stat.ty.span,
                });
            };
            if *len > 256 {
                return Err(SemaError::Custom {
                    message: format!(
                        "a generated table holds at most 256 entries, because its index is a \
                         `u8`; this one declares {len}"
                    ),
                    span: stat.ty.span,
                });
            }
            let (elem, len) = ((**elem).clone(), *len);
            self.fold_array_gen(param, body, &elem, len, stat.init.span)?;
        }

        // If it's a non-mutable static (const), evaluate it and add to const_env
        if !stat.mutable {
            match eval_const_expr_with_env(&stat.init, &self.const_env) {
                Ok(val) => {
                    // A scalar const folds into const_env and is substituted
                    // at its use sites, so the value's kind must be the
                    // type's kind: `const C: u8 = "hello"` would otherwise
                    // fold a string into byte contexts.
                    let kind_fits = match &declared_ty {
                        Type::String => matches!(val, ConstValue::String(_)),
                        Type::Primitive(_) => !matches!(val, ConstValue::String(_)),
                        _ => true,
                    };
                    if !kind_fits {
                        let found = match &val {
                            ConstValue::Integer(_) => "an integer",
                            ConstValue::Bool(_) => "a boolean",
                            ConstValue::String(_) => "a string",
                        };
                        return Err(SemaError::TypeMismatch {
                            expected: declared_ty.display_name(),
                            found: found.to_string(),
                            span: stat.init.span,
                        });
                    }
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
                Err(err) => {
                    // Aggregate consts (arrays) are fine unevaluated: their data
                    // is emitted to ROM by codegen. A scalar const has no other
                    // path — if it doesn't fold, it has no value anywhere, and
                    // every use reads the Absolute(0) sentinel. Reject it.
                    if declared_ty.is_primitive() || matches!(&declared_ty, Type::String) {
                        return Err(SemaError::Custom {
                            message: format!(
                                "const '{}' must have a constant initializer ({}); \
                                 only arrays may reference runtime data",
                                name, err
                            ),
                            span: stat.init.span,
                        });
                    }
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
            let bytes = self.static_init_bytes(&name, &stat.init, &declared_ty)?;
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
            decl_span: Some(stat.name.span),
        };
        self.table.insert(name, info);

        Ok(())
    }

    /// Check that `#[soa]` can be honoured here, and record the column layout.
    ///
    /// Two restrictions, both of them about what a column *is*. The type has to
    /// be an array of structs, because a column is a field; and every field has
    /// to be one or two bytes, because a column is indexed by scaling the index
    /// by the field's size and the machine can scale by one or two without a
    /// multiply. A field that is itself a struct or an array would need its own
    /// nested column scheme, which is a different feature.
    fn register_soa(
        &mut self,
        name: &str,
        declared_ty: &Type,
        at: crate::ast::Span,
        ty_span: crate::ast::Span,
    ) -> Result<(), SemaError> {
        let Type::Array(elem, len) = declared_ty else {
            return Err(SemaError::Custom {
                message: format!(
                    "#[soa] stores an array of structs as one column per field, so it needs \
                     an array type, not {}",
                    declared_ty.display_name()
                ),
                span: ty_span,
            });
        };
        let Type::Named(struct_name) = &**elem else {
            return Err(SemaError::Custom {
                message: format!(
                    "#[soa] needs an array of structs; the elements here are {}, which has no \
                     fields to make columns of",
                    elem.display_name()
                ),
                span: ty_span,
            });
        };
        let Some(sdef) = self.type_registry.get_struct(struct_name).cloned() else {
            return Err(SemaError::Custom {
                message: format!("#[soa] needs an array of structs; '{struct_name}' is not one"),
                span: ty_span,
            });
        };

        for field in &sdef.fields {
            // A column is indexed by scaling the index by the field's size,
            // which the machine does without a multiply for one or two bytes.
            // An aggregate field is excluded even at two bytes: its own parts
            // would each want a column, which is a different feature.
            let scalar = matches!(
                field.ty,
                Type::Primitive(_) | Type::Pointer(_) | Type::Function(..)
            ) || matches!(&field.ty, Type::Named(n) if self.type_registry.get_enum(n).is_some());
            let size = crate::sema::init::size_of(&field.ty, &self.type_registry);
            if !scalar || size == 0 || size > 2 {
                return Err(SemaError::Custom {
                    message: format!(
                        "#[soa] needs every field to be a scalar of one or two bytes, so that \
                         indexing a column costs no multiply; '{}.{}' is {}",
                        struct_name,
                        field.name,
                        field.ty.display_name()
                    ),
                    span: at,
                });
            }
        }

        self.soa_arrays.insert(
            name.to_string(),
            crate::sema::SoaLayout {
                elem: struct_name.clone(),
                len: *len,
            },
        );
        Ok(())
    }

    /// Record every symbol a static's initializer names, so dead-code
    /// elimination and the address-taken set can see through it.
    fn record_initializer_refs(&mut self, owner: &str, init: &Spanned<crate::ast::Expr>) {
        let mut names: Vec<String> = Vec::new();
        collect_variable_names(init, &mut names);
        for n in names {
            let is_function = self
                .table
                .lookup(&n)
                .is_some_and(|s| matches!(s.ty, Type::Function(..)));
            if is_function {
                // Installed in a table rather than called by name: it needs the
                // indirect-argument staging prologue, and it is a *use*, or
                // every driver entry point would be reported as unused.
                self.address_taken_functions.insert(n.clone());
                self.called_functions.insert(n.clone());
            }
            self.symbol_refs
                .entry(Some(owner.to_string()))
                .or_default()
                .insert(n.clone());
            self.all_used_symbols.insert(n);
        }
    }

    /// Flatten a mutable static's initializer into the exact bytes to write at
    /// startup.
    ///
    /// The walk itself lives in `sema::init`, shared with the `const` array
    /// path in codegen — they used to be two shallow copies that both stopped
    /// at one level of nesting.
    fn static_init_bytes(
        &self,
        name: &str,
        init: &Spanned<crate::ast::Expr>,
        ty: &Type,
    ) -> Result<Vec<crate::sema::InitByte>, SemaError> {
        // A scalar the compiler cannot evaluate simply starts at zero: RAM is
        // undefined at power-on and that is what BSS means. An aggregate is not
        // tolerated, because writing out a table and silently getting zeros back
        // is the bug this shares its implementation to avoid.
        crate::sema::init::flatten_top(init, ty, self, true, self.soa_arrays.get(name)).map_err(
            |e| SemaError::Custom {
                message: e.message,
                span: e.span,
            },
        )
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
    ) -> Result<u16, crate::sema::init::InitError> {
        use crate::ast::Expr;
        let err = |message: String, span| crate::sema::init::InitError {
            message,
            span,
            fatal: true,
        };

        let mut operand = operand;
        while let Expr::Paren(inner) = &operand.node {
            operand = inner;
        }
        let Expr::Variable(name) = &operand.node else {
            return Err(err(
                "a static's initializer can only take the address of another static".to_string(),
                operand.span,
            ));
        };

        match self.table.lookup(name) {
            // An immutable const lives in ROM and is referenced by label, so
            // `Absolute(0)` is a sentinel rather than an address — the same
            // reason `&CONST` is rejected in ordinary code.
            Some(sym) if sym.kind == SymbolKind::Constant => Err(err(
                format!(
                    "cannot take the address of the constant '{}'; constants live in ROM \
                     and are referenced by label, not by address",
                    name
                ),
                operand.span,
            )),
            Some(sym) => match sym.location {
                SymbolLocation::Absolute(addr) => Ok(addr),
                _ => Err(err(
                    format!(
                        "cannot take the address of '{}' here; only a static has a fixed \
                         address at startup",
                        name
                    ),
                    operand.span,
                )),
            },
            None => Err(err(
                format!(
                    "'{}' is not declared yet; statics are laid out in declaration order, \
                     so a static's initializer can only name one declared above it",
                    name
                ),
                operand.span,
            )),
        }
    }

    /// Reserve `size` bytes of BSS (RAM) for a mutable global, returning its
    /// address. Statics are allocated in declaration order from the start of the
    /// BSS section; unlike function frames they are never reused or colored,
    /// because they are live for the whole program and shared with interrupts.
    /// The cursor is shared across the whole import graph, so no two modules'
    /// globals ever collide, whoever allocates first.
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
        let mut ctx = self.import_context.borrow_mut();
        let base = ctx.bss_cursor.unwrap_or(start);
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
        ctx.bss_cursor = Some(base + size);
        Ok(base)
    }

    /// Repack BSS so a static the output drops costs nothing.
    ///
    /// Addresses are handed out during registration, in declaration order,
    /// long before liveness is known — an initializer's `&OTHER` has to
    /// resolve to a number as it is flattened, and that number has to exist.
    /// So a dropped static still reserved its bytes: codegen stopped emitting
    /// its initializer, but everything after it stayed where it was.
    ///
    /// Rather than defer allocation, repack once liveness *is* known. Sizes
    /// come from the gaps between consecutive addresses — BSS is handed out
    /// contiguously and statics are its only consumer until `finalize_frames`
    /// lays local-array blocks above the cursor — so nothing needs to re-derive
    /// a type's width here. Every live static's symbol is moved first, and only
    /// then are the initializers re-flattened, so an `&OTHER` picks up the
    /// address its target *ends up at* rather than the one it started with.
    ///
    /// A program that drops nothing keeps every address it had.
    pub(super) fn compact_bss(
        &mut self,
        source: &crate::ast::SourceFile,
        reachable: &HashSet<String>,
    ) -> Result<(), SemaError> {
        use crate::ast::Item;

        if self.static_inits.is_empty() {
            return Ok(());
        }
        let dropped = self
            .static_inits
            .iter()
            .any(|i| !reachable.contains(&i.name));
        if !dropped {
            return Ok(());
        }

        let (start, cursor_end) = {
            let ctx = self.import_context.borrow();
            let start = self
                .memory_config
                .get_section("BSS")
                .map(|s| s.start)
                .unwrap_or(0x0400);
            (start, ctx.bss_cursor.unwrap_or(start))
        };

        // Each static's allocated width, from where the next one begins.
        let sizes: Vec<u16> = self
            .static_inits
            .iter()
            .enumerate()
            .map(|(i, init)| {
                let next = self
                    .static_inits
                    .get(i + 1)
                    .map(|n| n.addr)
                    .unwrap_or(cursor_end);
                next.saturating_sub(init.addr)
            })
            .collect();

        // Pass 1: new addresses, in the order they were originally given out.
        let mut cursor = start;
        let mut kept: Vec<(String, u16)> = Vec::new();
        for (init, size) in self.static_inits.iter().zip(sizes.iter()) {
            if !reachable.contains(&init.name) {
                continue;
            }
            kept.push((init.name.clone(), cursor));
            cursor += size;
        }

        // Pass 2: move the symbols, so the re-flattening below sees the final
        // layout no matter which order the initializers reference each other in.
        //
        // The symbol table is not the only place an address lives. Body
        // analysis snapshots the whole `SymbolInfo` under each use's span, and
        // codegen reads *those* — so a static moved here but not there keeps
        // being loaded from where it used to be. (The fuzzer caught exactly
        // that: a program whose only surviving static had shifted down read a
        // function pointer out of the hole the dropped one left.) Both are
        // rewritten, and a snapshot is only touched when its name *and* its
        // current address match, so a local shadowing a static's name is left
        // alone.
        let mut moves: HashMap<String, (u16, u16)> = HashMap::default();
        for (name, addr) in &kept {
            if let Some(sym) = self.table.lookup(name) {
                if let SymbolLocation::Absolute(old) = sym.location {
                    moves.insert(name.clone(), (old, *addr));
                }
                let mut moved = sym.clone();
                moved.location = SymbolLocation::Absolute(*addr);
                self.table.insert(name.clone(), moved);
            }
        }
        let relocate = |sym: &mut SymbolInfo| {
            if let Some((old, new)) = moves.get(&sym.name)
                && sym.location == SymbolLocation::Absolute(*old)
            {
                sym.location = SymbolLocation::Absolute(*new);
            }
        };
        self.resolved_symbols.values_mut().for_each(relocate);
        // `inline_param_symbols` is a third copy, and its name undersells it:
        // it holds every symbol a function's body analysis resolved, not just
        // its parameters. An inline expansion merges those over
        // `resolved_symbols` at the call site, so a static corrected in the
        // other two stores is put back to its old address here.
        // `rewrite_frame_offsets` keeps the same three in step for the same
        // reason.
        for meta in self.function_metadata.values_mut() {
            if let Some(syms) = meta.inline_param_symbols.as_mut() {
                syms.values_mut().for_each(relocate);
            }
        }

        // Pass 3: re-flatten. An initializer that embeds `&OTHER` resolved it
        // to a number at registration time, and that number may have moved.
        let mut asts: HashMap<String, Spanned<crate::ast::Expr>> = HashMap::default();
        for item in self.imported_items.iter().chain(source.items.iter()) {
            if let Item::Static(st) = &item.node
                && st.mutable
            {
                asts.insert(st.name.node.clone(), st.init.clone());
            }
        }

        let mut repacked = Vec::with_capacity(kept.len());
        for (name, addr) in kept {
            let ty = match self.table.lookup(&name) {
                Some(sym) => sym.ty.clone(),
                None => continue,
            };
            let bytes = match asts.get(&name) {
                Some(init) => self.static_init_bytes(&name, init, &ty)?,
                // No declaration to re-read: keep what registration produced.
                None => match self.static_inits.iter().find(|i| i.name == name) {
                    Some(old) => old.bytes.clone(),
                    None => continue,
                },
            };
            repacked.push(crate::sema::StaticInit { name, addr, bytes });
        }

        self.static_inits = repacked;
        self.import_context.borrow_mut().bss_cursor = Some(cursor);
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
            decl_span: Some(addr.name.span),
        };
        self.table.insert(name, info);

        Ok(())
    }

    /// Read, parse and analyze one module file, then store the result for
    /// replay by later importers. Analysis errors are rendered against the
    /// module's own source and carried upward with the import trail.
    fn analyze_module_file(
        &mut self,
        import_path: &PathBuf,
        import: &Import,
    ) -> Result<(Rc<SemanticAnalyzer>, Vec<Spanned<crate::ast::Item>>), SemaError> {
        let result = self.analyze_module_file_inner(import_path, import);
        if result.is_err() {
            self.import_context
                .borrow_mut()
                .failed
                .insert(import_path.clone());
        }
        result
    }

    /// The body of [`Self::analyze_module_file`], wrapped so that every failure
    /// path marks the module as reported exactly once.
    fn analyze_module_file_inner(
        &mut self,
        import_path: &PathBuf,
        import: &Import,
    ) -> Result<(Rc<SemanticAnalyzer>, Vec<Spanned<crate::ast::Item>>), SemaError> {
        // A module that already failed has already been reported. Every other
        // path that reaches it would otherwise render the same diagnostics
        // again — a diamond turns three mistakes into six.
        if self.import_context.borrow().failed.contains(import_path) {
            return Err(SemaError::ImportFailedElsewhere {
                path: import.path.node.clone(),
                span: import.path.span,
            });
        }

        // Load and parse the imported file
        let source = std::fs::read_to_string(import_path).map_err(|e| SemaError::ImportError {
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
        self.import_context
            .borrow_mut()
            .stack
            .push(import_path.clone());
        let mut child = SemanticAnalyzer::with_base_path(import_path.clone());
        child.import_context = Rc::clone(&self.import_context);
        let result = child.analyze_module(&ast);
        self.import_context.borrow_mut().stack.pop();
        if let Err(e) = result {
            return Err(match e {
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
            });
        }

        let analyzer = Rc::new(child);
        self.import_context
            .borrow_mut()
            .modules
            .insert(import_path.clone(), (analyzer.clone(), ast.items.clone()));
        Ok((analyzer, ast.items))
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

        // Canonicalize so the same file reached by two spellings ("b.wr",
        // "./b.wr", an absolute path) is one node in the import graph.
        let import_path = std::fs::canonicalize(&import_path).unwrap_or(import_path);

        // A path already on the current chain is a true cycle (A -> B -> A).
        // A module that merely finished analyzing earlier (a diamond: A
        // imports B and C, and C also imports B) is not a cycle — its stored
        // analysis is replayed into this importer below.
        if self.import_context.borrow().stack.contains(&import_path) {
            let chain = self
                .import_context
                .borrow()
                .stack
                .iter()
                .map(|p| p.to_string_lossy().to_string())
                .collect();
            return Err(SemaError::CircularImport {
                path: import.path.node.clone(),
                chain,
            });
        }

        // Replay a completed module, or analyze it now. Either way we end up
        // with the module's analyzer and its items; the merge below is
        // identical for both paths, which is what makes diamond imports
        // order-independent.
        let replay = self
            .import_context
            .borrow()
            .modules
            .get(&import_path)
            .cloned();
        let (imported_analyzer, module_items, is_replay) = if let Some((an, items)) = replay {
            (an, items, true)
        } else {
            let (an, items) = self.analyze_module_file(&import_path, import)?;
            (an, items, false)
        };

        // Collect all items from the imported file for codegen
        // We collect ALL items, not just the imported symbols, because functions
        // may depend on other functions in the same module
        self.imported_items.extend(module_items.iter().cloned());

        // Also collect items from transitively imported modules
        self.imported_items
            .extend(imported_analyzer.imported_items.iter().cloned());

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
        for item in module_items
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

        self.merge_imported(&imported_analyzer, is_replay);

        Ok(())
    }

    /// Merge an imported module's analyzer state into this one.
    ///
    /// THE merge: there is one of these on purpose. Analyzer state that an
    /// imported function's codegen depends on but that wasn't merged here has
    /// been a recurring bug class (const_env, accessor_fields, local_arrays,
    /// static_inits all slipped once). When you add state to
    /// SemanticAnalyzer, decide here how it crosses an import.
    ///
    /// Deliberately NOT merged:
    /// - the symbol table (only the import's *requested* names come in, in
    ///   process_import, with pub checks) and function_signatures (idem, per
    ///   item, sibling-inclusive);
    /// - per-file warning state (declared_variables/used_variables/
    ///   declared_functions/imported_symbols): an unused local in an imported
    ///   function was already warned about when the module was analyzed;
    /// - transient analysis state (current_function, current_return_type,
    ///   expected_type, checking_assignment_target, loop_depth, frame_cursor,
    ///   array_cursor, loop_bound_free) — meaningless outside the body being
    ///   analyzed;
    /// - configuration (memory_config, base_path, import_context): the
    ///   importer's governs the whole program.
    fn merge_imported(&mut self, imported_analyzer: &SemanticAnalyzer, is_replay: bool) {
        // Merge the child's whole type registry, not just the types the import
        // names: an imported function may use a struct or enum internally (a
        // local, a field type) that the importing module never mentions, and
        // codegen resolves field layouts through this registry.
        for (name, def) in &imported_analyzer.type_registry.structs {
            if self.type_registry.get_struct(name).is_none() {
                self.type_registry.add_struct(def.clone());
            }
        }
        for (name, def) in &imported_analyzer.type_registry.enums {
            if self.type_registry.get_enum(name).is_none() {
                self.type_registry.add_enum(def.clone());
            }
        }

        // Merge ALL resolved_symbols from the imported module
        // This is necessary because when we emit imported functions during codegen,
        // they reference symbols (variables, constants, addresses) using their original spans
        for (span, symbol) in &imported_analyzer.resolved_symbols {
            self.resolved_symbols.insert(*span, symbol.clone());

            // Also add constants, addresses, and mutable statics to the symbol
            // table so they're visible to code in this module — and so codegen
            // can find them when emitting the child's items (its mutable
            // statics are looked up by name when their BSS equate is written).
            let is_global = symbol.containing_function.is_none()
                && matches!(
                    symbol.location,
                    crate::sema::table::SymbolLocation::Absolute(_)
                );
            let mergeable = matches!(symbol.kind, SymbolKind::Constant | SymbolKind::Address)
                || (symbol.kind == SymbolKind::Variable && is_global);
            if mergeable && self.table.lookup(&symbol.name).is_none() {
                self.table.insert(symbol.name.clone(), symbol.clone());
            }
        }

        // Merge folded_constants so constant expressions from imported modules are available
        for (span, value) in &imported_analyzer.folded_constants {
            self.folded_constants.insert(*span, value.clone());
        }

        // Merge the constant environment, or an imported `pub const` scalar has
        // no value here: uses of it don't fold, and codegen reads the
        // Absolute(0) sentinel — $0000 — instead of the constant. Same for a
        // `const D: u8 = C + 1` written in this module.
        for (name, value) in &imported_analyzer.const_env {
            self.const_env.entry(name.clone()).or_insert(value.clone());
        }

        // Merge diagnostics the child collected; its analysis is part of this
        // compile, so its warnings are too. A replayed module's warnings were
        // already reported by the first import.
        if !is_replay {
            self.warnings.extend(imported_analyzer.warnings.clone());
        }

        // Merge span-keyed resolutions produced while checking the child's
        // bodies. Without these, codegen sees the AST forms but not their
        // decisions: anonymous struct literals lose their resolved name, a
        // struct field named `len`/`low`/`high` is misread as the built-in
        // accessor, and statements the child proved unreachable are emitted.
        for (span, name) in &imported_analyzer.resolved_struct_names {
            self.resolved_struct_names.insert(*span, name.clone());
        }
        self.accessor_fields
            .extend(imported_analyzer.accessor_fields.iter().copied());
        self.unreachable_stmts
            .extend(imported_analyzer.unreachable_stmts.iter().copied());

        // Merge local-array placements. Without these, an imported function's
        // local array is emitted inline in the CODE section again — stores to
        // it write ROM, a silent no-op on hardware (the regression
        // ProgramInfo::local_arrays was built to fix).
        for (span, la) in &imported_analyzer.local_arrays {
            self.local_arrays.insert(*span, la.clone());
        }
        for (span, eb) in &imported_analyzer.enum_blocks {
            self.enum_blocks.insert(*span, eb.clone());
        }
        for (span, st) in &imported_analyzer.struct_temps {
            self.struct_temps.insert(*span, st.clone());
        }
        for (name, size) in &imported_analyzer.array_block_sizes {
            self.array_block_sizes.entry(name.clone()).or_insert(*size);
        }

        // Merge mutable statics' startup images. The shared BSS cursor means
        // their addresses are already final — but a replayed module's images
        // are already here from its first import, so dedupe by name.
        for init in &imported_analyzer.static_inits {
            if !self.static_inits.iter().any(|i| i.name == init.name) {
                self.static_inits.push(init.clone());
            }
        }

        // Merge loop-bound slots so for-loops in imported functions keep their
        // hidden frame slots when emitted from this module. They stay at
        // FrameOffset until the root finalize pass rewrites them.
        for (span, info) in &imported_analyzer.slice_return_temps {
            self.slice_return_temps.insert(*span, info.clone());
        }
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
        self.indirect_callers
            .extend(imported_analyzer.indirect_callers.iter().cloned());

        // Merge the imported module's reference graph. Liveness is computed over
        // the whole program at the root, so an imported function that calls a
        // sibling keeps that sibling alive without the root ever naming it.
        for (owner, refs) in &imported_analyzer.symbol_refs {
            self.symbol_refs
                .entry(owner.clone())
                .or_default()
                .extend(refs.iter().cloned());
        }
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

        // A struct that contains itself by value (directly, through an array, or
        // through a cycle of by-value struct fields) has no finite size; the
        // self-field silently sized to 0 above, laying the struct out too small.
        // A pointer field (`&Node`) breaks the cycle and is the intended shape.
        let mut visited = HashSet::default();
        if fields
            .iter()
            .any(|f| self.type_reaches(&f.ty, &name, &mut visited))
        {
            return Err(SemaError::Custom {
                message: format!(
                    "struct '{name}' contains itself by value, which has no finite size; \
                     store it behind a pointer (`&{name}`) instead"
                ),
                span: struct_def.name.span,
            });
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
                decl_span: Some(struct_def.name.span),
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
                decl_span: Some(enum_def.name.span),
            },
        );

        Ok(())
    }
}

/// How semantic analysis resolves the names inside a static's initializer.
impl crate::sema::init::InitContext for SemanticAnalyzer {
    fn generated_table(&self, span: crate::ast::Span) -> Option<&[i64]> {
        self.generated_tables.get(&span).map(|v| v.as_slice())
    }

    fn registry(&self) -> &crate::sema::type_defs::TypeRegistry {
        &self.type_registry
    }

    fn integer(&self, expr: &Spanned<crate::ast::Expr>) -> Option<i64> {
        eval_const_expr_with_env(expr, &self.const_env)
            .ok()
            .and_then(|v| v.as_integer())
    }

    fn function_name(&self, expr: &Spanned<crate::ast::Expr>) -> Option<String> {
        let crate::ast::Expr::Variable(n) = &expr.node else {
            return None;
        };
        self.table
            .lookup(n)
            .filter(|s| matches!(s.ty, Type::Function(..)))
            .map(|_| n.clone())
    }

    fn address_of(
        &self,
        operand: &Spanned<crate::ast::Expr>,
    ) -> Result<u16, crate::sema::init::InitError> {
        self.static_address_for_init(operand)
    }
}

/// Every bare name appearing anywhere in an expression.
fn collect_variable_names(e: &Spanned<crate::ast::Expr>, out: &mut Vec<String>) {
    use crate::ast::{Expr, Literal};
    match &e.node {
        Expr::Variable(n) => out.push(n.clone()),
        Expr::Paren(i) => collect_variable_names(i, out),
        Expr::Unary { operand, .. } => collect_variable_names(operand, out),
        Expr::Cast { expr, .. } => collect_variable_names(expr, out),
        Expr::Binary { left, right, .. } => {
            collect_variable_names(left, out);
            collect_variable_names(right, out);
        }
        Expr::Field { object, .. } => collect_variable_names(object, out),
        Expr::Index { object, index } => {
            collect_variable_names(object, out);
            collect_variable_names(index, out);
        }
        Expr::Literal(Literal::Array(elems)) => {
            for x in elems {
                collect_variable_names(x, out);
            }
        }
        Expr::Literal(Literal::ArrayFill { value, .. }) => collect_variable_names(value, out),
        // A generated table's body names constants like any other initializer;
        // its index parameter comes along too, which is harmless because no
        // declared symbol answers to it.
        Expr::Literal(Literal::ArrayGen { body, .. }) => collect_variable_names(body, out),
        Expr::StructInit { fields, .. } | Expr::AnonStructInit { fields } => {
            for f in fields {
                collect_variable_names(&f.value, out);
            }
        }
        _ => {}
    }
}
