//! Function call code generation
//!
//! This module handles:
//! - Normal function calls (JSR-based with zero-page parameter passing)
//! - Inline function expansion (body substitution)
//! - Parameter storage and register invalidation
//! - Return value handling

use crate::ast::{Expr, Spanned};
use crate::codegen::{CodegenError, Emitter, StringCollector};
use crate::sema::ProgramInfo;
use crate::sema::types::Type;

// Import generate_expr from parent module for recursive calls
use super::generate_expr;

/// Generate code for function calls
///
/// Dispatches to either:
/// - `generate_inline_call` for `#[inline]` functions
/// - Regular JSR-based call for normal functions
///
/// Regular calling convention:
/// - Arguments passed in zero-page starting at `param_base`
/// - 16-bit arguments take 2 consecutive bytes
/// - Return value in A (8-bit) or A+Y (16-bit)
/// - All registers invalidated after call
pub(super) fn generate_call(
    function: &Spanned<String>,
    args: &[Spanned<Expr>],
    emitter: &mut Emitter,
    info: &ProgramInfo,
    string_collector: &mut StringCollector,
) -> Result<(), CodegenError> {
    // Check if function should be inlined
    if let Some(metadata) = info.function_metadata.get(&function.node)
        && metadata.is_inline
    {
        // Inline the function call
        return generate_inline_call(function, args, emitter, info, metadata, string_collector);
    }

    // Indirect call through a function-pointer variable: sema recorded the
    // variable's symbol under the call span. Its location holds a 2-byte code
    // address we call through the trampoline.
    if let Some(sym) = info.resolved_symbols.get(&function.span)
        && let Type::Function(param_types, _) = &sym.ty
    {
        let loc = sym.location.clone();
        let ptypes = param_types.clone();
        return generate_indirect_call(
            CalleeSource::Location(loc),
            &ptypes,
            args,
            emitter,
            info,
            string_collector,
        );
    }

    // 6502 calling convention: arguments are passed in the callee's zero-page
    // frame (its parameter block sits at the frame base). This avoids the small,
    // slow hardware stack and, because frames are colored by the call graph, a
    // callee's parameter writes never touch the caller's live frame.
    //
    // Exception: an address-taken function (its pointer is used indirectly) takes
    // its arguments in the fixed indirect-arg staging block instead, and copies
    // them into its frame in a prologue — so direct and indirect callers agree.
    let callee_frame = info.function_frames.get(&function.node).copied();
    let is_address_taken = info.address_taken_functions.contains(&function.node);
    let param_base = if is_address_taken {
        crate::codegen::memory_layout::INDIRECT_ARG_BASE
    } else {
        match callee_frame {
            Some(f) => f.base,
            None => {
                return Err(CodegenError::Internal(format!(
                    "no frame assigned for called function '{}'",
                    function.node
                )));
            }
        }
    };

    // A recursive call (an edge inside a call-graph cycle) must save and restore
    // the callee's frame so re-entry cannot destroy values the caller still needs.
    let caller_name = emitter.current_function().map(|s| s.to_string());
    let is_recursive_edge = caller_name.as_ref().is_some_and(|c| {
        info.recursive_call_edges
            .contains(&(c.clone(), function.node.clone()))
    });

    // Emit descriptive call comment
    if args.is_empty() {
        emitter.emit_comment(&format!("Call: {}()", function.node));
    } else {
        emitter.emit_comment(&format!(
            "Call: {}(...) [{} arg{}]",
            function.node,
            args.len(),
            if args.len() == 1 { "" } else { "s" }
        ));
    }

    // Document parameter storage in verbose mode
    if emitter.is_verbose() && !args.is_empty() {
        emitter.emit_comment(&format!(
            "Parameters: [${:02X}-${:02X}] = {} arg{}",
            param_base,
            param_base + args.len() as u8 - 1,
            args.len(),
            if args.len() == 1 { "" } else { "s" }
        ));
    }

    // Get the callee's parameter types to marshal arguments correctly. Prefer
    // the symbol table, but fall back to the function-signature side-table, which
    // also covers imported-module functions this module never named (e.g. one
    // imported stdlib function calling another). Without the signature every
    // argument would be treated as a single byte and the call would be corrupt.
    let signature_ty = info
        .table
        .lookup(&function.node)
        .map(|sym| &sym.ty)
        .or_else(|| info.function_signatures.get(&function.node));
    let param_types = match signature_ty {
        Some(crate::sema::types::Type::Function(params, _)) => params.clone(),
        _ => Vec::new(),
    };

    // Fast path: all-trivial 8-bit args (literal or variable) to a
    // non-recursive, non-address-taken callee go straight into the callee's
    // frame. Frame coloring guarantees the callee's param slots don't alias
    // the caller's live frame — that only breaks down within a recursion
    // SCC, which is excluded — so the temp-pool round-trip below is pure
    // overhead here (`LDA #$03; STA $F4; LDA $F4; STA $40` per arg).
    let params_all_u8 = param_types.len() == args.len()
        && param_types.iter().all(|ty| {
            matches!(
                ty,
                crate::sema::types::Type::Primitive(crate::ast::PrimitiveType::U8)
                    | crate::sema::types::Type::Primitive(crate::ast::PrimitiveType::I8)
                    | crate::sema::types::Type::Primitive(crate::ast::PrimitiveType::Bool)
            )
        });
    let args_trivial = args.iter().all(|arg| {
        matches!(
            arg.node,
            crate::ast::Expr::Literal(crate::ast::Literal::Integer(_))
                | crate::ast::Expr::Variable(_)
        )
    });
    if !args.is_empty() && params_all_u8 && args_trivial && !is_address_taken && !is_recursive_edge
    {
        emitter.invalidate_registers();
        for (i, arg) in args.iter().enumerate() {
            generate_expr(arg, emitter, info, string_collector)?;
            emitter.emit_inst("STA", &format!("${:02X}", param_base + i as u8));
        }
        emitter.emit_inst("JSR", &function.node);
        emitter.reg_state.invalidate_all();
        return Ok(());
    }

    // STEP 1: Evaluate all arguments into TEMPORARY storage first
    // This prevents recursive calls from overwriting parameters that are still needed
    //
    // CRITICAL: We CANNOT use temp_storage_start ($20) because evaluating
    // expressions (especially binary operations) uses $20 as TEMP register!
    // This would overwrite previously evaluated arguments.
    // Use the arg temp pool ($F4-$FE) managed by TempAllocator.

    // Bytes each argument occupies while it waits, and the totals derived from
    // them: the whole block for pool staging, the widest single one for the
    // scratch slot stack staging reuses.
    //
    // Sized by [`ParamClass`], the one classification, rather than by a
    // private list of the wide types. This site used to keep its own and had
    // left a function pointer off it: `apply(add_one, 7)` reserved one byte
    // for `add_one`, staged its low byte alone, and every parameter after it
    // landed a byte early.
    //
    // A signature that does not cover the arguments used to default each
    // unknown to a byte, which is the same silent-fraction shape. Said out
    // loud instead: sema type-checks every call against a signature, so
    // getting here without one is a compiler bug and not a program's fault.
    if param_types.len() < args.len() {
        return Err(CodegenError::Internal(format!(
            "no signature for '{}': {} arguments against {} parameters",
            function.node,
            args.len(),
            param_types.len()
        )));
    }

    // Direct path: when no argument contains a call, nothing between producing
    // an argument and the `JSR` can disturb a parameter slot already written.
    // A non-recursive callee's frame is coloured apart from the caller's live
    // frame (the trivial fast path above rests on the same fact), and with no
    // nested call there is no reuse of the fixed argument pool or of a frame to
    // shelter against — so each argument goes straight into its slot through
    // the one staging routine, and the pool round-trip and its byte-by-byte
    // copy back into the frame are skipped entirely. An address-taken callee is
    // excluded: it reads arguments from the indirect-staging block, not its
    // frame, via a prologue copy.
    if !args.is_empty()
        && !is_address_taken
        && !is_recursive_edge
        && !args.iter().any(|a| super::binary::contains_call(&a.node))
    {
        emitter.invalidate_registers();
        let mut byte_offset = 0u8;
        for (i, arg) in args.iter().enumerate() {
            let width = stage_argument(
                arg,
                &param_types[i],
                param_base + byte_offset,
                StagingSite::Direct {
                    callee: &function.node,
                    index: i,
                },
                emitter,
                info,
                string_collector,
            )?;
            byte_offset += width;
        }
        emitter.emit_inst("JSR", &function.node);
        emitter.reg_state.invalidate_all();
        return Ok(());
    }

    let arg_sizes: Vec<u8> = param_types
        .iter()
        .take(args.len())
        .map(|ty| ParamClass::of(ty, info).width())
        .collect();
    let total_bytes: u8 = arg_sizes.iter().sum();
    let widest_arg: u8 = arg_sizes.iter().copied().max().unwrap_or(0);

    // Where this call's arguments wait between being evaluated and being copied
    // into the callee's frame.
    //
    // The pool is a fixed zero-page region, so a call nested in another call's
    // argument list needs room for both lists at once and four 16-bit
    // arguments inside four more will not fit. That used to be a compile
    // error. When the block does not fit, each argument is moved to the
    // software stack as soon as it is evaluated instead, so the pool holds one
    // argument at a time and the depth is bounded by the stack's 256 bytes.
    //
    // The pool is still tried first and is still the common case: staging
    // there is `LDA temp; STA param` per byte, where the stack costs a push
    // and a pop. Nothing that used to fit changes.
    let staging = if total_bytes == 0 {
        // No arguments: the base is unused, the copy loop runs zero times.
        Staging::Pool { base: 0 }
    } else if let Some(base) = emitter.temp_alloc.alloc_arg(total_bytes) {
        Staging::Pool { base }
    } else {
        // Room for one argument at a time — this call's widest, not the
        // language's, so a call whose parameters are all bytes needs one.
        let scratch = emitter.temp_alloc.alloc_arg(widest_arg).ok_or_else(|| {
            emitter.pool_error(&format!(
                "argument-evaluation pool exhausted calling '{}': {} free of {}, and even \
                 one argument at a time needs {}",
                function.node,
                emitter.temp_alloc.arg_bytes_free(),
                crate::codegen::memory_layout::TempAllocator::ARG_SIZE,
                widest_arg,
            ))
        })?;
        Staging::Stack { scratch }
    };
    let temp_base = match staging {
        Staging::Pool { base } => base,
        Staging::Stack { scratch } => scratch,
    };

    // A frame save pushes the callee's frame onto the same software stack the
    // arguments are being parked on, so with stack staging it has to happen
    // *before* them or it would bury them. Saving earlier is safe: the frame
    // still holds this invocation's live values, argument expressions only
    // read it, and its parameter slots are not written until the copy below.
    if matches!(staging, Staging::Stack { .. })
        && is_recursive_edge
        && let Some(frame) = callee_frame
    {
        emitter.push_frame(frame.base, frame.size);
    }
    let mut temp_offset = 0u8;
    let mut arg_info = Vec::new(); // Track argument sizes and temp locations

    // Argument staging below interleaves generate_expr (which consults register
    // tracking to elide loads) with raw STA temp-pool stores (which don't update
    // it). A belief left over from before the call — e.g. `a = ZeroPage($40)`
    // after storing a string local there — would otherwise wrongly elide a
    // later argument's own load from that same address, passing a stale value.
    // Evaluating arguments clobbers the registers anyway, so drop all beliefs.
    emitter.invalidate_registers();

    // Bytes of the argument pool currently parked on the software stack. See
    // the shelter comment in the loop; the pop is deferred to the next
    // iteration (and to after the loop) because the body below leaves through
    // a dozen different `continue`s.
    let mut sheltered = 0u8;

    for (i, arg) in args.iter().enumerate() {
        if sheltered > 0 {
            emitter.pop_frame(temp_base, sheltered);
            sheltered = 0;
        }
        // Stack staging reuses one scratch slot, so the previous argument has
        // to be parked before this one overwrites it. Deferred to here, and to
        // after the loop, for the same reason the shelter's pop is: the body
        // below leaves through a dozen different `continue`s.
        if let Staging::Stack { scratch } = staging
            && let Some((_, size)) = arg_info.last()
        {
            emitter.push_frame(scratch, *size);
        }
        let temp_addr = match staging {
            Staging::Pool { base } => base + temp_offset,
            Staging::Stack { scratch } => scratch,
        };

        // The argument pool is a *fixed* zero-page region ($F4-$FE) and the
        // allocator is reset at every function boundary, so a callee stages its
        // own arguments over the same bytes. An argument containing a call
        // therefore destroys the arguments already staged beside it:
        // `f(62, g(0, v), v)` passed g's first argument as f's, because g wrote
        // $F4 on its way in.
        //
        // Park what is staged so far on the software stack across the
        // evaluation. That stack is indexed through $FF, so it nests correctly
        // with the callee's own use of it — which the pool, being at a fixed
        // address, cannot. Only the bytes already written need saving, and only
        // when this argument can reach a call at all, so a call whose arguments
        // are plain values pays nothing.
        //
        // Stack staging needs none of this: each argument is already on the
        // stack before the next is evaluated, so the pool holds nothing worth
        // saving.
        if matches!(staging, Staging::Pool { .. })
            && temp_offset > 0
            && super::binary::contains_call(&arg.node)
        {
            emitter.push_frame(temp_base, temp_offset);
            sheltered = temp_offset;
        }

        // Everything about *how* this argument reaches the callee is the same
        // question the other three call forms ask, and is answered in one
        // place. What is left here is this site's own business: where the
        // bytes wait, and what has to be sheltered across a nested call.
        let width = stage_argument(
            arg,
            &param_types[i],
            temp_addr,
            StagingSite::Direct {
                callee: &function.node,
                index: i,
            },
            emitter,
            info,
            string_collector,
        )?;
        temp_offset += width;
        arg_info.push((temp_addr, width));
    }

    if sheltered > 0 {
        emitter.pop_frame(temp_base, sheltered);
    }
    // The last argument, still in the scratch slot.
    if let Staging::Stack { scratch } = staging
        && let Some((_, size)) = arg_info.last()
    {
        emitter.push_frame(scratch, *size);
    }

    // RECURSION SAVE: for a call inside a cycle, preserve the callee's frame
    // (which may hold the live values of an outer invocation) before we overwrite
    // its parameter slots. Done after argument evaluation and before the copy, so
    // the arguments (already parked in the temp pool) are unaffected. Stack
    // staging did this before the arguments went on, since they share a stack.
    if matches!(staging, Staging::Pool { .. })
        && is_recursive_edge
        && let Some(frame) = callee_frame
    {
        emitter.push_frame(frame.base, frame.size);
    }

    // STEP 2: move each argument into the callee's parameter slots.
    match staging {
        Staging::Pool { base } => {
            let mut byte_offset = 0u8;
            for (temp_addr, arg_size) in arg_info.iter() {
                let param_addr = param_base + byte_offset;
                for k in 0..*arg_size {
                    emitter.emit_inst("LDA", &format!("${:02X}", temp_addr + k));
                    emitter.emit_inst("STA", &format!("${:02X}", param_addr + k));
                }
                byte_offset += arg_size;
            }
            emitter.temp_alloc.free_arg(base, total_bytes);
        }
        // The stack hands them back in reverse, so walk the parameter block
        // backwards and pop each argument straight into its slot.
        Staging::Stack { scratch } => {
            let mut byte_offset = total_bytes;
            for (_, arg_size) in arg_info.iter().rev() {
                byte_offset -= arg_size;
                emitter.pop_frame(param_base + byte_offset, *arg_size);
            }
            emitter.temp_alloc.free_arg(scratch, widest_arg);
        }
    }

    // STEP 3: Call the function
    emitter.emit_inst("JSR", &function.node);

    // Invalidate register state after function call
    // (called function may modify any register; only A/Y contain known return value)
    emitter.reg_state.invalidate_all();

    // RECURSION RESTORE: restore the callee frame saved above. pop_frame
    // clobbers A and X, so stash the return low byte across the pop and reload
    // it. No hardware stack is used.
    //
    // A 16-bit scalar keeps its high byte in Y, which pop_frame leaves alone.
    // A struct, a `&T` or a `str` returns a *pointer* whose high byte is in X —
    // the register pop_frame uses as its stack index — so that one has to be
    // stashed too. Without this a recursive struct-returning function
    // dereferenced its own software-stack pointer as the result's high byte.
    if is_recursive_edge && let Some(frame) = callee_frame {
        // Asked of the same classification the arguments use. A return value
        // is not a parameter, but "which register holds the high byte of this
        // type" is the same question, and this was one more hand-written list
        // of the answer.
        let high_byte_in_x = info
            .table
            .lookup(&function.node)
            .and_then(|sym| match &sym.ty {
                Type::Function(_, ret) => Some(ParamClass::of(ret, info).high_byte_in_x()),
                _ => None,
            })
            .unwrap_or(false);

        let tmp = emitter.memory_layout.temp_reg();
        emitter.emit_inst("STA", &format!("${:02X}", tmp));
        if high_byte_in_x {
            emitter.emit_inst("STX", &format!("${:02X}", tmp + 1));
        }
        emitter.pop_frame(frame.base, frame.size);
        emitter.emit_inst("LDA", &format!("${:02X}", tmp));
        if high_byte_in_x {
            emitter.emit_inst("LDX", &format!("${:02X}", tmp + 1));
        }
        emitter.reg_state.invalidate_all();
    }

    // Result is returned in A register (no cleanup needed)
    if !emitter.is_minimal() {
        // Look up the function's return type to generate accurate comment
        if let Some(sym) = info.table.lookup(&function.node) {
            if let Type::Function(_, ret_type) = &sym.ty {
                match ret_type.as_ref() {
                    Type::Void => {
                        // No return value
                    }
                    Type::Primitive(crate::ast::PrimitiveType::U16)
                    | Type::Primitive(crate::ast::PrimitiveType::I16)
                    | Type::Primitive(crate::ast::PrimitiveType::B16) => {
                        emitter.emit_comment(&format!(
                            "Returns: A=result_low, Y=result_high ({})",
                            ret_type.display_name()
                        ));
                    }
                    ty => {
                        emitter.emit_comment(&format!("Returns: A=result ({})", ty.display_name()));
                    }
                }
            } else {
                // Fallback for non-function types
                emitter.emit_comment("Returns: A=result");
            }
        } else {
            // Fallback if function not in symbol table
            emitter.emit_comment("Returns: A=result");
        }
    }

    Ok(())
}

