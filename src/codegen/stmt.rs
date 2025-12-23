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
                use crate::sema::table::SymbolKind;

                match sym.location {
                    crate::sema::table::SymbolLocation::Absolute(addr) => {
                        // Check if this is an address declaration - use symbolic name
                        if sym.kind == SymbolKind::Address {
                            emitter.emit_sta_symbol(&name.node);
                        } else {
                            emitter.emit_sta_abs(addr);
                        }
                    }
                    crate::sema::table::SymbolLocation::ZeroPage(addr) => {
                        emitter.emit_sta_zp(addr);
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
            // Only emit RTS if we're not in an inline context
            // When inlining, the return value is already in A and we just continue
            if !emitter.is_inlining() {
                emitter.emit_inst("RTS", "");
            }
            Ok(())
        }
        Stmt::Assign { target, value } => {
            // 1. Generate code for value (result in A)
            generate_expr(value, emitter, info)?;

            // 2. Store A into target
            // We need a helper to generate store instructions based on target
            match &target.node {
                crate::ast::Expr::Variable(name) => {
                    // Look up by span in resolved_symbols first (for local vars)
                    let sym = info.resolved_symbols.get(&target.span)
                        .or_else(|| info.table.lookup(name)); // Fallback to global table

                    if let Some(sym) = sym {
                        use crate::sema::table::SymbolKind;

                        match sym.location {
                            crate::sema::table::SymbolLocation::Absolute(addr) => {
                                // Check if this is an address declaration - use symbolic name
                                if sym.kind == SymbolKind::Address {
                                    emitter.emit_sta_symbol(name);
                                } else {
                                    emitter.emit_sta_abs(addr);
                                }
                            }
                            crate::sema::table::SymbolLocation::ZeroPage(addr) => {
                                emitter.emit_sta_zp(addr);
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

            // Push loop context for break/continue
            emitter.push_loop(start_label.clone(), end_label.clone());

            // Body
            generate_stmt(body, emitter, info)?;

            // Pop loop context
            emitter.pop_loop();

            emitter.emit_inst("JMP", &start_label);

            emitter.emit_label(&end_label);
            Ok(())
        }
        Stmt::Loop { body } => {
            let loop_label = emitter.next_label("loop_start");
            let end_label = emitter.next_label("loop_end");

            emitter.emit_label(&loop_label);

            // Push loop context for break/continue
            emitter.push_loop(loop_label.clone(), end_label.clone());

            generate_stmt(body, emitter, info)?;

            // Pop loop context
            emitter.pop_loop();

            emitter.emit_inst("JMP", &loop_label);
            emitter.emit_label(&end_label);
            Ok(())
        }
        Stmt::For {
            var_name: _,
            var_type: _,
            range,
            body,
        } => {
            // Optimized for loop using X register for counter
            // This frees up zero page and is faster (INX vs INC, CPX vs CMP+LDA)
            let loop_end_temp = emitter.memory_layout.loop_end_temp();

            let loop_label = emitter.next_label("for_loop");
            let end_label = emitter.next_label("for_end");

            // Initialize loop variable with range start -> X register
            generate_expr(&range.start, emitter, info)?;
            emitter.emit_inst("TAX", ""); // Transfer A to X (counter in X)
            emitter.reg_state.transfer_a_to_x();

            // Generate end value and store in temp location
            generate_expr(&range.end, emitter, info)?;
            emitter.emit_inst("STA", &format!("${:02X}", loop_end_temp));

            // Loop start
            emitter.emit_label(&loop_label);

            // Check condition: compare X (counter) with end value
            emitter.emit_inst("CPX", &format!("${:02X}", loop_end_temp));

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

            // Push loop context for break/continue
            emitter.push_loop(loop_label.clone(), end_label.clone());

            // Execute body (note: X is used for counter, may need to save/restore if body uses X)
            // For now, we accept that the body can't use X register
            emitter.reg_state.invalidate_all(); // Body might use registers
            generate_stmt(body, emitter, info)?;

            // Pop loop context
            emitter.pop_loop();

            // Increment counter (X register)
            emitter.emit_inst("INX", ""); // 2 cycles vs INC zp (5 cycles)
            emitter.reg_state.modify_x();

            emitter.emit_inst("JMP", &loop_label);
            emitter.emit_label(&end_label);

            // After loop, X register is modified
            emitter.reg_state.modify_x();
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
            for line in lines {
                // Substitute {var} patterns with actual addresses
                let substituted = substitute_asm_vars(&line.instruction, info)?;

                let parts: Vec<&str> = substituted.split_whitespace().collect();
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

    let match_id = emitter.next_match_id();

    emitter.emit_comment("Match statement");

    // Check if we're matching on an enum by looking at the first pattern
    let is_enum_match = arms.iter().any(|arm| {
        matches!(arm.pattern.node, Pattern::EnumVariant { .. })
    });

    // Evaluate the matched expression into accumulator
    generate_expr(expr, emitter, info)?;

    if is_enum_match {
        // For enum matching, expression returns a pointer in A:X
        // Store pointer at $20 (low) and $21 (high)
        emitter.emit_inst("STA", "$20");
        emitter.emit_inst("STX", "$21");

        // Load the discriminant tag from the enum (first byte)
        emitter.emit_inst("LDY", "#$00");
        emitter.emit_inst("LDA", "($20),Y");
        emitter.emit_inst("STA", "$22"); // Store tag at $22
    } else {
        // For simple value matching, store value at $20
        emitter.emit_inst("STA", "$20");
    }

    // Generate code for each arm
    let mut has_wildcard = false;
    for (i, arm) in arms.iter().enumerate() {
        match &arm.pattern.node {
            Pattern::Literal(lit_expr) => {
                // Compare with literal value
                if let crate::ast::Expr::Literal(crate::ast::Literal::Integer(val)) = &lit_expr.node {
                    // For enum matching, we already have the tag in $22, but this is for literal patterns
                    // which shouldn't mix with enum patterns in the same match
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
            Pattern::EnumVariant { enum_name, variant, bindings } => {
                // Look up the enum definition
                let enum_def = info.type_registry.get_enum(&enum_name.node)
                    .ok_or_else(|| CodegenError::UnsupportedOperation(
                        format!("enum '{}' not found in type registry", enum_name.node)
                    ))?;

                // Find the variant
                let variant_info = enum_def.get_variant(&variant.node)
                    .ok_or_else(|| CodegenError::UnsupportedOperation(
                        format!("variant '{}' not found in enum '{}'", variant.node, enum_name.node)
                    ))?;

                // Compare the tag with the expected variant tag
                emitter.emit_inst("LDA", "$22"); // Load stored tag
                emitter.emit_inst("CMP", &format!("#${:02X}", variant_info.tag));
                emitter.emit_inst("BEQ", &format!("match_{}_arm_{}", match_id, i));

                // If bindings are present, we'll extract them in the arm body
                // For now, we just check the tag - bindings will be handled later
                if !bindings.is_empty() {
                    emitter.emit_comment(&format!("Variant has {} binding(s)", bindings.len()));
                }
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

        // TODO: Extract bindings for enum variant patterns
        // For now, enum pattern matching works but doesn't extract bound variables
        // To implement: load variant data from offset ($20)+1 onwards based on variant type

        generate_stmt(&arm.body, emitter, info)?;
        emitter.emit_inst("JMP", &format!("match_{}_end", match_id));
    }

    emitter.emit_label(&format!("match_{}_end", match_id));

    Ok(())
}

/// Substitute {variable} patterns in inline assembly with actual addresses
fn substitute_asm_vars(instruction: &str, info: &ProgramInfo) -> Result<String, CodegenError> {
    let mut result = instruction.to_string();

    // Find all {var} patterns
    while let Some(start) = result.find('{') {
        if let Some(end) = result[start..].find('}') {
            let end = start + end;
            let var_name = &result[start + 1..end];

            // Look up the variable in resolved_symbols (by name)
            // We search through resolved_symbols because the symbol table's scopes
            // have been exited after semantic analysis
            let symbol = info.resolved_symbols
                .values()
                .find(|s| s.name == var_name)
                .ok_or_else(|| {
                    CodegenError::SymbolNotFound(var_name.to_string())
                })?;

            // Convert the location to an address string
            let address = match symbol.location {
                crate::sema::table::SymbolLocation::ZeroPage(addr) => format!("${:02X}", addr),
                crate::sema::table::SymbolLocation::Absolute(addr) => format!("${:04X}", addr),
                crate::sema::table::SymbolLocation::Stack(offset) => {
                    // Stack variables are relative to a base address
                    // For 6502, we typically use zero page for stack frame pointer
                    // For now, use absolute addressing
                    let layout = crate::codegen::memory_layout::MemoryLayout::new();
                    let param_base = layout.param_base;
                    format!("${:02X}", (param_base as i16 + offset as i16) as u8)
                }
                crate::sema::table::SymbolLocation::None => {
                    return Err(CodegenError::SymbolNotFound(format!(
                        "{} has no memory location",
                        var_name
                    )));
                }
            };

            // Replace {var} with the address
            result.replace_range(start..=end, &address);
        } else {
            // Unmatched {, just break
            break;
        }
    }

    Ok(result)
}
