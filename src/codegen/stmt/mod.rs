//! Statement Code Generation
//!
//! Compiles statements into assembly instructions.

pub(super) use crate::ast::{Spanned, Stmt};
pub(super) use crate::codegen::expr::generate_expr;
pub(super) use crate::codegen::{CodegenError, Emitter, StringCollector};
pub(super) use crate::sema::ProgramInfo;
pub(super) use rustc_hash::FxHashMap as HashMap;

mod asm_stmt;
pub(crate) mod assign;
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
        Stmt::VarDecl { name, init, .. } => {
            generate_var_decl(name, init, emitter, info, string_collector)
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
            generate_assign(target, value, emitter, info, string_collector)
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
            range,
            body,
            ..
        } => generate_for(var_name, range, body, emitter, info, string_collector),
        Stmt::ForEach {
            var_name,
            iterable,
            body,
            index_var,
            ..
        } => generate_foreach(
            var_name,
            iterable,
            body,
            index_var,
            emitter,
            info,
            string_collector,
        ),
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