/// Generate an indirect call through a function-pointer variable.
///
/// Scalar arguments are written to the fixed indirect-arg staging block (the
/// address-taken callee's prologue copies them into its frame). The pointer is
/// loaded into the indirect vector at $EE/$EF and the shared trampoline
/// (`JMP ($EE)`) is JSR'd; the callee's RTS returns here. The return value
/// arrives in A (u8) / A:Y (u16) per the normal convention.
/// How the callee's address is obtained for an indirect call: either it is
/// stored in a variable at a known location, or it is produced by evaluating an
/// expression (a vtable field, a dispatch-table element, ...).
/// Where a call's arguments wait between evaluation and the copy into the
/// callee's frame.
#[derive(Clone, Copy)]
enum Staging {
    /// One contiguous block in the fixed zero-page pool, the common case.
    Pool { base: u8 },
    /// One argument at a time through `scratch`, each pushed to the software
    /// stack as soon as it is evaluated. Used when the whole block does not
    /// fit, which is what a call nested in another call's argument list can
    /// cause. The stack nests, so the depth is bounded by its 256 bytes.
    Stack { scratch: u8 },
}

enum CalleeSource<'a> {
    Location(crate::sema::table::SymbolLocation),
    Expr(&'a Spanned<Expr>),
}

/// How one argument reaches its parameter's slot.
///
/// The four call forms — a direct `JSR`, an inlined body, a tail-recursive
/// rebind, and an indirect call through a fixed block — each used to answer
/// this for itself by re-listing the types, which is three chances to differ
/// from the first. They did: a struct reached an inlined callee as the first
/// two bytes of its *contents*, a `str` and an enum reached a tail-recursive
/// one from the wrong register, and two of the four sized a function pointer
/// as a single byte.
///
/// So it is asked once, here. [`ParamClass::of`] is exhaustive over `Type`
/// with no catch-all, so a type added to the language has to say how it is
/// passed rather than defaulting to a byte.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ParamClass {
    /// One byte, in A.
    Byte,
    /// Two bytes, high in Y — the 16-bit numbers, and a function pointer,
    /// which is two bytes without being a number and is the case a list
    /// written as "the wide primitives, plus the things reached by a pointer"
    /// leaves out.
    Word,
    /// Two bytes, high in X: the pointer convention. A `&T`, a `str` or an
    /// enum — values that *are* addresses, produced in those registers by
    /// evaluating the expression.
    Address,
    /// Two bytes, high in X, but obtained by resolving a *place* to its
    /// address rather than by evaluating a value. A struct is passed by
    /// reference, and a struct local's slot holds the struct itself — so
    /// copying two bytes out of the slot hands the callee its contents.
    StructRef,
    /// Two bytes copied out of the slot that already holds them: an array's
    /// slot holds a pointer to its data.
    ArrayRef,
    /// Four bytes copied as a block — a slice's base-and-length descriptor,
    /// which no register pair can carry.
    Descriptor,
}

