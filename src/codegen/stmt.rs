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
                use crate::sema::types::Type;

                // Check if this is an array type (arrays need 2 bytes for pointer)
                let is_array = matches!(sym.ty, Type::Array(_, _));

                match sym.location {
                    crate::sema::table::SymbolLocation::Absolute(addr) => {
                        // Check if this is an address declaration - use symbolic name
                        if sym.kind == SymbolKind::Address {
                            emitter.emit_sta_symbol(&name.node);
                        } else {
                            emitter.emit_sta_abs(addr);
                            // For arrays, also store high byte (in X)
                            if is_array {
                                emitter.emit_inst("STX", &format!("${:04X}", addr + 1));
                            }
                        }
                    }
                    crate::sema::table::SymbolLocation::ZeroPage(addr) => {
                        emitter.emit_sta_zp(addr);
                        // For arrays, also store high byte (in X)
                        if is_array {
                            emitter.emit_inst("STX", &format!("${:02X}", addr + 1));
                        }
                    }
                    crate::sema::table::SymbolLocation::None => {
                        return Err(CodegenError::UnsupportedOperation(format!(
                            "VarDecl '{}' has no storage location",
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
            // Optimization: detect x = x + 1 and x = x - 1 patterns
            // Use INC/DEC instead of LDA/ADC/STA or LDA/SBC/STA
            if let crate::ast::Expr::Variable(target_name) = &target.node {
                if let crate::ast::Expr::Binary { left, op, right } = &value.node {
                    // Check if left side is the same variable as target
                    if let crate::ast::Expr::Variable(left_name) = &left.node {
                        if left_name == target_name {
                            // Check if right side is literal 1
                            if let crate::ast::Expr::Literal(crate::ast::Literal::Integer(n)) = &right.node {
                                if *n == 1 {
                                    // Look up variable location
                                    let sym = info.resolved_symbols.get(&target.span)
                                        .or_else(|| info.table.lookup(target_name));

                                    if let Some(sym) = sym {
                                        match (op, &sym.location) {
                                            (crate::ast::BinaryOp::Add, crate::sema::table::SymbolLocation::ZeroPage(addr)) => {
                                                // x = x + 1 -> INC $addr
                                                emitter.emit_inst("INC", &format!("${:02X}", *addr));
                                                emitter.reg_state.invalidate_zero_page(*addr);
                                                return Ok(());
                                            }
                                            (crate::ast::BinaryOp::Add, crate::sema::table::SymbolLocation::Absolute(addr)) => {
                                                // x = x + 1 -> INC $addr
                                                emitter.emit_inst("INC", &format!("${:04X}", *addr));
                                                emitter.reg_state.invalidate_memory(*addr);
                                                return Ok(());
                                            }
                                            (crate::ast::BinaryOp::Sub, crate::sema::table::SymbolLocation::ZeroPage(addr)) => {
                                                // x = x - 1 -> DEC $addr
                                                emitter.emit_inst("DEC", &format!("${:02X}", *addr));
                                                emitter.reg_state.invalidate_zero_page(*addr);
                                                return Ok(());
                                            }
                                            (crate::ast::BinaryOp::Sub, crate::sema::table::SymbolLocation::Absolute(addr)) => {
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
                    }
                }
            }

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
                            crate::sema::table::SymbolLocation::None => {
                                return Err(CodegenError::UnsupportedOperation(format!(
                                    "Variable '{}' has no storage location",
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
            let start_label = emitter.next_label("wh");
            let end_label = emitter.next_label("we");

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
            let loop_label = emitter.next_label("lp");
            let end_label = emitter.next_label("lx");

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

            let loop_label = emitter.next_label("fl");
            let end_label = emitter.next_label("fx");

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
                let continue_label = emitter.next_label("fc");
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
        Stmt::ForEach {
            var_name,
            var_type: _,
            iterable,
            body,
        } => {
            // ForEach loop: for item in array { ... }
            // Strategy:
            // 1. Evaluate iterable expression to get array pointer
            // 2. Use X register as loop counter (0..array_length)
            // 3. Load array[X] into the loop variable
            // 4. Execute body
            // 5. Increment X and loop

            emitter.emit_comment("ForEach loop");

            // Generate the iterable expression (should be an array)
            // For now, only support array variables
            let (array_base, array_size) = match &iterable.node {
                crate::ast::Expr::Variable(name) => {
                    // Look up the array to get its pointer location and size
                    let sym = info.resolved_symbols.get(&iterable.span)
                        .or_else(|| info.table.lookup(name))
                        .ok_or_else(|| CodegenError::SymbolNotFound(name.clone()))?;

                    // Get array size from type
                    let size = match &sym.ty {
                        crate::sema::types::Type::Array(_, sz) => *sz,
                        _ => {
                            return Err(CodegenError::UnsupportedOperation(
                                "ForEach requires array type".to_string()
                            ))
                        }
                    };

                    // Get the location where the array pointer is stored
                    let ptr_loc = match sym.location {
                        crate::sema::table::SymbolLocation::ZeroPage(addr) => addr,
                        crate::sema::table::SymbolLocation::Absolute(addr) if addr < 256 => addr as u8,
                        _ => {
                            return Err(CodegenError::UnsupportedOperation(
                                "ForEach requires array pointer in zero page".to_string()
                            ))
                        }
                    };

                    (ptr_loc, size)
                }
                _ => {
                    return Err(CodegenError::UnsupportedOperation(
                        "ForEach only supports array variables currently".to_string()
                    ))
                }
            };

            let loop_label = emitter.next_label("fe");
            let end_label = emitter.next_label("fz");

            // Initialize counter to 0 in X register
            emitter.emit_inst("LDX", "#$00");
            emitter.reg_state.set_x(crate::codegen::regstate::RegisterValue::Immediate(0));

            // Loop start
            emitter.emit_label(&loop_label);

            // Check if counter (X) >= array_size
            emitter.emit_inst("CPX", &format!("#${:02X}", array_size));
            emitter.emit_inst("BCS", &end_label); // Branch if X >= size

            // Push loop context for break/continue
            emitter.push_loop(loop_label.clone(), end_label.clone());

            // Load array[X] into A using indirect indexed: LDA (ptr),Y
            // Transfer X to Y for indexing
            emitter.emit_inst("TXA", "Save counter");
            emitter.emit_inst("TAY", "Use as index");
            emitter.emit_inst("LDA", &format!("(${:02X}),Y", array_base));

            // Store the element in the loop variable
            // Look up the loop variable (it should be in the current scope)
            if let Some(loop_var) = info.resolved_symbols.get(&var_name.span) {
                match loop_var.location {
                    crate::sema::table::SymbolLocation::ZeroPage(addr) => {
                        emitter.emit_sta_zp(addr);
                    }
                    crate::sema::table::SymbolLocation::Absolute(addr) => {
                        emitter.emit_sta_abs(addr);
                    }
                    _ => {
                        return Err(CodegenError::UnsupportedOperation(
                            "ForEach loop variable must have concrete location".to_string()
                        ))
                    }
                }
            } else {
                return Err(CodegenError::SymbolNotFound(var_name.node.clone()));
            }

            // Restore counter from stack (it's still in X, so no need)
            emitter.reg_state.invalidate_all();

            // Execute loop body
            generate_stmt(body, emitter, info)?;

            // Pop loop context
            emitter.pop_loop();

            // Increment counter
            emitter.emit_inst("INX", "Next element");
            emitter.reg_state.modify_x();

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

        // Extract bindings for enum variant patterns
        if let Pattern::EnumVariant { enum_name, variant, bindings } = &arm.pattern.node {
            if !bindings.is_empty() {
                // Look up the enum definition to get field information
                let enum_def = info.type_registry.get_enum(&enum_name.node)
                    .ok_or_else(|| CodegenError::UnsupportedOperation(
                        format!("enum '{}' not found in type registry", enum_name.node)
                    ))?;

                let variant_info = enum_def.get_variant(&variant.node)
                    .ok_or_else(|| CodegenError::UnsupportedOperation(
                        format!("variant '{}' not found in enum '{}'", variant.node, enum_name.node)
                    ))?;

                // Extract field values from enum data
                // Enum layout in memory: [tag: u8][field0][field1]...
                // The pointer at ($20) points to the tag byte
                // Field data starts at offset 1

                match &variant_info.data {
                    crate::sema::type_defs::VariantData::Tuple(field_types) => {
                        // Tuple variant: extract each field by position
                        if bindings.len() != field_types.len() {
                            return Err(CodegenError::UnsupportedOperation(
                                format!("Pattern binding count mismatch: expected {}, got {}",
                                    field_types.len(), bindings.len())
                            ));
                        }

                        let mut offset = 1; // Start after the tag byte
                        for (binding, field_type) in bindings.iter().zip(field_types.iter()) {
                            // Load field value using indirect indexed addressing
                            emitter.emit_inst("LDY", &format!("#${:02X}", offset));
                            emitter.emit_inst("LDA", "($20),Y");

                            // Store in the binding variable
                            // Look up the binding variable in resolved_symbols
                            if let Some(var_sym) = info.resolved_symbols.get(&binding.name.span) {
                                match var_sym.location {
                                    crate::sema::table::SymbolLocation::ZeroPage(addr) => {
                                        emitter.emit_sta_zp(addr);
                                    }
                                    crate::sema::table::SymbolLocation::Absolute(addr) => {
                                        emitter.emit_sta_abs(addr);
                                    }
                                    _ => {
                                        return Err(CodegenError::UnsupportedOperation(
                                            format!("Binding '{}' has unsupported location", binding.name.node)
                                        ));
                                    }
                                }
                            } else {
                                return Err(CodegenError::SymbolNotFound(binding.name.node.clone()));
                            }

                            // Move to next field (assuming u8 fields for now)
                            offset += field_type.size() as u8;
                        }
                    }
                    crate::sema::type_defs::VariantData::Struct(_) => {
                        // Struct variant: bindings should match field names
                        // For now, not implemented
                        return Err(CodegenError::UnsupportedOperation(
                            "Pattern bindings for struct variants not yet implemented".to_string()
                        ));
                    }
                    crate::sema::type_defs::VariantData::Unit => {
                        // Unit variant shouldn't have bindings
                        if !bindings.is_empty() {
                            return Err(CodegenError::UnsupportedOperation(
                                "Unit variant should not have bindings".to_string()
                            ));
                        }
                    }
                }
            }
        }

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
