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
        return generate_indirect_call(loc, &ptypes, args, emitter, info, string_collector);
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

    // Get function parameter types from symbol table
    let param_types = if let Some(sym) = info.table.lookup(&function.node) {
        if let crate::sema::types::Type::Function(params, _) = &sym.ty {
            params.clone()
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };

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
        total_bytes += if is_16bit || is_struct || is_array || is_string {
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
                    "argument-evaluation pool exhausted calling '{}' ({} bytes of args); \
                     expression nesting is too deep",
                    function.node, total_bytes
                )));
            }
        }
    };
    let mut temp_offset = 0u8;
    let mut arg_info = Vec::new(); // Track argument sizes and temp locations

    for (i, arg) in args.iter().enumerate() {
        let temp_addr = temp_base + temp_offset;

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
            arg_info.push((temp_addr, true));
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
            arg_info.push((temp_addr, true));
            continue;
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
            arg_info.push((temp_addr, true)); // 2-byte pointer
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
                arg_info.push((temp_addr, true)); // 2-byte pointer
                continue;
            }
            // If not a simple variable, fall through to normal expression handling
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

        arg_info.push((temp_addr, is_16bit));
    }

    // RECURSION SAVE: for a call inside a cycle, preserve the callee's frame
    // (which may hold the live values of an outer invocation) before we overwrite
    // its parameter slots. Done after argument evaluation and before the copy, so
    // the arguments (already parked in the temp pool) are unaffected.
    if is_recursive_edge && let Some(frame) = callee_frame {
        emitter.push_frame(frame.base, frame.size);
    }

    // STEP 2: Copy arguments from temporary storage to the callee's parameter slots.
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

    // STEP 3: Call the function
    emitter.emit_inst("JSR", &function.node);

    // Invalidate register state after function call
    // (called function may modify any register; only A/Y contain known return value)
    emitter.reg_state.invalidate_all();

    // RECURSION RESTORE: restore the callee frame saved above. pop_frame clobbers
    // A and X, so stash the return low byte in $20 across the pop and reload it
    // (the high byte, if any, is in Y, which pop_frame leaves untouched). No
    // hardware stack is used.
    if is_recursive_edge && let Some(frame) = callee_frame {
        let tmp = emitter.memory_layout.temp_reg();
        emitter.emit_inst("STA", &format!("${:02X}", tmp));
        emitter.pop_frame(frame.base, frame.size);
        emitter.emit_inst("LDA", &format!("${:02X}", tmp));
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
fn generate_indirect_call(
    location: crate::sema::table::SymbolLocation,
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

    let addr = match location {
        SymbolLocation::ZeroPage(a) => a as u16,
        SymbolLocation::Absolute(a) => a,
        _ => {
            return Err(CodegenError::UnsupportedOperation(
                "function-pointer variable has no concrete storage location".to_string(),
            ));
        }
    };

    // Copy the pointer into the indirect vector $EE/$EF.
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
    emitter.needs_indirect_call = true;
    emitter.emit_inst("JSR", "__indirect_call");
    emitter.invalidate_registers();
    Ok(())
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
    for (i, arg) in args.iter().enumerate() {
        generate_expr(arg, emitter, info, string_collector)?;

        // Get the parameter info for this position
        let param = &params[i];

        // Look up the parameter's allocated location from inline_param_symbols
        if let Some(ref param_symbols) = metadata.inline_param_symbols {
            if let Some(param_info) = param_symbols.get(&param.name.span) {
                match param_info.location {
                    crate::sema::table::SymbolLocation::ZeroPage(addr) => {
                        emitter.emit_inst("STA", &format!("${:02X}", addr));
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

    // For inline functions from imported modules, we need to merge the parameter symbols
    // from the original module into the current ProgramInfo so the function body can
    // reference its parameters correctly
    let result = if let Some(ref param_symbols) = metadata.inline_param_symbols {
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
            type_registry: info.type_registry.clone(),
            resolved_types: info.resolved_types.clone(),
            imported_items: info.imported_items.clone(),
            warnings: info.warnings.clone(),
            unreachable_stmts: info.unreachable_stmts.clone(),
            tail_call_info: info.tail_call_info.clone(),
            resolved_struct_names: info.resolved_struct_names.clone(),
            string_pool: info.string_pool.clone(),
            function_frames: info.function_frames.clone(),
            recursive_call_edges: info.recursive_call_edges.clone(),
            interrupt_save_info: info.interrupt_save_info.clone(),
            address_taken_functions: info.address_taken_functions.clone(),
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

    // Pop inline context
    emitter.pop_inline();

    result
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