impl ParamClass {
    fn of(ty: &Type, info: &ProgramInfo) -> Self {
        use crate::ast::PrimitiveType;
        match ty {
            Type::Primitive(PrimitiveType::U16 | PrimitiveType::I16 | PrimitiveType::B16) => {
                Self::Word
            }
            Type::Primitive(_) => Self::Byte,
            // A function pointer is two bytes, high in Y, like a `u16`.
            Type::Function(..) => Self::Word,
            Type::Pointer(_) | Type::String => Self::Address,
            // An enum's value is already a pointer to its data block and
            // arrives in A:X; a struct's address has to be resolved from a
            // place. Both are spelled `Named` and only the registry separates
            // them. A `Named` the registry knows neither way cannot reach
            // codegen — and if it somehow does, `StructRef` errors rather than
            // staging one byte of it.
            Type::Named(n) if info.type_registry.get_enum(n).is_some() => Self::Address,
            Type::Named(_) => Self::StructRef,
            Type::Array(_, _) => Self::ArrayRef,
            Type::Slice(_) => Self::Descriptor,
            // Neither can be a parameter's type: `Void` has no values, and
            // `Error` is a poison that `analyze_module` stops before codegen.
            // Classified rather than panicked, so a malformed signature stages
            // one byte and the type error is what the user sees.
            Type::Void | Type::Error => Self::Byte,
        }
    }

