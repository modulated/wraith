//! Statement Code Generation
//!
//! Compiles statements into assembly instructions.

pub(super) use crate::ast::{Spanned, Stmt};
pub(super) use crate::codegen::expr::generate_expr;
pub(super) use crate::codegen::{CodegenError, Emitter, StringCollector};
pub(super) use crate::sema::ProgramInfo;
pub(super) use rustc_hash::FxHashMap as HashMap;

mod asm_stmt;
mod assign;
mod loops;
mod match_stmt;

use asm_stmt::*;
use assign::*;
use loops::*;
pub(crate) use match_stmt::extract_enum_bindings;
use match_stmt::*;

/// Recognize an `if` condition that folds into a 65C02 bit-test-branch, and
/// return the `(mnemonic, zero-page byte)` to test-and-branch-to-`then`.
///
/// `if x.bit(n)` takes the `then` branch when bit n is set — `BBSn`. `if
/// !x.bit(n)` takes it when bit n is clear — `BBRn`. Both require the target
/// byte to be zero-page addressable (the Rockwell ops have no absolute form);
/// anything else (absolute/MMIO byte, ROM constant, indirect target, non-CMOS
/// target) returns `None` and the caller emits the mask-and-compare read.
fn fusible_bit_branch(
    condition: &Spanned<crate::ast::Expr>,
    emitter: &Emitter,
    info: &ProgramInfo,
) -> Option<(String, u8)> {
    use crate::ast::{BitOpKind, Expr, UnaryOp};

    if !emitter.target.has_rockwell_bit_ops() {
        return None;
    }

    // `!x.bit(n)` -> BBRn (branch if the bit is reset/clear).
    // `x.bit(n)`  -> BBSn (branch if the bit is set).
    let (negated, bitop) = match &condition.node {
        Expr::Unary {
            op: UnaryOp::Not,
            operand,
        } => (true, operand.as_ref()),
        _ => (false, condition),
    };

    let Expr::BitOp {
        object,
        kind: BitOpKind::Get,
        bit,
    } = &bitop.node
    else {
        return None;
    };

    let (zp, bit_in_byte) = crate::codegen::expr::bit_test_zp(object, bit, info)?;
    let mnem = if negated {
        format!("BBR{}", bit_in_byte)
    } else {
        format!("BBS{}", bit_in_byte)
    };
    Some((mnem, zp))
}

/// Check if a statement unconditionally terminates control flow
///
/// Used for dead code elimination - if a match arm ends with return/break/continue,
/// we don't need to emit a JMP to the match end label.
fn stmt_terminates(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::Return(_) | Stmt::Break | Stmt::Continue => true,
        Stmt::Block(stmts) => stmts
            .last()
            .map(|s| stmt_terminates(&s.node))
            .unwrap_or(false),
        Stmt::If {
            then_branch,
            else_branch: Some(else_branch),
            ..
        } => stmt_terminates(&then_branch.node) && stmt_terminates(&else_branch.node),
        Stmt::Match { arms, .. } => {
            // A match terminates if all arms terminate
            !arms.is_empty() && arms.iter().all(|arm| stmt_terminates(&arm.body.node))
        }
        _ => false,
    }
}

