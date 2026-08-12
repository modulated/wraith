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

    // Calculate total bytes needed for all arguments
    let mut total_bytes = 0u8;
    for (i, _arg) in args.iter().enumerate() {
        let is_16bit = param_types.get(i).is_some_and(|param_ty| {
            matches!(
                param_ty,
                crate::sema::types::Type::Primitive(crate::ast::PrimitiveType::U16)
                    | crate::sema::types::Type::Primitive(crate::ast::PrimitiveType::I16)
                    | crate::sema::types::Type::Primitive(crate::ast::PrimitiveType::B16)
            )
        });
        // Struct, array, and string parameters take 2 bytes (pointer)
        let is_struct = param_types
            .get(i)
            .is_some_and(|param_ty| matches!(param_ty, Type::Named(_)));
        let is_array = param_types
            .get(i)
            .is_some_and(|param_ty| matches!(param_ty, Type::Array(_, _)));
        let is_string = param_types
            .get(i)
            .is_some_and(|param_ty| matches!(param_ty, Type::String));
        // A slice parameter is a 4-byte fat-pointer descriptor (base + length).
        let is_slice = param_types
            .get(i)
            .is_some_and(|param_ty| matches!(param_ty, Type::Slice(_)));
        let is_pointer = param_types
            .get(i)
            .is_some_and(|param_ty| matches!(param_ty, Type::Pointer(_)));
        total_bytes += if is_slice {
            4
        } else if is_16bit || is_struct || is_array || is_string || is_pointer {
            2
        } else {
            1
        };
    }

    // Allocate temp storage for all arguments at once
    let temp_base = if total_bytes == 0 {
        0 // No arguments: base is unused (the copy loop below runs zero times).
    } else {
        match emitter.temp_alloc.alloc_arg(total_bytes) {
            Some(addr) => addr,
            None => {
                return Err(CodegenError::Internal(format!(
                    "argument-evaluation pool exhausted calling '{}' ({} bytes of arguments, \
                     {} free of {}). Arguments are staged in a fixed zero-page pool, and a \
                     call nested in another call's argument list needs room for both at \
                     once. Bind the inner call to a `let` first.",
                    function.node,
                    total_bytes,
                    emitter.temp_alloc.arg_bytes_free(),
                    crate::codegen::memory_layout::TempAllocator::ARG_SIZE,
                )));
            }
        }
    };
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
        let temp_addr = temp_base + temp_offset;

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
        if temp_offset > 0 && super::binary::contains_call(&arg.node) {
            emitter.push_frame(temp_base, temp_offset);
            sheltered = temp_offset;
        }

        // Check argument type
        let arg_type = info.resolved_types.get(&arg.span);

        // Check if this is a struct argument (pass by reference)
        let is_struct = arg_type.is_some_and(|ty| matches!(ty, Type::Named(_)));

        // Check for Enums specifically (passed as 2-byte value pointers)
        let is_enum_arg = arg_type.is_some_and(|ty| {
            if let Type::Named(name) = ty {
                info.type_registry.enums.contains_key(name)
            } else {
                false
            }
        });

        // Check if parameter is an enum
        let param_is_enum = param_types.get(i).is_some_and(|ty| {
            if let Type::Named(name) = ty {
                info.type_registry.enums.contains_key(name)
            } else {
                false
            }
        });

        // Handle enum passing (pass the 2-byte pointer value, stored in A:X)
        if is_enum_arg && param_is_enum {
            // Generate expression (returns pointer in A:X)
            generate_expr(arg, emitter, info, string_collector)?;

            // Store to TEMPORARY location
            emitter.emit_inst("STA", &format!("${:02X}", temp_addr));
            // Store high byte from X (enums use A:X convention)
            emitter.emit_inst("STX", &format!("${:02X}", temp_addr + 1));

            temp_offset += 2;
            arg_info.push((temp_addr, 2));
            continue;
        }

        // A pointer argument is a 2-byte address in A:X, like an enum or a
        // string. It is a *value* produced by an expression, not a descriptor
        // copied out of a slot, so it does not belong on the array/struct
        // pass-by-reference paths further down.
        let param_is_pointer = param_types
            .get(i)
            .is_some_and(|ty| matches!(ty, Type::Pointer(_)));
        if param_is_pointer {
            generate_expr(arg, emitter, info, string_collector)?;
            emitter.emit_inst("STA", &format!("${:02X}", temp_addr));
            emitter.emit_inst("STX", &format!("${:02X}", temp_addr + 1));
            temp_offset += 2;
            arg_info.push((temp_addr, 2));
            continue;
        }

        // Check if argument is a string (2-byte pointer, stored in A:X)
        let is_string_arg = arg_type.is_some_and(|ty| matches!(ty, Type::String));
        let param_is_string = param_types
            .get(i)
            .is_some_and(|param_ty| matches!(param_ty, Type::String));

        // Handle string passing (pass the 2-byte pointer, stored in A:X)
        if is_string_arg && param_is_string {
            // Generate expression (returns pointer in A:X)
            generate_expr(arg, emitter, info, string_collector)?;

            // Store to TEMPORARY location
            emitter.emit_inst("STA", &format!("${:02X}", temp_addr));
            // Store high byte from X (strings use A:X convention like enums)
            emitter.emit_inst("STX", &format!("${:02X}", temp_addr + 1));

            temp_offset += 2;
            arg_info.push((temp_addr, 2));
            continue;
        }

        // Check if argument is a slice (4-byte fat-pointer descriptor). Copy the
        // descriptor by value from the slice variable's frame slot to the temp
        // pool; the callee reads it inline from its parameter slot.
        let is_slice_arg = arg_type.is_some_and(|ty| matches!(ty, Type::Slice(_)));
        let param_is_slice = param_types
            .get(i)
            .is_some_and(|param_ty| matches!(param_ty, Type::Slice(_)));
        if is_slice_arg && param_is_slice {
            // A bound slice variable: copy its descriptor across.
            if let crate::ast::Expr::Variable(var_name) = &arg.node
                && let Some(sym) = info
                    .resolved_symbols
                    .get(&arg.span)
                    .or_else(|| info.table.lookup(var_name))
                && let crate::sema::table::SymbolLocation::ZeroPage(addr) = sym.location
            {
                for k in 0..4u8 {
                    emitter.emit_inst("LDA", &format!("${:02X}", addr + k));
                    emitter.emit_inst("STA", &format!("${:02X}", temp_addr + k));
                }
                temp_offset += 4;
                arg_info.push((temp_addr, 4));
                continue;
            }

            // A slice *expression* — `total(a[1..4])`. The staging slot is a
            // zero-page address and the materializer writes a descriptor to
            // exactly that, so it can build in place; there is no intermediate
            // to allocate and nothing to copy afterwards.
            if let crate::ast::Expr::Slice {
                object,
                start,
                end,
                inclusive,
            } = &arg.node
            {
                let Some(Type::Slice(elem)) = arg_type else {
                    return Err(CodegenError::Internal(
                        "slice argument without a slice type".to_string(),
                    ));
                };
                crate::codegen::stmt::assign::generate_slice_materialize(
                    temp_addr,
                    elem,
                    object,
                    start,
                    end,
                    *inclusive,
                    emitter,
                    info,
                    string_collector,
                )?;
                temp_offset += 4;
                arg_info.push((temp_addr, 4));
                continue;
            }
        }

        // Check if this PARAMETER (not argument) is a 16-bit type
        // This is critical for correct code generation when passing smaller types to larger parameters
        let is_16bit = param_types.get(i).is_some_and(|param_ty| {
            matches!(
                param_ty,
                crate::sema::types::Type::Primitive(crate::ast::PrimitiveType::U16)
                    | crate::sema::types::Type::Primitive(crate::ast::PrimitiveType::I16)
                    | crate::sema::types::Type::Primitive(crate::ast::PrimitiveType::B16)
            )
        });

        // Check if argument is an array (pass by reference as 2-byte pointer)
        let is_array = arg_type.is_some_and(|ty| matches!(ty, Type::Array(_, _)));
        let param_is_array = param_types
            .get(i)
            .is_some_and(|param_ty| matches!(param_ty, Type::Array(_, _)));

        // For struct arguments, check if the parameter is also a struct (pass-by-reference)
        let param_is_struct = param_types
            .get(i)
            .is_some_and(|param_ty| matches!(param_ty, Type::Named(_)));

        // Handle array pass-by-reference (pass the 2-byte pointer stored in ZP variable)
        if is_array
            && param_is_array
            && let crate::ast::Expr::Variable(var_name) = &arg.node
            && let Some(sym) = info
                .resolved_symbols
                .get(&arg.span)
                .or_else(|| info.table.lookup(var_name))
            && let crate::sema::table::SymbolLocation::ZeroPage(addr) = sym.location
        {
            // Load the 2-byte pointer from the array variable's ZP location
            emitter.emit_inst("LDA", &format!("${:02X}", addr)); // Low byte
            emitter.emit_inst("LDY", &format!("${:02X}", addr + 1)); // High byte

            // Store 2-byte pointer to temp
            emitter.emit_inst("STA", &format!("${:02X}", temp_addr));
            emitter.emit_inst("STY", &format!("${:02X}", temp_addr + 1));
            temp_offset += 2;
            arg_info.push((temp_addr, 2)); // 2-byte pointer
            continue;
        }

        if is_struct && param_is_struct {
            // Struct pass-by-reference: pass the 2-byte ZP address
            // The argument must be a variable (for now)
            if let crate::ast::Expr::Variable(var_name) = &arg.node
                && let Some(sym) = info
                    .resolved_symbols
                    .get(&arg.span)
                    .or_else(|| info.table.lookup(var_name))
                && let crate::sema::table::SymbolLocation::ZeroPage(addr) = sym.location
            {
                // Load the ADDRESS of the struct (not its value)
                emitter.emit_inst("LDA", &format!("#${:02X}", addr)); // Low byte of address
                emitter.emit_inst("LDY", "#$00"); // High byte (ZP, so always 0)

                // Store 2-byte pointer to temp
                emitter.emit_inst("STA", &format!("${:02X}", temp_addr));
                emitter.emit_inst("STY", &format!("${:02X}", temp_addr + 1));
                temp_offset += 2;
                arg_info.push((temp_addr, 2)); // 2-byte pointer
                continue;
            }

            // Any other struct-valued argument — a struct literal, or a call
            // returning a struct — evaluates to a pointer to the bytes in A:X.
            // This used to fall through to the scalar path, which staged only
            // A: the callee then dereferenced an address whose high byte was
            // whatever the staging temp already held. A constant literal
            // (`sum(P { x: 6, y: 7 })`) points into ROM and so has a non-zero
            // high byte, which is why dropping it was a silent miscompile
            // rather than merely a zero-page assumption.
            if matches!(
                &arg.node,
                crate::ast::Expr::StructInit { .. }
                    | crate::ast::Expr::AnonStructInit { .. }
                    | crate::ast::Expr::Call { .. }
            ) {
                generate_expr(arg, emitter, info, string_collector)?;
                emitter.emit_inst("STA", &format!("${:02X}", temp_addr));
                emitter.emit_inst("STX", &format!("${:02X}", temp_addr + 1));
                temp_offset += 2;
                arg_info.push((temp_addr, 2)); // 2-byte pointer
                continue;
            }
        }

        // Generate argument expression (result in A for 8-bit, A+Y for 16-bit)
        generate_expr(arg, emitter, info, string_collector)?;

        // Check if argument type is 8-bit but parameter is 16-bit (implicit cast)
        let arg_is_8bit = arg_type.is_some_and(|ty| {
            matches!(
                ty,
                crate::sema::types::Type::Primitive(crate::ast::PrimitiveType::U8)
                    | crate::sema::types::Type::Primitive(crate::ast::PrimitiveType::I8)
                    | crate::sema::types::Type::Primitive(crate::ast::PrimitiveType::B8)
                    | crate::sema::types::Type::Primitive(crate::ast::PrimitiveType::Bool)
            )
        });

        // Store to TEMPORARY location
        emitter.emit_inst("STA", &format!("${:02X}", temp_addr));
        if is_16bit {
            // For 16-bit parameters
            if arg_is_8bit {
                // Argument is 8-bit but parameter is 16-bit: zero-extend
                emitter.emit_inst("LDY", "#$00");
            }
            // Store high byte (in Y) to next location
            emitter.emit_inst("STY", &format!("${:02X}", temp_addr + 1));
            temp_offset += 2;
        } else {
            temp_offset += 1;
        }

        arg_info.push((temp_addr, if is_16bit { 2 } else { 1 }));
    }

    if sheltered > 0 {
        emitter.pop_frame(temp_base, sheltered);
    }

    // RECURSION SAVE: for a call inside a cycle, preserve the callee's frame
    // (which may hold the live values of an outer invocation) before we overwrite
    // its parameter slots. Done after argument evaluation and before the copy, so
    // the arguments (already parked in the temp pool) are unaffected.
    if is_recursive_edge && let Some(frame) = callee_frame {
        emitter.push_frame(frame.base, frame.size);
    }

    // STEP 2: Copy each argument (arg_size bytes) from temp storage to the
    // callee's parameter slots.
    let mut byte_offset = 0u8;
    for (temp_addr, arg_size) in arg_info.iter() {
        let param_addr = param_base + byte_offset;
        for k in 0..*arg_size {
            emitter.emit_inst("LDA", &format!("${:02X}", temp_addr + k));
            emitter.emit_inst("STA", &format!("${:02X}", param_addr + k));
        }
        byte_offset += arg_size;
    }

    // Free the temp storage after copying to parameters
    if total_bytes > 0 {
        emitter.temp_alloc.free_arg(temp_base, total_bytes);
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
        let high_byte_in_x = info
            .table
            .lookup(&function.node)
            .and_then(|sym| match &sym.ty {
                Type::Function(_, ret) => Some(matches!(
                    ret.as_ref(),
                    Type::Named(_) | Type::Pointer(_) | Type::String
                )),
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
enum CalleeSource<'a> {
    Location(crate::sema::table::SymbolLocation),
    Expr(&'a Spanned<Expr>),
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

    let is_16 = |ty: Option<&Type>| {
        matches!(
            ty,
            Some(Type::Primitive(
                crate::ast::PrimitiveType::U16
                    | crate::ast::PrimitiveType::I16
                    | crate::ast::PrimitiveType::B16
            ))
        )
    };

    emitter.emit_comment("Indirect call through function pointer");

    if !args.is_empty() {
        // Only scalar (1- or 2-byte) parameters are supported indirectly.
        for pty in param_types {
            if !matches!(pty, Type::Primitive(_)) {
                return Err(CodegenError::UnsupportedOperation(
                    "indirect calls only support scalar (u8/i8/u16/i16/bool) arguments".to_string(),
                ));
            }
        }
        let total: u8 = param_types
            .iter()
            .map(|p| if is_16(Some(p)) { 2 } else { 1 })
            .sum();
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
            CodegenError::Internal("argument-evaluation pool exhausted (indirect call)".to_string())
        })?;
        let mut off = 0u8;
        let mut placed = Vec::new();
        for (arg, pty) in args.iter().zip(param_types.iter()) {
            let p16 = is_16(Some(pty));
            let arg16 = is_16(info.resolved_types.get(&arg.span));
            generate_expr(arg, emitter, info, string_collector)?;
            emitter.emit_inst("STA", &format!("${:02X}", temp_base + off));
            if p16 {
                if !arg16 {
                    emitter.emit_inst("LDY", "#$00"); // zero-extend u8 arg -> u16 param
                }
                emitter.emit_inst("STY", &format!("${:02X}", temp_base + off + 1));
            }
            placed.push((temp_base + off, p16));
            off += if p16 { 2 } else { 1 };
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
/// The register conventions mirror `generate_call`: 16-bit scalars come back in
/// A:Y, pointer-like values (enums, strings) in A:X, and aggregates are copied
/// from the variable's own slot rather than evaluated into registers.
fn store_inline_arg(
    arg: &Spanned<Expr>,
    param_ty: &Type,
    dest: u8,
    emitter: &mut Emitter,
    info: &ProgramInfo,
    string_collector: &mut StringCollector,
) -> Result<(), CodegenError> {
    let arg_ty = info.resolved_types.get(&arg.span);

    // Aggregates are passed by reference: copy the descriptor straight out of
    // the argument variable's slot. A slice is 4 bytes (base + length); a struct
    // or array is a 2-byte pointer.
    let aggregate_bytes = match param_ty {
        Type::Slice(_) => Some(4u8),
        Type::Named(name) if !info.type_registry.enums.contains_key(name) => Some(2),
        Type::Array(_, _) => Some(2),
        _ => None,
    };
    if let Some(width) = aggregate_bytes {
        if let Expr::Variable(var_name) = &arg.node
            && let Some(sym) = info
                .resolved_symbols
                .get(&arg.span)
                .or_else(|| info.table.lookup(var_name))
            && let crate::sema::table::SymbolLocation::ZeroPage(src) = sym.location
        {
            for k in 0..width {
                emitter.emit_inst("LDA", &format!("${:02X}", src + k));
                emitter.emit_inst("STA", &format!("${:02X}", dest + k));
            }
            emitter.invalidate_registers();
            return Ok(());
        }
        return Err(CodegenError::UnsupportedOperation(format!(
            "passing this expression to an inline function's `{}` parameter is not supported;              use a variable",
            format_type_name(param_ty)
        )));
    }

    generate_expr(arg, emitter, info, string_collector)?;

    // Enums and strings are 2-byte pointers returned in A:X.
    let is_pointer_pair = match param_ty {
        Type::Named(name) => info.type_registry.enums.contains_key(name),
        Type::String | Type::Pointer(_) => true,
        _ => false,
    };
    if is_pointer_pair {
        emitter.emit_inst("STA", &format!("${:02X}", dest));
        emitter.emit_inst("STX", &format!("${:02X}", dest + 1));
        return Ok(());
    }

    // 16-bit scalars come back in A:Y. An 8-bit argument widening to a 16-bit
    // parameter has no high byte to store, so zero-extend it.
    let is_16bit = matches!(
        param_ty,
        Type::Primitive(crate::ast::PrimitiveType::U16)
            | Type::Primitive(crate::ast::PrimitiveType::I16)
            | Type::Primitive(crate::ast::PrimitiveType::B16)
    );
    emitter.emit_inst("STA", &format!("${:02X}", dest));
    if is_16bit {
        let arg_is_8bit = arg_ty.is_some_and(|ty| {
            matches!(
                ty,
                Type::Primitive(crate::ast::PrimitiveType::U8)
                    | Type::Primitive(crate::ast::PrimitiveType::I8)
                    | Type::Primitive(crate::ast::PrimitiveType::B8)
                    | Type::Primitive(crate::ast::PrimitiveType::Bool)
            )
        });
        if arg_is_8bit {
            emitter.emit_inst("LDY", "#$00");
        }
        emitter.emit_inst("STY", &format!("${:02X}", dest + 1));
    }
    Ok(())
}

/// A type's name for diagnostics.
fn format_type_name(ty: &Type) -> String {
    match ty {
        Type::Slice(_) => "slice".to_string(),
        Type::Pointer(p) => format!("&{}", p.display_name()),
        Type::Array(_, _) => "array".to_string(),
        Type::Named(n) => n.clone(),
        other => format!("{:?}", other),
    }
}

/// Generate parameter updates for tail recursive calls
///
/// This is similar to generate_call but WITHOUT the JSR instruction.
/// It evaluates arguments and updates parameter locations in place,
/// allowing a JMP back to the function start for tail call optimization.
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

    // Calculate total bytes needed for all arguments
    let mut total_bytes = 0u8;
    for arg in args.iter() {
        let arg_type = info.resolved_types.get(&arg.span);
        let is_16bit = arg_type.is_some_and(|ty| {
            matches!(
                ty,
                crate::sema::types::Type::Primitive(crate::ast::PrimitiveType::U16)
                    | crate::sema::types::Type::Primitive(crate::ast::PrimitiveType::I16)
                    | crate::sema::types::Type::Primitive(crate::ast::PrimitiveType::B16)
            )
        });
        total_bytes += if is_16bit { 2 } else { 1 };
    }

    // Allocate temp storage for all arguments at once
    let temp_base = if total_bytes == 0 {
        0
    } else {
        match emitter.temp_alloc.alloc_arg(total_bytes) {
            Some(addr) => addr,
            None => {
                return Err(CodegenError::Internal(
                    "argument-evaluation pool exhausted in tail-recursive update".to_string(),
                ));
            }
        }
    };
    let mut temp_offset = 0u8;
    let mut arg_info = Vec::new();

    for arg in args.iter() {
        let temp_addr = temp_base + temp_offset;

        // Check if this argument is a 16-bit type
        let arg_type = info.resolved_types.get(&arg.span);
        let is_16bit = arg_type.is_some_and(|ty| {
            matches!(
                ty,
                crate::sema::types::Type::Primitive(crate::ast::PrimitiveType::U16)
                    | crate::sema::types::Type::Primitive(crate::ast::PrimitiveType::I16)
                    | crate::sema::types::Type::Primitive(crate::ast::PrimitiveType::B16)
            )
        });

        // Generate argument expression (result in A for 8-bit, A+Y for 16-bit)
        generate_expr(arg, emitter, info, string_collector)?;

        // Store to TEMPORARY location
        emitter.emit_inst("STA", &format!("${:02X}", temp_addr));
        if is_16bit {
            // For 16-bit types, also store high byte (in Y) to next location
            emitter.emit_inst("STY", &format!("${:02X}", temp_addr + 1));
            temp_offset += 2;
        } else {
            temp_offset += 1;
        }

        arg_info.push((temp_addr, is_16bit));
    }

    // STEP 2: Copy arguments from temporary storage to parameter locations
    // Now we can safely update all parameters without conflicts
    let mut byte_offset = 0u8;
    for (temp_addr, is_16bit) in arg_info.iter() {
        let param_addr = param_base + byte_offset;

        // Copy from temp to param location
        emitter.emit_inst("LDA", &format!("${:02X}", temp_addr));
        emitter.emit_inst("STA", &format!("${:02X}", param_addr));

        if *is_16bit {
            // For 16-bit types, also copy high byte
            emitter.emit_inst("LDA", &format!("${:02X}", temp_addr + 1));
            emitter.emit_inst("STA", &format!("${:02X}", param_addr + 1));
            byte_offset += 2;
        } else {
            byte_offset += 1;
        }
    }

    // Free the temp storage after copying to parameters
    if total_bytes > 0 {
        emitter.temp_alloc.free_arg(temp_base, total_bytes);
    }

    // NOTE: No JSR instruction - caller will emit JMP to loop label

    Ok(())
}