    /// Bytes this parameter occupies, in the staging pool and in the frame.
    fn width(self) -> u8 {
        match self {
            Self::Byte => 1,
            Self::Word | Self::Address | Self::StructRef | Self::ArrayRef => 2,
            Self::Descriptor => 4,
        }
    }

    /// Which register holds the high byte, for the classes produced *in* a
    /// register pair. [`Self::ArrayRef`] and [`Self::Descriptor`] are copied
    /// slot to slot and never go through one.
    fn high_byte_in_x(self) -> bool {
        matches!(self, Self::Address | Self::StructRef)
    }

    /// Whether an indirect call can stage this class.
    ///
    /// The callee is unknown, so every argument has to arrive at a fixed
    /// address rather than in the callee's own frame. That suits anything one
    /// or two bytes wide with a settled register convention. An array
    /// parameter is a descriptor whose shape depends on the callee, and a
    /// slice is four bytes, so neither can.
    fn fits_indirect_block(self) -> bool {
        !matches!(self, Self::ArrayRef | Self::Descriptor)
    }
}

/// Which of the four call forms is staging an argument.
///
/// Only what genuinely differs between them lives here: how the site names
/// itself when an expression cannot be staged. Everything else — the width,
/// the register convention, which expression forms can supply each class — is
/// the same question at all four and is answered once in [`stage_argument`].
#[derive(Clone, Copy)]
enum StagingSite<'a> {
    /// A direct `JSR` to a named function, staging argument `index` (0-based).
    Direct { callee: &'a str, index: usize },
    /// An inlined body, writing straight into the parameter's slot.
    Inline,
    /// A tail-recursive rebind of the function's own parameters.
    TailCall,
    /// Through the fixed indirect-arg block, for a callee that is not known
    /// until run time.
    Indirect,
}

impl StagingSite<'_> {
    /// The error for an expression that cannot supply this parameter.
    ///
    /// Reaching it means every path for the parameter's class declined the
    /// expression. That is a refusal, not a fallback: the alternative is
    /// staging a fraction of what the callee will read, which is how a struct
    /// argument came to be one byte of its own contents.
    fn cannot_stage(self, param_ty: &Type) -> CodegenError {
        let ty = param_ty.display_name();
        CodegenError::UnsupportedOperation(match self {
            Self::Direct { callee, index } => format!(
                "cannot pass this expression as argument {} of '{callee}': a `{ty}` is passed \
                 by address or by descriptor, and this expression provides neither. Bind it \
                 to a `let` first and pass that",
                index + 1
            ),
            Self::Inline => format!(
                "cannot pass this expression to an inlined function's `{ty}` parameter: it is \
                 passed by address or by descriptor, and this expression provides neither. \
                 Bind it to a `let` first and pass that"
            ),
            Self::TailCall => format!(
                "cannot rebind the `{ty}` parameter from this expression in a tail-recursive \
                 call: it is passed by address or by descriptor, and this expression provides \
                 neither. Bind it to a `let` first and pass that"
            ),
            Self::Indirect => format!(
                "an indirect call cannot take a {ty} argument: it is staged at a fixed \
                 address for a callee that is not known until run time, which suits a \
                 scalar, a pointer, a string, an enum or a struct. Pass a `&T` to it instead"
            ),
        })
    }
}