pub fn generate_stmt(
    stmt: &Spanned<Stmt>,
    emitter: &mut Emitter,
    info: &ProgramInfo,
    string_collector: &mut StringCollector,
) -> Result<(), CodegenError> {
    // Dead code elimination: skip unreachable statements
    if info.unreachable_stmts.contains(&stmt.span) {
        emitter.emit_comment("Unreachable code eliminated");
        return Ok(());
    }

    match &stmt.node {
        Stmt::Block(stmts) => {
            for s in stmts {
                generate_stmt(s, emitter, info, string_collector)?;
            }
            Ok(())
        }
        Stmt::VarDecl {
            name,
            ty: _,
            init,
            mutable: _,
        } => {
            // Look up variable info first
            if let Some(sym) = info.resolved_symbols.get(&name.span) {
                use crate::sema::table::SymbolKind;
                use crate::sema::types::Type;

                // A `str<N>` buffer: fill its RAM block from the literal, then
                // point the frame slot at the block. Downstream, the slot reads
                // as a 2-byte pointer to `[len][bytes]` — identical to a str
                // literal binding — so every str read path works unchanged. The
                // difference is the data lives in RAM and can be edited.
                if let Some(buf) = info.string_buffers.get(&name.span)
                    && let crate::sema::table::SymbolLocation::ZeroPage(slot) = sym.location
                {
                    let bytes: Vec<u8> = match &init.node {
                        crate::ast::Expr::Literal(crate::ast::Literal::String(s)) => {
                            s.as_bytes().to_vec()
                        }
                        // sema requires a string-literal initializer for a buffer
                        _ => Vec::new(),
                    };
                    let len = bytes.len() as u8;
                    emitter.emit_comment(&format!(
                        "str<{}> buffer {} @ ${:04X} ({}/{} bytes, RAM)",
                        buf.size - 1,
                        name.node,
                        buf.addr,
                        len,
                        buf.size - 1
                    ));
                    // Length prefix, then each content byte as an immediate store.
                    emitter.emit_inst("LDA", &format!("#${:02X}", len));
                    emitter.emit_inst("STA", &format!("${:04X}", buf.addr));
                    for (i, b) in bytes.iter().enumerate() {
                        emitter.emit_inst("LDA", &format!("#${:02X}", b));
                        emitter.emit_inst("STA", &format!("${:04X}", buf.addr as usize + 1 + i));
                    }
                    // Point the frame slot at the block.
                    emitter.emit_inst("LDA", &format!("#${:02X}", buf.addr & 0xFF));
                    emitter.emit_inst("STA", &format!("${:02X}", slot));
                    emitter.emit_inst("LDA", &format!("#${:02X}", buf.addr >> 8));
                    emitter.emit_inst("STA", &format!("${:02X}", slot + 1));
                    emitter.invalidate_registers();
                    return Ok(());
                }

                // Check if this is a struct variable initialized with a struct literal
                // Use runtime initialization for struct literals only (not enum variants)
                if let Type::Named(struct_name) = &sym.ty {
                    // Only use runtime init if the init expression is a struct literal
                    let is_struct_literal = matches!(
                        &init.node,
                        crate::ast::Expr::StructInit { .. }
                            | crate::ast::Expr::AnonStructInit { .. }
                    );

                    // Also verify this is actually a struct type (not an enum)
                    let is_struct_type = info.type_registry.get_struct(struct_name).is_some();

                    if is_struct_literal
                        && is_struct_type
                        && let crate::sema::table::SymbolLocation::ZeroPage(addr) = sym.location
                    {
                        // Get fields from the init expression
                        let fields = match &init.node {
                            crate::ast::Expr::StructInit { fields, .. } => fields,
                            crate::ast::Expr::AnonStructInit { fields } => fields,
                            _ => unreachable!(),
                        };

                        // Use runtime struct initialization directly to ZP address
                        crate::codegen::expr::generate_struct_init_runtime(
                            struct_name,
                            fields,
                            addr,
                            emitter,
                            info,
                            string_collector,
                        )?;
                        return Ok(());
                    }

                    // Struct-by-value initialization from a call, e.g.
                    // `let p: Point = make();`. A struct-returning function
                    // leaves a pointer to the struct bytes in A:X; copy the
                    // whole struct into this local's inline storage (frame
                    // coloring keeps the returned pointer valid until the next
                    // call, and the copy is the first thing after the call).
                    if is_struct_type
                        && matches!(&init.node, crate::ast::Expr::Call { .. })
                        && let crate::sema::table::SymbolLocation::ZeroPage(dest) = sym.location
                        && let Some(sdef) = info.type_registry.get_struct(struct_name)
                    {
                        let total = sdef.total_size as u8;
                        emitter.emit_comment(&format!(
                            "Struct return-by-value: copy {} bytes into ${:02X}",
                            total, dest
                        ));
                        generate_expr(init, emitter, info, string_collector)?;
                        emit_return_by_value_copy(emitter, dest, total);
                        emitter.invalidate_registers();
                        return Ok(());
                    }
                }

                // Local array: the data lives in RAM (see `LocalArray`), not
                // inline in the code stream where it used to be emitted and
                // where writes would have gone to ROM on a real board. Fill the
                // block from the initializer, then put its address in the
                // frame slot — every downstream path still reads the slot as a
                // 2-byte pointer, so indexing is unchanged.
                if let Some(arr) = info.local_arrays.get(&name.span)
                    && let crate::sema::table::SymbolLocation::ZeroPage(slot) = sym.location
                {
                    let elem_size = match &sym.ty {
                        Type::Array(elem, _) => {
                            crate::codegen::expr::type_byte_size(elem, info).max(1)
                        }
                        _ => 1,
                    };
                    emitter.emit_comment(&format!(
                        "local array {} @ ${:04X} ({} bytes, RAM)",
                        name.node, arr.addr, arr.size
                    ));
                    generate_local_array_init(arr.addr, arr.size, elem_size, init, emitter, info)?;
                    // Point the frame slot at the block.
                    emitter.emit_inst("LDA", &format!("#${:02X}", arr.addr & 0xFF));
                    emitter.emit_inst("STA", &format!("${:02X}", slot));
                    emitter.emit_inst("LDA", &format!("#${:02X}", arr.addr >> 8));
                    emitter.emit_inst("STA", &format!("${:02X}", slot + 1));
                    emitter.invalidate_registers();
                    return Ok(());
                }

                // Slice value: `let s: &[T] = arr[start..end];`. Materialize the
                // 4-byte fat-pointer descriptor into s's frame slot:
                //   slot[0..1] = base = arr's data pointer + start*elem_size
                //   slot[2..3] = len  = (end - start) in elements
                if let Type::Slice(elem) = &sym.ty
                    && let crate::ast::Expr::Slice {
                        object,
                        start,
                        end,
                        inclusive,
                    } = &init.node
                    && let crate::sema::table::SymbolLocation::ZeroPage(dest) = sym.location
                {
                    generate_slice_materialize(
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
                    return Ok(());
                }

                // Slice returned from a call: the callee left a pointer to its
                // 4-byte descriptor in A:X; copy the descriptor into this slot.
                if let Type::Slice(_) = &sym.ty
                    && matches!(&init.node, crate::ast::Expr::Call { .. })
                    && let crate::sema::table::SymbolLocation::ZeroPage(dest) = sym.location
                {
                    emitter.emit_comment("Slice return-by-value: copy 4-byte descriptor");
                    generate_expr(init, emitter, info, string_collector)?;
                    emit_return_by_value_copy(emitter, dest, 4);
                    emitter.invalidate_registers();
                    return Ok(());
                }

                // Array of structs: stored inline. Runtime-initialize each element
                // struct literal directly at addr + i*element_size.
                if let Type::Array(elem, _n) = &sym.ty
                    && let Type::Named(elem_struct) = &**elem
                    && let Some(sdef) = info.type_registry.get_struct(elem_struct)
                    && let crate::ast::Expr::Literal(crate::ast::Literal::Array(elements)) =
                        &init.node
                    && let crate::sema::table::SymbolLocation::ZeroPage(base) = sym.location
                {
                    let elem_size = sdef.total_size as u8;
                    emitter.emit_comment(&format!(
                        "Array of {} {}: {} elements inline at ${:02X}",
                        elem_struct,
                        "structs",
                        elements.len(),
                        base
                    ));
                    for (i, elem_expr) in elements.iter().enumerate() {
                        let elem_addr = base + (i as u8) * elem_size;
                        let fields = match &elem_expr.node {
                            crate::ast::Expr::StructInit { fields, .. }
                            | crate::ast::Expr::AnonStructInit { fields } => fields,
                            _ => {
                                return Err(CodegenError::UnsupportedOperation(
                                    "array-of-struct elements must be struct literals".to_string(),
                                ));
                            }
                        };
                        crate::codegen::expr::generate_struct_init_runtime(
                            elem_struct,
                            fields,
                            elem_addr,
                            emitter,
                            info,
                            string_collector,
                        )?;
                    }
                    return Ok(());
                }

                // Check for shorthand array syntax: [value] expanding to [value, value, ...]
                // If init is a single-element array and target is a larger array, synthesize an ArrayFill
                let modified_init;
                let init_expr = if let Type::Array(_, target_size) = &sym.ty {
                    if let crate::ast::Expr::Literal(crate::ast::Literal::Array(elements)) =
                        &init.node
                    {
                        if elements.len() == 1 && *target_size > 1 {
                            // Shorthand syntax detected! Convert to ArrayFill
                            emitter.emit_comment(&format!(
                                "Expanding [value] to [{} elements]",
                                target_size
                            ));
                            modified_init = crate::ast::Spanned {
                                node: crate::ast::Expr::Literal(crate::ast::Literal::ArrayFill {
                                    value: Box::new(elements[0].clone()),
                                    count: *target_size,
                                }),
                                span: init.span,
                            };
                            &modified_init
                        } else {
                            init
                        }
                    } else {
                        init
                    }
                } else {
                    init
                };

                // Generate initialization expression (result in A, and X if u16)
                generate_expr(init_expr, emitter, info, string_collector)?;

                // Check if we need to zero-extend (u8 -> u16)
                // Get the init expression type from resolved_types
                let init_type = info.resolved_types.get(&init.span);
                let target_type = &sym.ty;

                let needs_zero_extend = if let Some(init_ty) = init_type {
                    matches!(init_ty, Type::Primitive(crate::ast::PrimitiveType::U8))
                        && matches!(
                            target_type,
                            Type::Primitive(crate::ast::PrimitiveType::U16)
                                | Type::Primitive(crate::ast::PrimitiveType::I16)
                                | Type::Primitive(crate::ast::PrimitiveType::B16)
                        )
                } else {
                    false
                };

                // If we need to zero-extend, set Y=0 for the high byte
                if needs_zero_extend {
                    emitter.emit_inst("LDY", "#$00");
                }

                // Check if this is a multi-byte type (arrays, u16, i16, b16, enums)
                // Enums store a 2-byte pointer like arrays
                let is_enum = if let Type::Named(type_name) = &sym.ty {
                    info.type_registry.get_enum(type_name).is_some()
                } else {
                    false
                };

                let is_multibyte = matches!(
                    sym.ty,
                    Type::Array(_, _)
                        | Type::String
                        | Type::Pointer(_)
                        | Type::Function(_, _)
                        | Type::Primitive(crate::ast::PrimitiveType::U16)
                        | Type::Primitive(crate::ast::PrimitiveType::I16)
                        | Type::Primitive(crate::ast::PrimitiveType::B16)
                ) || is_enum;

                // Arrays, enums, strings and pointers carry an address in
                // A (low) and X (high); other 16-bit values use A (low) and
                // Y (high). Storing only A leaves the high byte as whatever was
                // in the slot, which reads as a pointer into an arbitrary page.
                let is_array_or_enum =
                    matches!(sym.ty, Type::Array(_, _) | Type::String | Type::Pointer(_))
                        || is_enum;

                // An enum value binds by *copy*: the constructed bytes live in
                // shared codegen scratch until here (and a returned enum points
                // into the callee's reused region), so without the copy two
                // live enums alias and any later temp use destroys the payload.
                // The variable's slot points at its own per-declaration block
                // from here on.
                let enum_block = if is_enum {
                    info.enum_blocks.get(&name.span).cloned()
                } else {
                    None
                };
                if let Some(block) = &enum_block {
                    emit_enum_copy_to_block(block, emitter)?;
                }

                match sym.location {
                    crate::sema::table::SymbolLocation::FrameOffset(_) => {
                        return Err(CodegenError::Internal(
                            "unresolved FrameOffset reached codegen (frame finalization skipped)"
                                .to_string(),
                        ));
                    }
                    crate::sema::table::SymbolLocation::Absolute(addr) => {
                        // Check if this is an address declaration - use symbolic name
                        if sym.kind == SymbolKind::Address {
                            emitter.emit_sta_symbol(&name.node);
                        } else {
                            emitter.emit_sta_abs(addr);
                            // For multi-byte types, also store high byte
                            if is_multibyte {
                                let hi_inst = if is_array_or_enum { "STX" } else { "STY" };
                                emitter.emit_inst(hi_inst, &format!("${:04X}", addr + 1));
                            }
                        }
                    }
                    crate::sema::table::SymbolLocation::ZeroPage(addr) => {
                        emitter.emit_sta_zp(addr);
                        // For multi-byte types, also store high byte
                        if is_multibyte {
                            let hi_inst = if is_array_or_enum { "STX" } else { "STY" };
                            emitter.emit_inst(hi_inst, &format!("${:02X}", addr + 1));
                        }
                    }
                    crate::sema::table::SymbolLocation::None => {
                        return Err(CodegenError::UnsupportedOperation(format!(
                            "VarDecl '{}' has no storage location",
                            name.node
                        )));
                    }
                }
            } else {
                return Err(CodegenError::SymbolNotFound(name.node.clone()));
            }
            Ok(())
        }
        Stmt::Return(expr) => {
            if let Some(e) = expr {
                // Check if this is a tail recursive call
                // Pattern: return func(...) where func is the current function
                let is_tail_recursive = if let crate::ast::Expr::Call { function, .. } = &e.node {
                    // Check if calling the same function we're currently in
                    emitter
                        .current_function()
                        .map(|current_fn| current_fn == function.node.as_str())
                        .unwrap_or(false)
                } else {
                    false
                };

                if is_tail_recursive {
                    // Tail recursive call optimization: convert to loop
                    // Generate the call expression which will:
                    // 1. Evaluate arguments
                    // 2. Store them to parameter locations
                    // 3. Call the function with JSR
                    // But we'll intercept this and generate different code

                    // For now, extract the function call and generate optimized code
                    if let crate::ast::Expr::Call { function, args } = &e.node {
                        emitter.emit_comment(&format!(
                            "Tail recursive call to {} - optimized to loop",
                            function.node
                        ));

                        // Evaluate arguments and store to parameter locations
                        // This is similar to what generate_call does, but without JSR
                        crate::codegen::expr::generate_tail_recursive_update(
                            function,
                            args,
                            emitter,
                            info,
                            string_collector,
                        )?;

                        // Jump back to function start instead of JSR
                        if let Some(loop_label) = emitter.tail_call_loop_label() {
                            emitter.emit_inst("JMP", &loop_label);
                        } else {
                            // Fallback: this shouldn't happen if tail call detection worked
                            return Err(CodegenError::UnsupportedOperation(
                                "Tail recursive call without loop label".to_string(),
                            ));
                        }
                    }
                } else {
                    // Slice return-by-value: a slice variable returns a pointer
                    // to its 4-byte descriptor in A:X (the descriptor lives in the
                    // callee frame, valid until the caller copies it out right
                    // after the call). Mirrors struct return-by-value.
                    let slice_var_return = matches!(
                        info.resolved_types.get(&e.span),
                        Some(crate::sema::types::Type::Slice(_))
                    )
                    .then(|| {
                        if let crate::ast::Expr::Variable(vname) = &e.node
                            && let Some(sym) = info
                                .resolved_symbols
                                .get(&e.span)
                                .or_else(|| info.table.lookup(vname))
                            && let crate::sema::table::SymbolLocation::ZeroPage(addr) = sym.location
                        {
                            Some(addr)
                        } else {
                            None
                        }
                    })
                    .flatten();

                    if let Some(addr) = slice_var_return {
                        emitter.emit_comment("Return slice descriptor pointer (A:X)");
                        emitter.emit_inst("LDA", &format!("#${:02X}", addr));
                        emitter.emit_inst("LDX", "#$00");
                        emitter.mark_a_unknown();
                    } else {
                        // Normal return with value
                        generate_expr(e, emitter, info, string_collector)?;
                    }

                    // A 16-bit return convention is A (low) / Y (high). If the
                    // returned expression is 8-bit (e.g. `return 255;` from a
                    // `-> u16` function, where the literal fits u8), Y still
                    // holds junk from earlier code - extend it explicitly.
                    {
                        use crate::ast::PrimitiveType;
                        use crate::sema::types::Type;
                        let ret_ty = emitter
                            .current_function()
                            .and_then(|name| info.table.lookup(name))
                            .and_then(|sym| match &sym.ty {
                                Type::Function(_, ret) => Some((**ret).clone()),
                                _ => None,
                            });
                        let expr_is_8bit = info.resolved_types.get(&e.span).is_some_and(|t| {
                            matches!(
                                t,
                                Type::Primitive(
                                    PrimitiveType::U8
                                        | PrimitiveType::I8
                                        | PrimitiveType::B8
                                        | PrimitiveType::Bool
                                )
                            )
                        });
                        match ret_ty {
                            Some(Type::Primitive(PrimitiveType::U16 | PrimitiveType::B16))
                                if expr_is_8bit =>
                            {
                                emitter.emit_inst("LDY", "#$00"); // zero-extend
                            }
                            Some(Type::Primitive(PrimitiveType::I16)) if expr_is_8bit => {
                                // Sign-extend A into Y without destroying A.
                                let pos_label = emitter.next_label("sx");
                                emitter.emit_inst("LDY", "#$00");
                                emitter.emit_inst("CMP", "#$80");
                                emitter.emit_inst("BCC", &pos_label);
                                emitter.emit_inst("DEY", ""); // Y = $FF
                                emitter.emit_label(&pos_label);
                            }
                            _ => {}
                        }
                    }

                    // Only emit RTS if we're not in an inline context
                    if !emitter.is_inlining() {
                        emitter.emit_inst("RTS", "");
                    } else if let Some(end) = emitter.inline_end_label() {
                        // An inline expansion has no frame to return from; an
                        // early return jumps past the rest of the body so the
                        // value just computed survives.
                        let end = end.to_string();
                        emitter.emit_inst("JMP", &end);
                    }
                }
            } else {
                // Return with no value
                if !emitter.is_inlining() {
                    emitter.emit_inst("RTS", "");
                } else if let Some(end) = emitter.inline_end_label() {
                    let end = end.to_string();
                    emitter.emit_inst("JMP", &end);
                }
            }
            Ok(())
        }
        Stmt::Assign { target, value } => {
            // Slice reassignment: `s = arr[a..b];` materializes a new descriptor
            // into the slice variable's slot (same as the `let` form).
            if let crate::ast::Expr::Variable(target_name) = &target.node
                && let crate::ast::Expr::Slice {
                    object,
                    start,
                    end,
                    inclusive,
                } = &value.node
                && let Some(sym) = info
                    .resolved_symbols
                    .get(&target.span)
                    .or_else(|| info.table.lookup(target_name))
                && let crate::sema::types::Type::Slice(elem) = &sym.ty
                && let crate::sema::table::SymbolLocation::ZeroPage(dest) = sym.location
            {
                generate_slice_materialize(
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
                return Ok(());
            }

            // Optimization: detect x = x + 1 and x = x - 1 patterns
            // Use INC/DEC instead of LDA/ADC/STA or LDA/SBC/STA
            if let crate::ast::Expr::Variable(target_name) = &target.node
                && let crate::ast::Expr::Binary { left, op, right } = &value.node
            {
                // Check if left side is the same variable as target
                if let crate::ast::Expr::Variable(left_name) = &left.node
                    && left_name == target_name
                {
                    // Check if right side is literal 1
                    if let crate::ast::Expr::Literal(crate::ast::Literal::Integer(n)) = &right.node
                        && *n == 1
                    {
                        // Look up variable location
                        let sym = info
                            .resolved_symbols
                            .get(&target.span)
                            .or_else(|| info.table.lookup(target_name));

                        // INC/DEC operate on a single byte and do not touch the
                        // carry, so they cannot implement ±1 on a multi-byte
                        // value: `a = a + 1` on a u16 holding $00FF must carry
                        // into the high byte, which INC alone never does.
                        let is_single_byte = sym.is_some_and(|s| {
                            matches!(
                                s.ty,
                                crate::sema::types::Type::Primitive(
                                    crate::ast::PrimitiveType::U8
                                        | crate::ast::PrimitiveType::I8
                                        | crate::ast::PrimitiveType::B8
                                        | crate::ast::PrimitiveType::Bool
                                )
                            )
                        });

                        if let Some(sym) = sym
                            && is_single_byte
                        {
                            match (op, &sym.location) {
                                (
                                    crate::ast::BinaryOp::Add,
                                    crate::sema::table::SymbolLocation::ZeroPage(addr),
                                ) => {
                                    // x = x + 1 -> INC $addr
                                    emitter.emit_inst("INC", &format!("${:02X}", *addr));
                                    emitter.reg_state.invalidate_zero_page(*addr);
                                    return Ok(());
                                }
                                (
                                    crate::ast::BinaryOp::Add,
                                    crate::sema::table::SymbolLocation::Absolute(addr),
                                ) => {
                                    // x = x + 1 -> INC $addr
                                    emitter.emit_inst("INC", &format!("${:04X}", *addr));
                                    emitter.reg_state.invalidate_memory(*addr);
                                    return Ok(());
                                }
                                (
                                    crate::ast::BinaryOp::Sub,
                                    crate::sema::table::SymbolLocation::ZeroPage(addr),
                                ) => {
                                    // x = x - 1 -> DEC $addr
                                    emitter.emit_inst("DEC", &format!("${:02X}", *addr));
                                    emitter.reg_state.invalidate_zero_page(*addr);
                                    return Ok(());
                                }
                                (
                                    crate::ast::BinaryOp::Sub,
                                    crate::sema::table::SymbolLocation::Absolute(addr),
                                ) => {
                                    // x = x - 1 -> DEC $addr
                                    emitter.emit_inst("DEC", &format!("${:04X}", *addr));
                                    emitter.reg_state.invalidate_memory(*addr);
                                    return Ok(());
                                }
                                _ => {
                                    // Not an INC/DEC pattern, fall through to normal codegen
                                }
                            }
                        }
                    }
                }
            }

            // `*p = v` — write through a pointer. Handled before the value is
            // evaluated below, because the pointer has to be staged first and
            // the generic path assumes the target is a name.
            if let crate::ast::Expr::Unary {
                op: crate::ast::UnaryOp::Deref,
                operand,
            } = &target.node
            {
                return generate_deref_assignment(operand, value, emitter, info, string_collector);
            }

            // 1. Generate code for value (result in A)
            generate_expr(value, emitter, info, string_collector)?;

            // 2. Store A into target
            // We need a helper to generate store instructions based on target
            match &target.node {
                crate::ast::Expr::Variable(name) => {
                    // Look up by span in resolved_symbols first (for local vars)
                    let sym = info
                        .resolved_symbols
                        .get(&target.span)
                        .or_else(|| info.table.lookup(name)); // Fallback to global table

                    if let Some(sym) = sym {
                        use crate::sema::table::SymbolKind;
                        use crate::sema::types::Type;

                        // Struct-by-value assignment from a call, e.g.
                        // `p = make();`. The value expression (already generated
                        // above) left a pointer to the struct bytes in A:X; copy
                        // the whole struct into the target's inline storage
                        // rather than storing just the low byte of the pointer.
                        if matches!(&value.node, crate::ast::Expr::Call { .. })
                            && let Type::Named(sname) = &sym.ty
                            && let Some(sdef) = info.type_registry.get_struct(sname)
                            && let crate::sema::table::SymbolLocation::ZeroPage(dest) = sym.location
                        {
                            let total = sdef.total_size as u8;
                            emitter.emit_comment(&format!(
                                "Struct return-by-value assign: copy {} bytes into ${:02X}",
                                total, dest
                            ));
                            emit_return_by_value_copy(emitter, dest, total);
                            emitter.invalidate_registers();
                            return Ok(());
                        }

                        // Check if this is an enum type
                        let is_enum = if let Type::Named(type_name) = &sym.ty {
                            info.type_registry.get_enum(type_name).is_some()
                        } else {
                            false
                        };

                        // Check if this is a multi-byte type (u16/i16/b16, arrays,
                        // enums, function pointers)
                        let is_multibyte = matches!(
                            sym.ty,
                            Type::Array(_, _)
                                | Type::Pointer(_)
                                | Type::Function(_, _)
                                | Type::Primitive(crate::ast::PrimitiveType::U16)
                                | Type::Primitive(crate::ast::PrimitiveType::I16)
                                | Type::Primitive(crate::ast::PrimitiveType::B16)
                        ) || is_enum;

                        // Arrays, enums and pointers carry an address in
                        // A (low) and X (high); other 16-bit values use A:Y.
                        let is_array_or_enum =
                            matches!(sym.ty, Type::Array(_, _) | Type::Pointer(_)) || is_enum;

                        // Same copy-on-bind as the VarDecl path: an enum
                        // reassignment must land in the variable's own block,
                        // found through the declaration's span. A target
                        // without a block (a parameter) keeps the old
                        // pointer-store behavior.
                        if is_enum
                            && let Some(block) = sym
                                .decl_span
                                .and_then(|sp| info.enum_blocks.get(&sp))
                                .cloned()
                        {
                            emit_enum_copy_to_block(&block, emitter)?;
                        }

                        match sym.location {
                            crate::sema::table::SymbolLocation::FrameOffset(_) => {
                                return Err(CodegenError::Internal(
                                    "unresolved FrameOffset reached codegen (frame finalization skipped)"
                                        .to_string(),
                                ));
                            }
                            crate::sema::table::SymbolLocation::Absolute(addr) => {
                                // Check if this is an address declaration - use symbolic name
                                if sym.kind == SymbolKind::Address {
                                    emitter.emit_sta_symbol(name);
                                } else {
                                    emitter.emit_sta_abs(addr);
                                    // For multi-byte types, also store high byte
                                    if is_multibyte {
                                        let hi_inst = if is_array_or_enum { "STX" } else { "STY" };
                                        emitter.emit_inst(hi_inst, &format!("${:04X}", addr + 1));
                                        // Raw STX/STY overwrites the high byte without
                                        // updating tracking; forget any register cached
                                        // to it so a later load isn't wrongly elided.
                                        emitter.invalidate_abs(addr + 1);
                                    }
                                }
                            }
                            crate::sema::table::SymbolLocation::ZeroPage(addr) => {
                                emitter.emit_sta_zp(addr);
                                // For multi-byte types, also store high byte
                                if is_multibyte {
                                    let hi_inst = if is_array_or_enum { "STX" } else { "STY" };
                                    emitter.emit_inst(hi_inst, &format!("${:02X}", addr + 1));
                                    // Raw STX/STY overwrites the high byte without
                                    // updating tracking; forget any register cached
                                    // to it so a later load isn't wrongly elided.
                                    emitter.invalidate_zp(addr + 1);
                                }
                            }
                            crate::sema::table::SymbolLocation::None => {
                                return Err(CodegenError::UnsupportedOperation(format!(
                                    "Variable '{}' has no storage location",
                                    name
                                )));
                            }
                        }
                    } else {
                        return Err(CodegenError::SymbolNotFound(name.clone()));
                    }
                }
                crate::ast::Expr::Index { object, index } => {
                    generate_index_assignment(
                        object,
                        index,
                        value,
                        emitter,
                        info,
                        string_collector,
                    )?;
                }
                crate::ast::Expr::Field { object, field } => {
                    generate_field_assignment(
                        object,
                        field,
                        value,
                        emitter,
                        info,
                        string_collector,
                    )?;
                }
                // A `.len`/`.low`/`.high` target that sema re-resolved as a
                // struct field access stores like a plain field.
                crate::ast::Expr::SliceLen(object)
                    if info.accessor_fields.contains(&target.span) =>
                {
                    let field = crate::ast::Spanned::new("len".to_string(), target.span);
                    generate_field_assignment(
                        object,
                        &field,
                        value,
                        emitter,
                        info,
                        string_collector,
                    )?;
                }
                crate::ast::Expr::U16Low(object) if info.accessor_fields.contains(&target.span) => {
                    let field = crate::ast::Spanned::new("low".to_string(), target.span);
                    generate_field_assignment(
                        object,
                        &field,
                        value,
                        emitter,
                        info,
                        string_collector,
                    )?;
                }
                crate::ast::Expr::U16High(object)
                    if info.accessor_fields.contains(&target.span) =>
                {
                    let field = crate::ast::Spanned::new("high".to_string(), target.span);
                    generate_field_assignment(
                        object,
                        &field,
                        value,
                        emitter,
                        info,
                        string_collector,
                    )?;
                }
                crate::ast::Expr::Slice {
                    object,
                    start,
                    end,
                    inclusive,
                } => {
                    generate_slice_assignment(
                        object,
                        start,
                        end,
                        *inclusive,
                        value,
                        emitter,
                        info,
                        string_collector,
                    )?;
                }
                _ => {
                    return Err(CodegenError::UnsupportedOperation(
                        "Only variable, index, field, and slice assignment supported".to_string(),
                    ));
                }
            }
            Ok(())
        }
        Stmt::Expr(expr) => {
            generate_expr(expr, emitter, info, string_collector)?;
            Ok(())
        }
        Stmt::If {
            condition,
            then_branch,
            else_branch,
        } => {
            let then_label = emitter.next_label("then");
            let else_label = emitter.next_label("else");
            let end_label = emitter.next_label("end");

            // 65C02 fusion: `if x.bit(n)` and `if !x.bit(n)` on a zero-page byte
            // fold the mask-and-compare read into a single bit-test-branch
            // (`BBSn`/`BBRn`). The branch-over-jump shape below keeps the `then`
            // target always within a few bytes, so the ±127 reach is never a
            // concern and no range fallback is needed.
            if let Some((mnem, zp)) = fusible_bit_branch(condition, emitter, info) {
                if !emitter.is_minimal() {
                    emitter.emit_comment("Bit-test branch to then");
                }
                emitter.emit_inst(&mnem, &format!("${:02X},{}", zp, then_label));
                emitter.emit_inst("JMP", &else_label);
            } else {
                // Condition
                generate_expr(condition, emitter, info, string_collector)?;

                // For large if statements, we need to avoid forward branches
                // that might exceed 127 bytes. Use this structure:
                //   condition
                //   BNE then      ; branch if true (short forward jump)
                //   JMP else      ; jump if false
                // then:
                //   then_body
                //   JMP end
                // else:
                //   else_body
                // end:

                if !emitter.is_minimal() {
                    emitter.emit_comment("Branch to then if condition is true");
                }
                emitter.emit_inst("CMP", "#$00");
                emitter.emit_inst("BNE", &then_label);
                emitter.emit_inst("JMP", &else_label);
            }

            // Then
            emitter.emit_label(&then_label);
            generate_stmt(then_branch, emitter, info, string_collector)?;
            emitter.emit_inst("JMP", &end_label);

            // Else
            emitter.emit_label(&else_label);
            if let Some(else_b) = else_branch {
                generate_stmt(else_b, emitter, info, string_collector)?;
            }

            // End
            emitter.emit_label(&end_label);
            // Invalidate register state after control flow merge
            // (we don't know which branch was taken)
            emitter.reg_state.invalidate_all();
            Ok(())
        }
        Stmt::While { condition, body } => {
            let body_label = emitter.next_label("wb");
            let check_label = emitter.next_label("wc");
            let end_label = emitter.next_label("we");

            // Structure that avoids long branches:
            // check:
            //   ...condition...
            //   BNE body      ; branch if true (2 bytes forward to body label)
            //   JMP end       ; exit if false (3 bytes)
            // body:
            //   ...body...    ; can be any size
            //   JMP check     ; 3 bytes back
            // end:
            //
            // The BNE only needs to jump 3 bytes forward (past the JMP),
            // so it's always within the 127-byte limit regardless of body size.

            // Condition check
            emitter.emit_label(&check_label);
            // check_label is a back-edge target: register state from before
            // the loop (or from the previous iteration) is stale here.
            emitter.reg_state.invalidate_all();
            generate_expr(condition, emitter, info, string_collector)?;

            if !emitter.is_minimal() {
                emitter.emit_comment("Continue to body if condition is true");
            }
            emitter.emit_inst("CMP", "#$00");
            // BNE jumps only 3 bytes forward (size of JMP instruction)
            // This is always within the 127-byte branch limit
            emitter.emit_inst("BNE", &body_label);
            emitter.emit_inst("JMP", &end_label);

            emitter.emit_label(&body_label);

            // Push loop context for break/continue
            emitter.push_loop(check_label.clone(), end_label.clone());

            // Body
            generate_stmt(body, emitter, info, string_collector)?;

            // Pop loop context
            emitter.pop_loop();

            // Jump back to condition check
            emitter.emit_inst("JMP", &check_label);

            emitter.emit_label(&end_label);
            // Invalidate register state after loop end
            emitter.reg_state.invalidate_all();
            Ok(())
        }
        Stmt::Loop { body } => {
            let loop_label = emitter.next_label("lp");
            let end_label = emitter.next_label("lx");

            emitter.emit_label(&loop_label);
            // loop_label is a back-edge target: register state from before
            // the loop (or from the previous iteration) is stale here.
            emitter.reg_state.invalidate_all();

            // Push loop context for break/continue
            emitter.push_loop(loop_label.clone(), end_label.clone());

            generate_stmt(body, emitter, info, string_collector)?;

            // Pop loop context
            emitter.pop_loop();

            emitter.emit_inst("JMP", &loop_label);
            emitter.emit_label(&end_label);
            Ok(())
        }
        Stmt::For {
            var_name,
            var_type: _,
            range,
            body,
        } => {
            // Check if loop can be unrolled (constant bounds, small count)
            let start_const = info.folded_constants.get(&range.start.span);
            let end_const = info.folded_constants.get(&range.end.span);

            // Threshold for unrolling: 8 iterations or fewer
            const UNROLL_THRESHOLD: i64 = 8;

            if let (
                Some(crate::sema::const_eval::ConstValue::Integer(start)),
                Some(crate::sema::const_eval::ConstValue::Integer(end)),
            ) = (start_const, end_const)
            {
                // Calculate iteration count
                let count = if range.inclusive {
                    end - start + 1
                } else {
                    end - start
                };

                if count > 0 && count <= UNROLL_THRESHOLD {
                    // LOOP UNROLLING: Generate inline code for small constant loops
                    emitter.emit_comment(&format!(
                        "Loop unrolled: {} iteration{}",
                        count,
                        if count == 1 { "" } else { "s" }
                    ));

                    // Resolve the loop variable's actual frame slot (registered at
                    // its declaration span during analysis) rather than assuming the
                    // first variable address.
                    let loop_var_addr = match info
                        .resolved_symbols
                        .get(&var_name.span)
                        .map(|s| &s.location)
                    {
                        Some(crate::sema::table::SymbolLocation::ZeroPage(addr)) => *addr,
                        _ => {
                            return Err(CodegenError::Internal(format!(
                                "unrolled loop variable '{}' has no zero-page frame slot",
                                var_name.node
                            )));
                        }
                    };

                    // A 16-bit loop variable needs its high byte written too,
                    // or body reads of the counter see a garbage high byte.
                    let loop_var_is_16bit =
                        info.resolved_symbols.get(&var_name.span).is_some_and(|s| {
                            matches!(
                                s.ty,
                                crate::sema::types::Type::Primitive(
                                    crate::ast::PrimitiveType::U16
                                        | crate::ast::PrimitiveType::I16
                                        | crate::ast::PrimitiveType::B16
                                )
                            )
                        });

                    // Create end label for break statements
                    let end_label = emitter.next_label("ux");

                    // Generate body for each iteration with loop variable set
                    for i in 0..count {
                        let iter_val = start + i;

                        // Set loop variable to current iteration value
                        emitter.emit_comment(&format!("{} = {}", var_name.node, iter_val));
                        emitter.emit_inst("LDA", &format!("#${:02X}", iter_val as u8));
                        emitter.emit_inst("STA", &format!("${:02X}", loop_var_addr));
                        if loop_var_is_16bit {
                            emitter.emit_inst("LDA", &format!("#${:02X}", (iter_val >> 8) as u8));
                            emitter.emit_inst(
                                "STA",
                                &format!("${:02X}", loop_var_addr.wrapping_add(1)),
                            );
                        }

                        // Create iteration label for continue statements
                        let iter_label = emitter.next_label("ui");

                        // Push loop context so break/continue work
                        emitter.push_loop(iter_label.clone(), end_label.clone());

                        // Execute body
                        emitter.reg_state.invalidate_all();
                        generate_stmt(body, emitter, info, string_collector)?;

                        // Pop loop context
                        emitter.pop_loop();

                        // Emit iteration label for continue
                        emitter.emit_label(&iter_label);
                    }

                    // Emit end label for break
                    emitter.emit_label(&end_label);

                    return Ok(());
                }
            }

            // NORMAL LOOP: Generate standard loop code
            generate_normal_loop(var_name, range, body, emitter, info, string_collector)
        }
        Stmt::ForEach {
            var_name,
            var_type: _,
            iterable,
            body,
            index_var,
        } => {
            // ForEach loop: for item in iterable { ... } or for (index, item) in iterable { ... }
            // Supports arrays and strings
            // Strategy:
            // 1. Evaluate iterable expression to get pointer
            // 2. Use X register as loop counter (0..length)
            // 3. Load iterable[X] into the loop variable
            // 4. Store X in index variable if present
            // 5. Execute body
            // 6. Increment X and loop

            emitter.emit_comment("ForEach loop");

            // Generate the iterable expression (can be array or string variable)
            let (iterable_info, is_string) = match &iterable.node {
                crate::ast::Expr::Variable(name) => {
                    // Look up the variable to get its pointer location and type
                    let sym = info
                        .resolved_symbols
                        .get(&iterable.span)
                        .or_else(|| info.table.lookup(name))
                        .ok_or_else(|| CodegenError::SymbolNotFound(name.clone()))?;

                    // Check if it's an array or string
                    let is_str = matches!(sym.ty, crate::sema::types::Type::String);

                    // Get the location where the pointer is stored
                    let ptr_loc = match sym.location {
                        crate::sema::table::SymbolLocation::ZeroPage(addr) => addr,
                        crate::sema::table::SymbolLocation::Absolute(addr) if addr < 256 => {
                            addr as u8
                        }
                        _ => {
                            return Err(CodegenError::UnsupportedOperation(
                                "ForEach requires pointer in zero page".to_string(),
                            ));
                        }
                    };

                    ((ptr_loc, sym.ty.clone()), is_str)
                }
                _ => {
                    return Err(CodegenError::UnsupportedOperation(
                        "ForEach only supports variables currently".to_string(),
                    ));
                }
            };

            let (iterable_base, iterable_ty) = iterable_info;
            let is_slice = matches!(&iterable_ty, crate::sema::types::Type::Slice(_));

            let loop_label = emitter.next_label("fe");
            let continue_label = emitter.next_label("fc");
            let end_label = emitter.next_label("fz");

            // Slices iterate with a 16-bit counter (they can exceed 255 elements),
            // so they use a dedicated loop that re-reads the descriptor each
            // iteration and advances an element pointer. Handled separately here.
            if is_slice {
                generate_foreach_slice(
                    iterable,
                    iterable_base,
                    &iterable_ty,
                    var_name,
                    index_var.as_ref(),
                    body,
                    &loop_label,
                    &continue_label,
                    &end_label,
                    emitter,
                    info,
                    string_collector,
                )?;
                return Ok(());
            }

            // The loop counter lives in a hidden zero-page slot, not just in X,
            // so the body may clobber X freely (a u8 multiply uses it as a bit
            // counter, a nested loop as its own counter) without corrupting the
            // iteration. X is reloaded from the slot at each loop head and the
            // slot is advanced with INC at continue.
            let counter = match info
                .loop_bound_slots
                .get(&iterable.span)
                .map(|s| &s.location)
            {
                Some(crate::sema::table::SymbolLocation::ZeroPage(a)) => *a,
                _ => {
                    return Err(CodegenError::Internal(
                        "string/array foreach: counter slot was not allocated".to_string(),
                    ));
                }
            };
            emitter.emit_inst("LDA", "#$00");
            emitter.emit_inst("STA", &format!("${:02X}", counter));

            // For arrays the size is a compile-time constant. Strings and
            // slices have a runtime length, staged (with the string pointer)
            // at the loop *head* below.
            let array_size = if is_string || is_slice {
                None
            } else {
                match &iterable_ty {
                    crate::sema::types::Type::Array(_, sz) => Some(*sz),
                    _ => {
                        return Err(CodegenError::UnsupportedOperation(
                            "ForEach requires array, slice, or string type".to_string(),
                        ));
                    }
                }
            };

            // Loop start
            emitter.emit_label(&loop_label);

            // Reload the counter into X. The loop head is a branch target (entry
            // and back-edge), and the body may have left anything in X, so no
            // prior belief holds and X must come from the slot.
            emitter.invalidate_registers();
            emitter.emit_inst("LDX", &format!("${:02X}", counter));

            // String/slice staging ($F0-$F2) lives only from here to the
            // element read — never across the body, which is arbitrary code
            // and may use those bytes itself (an earlier version staged once
            // before the loop, and a body's index assignment silently
            // destroyed the string pointer). It is therefore re-staged on
            // every iteration, which costs a handful of cycles per element.
            if is_string {
                // Re-stage the string pointer and read the length prefix.
                emitter.emit_comment("String iteration - load length");
                emitter.emit_inst("LDA", &format!("${:02X}", iterable_base));
                emitter.emit_inst("STA", "$F0");
                emitter.emit_inst("LDA", &format!("${:02X}", iterable_base + 1));
                emitter.emit_inst("STA", "$F1");
                emitter.emit_inst("LDY", "#$00");
                emitter.emit_inst("LDA", "($F0),Y");
                emitter.emit_inst("STA", "$F2");
                emitter.emit_inst("CPX", "$F2");
            } else if is_slice {
                // Length is the low byte of the descriptor at base+2
                // (iteration is bounded to 255 elements).
                emitter.emit_inst("LDA", &format!("${:02X}", iterable_base + 2));
                emitter.emit_inst("STA", "$F2");
                emitter.emit_inst("CPX", "$F2");
            } else if let Some(size) = array_size {
                // Compare X against known array size
                emitter.emit_inst("CPX", &format!("#${:02X}", size));
            }
            // Exit when X >= length. end_label sits past the whole body, so a
            // plain `BCS end_label` overflows its ±127 range once the body is
            // large (an inlined call, say). Route the far jump through a JMP and
            // let the conditional branch hop only over it.
            let fe_body = emitter.next_label("feb");
            emit_far_arm_branch(emitter, "BCS", &end_label, &fe_body);

            // Push loop context for break/continue. `continue` must land on the
            // increment (continue_label), NOT the loop head — otherwise the index
            // in X is never advanced and the loop spins forever.
            emitter.push_loop(continue_label.clone(), end_label.clone());

            // Store index in index variable if present
            if let Some(idx_var) = index_var
                && let Some(idx_sym) = info.resolved_symbols.get(&idx_var.span)
            {
                match idx_sym.location {
                    crate::sema::table::SymbolLocation::ZeroPage(addr) => {
                        emitter.emit_comment(&format!("Store index in {}", idx_var.node));
                        emitter.emit_inst("STX", &format!("${:02X}", addr));
                    }
                    _ => {
                        return Err(CodegenError::UnsupportedOperation(
                            "ForEach index variable must be in zero page".to_string(),
                        ));
                    }
                }
            }

            // Whether the array/slice element type is 16-bit (u16/i16/b16).
            // Strings always iterate u8 characters.
            let elem_multibyte = {
                let elem = match &iterable_ty {
                    crate::sema::types::Type::Array(elem, _)
                    | crate::sema::types::Type::Slice(elem) => Some(&**elem),
                    _ => None,
                };
                matches!(
                    elem,
                    Some(crate::sema::types::Type::Primitive(
                        crate::ast::PrimitiveType::U16
                            | crate::ast::PrimitiveType::I16
                            | crate::ast::PrimitiveType::B16
                    ))
                )
            };

            // Resolve the loop variable's storage up front.
            let loopvar_loc = info
                .resolved_symbols
                .get(&var_name.span)
                .map(|s| s.location.clone())
                .ok_or_else(|| CodegenError::SymbolNotFound(var_name.node.clone()))?;

            // Index into the iterable: Y = X, scaled ×2 for u16 elements.
            emitter.emit_inst("TXA", "");
            if elem_multibyte {
                emitter.emit_inst("ASL", "A");
            }
            emitter.emit_inst("TAY", "");

            // Emit "load byte at (base),Y then store to <dest>" for the low byte,
            // and (for u16 elements) INY + load/store the high byte.
            let store_lo = |emitter: &mut Emitter, load: &dyn Fn(&mut Emitter)| match loopvar_loc {
                crate::sema::table::SymbolLocation::ZeroPage(addr) => {
                    load(emitter);
                    emitter.emit_inst("STA", &format!("${:02X}", addr));
                    if elem_multibyte {
                        emitter.emit_inst("INY", "");
                        load(emitter);
                        emitter.emit_inst("STA", &format!("${:02X}", addr + 1));
                    }
                    Ok(())
                }
                crate::sema::table::SymbolLocation::Absolute(addr) => {
                    load(emitter);
                    emitter.emit_sta_abs(addr);
                    if elem_multibyte {
                        emitter.emit_inst("INY", "");
                        load(emitter);
                        emitter.emit_sta_abs(addr + 1);
                    }
                    Ok(())
                }
                _ => Err(CodegenError::UnsupportedOperation(
                    "ForEach loop variable must have concrete location".to_string(),
                )),
            };

            if is_string {
                // Strings: skip the length byte, u8 elements only.
                emitter.emit_inst("INY", "");
                emitter.emit_inst("LDA", "($F0),Y");
                match loopvar_loc {
                    crate::sema::table::SymbolLocation::ZeroPage(addr) => emitter.emit_sta_zp(addr),
                    crate::sema::table::SymbolLocation::Absolute(addr) => {
                        emitter.emit_sta_abs(addr)
                    }
                    _ => {
                        return Err(CodegenError::UnsupportedOperation(
                            "ForEach loop variable must have concrete location".to_string(),
                        ));
                    }
                }
            } else {
                let base = iterable_base;
                store_lo(emitter, &move |e: &mut Emitter| {
                    e.emit_inst("LDA", &format!("(${:02X}),Y", base));
                })?;
            }

            // The body is arbitrary code; drop all register beliefs before it.
            // The counter of record is the memory slot, reloaded into X at the
            // next loop head, so the body clobbering X is harmless.
            emitter.reg_state.invalidate_all();

            // Execute loop body
            generate_stmt(body, emitter, info, string_collector)?;

            // Pop loop context
            emitter.pop_loop();

            // Continue target: advance the counter in memory, then re-test at
            // the loop head (which reloads X from it). Advancing the slot rather
            // than X is what lets a body that clobbers X iterate correctly.
            emitter.emit_label(&continue_label);
            emitter.emit_inst("INC", &format!("${:02X}", counter));

            emitter.emit_inst("JMP", &loop_label);
            emitter.emit_label(&end_label);

            Ok(())
        }
        Stmt::Break => {
            if let Some(loop_ctx) = emitter.current_loop() {
                let break_label = loop_ctx.break_label.clone();
                emitter.emit_inst("JMP", &break_label);
                Ok(())
            } else {
                // This should be caught by semantic analysis
                Err(CodegenError::UnsupportedOperation(
                    "break statement outside of loop".to_string(),
                ))
            }
        }
        Stmt::Continue => {
            if let Some(loop_ctx) = emitter.current_loop() {
                let continue_label = loop_ctx.continue_label.clone();
                emitter.emit_inst("JMP", &continue_label);
                Ok(())
            } else {
                // This should be caught by semantic analysis
                Err(CodegenError::UnsupportedOperation(
                    "continue statement outside of loop".to_string(),
                ))
            }
        }
        Stmt::Asm { lines } => {
            // Inline assembly - emit lines directly with variable substitution
            let current_fn = emitter.current_function().map(|s| s.to_string());
            for line in lines {
                // Substitute {var} patterns with actual addresses
                let substituted =
                    substitute_asm_vars(&line.instruction, info, current_fn.as_deref())?;

                // If we're inside an inline function expansion, uniquify labels
                let final_line = if let Some(suffix) = emitter.inline_label_suffix() {
                    uniquify_asm_labels(&substituted, suffix)
                } else {
                    substituted
                };

                let parts: Vec<&str> = final_line.split_whitespace().collect();
                if parts.is_empty() {
                    continue;
                }

                let mnemonic = parts[0];
                let operand = if parts.len() > 1 {
                    parts[1..].join(" ")
                } else {
                    String::new()
                };

                emitter.emit_inst(mnemonic, &operand);
            }
            // Invalidate register state after inline assembly
            // (we don't know what the assembly does to registers)
            emitter.reg_state.invalidate_all();
            Ok(())
        }
        Stmt::Match { expr, arms } => generate_match(expr, arms, emitter, info, string_collector),
    }
}

/// Copy an enum value's bytes from the pointer in A:X into the variable's own
/// per-declaration block, then load the block's address into A:X for the slot
/// store that follows. Without this the slot would point at shared codegen
/// scratch (construction) or a dead frame (call result), and two live enums
/// would alias.
fn emit_enum_copy_to_block(
    block: &crate::sema::LocalArray,
    emitter: &mut Emitter,
) -> Result<(), CodegenError> {
    if block.size == 0 || block.size > 255 {
        return Err(CodegenError::Internal(format!(
            "enum data block of {} bytes cannot be copied by the byte loop",
            block.size
        )));
    }
    let ptr = emitter.memory_layout.deref_ptr();
    emitter.emit_comment("Copy enum value into its own storage");
    emitter.emit_inst("STA", &format!("${:02X}", ptr));
    emitter.emit_inst("STX", &format!("${:02X}", ptr + 1));
    let copy_label = emitter.next_label("encp");
    emitter.emit_inst("LDY", "#$00");
    emitter.emit_label(&copy_label);
    emitter.emit_inst("LDA", &format!("(${:02X}),Y", ptr));
    emitter.emit_inst("STA", &format!("${:04X},Y", block.addr));
    emitter.emit_inst("INY", "");
    emitter.emit_inst("CPY", &format!("#${:02X}", block.size as u8));
    emitter.emit_inst("BNE", &copy_label);
    // The slot now points at the block, whose address is a compile-time constant.
    emitter.emit_inst("LDA", &format!("#${:02X}", block.addr & 0xFF));
    emitter.emit_inst("LDX", &format!("#${:02X}", (block.addr >> 8) & 0xFF));
    emitter.invalidate_registers();
    Ok(())
}

/// Copy a return-by-value aggregate out of the callee's storage into the
/// caller's slot. On entry `A:X` holds a pointer to `size` source bytes; they
/// are copied into `dest..dest+size` (zero page).
///
/// Below the threshold the copy is unrolled (`LDY #i; LDA ($20),Y; STA dest+i`,
/// ~6 bytes/byte — smallest for a handful of bytes). At or above it, an
/// `INY`/`CPY`/`BNE` loop is emitted instead: 12 bytes flat regardless of size,
/// so it wins once the aggregate exceeds ~3 bytes (a `DEX/BNE`-class trade of a
/// little speed for much smaller code). `A:X` are dead afterwards; callers
/// invalidate register state.
fn emit_return_by_value_copy(emitter: &mut Emitter, dest: u8, size: u8) {
    /// Aggregate size at/above which the loop is smaller than unrolling.
    const COPY_LOOP_THRESHOLD: u8 = 4;

    // A = pointer low, X = pointer high -> $20/$21 vector.
    emitter.emit_inst("STA", "$20");
    emitter.emit_inst("STX", "$21");

    if size < COPY_LOOP_THRESHOLD {
        for i in 0..size {
            emitter.emit_inst("LDY", &format!("#${:02X}", i));
            emitter.emit_inst("LDA", "($20),Y");
            emitter.emit_inst("STA", &format!("${:02X}", dest + i));
        }
    } else {
        let loop_label = emitter.next_label("rbvcp");
        emitter.emit_inst("LDY", "#$00");
        emitter.emit_label(&loop_label);
        emitter.emit_inst("LDA", "($20),Y");
        // Absolute,Y: `STA zp,Y` has no encoding, so the zero-page dest is
        // addressed as a 16-bit base ($00dd) with the Y index.
        emitter.emit_inst("STA", &format!("${:04X},Y", dest as u16));
        emitter.emit_inst("INY", "");
        emitter.emit_inst("CPY", &format!("#${:02X}", size));
        emitter.emit_inst("BNE", &loop_label);
    }
}
