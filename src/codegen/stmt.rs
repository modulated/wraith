//! Statement Code Generation
//!
//! Compiles statements into assembly instructions.

use crate::ast::{Spanned, Stmt};
use crate::codegen::expr::generate_expr;
use crate::codegen::{CodegenError, Emitter};
use crate::sema::ProgramInfo;

pub fn generate_stmt(
    stmt: &Spanned<Stmt>,
    emitter: &mut Emitter,
    info: &ProgramInfo,
) -> Result<(), CodegenError> {
    match &stmt.node {
        Stmt::Block(stmts) => {
            for s in stmts {
                generate_stmt(s, emitter, info)?;
            }
            Ok(())
        }
        Stmt::VarDecl {
            name,
            ty: _,
            init,
            mutable: _,
            zero_page: _,
        } => {
            // Generate initialization expression
            generate_expr(init, emitter, info)?;

            // Store in variable location
            // Look up by span in resolved_symbols since local vars aren't in global table
            if let Some(sym) = info.resolved_symbols.get(&name.span) {
                match sym.location {
                    crate::sema::table::SymbolLocation::Absolute(addr) => {
                        emitter.emit_inst("STA", &format!("${:04X}", addr));
                    }
                    crate::sema::table::SymbolLocation::ZeroPage(addr) => {
                        emitter.emit_inst("STA", &format!("${:02X}", addr));
                    }
                    crate::sema::table::SymbolLocation::Stack(offset) => {
                        // TODO: Implement stack frame management
                        emitter.emit_comment(&format!(
                            "VarDecl {} (stack offset {})",
                            name.node, offset
                        ));
                    }
                    _ => {
                        return Err(CodegenError::UnsupportedOperation(format!(
                            "VarDecl '{}' has unsupported location type",
                            name.node
                        )))
                    }
                }
            } else {
                return Err(CodegenError::SymbolNotFound(name.node.clone()));
            }
            Ok(())
        }
        Stmt::Return(expr) => {
            if let Some(e) = expr {
                generate_expr(e, emitter, info)?;
            }
            // RTS is handled by function epilogue, but we might need a jump to end
            // For now, assuming return is last statement or we just emit RTS here
            // Note: Multiple returns require jumping to epilogue
            emitter.emit_inst("RTS", "");
            Ok(())
        }
        Stmt::Assign { target, value } => {
            // 1. Generate code for value (result in A)
            generate_expr(value, emitter, info)?;

            // 2. Store A into target
            // We need a helper to generate store instructions based on target
            match &target.node {
                crate::ast::Expr::Variable(name) => {
                    if let Some(sym) = info.table.lookup(name) {
                        match sym.location {
                            crate::sema::table::SymbolLocation::Absolute(addr) => {
                                emitter.emit_inst("STA", &format!("${:04X}", addr));
                            }
                            crate::sema::table::SymbolLocation::ZeroPage(addr) => {
                                emitter.emit_inst("STA", &format!("${:02X}", addr));
                            }
                            crate::sema::table::SymbolLocation::Stack(offset) => {
                                // Placeholder for stack store
                                emitter.emit_comment(&format!(
                                    "Store local {} (offset {})",
                                    name, offset
                                ));
                            }
                            _ => {
                                return Err(CodegenError::UnsupportedOperation(format!(
                                    "Variable '{}' has unsupported location type",
                                    name
                                )))
                            }
                        }
                    } else {
                        return Err(CodegenError::SymbolNotFound(name.clone()));
                    }
                }
                _ => {
                    return Err(CodegenError::UnsupportedOperation(
                        "Only variable assignment supported".to_string(),
                    ))
                }
            }
            Ok(())
        }
        Stmt::Expr(expr) => {
            generate_expr(expr, emitter, info)?;
            Ok(())
        }
        Stmt::If {
            condition,
            then_branch,
            else_branch,
        } => {
            let else_label = emitter.next_label("else");
            let end_label = emitter.next_label("end");

            // Condition
            generate_expr(condition, emitter, info)?;
            emitter.emit_inst("CMP", "#$00");
            emitter.emit_inst("BEQ", &else_label);

            // Then
            generate_stmt(then_branch, emitter, info)?;
            emitter.emit_inst("JMP", &end_label);

            // Else
            emitter.emit_label(&else_label);
            if let Some(else_b) = else_branch {
                generate_stmt(else_b, emitter, info)?;
            }

            // End
            emitter.emit_label(&end_label);
            Ok(())
        }
        Stmt::While { condition, body } => {
            let start_label = emitter.next_label("while_start");
            let end_label = emitter.next_label("while_end");

            emitter.emit_label(&start_label);

            // Condition
            generate_expr(condition, emitter, info)?;
            emitter.emit_inst("CMP", "#$00");
            emitter.emit_inst("BEQ", &end_label);

            // Body
            generate_stmt(body, emitter, info)?;
            emitter.emit_inst("JMP", &start_label);

            emitter.emit_label(&end_label);
            Ok(())
        }
        Stmt::Loop { body } => {
            let loop_label = emitter.next_label("loop_start");
            emitter.emit_label(&loop_label);
            generate_stmt(body, emitter, info)?;
            emitter.emit_inst("JMP", &loop_label);
            Ok(())
        }
        Stmt::For {
            var_name: _,
            var_type: _,
            range,
            body,
        } => {
            // For loop: initialize counter, loop with condition, increment
            // Use fixed zero page location for loop counter (simplified approach)
            const LOOP_COUNTER: u8 = 0x10;
            const LOOP_END_TEMP: u8 = 0x21;

            let loop_label = emitter.next_label("for_loop");
            let end_label = emitter.next_label("for_end");

            // Initialize loop variable with range start
            generate_expr(&range.start, emitter, info)?;
            emitter.emit_inst("STA", &format!("${:02X}", LOOP_COUNTER));

            // Generate end value and store in temp location
            generate_expr(&range.end, emitter, info)?;
            emitter.emit_inst("STA", &format!("${:02X}", LOOP_END_TEMP));

            // Loop start
            emitter.emit_label(&loop_label);

            // Check condition: load counter and compare with end
            emitter.emit_inst("LDA", &format!("${:02X}", LOOP_COUNTER));
            emitter.emit_inst("CMP", &format!("${:02X}", LOOP_END_TEMP));

            if range.inclusive {
                // If counter > end, exit
                let continue_label = emitter.next_label("for_continue");
                emitter.emit_inst("BEQ", &continue_label); // Equal is ok for inclusive
                emitter.emit_inst("BCS", &end_label); // Counter > end, exit
                emitter.emit_label(&continue_label);
            } else {
                // If counter >= end, exit
                emitter.emit_inst("BCS", &end_label);
            }

            // Execute body
            generate_stmt(body, emitter, info)?;

            // Increment counter
            emitter.emit_inst("INC", &format!("${:02X}", LOOP_COUNTER));

            emitter.emit_inst("JMP", &loop_label);
            emitter.emit_label(&end_label);
            Ok(())
        }
        Stmt::Break => {
            // TODO: Need loop context to know where to jump
            emitter.emit_comment("break (requires loop context)");
            Ok(())
        }
        Stmt::Continue => {
            // TODO: Need loop context to know where to jump
            emitter.emit_comment("continue (requires loop context)");
            Ok(())
        }
        Stmt::Asm { lines } => {
            // Inline assembly - emit lines directly
            for line in lines {
                // Parse the instruction (could have {var} substitutions)
                // For now, just emit as-is
                let parts: Vec<&str> = line.instruction.split_whitespace().collect();
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
            Ok(())
        }
        Stmt::Match { expr, arms } => {
            generate_match(expr, arms, emitter, info)
        }
        _ => Ok(()), // TODO: Implement other statements
    }
}

fn generate_match(
    expr: &Spanned<crate::ast::Expr>,
    arms: &[crate::ast::MatchArm],
    emitter: &mut Emitter,
    info: &ProgramInfo,
) -> Result<(), CodegenError> {
    use crate::ast::Pattern;

    static mut MATCH_COUNTER: u32 = 0;
    let match_id = unsafe {
        let id = MATCH_COUNTER;
        MATCH_COUNTER += 1;
        id
    };

    emitter.emit_comment("Match statement");

    // Evaluate the matched expression into accumulator
    generate_expr(expr, emitter, info)?;

    // Store it temporarily at $20 (we need A for comparisons)
    emitter.emit_inst("STA", "$20");

    // Generate code for each arm
    let mut has_wildcard = false;
    for (i, arm) in arms.iter().enumerate() {
        match &arm.pattern.node {
            Pattern::Literal(lit_expr) => {
                // Compare with literal value
                if let crate::ast::Expr::Literal(crate::ast::Literal::Integer(val)) = &lit_expr.node {
                    emitter.emit_inst("LDA", "$20");
                    emitter.emit_inst("CMP", &format!("#${:02X}", val));
                    emitter.emit_inst("BEQ", &format!("match_{}_arm_{}", match_id, i));
                }
            }
            Pattern::Range { start, end, inclusive } => {
                // Range check: value >= start && value <= end (or < end+1 for inclusive)
                if let (
                    crate::ast::Expr::Literal(crate::ast::Literal::Integer(start_val)),
                    crate::ast::Expr::Literal(crate::ast::Literal::Integer(end_val)),
                ) = (&start.node, &end.node) {
                    emitter.emit_inst("LDA", "$20");

                    // Check if value < start, skip this arm
                    emitter.emit_inst("CMP", &format!("#${:02X}", start_val));
                    emitter.emit_inst("BCC", &format!("match_{}_arm_{}_end", match_id, i));

                    // Check if value <= end (or < end+1)
                    let upper_bound = if *inclusive { end_val + 1 } else { *end_val };
                    emitter.emit_inst("CMP", &format!("#${:02X}", upper_bound));
                    emitter.emit_inst("BCC", &format!("match_{}_arm_{}", match_id, i));

                    emitter.emit_label(&format!("match_{}_arm_{}_end", match_id, i));
                }
            }
            Pattern::Wildcard => {
                // Wildcard catches everything - no comparison needed
                has_wildcard = true;
                emitter.emit_inst("JMP", &format!("match_{}_arm_{}", match_id, i));
            }
            Pattern::Variable(_) => {
                // Variable pattern binds the value - like wildcard but with binding
                // TODO: Store value in the variable
                emitter.emit_inst("JMP", &format!("match_{}_arm_{}", match_id, i));
            }
            Pattern::EnumVariant { .. } => {
                // TODO: Implement enum pattern matching
                return Err(CodegenError::UnsupportedOperation("enum pattern matching not yet implemented".to_string()));
            }
        }
    }

    // If no pattern matched and no wildcard, this is an error (should be caught in semantic analysis)
    if !has_wildcard {
        emitter.emit_comment("No pattern matched - should not reach here");
    }

    // Generate bodies for each arm
    for (i, arm) in arms.iter().enumerate() {
        emitter.emit_label(&format!("match_{}_arm_{}", match_id, i));
        generate_stmt(&arm.body, emitter, info)?;
        emitter.emit_inst("JMP", &format!("match_{}_end", match_id));
    }

    emitter.emit_label(&format!("match_{}_end", match_id));

    Ok(())
}