/// The zero-page slot a copied class reads its bytes out of.
///
/// A slice's four-byte descriptor and an array's two-byte data pointer are
/// both *named* by a variable rather than produced by an expression, so both
/// are found the same way. The class of the symbol's own type has to match
/// what the parameter wants, which is what stops an array being copied into a
/// slice parameter four bytes wide.
fn copied_source_slot(expr: &Spanned<Expr>, info: &ProgramInfo, want: ParamClass) -> Option<u8> {
    let mut cur = expr;
    while let Expr::Paren(inner) = &cur.node {
        cur = inner;
    }
    let Expr::Variable(name) = &cur.node else {
        return None;
    };
    let sym = info
        .resolved_symbols
        .get(&cur.span)
        .or_else(|| info.table.lookup(name))?;
    let crate::sema::table::SymbolLocation::ZeroPage(slot) = sym.location else {
        return None;
    };
    (ParamClass::of(&sym.ty, info) == want).then_some(slot)
}

/// Stage one argument at `dest`, and report how many bytes it wrote.
///
/// The one place a call's argument is put where the callee will look for it.
/// `dest` is a zero-page address and means something different at each site —
/// a staging-pool slot for a direct call, the fixed block's slot for an
/// indirect one, the parameter's own slot for an inlined body — but the code
/// that fills it does not care which, so it is written once.
///
/// The bytes written are always `ParamClass::of(param_ty).width()`, which is
/// also what the caller reserved. That is the invariant the four separate
/// copies of this used to break: one sized a function pointer as a single
/// byte while writing two, and the parameter after it landed a byte early.
fn stage_argument(
    arg: &Spanned<Expr>,
    param_ty: &Type,
    dest: u8,
    site: StagingSite<'_>,
    emitter: &mut Emitter,
    info: &ProgramInfo,
    string_collector: &mut StringCollector,
) -> Result<u8, CodegenError> {
    let class = ParamClass::of(param_ty, info);
    let arg_ty = info.resolved_types.get(&arg.span);

    match class {
        // Four bytes of base and length. No register pair carries them, so
        // every path here is a copy or an in-place build.
        ParamClass::Descriptor => {
            // A bound slice variable, or a slice parameter — both hold their
            // descriptor inline.
            if let Some(src) = copied_source_slot(arg, info, ParamClass::Descriptor) {
                for k in 0..class.width() {
                    emitter.emit_inst("LDA", &format!("${:02X}", src + k));
                    emitter.emit_inst("STA", &format!("${:02X}", dest + k));
                }
                emitter.invalidate_registers();
                return Ok(class.width());
            }

            // A slice *expression* — `total(a[1..4])`. The materializer writes
            // a descriptor to a zero-page address, and `dest` is one, so it
            // builds in place with nothing to copy afterwards.
            if let Expr::Slice {
                object,
                start,
                end,
                inclusive,
            } = &arg.node
            {
                let Some(Type::Slice(elem)) = arg_ty else {
                    return Err(CodegenError::Internal(
                        "slice argument without a slice type".to_string(),
                    ));
                };
                crate::codegen::stmt::assign::generate_slice_materialize(
                    dest,
                    elem,
                    object,
                    start,
                    end,
                    *inclusive,
                    emitter,
                    info,
                    string_collector,
                )?;
                emitter.invalidate_registers();
                return Ok(class.width());
            }

            // A slice from a call — `total(mk())`, direct or through a
            // function pointer. The callee leaves a *pointer* to its
            // descriptor in A:X, so the four bytes are copied through it.
            // Staging A alone handed the callee one byte of that pointer as
            // though it were the base of the slice.
            if crate::codegen::expr::is_call(arg) {
                generate_expr(arg, emitter, info, string_collector)?;
                crate::codegen::stmt::emit_return_by_value_copy(
                    emitter,
                    dest as u16,
                    class.width(),
                );
                emitter.invalidate_registers();
                return Ok(class.width());
            }

            Err(site.cannot_stage(param_ty))
        }

        // An array's slot holds a two-byte pointer to its data, so the
        // parameter takes a copy of the slot.
        ParamClass::ArrayRef => {
            let Some(src) = copied_source_slot(arg, info, ParamClass::ArrayRef) else {
                return Err(site.cannot_stage(param_ty));
            };
            emitter.emit_inst("LDA", &format!("${:02X}", src));
            emitter.emit_inst("LDY", &format!("${:02X}", src + 1));
            emitter.emit_inst("STA", &format!("${:02X}", dest));
            emitter.emit_inst("STY", &format!("${:02X}", dest + 1));
            emitter.invalidate_registers();
            Ok(class.width())
        }

        // A struct is passed by *address*, and its slot is not where that
        // address lives — a local holds the struct inline. Every place has an
        // address, not just a zero-page local: a `static`, a nested field and
        // an array element all do, and matching only `Variable` is how
        // `sum(PS[i])` came to read whatever the first byte of the contents
        // happened to address.
        ParamClass::StructRef => {
            if crate::codegen::expr::emit_struct_place_address(
                arg,
                emitter,
                info,
                string_collector,
            )?
            .is_some()
            {
                emitter.emit_inst("STA", &format!("${:02X}", dest));
                emitter.emit_inst("STX", &format!("${:02X}", dest + 1));
                emitter.invalidate_registers();
                return Ok(class.width());
            }

            // A struct literal, or a call returning one, evaluates to a
            // pointer to its own bytes in A:X — the same convention, without
            // a place to resolve. A constant literal points into ROM and so
            // has a non-zero high byte, which is why dropping it was a silent
            // miscompile rather than merely a zero-page assumption.
            if crate::codegen::expr::is_call(arg)
                || matches!(
                    &arg.node,
                    Expr::StructInit { .. } | Expr::AnonStructInit { .. }
                )
            {
                generate_expr(arg, emitter, info, string_collector)?;
                emitter.emit_inst("STA", &format!("${:02X}", dest));
                emitter.emit_inst("STX", &format!("${:02X}", dest + 1));
                emitter.invalidate_registers();
                return Ok(class.width());
            }

            Err(site.cannot_stage(param_ty))
        }

        // The classes a register pair carries. The expression produces the
        // value; all that is left is which registers it arrived in.
        ParamClass::Byte | ParamClass::Word | ParamClass::Address => {
            generate_expr(arg, emitter, info, string_collector)?;

            // A narrow argument reaching a wide parameter is widened by the
            // language, and by the *source's* signedness — a negative `i8`
            // passed to an `i16` parameter keeps its sign. Before the low byte
            // is stored, because sign-extending works through A and X.
            if class == ParamClass::Word
                && let Some(signed) = crate::codegen::expr::implicit_widening(arg_ty, param_ty)
            {
                crate::codegen::expr::emit_widen_a_into_y(emitter, signed);
            }

            emitter.emit_inst("STA", &format!("${:02X}", dest));
            if class.width() == 2 {
                if class.high_byte_in_x() {
                    emitter.emit_inst("STX", &format!("${:02X}", dest + 1));
                } else {
                    // A parameter whose argument's type sema did not resolve
                    // still needs a defined high byte.
                    if arg_ty.is_none() {
                        emitter.emit_inst("LDY", "#$00");
                    }
                    emitter.emit_inst("STY", &format!("${:02X}", dest + 1));
                }
            }
            Ok(class.width())
        }
    }
}

fn generate_indirect_call(
    callee: CalleeSource<'_>,
    param_types: &[Type],
    args: &[Spanned<Expr>],
    emitter: &mut Emitter,
    info: &ProgramInfo,
    string_collector: &mut StringCollector,
) -> Result<(), CodegenError> {
    use crate::codegen::memory_layout::{INDIRECT_ARG_BASE, INDIRECT_ARG_MAX};
    use crate::sema::table::SymbolLocation;

    emitter.emit_comment("Indirect call through function pointer");

    if !args.is_empty() {
        // How each parameter reaches the callee, from the same classification
        // the other three call forms use.
        //
        // The one restriction that belongs to *this* site rather than to the
        // parameter: an argument here is staged at a fixed address, because
        // the callee is not known until run time and every candidate reads
        // from the same place. A whole array and a four-byte descriptor do not
        // fit that, and are refused with a way out rather than staged wrong.
        let classes: Vec<ParamClass> = param_types
            .iter()
            .map(|ty| ParamClass::of(ty, info))
            .collect();
        for (class, pty) in classes.iter().zip(param_types.iter()) {
            if !class.fits_indirect_block() {
                return Err(StagingSite::Indirect.cannot_stage(pty));
            }
        }

        let total: u8 = classes.iter().map(|c| c.width()).sum();
        if total > INDIRECT_ARG_MAX {
            return Err(CodegenError::UnsupportedOperation(format!(
                "indirect call arguments exceed the {}-byte staging block",
                INDIRECT_ARG_MAX
            )));
        }

        // STEP 1: evaluate args into the arg pool (so a later arg containing a
        // call can't clobber an earlier one, and nothing touches the staging
        // block until every arg is ready).
        let temp_base = emitter.temp_alloc.alloc_arg(total).ok_or_else(|| {
            emitter.pool_error("argument-evaluation pool exhausted (indirect call)")
        })?;
        let mut off = 0u8;
        let mut placed = Vec::new();
        for (arg, pty) in args.iter().zip(param_types.iter()) {
            let width = stage_argument(
                arg,
                pty,
                temp_base + off,
                StagingSite::Indirect,
                emitter,
                info,
                string_collector,
            )?;
            placed.push((temp_base + off, width == 2));
            off += width;
        }

        // STEP 2: copy the staged args into the fixed staging block.
        let mut boff = 0u8;
        for (taddr, p16) in &placed {
            emitter.emit_inst("LDA", &format!("${:02X}", taddr));
            emitter.emit_inst("STA", &format!("${:02X}", INDIRECT_ARG_BASE + boff));
            if *p16 {
                emitter.emit_inst("LDA", &format!("${:02X}", taddr + 1));
                emitter.emit_inst("STA", &format!("${:02X}", INDIRECT_ARG_BASE + boff + 1));
                boff += 2;
            } else {
                boff += 1;
            }
        }
        emitter.temp_alloc.free_arg(temp_base, total);
    }

    // Copy the callee's address into the indirect vector $EE/$EF. Done after
    // argument staging, since evaluating the callee clobbers A/Y.
    match callee {
        CalleeSource::Location(location) => {
            let addr = match location {
                SymbolLocation::ZeroPage(a) => a as u16,
                SymbolLocation::Absolute(a) => a,
                _ => {
                    return Err(CodegenError::UnsupportedOperation(
                        "function-pointer variable has no concrete storage location".to_string(),
                    ));
                }
            };
            if addr < 0x100 {
                emitter.emit_inst("LDA", &format!("${:02X}", addr));
                emitter.emit_inst("STA", "$EE");
                emitter.emit_inst("LDA", &format!("${:02X}", addr + 1));
                emitter.emit_inst("STA", "$EF");
            } else {
                emitter.emit_inst("LDA", &format!("${:04X}", addr));
                emitter.emit_inst("STA", "$EE");
                emitter.emit_inst("LDA", &format!("${:04X}", addr + 1));
                emitter.emit_inst("STA", "$EF");
            }
        }
        CalleeSource::Expr(e) => {
            // A function-pointer value lands in A (low) : Y (high).
            generate_expr(e, emitter, info, string_collector)?;
            emitter.emit_inst("STA", "$EE");
            emitter.emit_inst("STY", "$EF");
        }
    }
    emitter.needs_indirect_call = true;
    emitter.emit_inst("JSR", "__indirect_call");
    emitter.invalidate_registers();
    Ok(())
}

/// Call through a computed callee (`dev.read(r)`, `handlers[i](x)`). The callee
/// expression yields a function pointer, which is loaded into the indirect
/// vector and invoked through the trampoline.
pub(super) fn generate_call_indirect(
    callee: &Spanned<Expr>,
    args: &[Spanned<Expr>],
    emitter: &mut Emitter,
    info: &ProgramInfo,
    string_collector: &mut StringCollector,
) -> Result<(), CodegenError> {
    // Sema resolved the callee's type; its parameter list drives arg staging.
    let param_types = match info.resolved_types.get(&callee.span) {
        Some(Type::Function(params, _)) => params.clone(),
        _ => {
            return Err(CodegenError::UnsupportedOperation(
                "callee expression is not a function pointer".to_string(),
            ));
        }
    };
    generate_indirect_call(
        CalleeSource::Expr(callee),
        &param_types,
        args,
        emitter,
        info,
        string_collector,
    )
}

/// Generate inline function call expansion
///
/// Expands the function body inline, substituting arguments for parameters.
/// - Arguments evaluated and stored in parameter zero-page locations
/// - Function body generated inline (no JSR)
/// - Return statements jump to end instead of RTS
/// - Parameter symbols merged into current context
fn generate_inline_call(
    function: &Spanned<String>,
    args: &[Spanned<Expr>],
    emitter: &mut Emitter,
    info: &ProgramInfo,
    metadata: &crate::sema::FunctionMetadata,
    string_collector: &mut StringCollector,
) -> Result<(), CodegenError> {
    // Emit inline expansion comment
    if args.is_empty() {
        emitter.emit_comment(&format!("Inline: {}()", function.node));
    } else {
        emitter.emit_comment(&format!(
            "Inline: {}(...) [{} arg{}]",
            function.node,
            args.len(),
            if args.len() == 1 { "" } else { "s" }
        ));
    }

    // Get inline function body and parameters
    let body = metadata.inline_body.as_ref().ok_or_else(|| {
        CodegenError::UnsupportedOperation(format!(
            "Inline function {} missing body",
            function.node
        ))
    })?;

    let params = metadata.inline_params.as_ref().ok_or_else(|| {
        CodegenError::UnsupportedOperation(format!(
            "Inline function {} missing parameters",
            function.node
        ))
    })?;

    // Verify argument count matches parameter count
    if args.len() != params.len() {
        return Err(CodegenError::UnsupportedOperation(format!(
            "Inline function {} expects {} args, got {}",
            function.node,
            params.len(),
            args.len()
        )));
    }

    // Store arguments to the parameter locations that were allocated during semantic analysis
    // Each parameter has a specific zero-page address that was assigned when the function was defined
    // We need to store the argument values at those exact addresses
    // Parameter bytes already written, for the shelter below.
    let mut written: Vec<(u8, u8)> = Vec::new();

    for (i, arg) in args.iter().enumerate() {
        // Get the parameter info for this position
        let param = &params[i];

        // An inlined call stores each argument straight into the callee's
        // parameter slots, so a call inside a *later* argument overwrites the
        // ones already there. When the nested callee is this same function the
        // slots are literally the same bytes and no amount of frame colouring
        // can separate them: `f(0, v, f(12, i, i))` returned 12, the inner
        // call's first argument, as the outer call's.
        //
        // Park the written slots on the software stack across the evaluation.
        // It nests LIFO with everything else that uses it, and costs nothing
        // for the ordinary case where no argument contains a call.
        let shelter = !written.is_empty() && super::binary::contains_call(&arg.node);
        if shelter {
            for (addr, size) in &written {
                emitter.push_frame(*addr, *size);
            }
        }

        // Look up the parameter's allocated location from inline_param_symbols
        if let Some(ref param_symbols) = metadata.inline_param_symbols {
            if let Some(param_info) = param_symbols.get(&param.name.span) {
                match param_info.location {
                    crate::sema::table::SymbolLocation::ZeroPage(addr) => {
                        store_inline_arg(
                            arg,
                            &param_info.ty,
                            addr,
                            emitter,
                            info,
                            string_collector,
                        )?;
                        // Restore the earlier arguments now that this one is in
                        // its slot. `pop_frame` clobbers A, which is dead here.
                        if shelter {
                            for (a, sz) in written.iter().rev() {
                                emitter.pop_frame(*a, *sz);
                            }
                        }
                        let size = param_info.ty.size().max(1) as u8;
                        written.push((addr, size));
                    }
                    _ => {
                        return Err(CodegenError::UnsupportedOperation(format!(
                            "Inline function parameter '{}' must be in zero page",
                            param.name.node
                        )));
                    }
                }
            } else {
                return Err(CodegenError::UnsupportedOperation(format!(
                    "Parameter symbol '{}' not found for inline function",
                    param.name.node
                )));
            }
        } else {
            return Err(CodegenError::UnsupportedOperation(format!(
                "No parameter symbols for inline function {}",
                function.node
            )));
        }
    }

    // Generate the function body inline
    // Push inline context so return statements won't emit RTS
    emitter.push_inline();

    // Early returns jump here (see push_inline_end): setting A and falling
    // through would let a later statement overwrite the returned value.
    let end_label = format!("inline_{}_end", emitter.inline_label_suffix().unwrap_or(0));
    emitter.push_inline_end(end_label.clone());

    // The body about to be emitted belongs to the *callee*, even though it lands
    // in the caller's output. Anything that scopes a lookup by "current function"
    // — inline-asm `{param}` substitution above all, which matches a symbol's
    // `containing_function` — must therefore see the callee's name. Leaving the
    // caller's name here made every `{param}` reference in an inline function's
    // assembly fail to resolve: the parameter belongs to `min`, the lookup asked
    // for one belonging to `main`.
    let saved_function = emitter.take_current_function();
    emitter.set_current_function(function.node.clone());

    // For inline functions from imported modules, we need to merge the parameter symbols
    // from the original module into the current ProgramInfo so the function body can
    // reference its parameters correctly
    let result = if let Some(ref param_symbols) = metadata.inline_param_symbols {
        // The merge below costs a full ProgramInfo clone — every symbol map, the
        // type registry, and every imported module's AST — at *each* inline call
        // site, so skip it when it would change nothing. It usually would: a
        // local inline function was analyzed by this same pass, and an imported
        // one had its `resolved_symbols` merged into ours by `process_import`,
        // so in both cases these entries are already present and identical.
        //
        // The comparison is by value, not just by key. Spans are byte offsets
        // with no file identity, so two modules can collide on one; there the
        // callee's own symbol must win for the duration of its body, which is
        // exactly what the merge does.
        if param_symbols
            .iter()
            .all(|(span, sym)| info.resolved_symbols.get(span).is_some_and(|e| e == sym))
        {
            use crate::codegen::stmt::generate_stmt;
            let r = generate_stmt(body, emitter, info, string_collector);
            emitter.emit_label(&end_label);
            emitter.restore_current_function(saved_function);
            emitter.pop_inline_end();
            emitter.pop_inline();
            return r;
        }

        // Create a modified ProgramInfo with merged resolved_symbols
        let mut merged_resolved = info.resolved_symbols.clone();
        for (span, symbol) in param_symbols {
            merged_resolved.insert(*span, symbol.clone());
        }

        let modified_info = crate::sema::ProgramInfo {
            table: info.table.clone(),
            resolved_symbols: merged_resolved,
            function_metadata: info.function_metadata.clone(),
            folded_constants: info.folded_constants.clone(),
            generated_tables: info.generated_tables.clone(),
            soa_arrays: info.soa_arrays.clone(),
            const_env: info.const_env.clone(),
            loop_bound_slots: info.loop_bound_slots.clone(),
            slice_return_temps: info.slice_return_temps.clone(),
            local_arrays: info.local_arrays.clone(),
            enum_blocks: info.enum_blocks.clone(),
            string_buffers: info.string_buffers.clone(),
            struct_temps: info.struct_temps.clone(),
            type_registry: info.type_registry.clone(),
            resolved_types: info.resolved_types.clone(),
            imported_items: info.imported_items.clone(),
            warnings: info.warnings.clone(),
            unreachable_stmts: info.unreachable_stmts.clone(),
            tail_call_info: info.tail_call_info.clone(),
            resolved_struct_names: info.resolved_struct_names.clone(),
            accessor_fields: info.accessor_fields.clone(),
            string_pool: info.string_pool.clone(),
            function_frames: info.function_frames.clone(),
            static_inits: info.static_inits.clone(),
            memory_config: info.memory_config.clone(),
            function_signatures: info.function_signatures.clone(),
            recursive_call_edges: info.recursive_call_edges.clone(),
            interrupt_save_info: info.interrupt_save_info.clone(),
            address_taken_functions: info.address_taken_functions.clone(),
            reachable_symbols: info.reachable_symbols.clone(),
        };

        use crate::codegen::stmt::generate_stmt;
        generate_stmt(body, emitter, &modified_info, string_collector)
    } else {
        // No parameter symbols stored - this indicates a bug in semantic analysis
        // Inline functions should always have parameter symbols populated
        return Err(CodegenError::UnsupportedOperation(format!(
            "Inline function {} has no parameter symbols (compiler bug)",
            function.node
        )));
    };

    // The early-return jump target, then pop inline context
    emitter.emit_label(&end_label);
    emitter.restore_current_function(saved_function);
    emitter.pop_inline_end();
    emitter.pop_inline();

    result
}

/// Store one argument into an inline function's parameter slot, at the width
/// the parameter actually is.
///
/// This used to emit a bare `STA`, whatever the parameter's type. Anything
/// wider than a byte therefore arrived with only its low byte set and the rest
/// left as whatever happened to be in the slot: `#[inline] fn f(v: u16)` called
/// as `f(0x1234)` stored `$34` and read garbage for the high byte, and an enum
/// parameter — a two-byte pointer — was dereferenced through a half-written
/// address. It compiled, and it silently did the wrong thing.
///
/// The register conventions no longer *mirror* `generate_call` — the two share
/// [`stage_argument`], which is the point. Mirroring is what let them drift: a
/// struct arrived here as the first two bytes of its contents, and a function
/// pointer as one byte of its address.
fn store_inline_arg(
    arg: &Spanned<Expr>,
    param_ty: &Type,
    dest: u8,
    emitter: &mut Emitter,
    info: &ProgramInfo,
    string_collector: &mut StringCollector,
) -> Result<(), CodegenError> {
    // An inlined body writes into the parameter's slot directly — there is no
    // pool to stage through, because there is no `JSR` for a nested call to
    // arrive between. So `dest` is the destination itself, and the width is
    // whatever the class says it is.
    stage_argument(
        arg,
        param_ty,
        dest,
        StagingSite::Inline,
        emitter,
        info,
        string_collector,
    )?;
    Ok(())
}

pub fn generate_tail_recursive_update(
    _function: &Spanned<String>,
    args: &[Spanned<Expr>],
    emitter: &mut Emitter,
    info: &ProgramInfo,
    string_collector: &mut StringCollector,
) -> Result<(), CodegenError> {
    // A tail-recursive call rebinds the current function's own parameters, so the
    // destination is this function's own frame base.
    let param_base = match emitter
        .current_function()
        .and_then(|f| info.function_frames.get(f))
    {
        Some(frame) => frame.base,
        None => {
            return Err(CodegenError::Internal(
                "tail-recursive update outside any framed function".to_string(),
            ));
        }
    };

    // STEP 1: Evaluate all arguments into TEMPORARY storage
    // This prevents arguments from overwriting parameters they depend on
    // Example: fib(n-1, acc*n) - both args need current n value
    //
    // CRITICAL: We CANNOT use temp_storage_start ($20) because evaluating
    // expressions (especially binary operations) uses $20 as TEMP register!
    // This would overwrite previously evaluated arguments.
    // Use the arg temp pool ($F4-$FE) managed by TempAllocator.

    // What each parameter occupies, read off the function's *own* signature.
    //
    // This used to size every argument by its own type and recognise only the
    // 16-bit primitives, which was wrong twice over. A slice parameter is four
    // bytes and counted as one, so the loop rebound a quarter of its
    // descriptor — and, worse, every parameter *after* a wide one was written
    // at an offset that assumed one byte each, so the third parameter landed
    // inside the second. A `u8` argument reaching a `u16` parameter had the
    // same shape.
    let param_types = match emitter
        .current_function()
        .and_then(|f| info.table.lookup(f).map(|s| &s.ty))
        .or_else(|| {
            emitter
                .current_function()
                .and_then(|f| info.function_signatures.get(f))
        }) {
        Some(crate::sema::types::Type::Function(params, _)) => params.clone(),
        _ => Vec::new(),
    };
    // A tail call rebinds this function's own parameters, so its own signature
    // says how wide each is. Missing an entry means the signature does not
    // cover the arguments, which for a *self* call is a compiler bug — said
    // out loud rather than defaulting each unknown to one byte, which is the
    // shape that used to write a slice's first quarter and shift every
    // parameter after it.
    if param_types.len() < args.len() {
        return Err(CodegenError::Internal(format!(
            "tail-recursive call passes {} arguments to a signature of {}",
            args.len(),
            param_types.len()
        )));
    }
    let widths: Vec<u8> = param_types
        .iter()
        .take(args.len())
        .map(|ty| ParamClass::of(ty, info).width())
        .collect();
    let total_bytes: u8 = widths.iter().sum();

    // Allocate temp storage for all arguments at once
    let temp_base = if total_bytes == 0 {
        0
    } else {
        match emitter.temp_alloc.alloc_arg(total_bytes) {
            Some(addr) => addr,
            None => {
                return Err(emitter
                    .pool_error("argument-evaluation pool exhausted in tail-recursive update"));
            }
        }
    };
    let mut temp_offset = 0u8;
    let mut arg_info: Vec<(u8, u8)> = Vec::new();

    for (arg, pty) in args.iter().zip(param_types.iter()) {
        let temp_addr = temp_base + temp_offset;
        let width = stage_argument(
            arg,
            pty,
            temp_addr,
            StagingSite::TailCall,
            emitter,
            info,
            string_collector,
        )?;
        temp_offset += width;
        arg_info.push((temp_addr, width));
    }

    // STEP 2: Copy arguments from temporary storage to parameter locations
    // Now we can safely update all parameters without conflicts
    let mut byte_offset = 0u8;
    for (temp_addr, width) in arg_info.iter() {
        let param_addr = param_base + byte_offset;
        for k in 0..*width {
            emitter.emit_inst("LDA", &format!("${:02X}", temp_addr + k));
            emitter.emit_inst("STA", &format!("${:02X}", param_addr + k));
        }
        byte_offset += width;
    }

    // Free the temp storage after copying to parameters
    if total_bytes > 0 {
        emitter.temp_alloc.free_arg(temp_base, total_bytes);
    }

    // NOTE: No JSR instruction - caller will emit JMP to loop label

    Ok(())
}
