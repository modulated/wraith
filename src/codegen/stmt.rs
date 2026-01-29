//! Statement Code Generation
//!
//! Compiles statements into assembly instructions.

use crate::ast::{Spanned, Stmt};
use crate::codegen::expr::generate_expr;
use crate::codegen::{CodegenError, Emitter, StringCollector};
use crate::sema::ProgramInfo;
use rustc_hash::FxHashMap as HashMap;

/// Strategy for generating match statement code
#[derive(Debug)]
enum MatchStrategy {
    /// Use sequential CMP/BEQ comparisons (for small matches)
    Sequential,
    /// Use jump table for efficient dispatch (for enum matches with 3+ arms)
    JumpTable {
        /// Maximum tag value in the enum
        max_tag: u8,
        /// Index of the wildcard arm, if any
        wildcard_arm_index: Option<usize>,
    },
}

/// Determine the best strategy for generating a match statement
///
/// Uses jump tables for enum matches with 3+ arms to avoid BEQ branch distance limits.
/// Sequential comparisons are used for small matches (1-2 arms) or non-enum matches.
fn determine_match_strategy(arms: &[crate::ast::MatchArm], info: &ProgramInfo) -> MatchStrategy {
    use crate::ast::Pattern;

    // Collect enum variant tags from the patterns
    let mut enum_tags: Vec<u8> = Vec::new();
    let mut wildcard_arm_index: Option<usize> = None;

    for (i, arm) in arms.iter().enumerate() {
        match &arm.pattern.node {
            Pattern::EnumVariant {
                enum_name, variant, ..
            } => {
                // Look up the enum definition and get the tag
                if let Some(enum_def) = info.type_registry.get_enum(&enum_name.node)
                    && let Some(variant_info) = enum_def.get_variant(&variant.node)
                {
                    enum_tags.push(variant_info.tag);
                }
            }
            Pattern::Wildcard | Pattern::Variable(_) => {
                wildcard_arm_index = Some(i);
            }
            _ => {}
        }
    }

    // Use jump table for enum matches with 3+ arms
    // This avoids the BEQ branch distance limitation for large match bodies
    if !enum_tags.is_empty() && arms.len() >= 3 {
        let max_tag = enum_tags.iter().copied().max().unwrap_or(0);
        MatchStrategy::JumpTable {
            max_tag,
            wildcard_arm_index,
        }
    } else {
        MatchStrategy::Sequential
    }
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
                        | Type::Primitive(crate::ast::PrimitiveType::U16)
                        | Type::Primitive(crate::ast::PrimitiveType::I16)
                        | Type::Primitive(crate::ast::PrimitiveType::B16)
                ) || is_enum;

                // Arrays, enums, and strings store address in A (low) and X (high)
                // Other u16 types store in A (low) and Y (high)
                let is_array_or_enum = matches!(sym.ty, Type::Array(_, _) | Type::String) || is_enum;

                match sym.location {
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
                    // Normal return with value
                    generate_expr(e, emitter, info, string_collector)?;

                    // Only emit RTS if we're not in an inline context
                    if !emitter.is_inlining() {
                        emitter.emit_inst("RTS", "");
                    }
                }
            } else {
                // Return with no value
                if !emitter.is_inlining() {
                    emitter.emit_inst("RTS", "");
                }
            }
            Ok(())
        }
        Stmt::Assign { target, value } => {
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

                        if let Some(sym) = sym {
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

                        // Check if this is an enum type
                        let is_enum = if let Type::Named(type_name) = &sym.ty {
                            info.type_registry.get_enum(type_name).is_some()
                        } else {
                            false
                        };

                        // Check if this is a multi-byte type (u16/i16/b16, arrays, enums)
                        let is_multibyte = matches!(
                            sym.ty,
                            Type::Array(_, _)
                                | Type::Primitive(crate::ast::PrimitiveType::U16)
                                | Type::Primitive(crate::ast::PrimitiveType::I16)
                                | Type::Primitive(crate::ast::PrimitiveType::B16)
                        ) || is_enum;

                        // Arrays and enums store address in A (low) and X (high)
                        // Other u16 types store in A (low) and Y (high)
                        let is_array_or_enum = matches!(sym.ty, Type::Array(_, _)) || is_enum;

                        match sym.location {
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

                    // Use first variable slot for loop variable (same as normal loops)
                    // This matches the allocation strategy in semantic analysis
                    let loop_var_addr = emitter.memory_layout.variable_alloc_start;

                    // Create end label for break statements
                    let end_label = emitter.next_label("ux");

                    // Generate body for each iteration with loop variable set
                    for i in 0..count {
                        let iter_val = start + i;

                        // Set loop variable to current iteration value
                        emitter.emit_comment(&format!("{} = {}", var_name.node, iter_val));
                        emitter.emit_inst("LDA", &format!("#${:02X}", iter_val as u8));
                        emitter.emit_inst("STA", &format!("${:02X}", loop_var_addr));

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

            let loop_label = emitter.next_label("fe");
            let end_label = emitter.next_label("fz");

            // Initialize counter to 0 in X register
            emitter.emit_inst("LDX", "#$00");
            emitter
                .reg_state
                .set_x(crate::codegen::regstate::RegisterValue::Immediate(0));

            // For strings, we need to load the length first
            // For arrays, the size is known at compile time
            let array_size = if is_string {
                // For strings, get length at runtime
                emitter.emit_comment("String iteration - load length");
                // Load string pointer to temp location
                emitter.emit_inst("LDA", &format!("${:02X}", iterable_base));
                emitter.emit_inst("STA", "$F0");
                emitter.emit_inst("LDA", &format!("${:02X}", iterable_base + 1));
                emitter.emit_inst("STA", "$F1");
                // Load length (first byte)
                emitter.emit_inst("LDY", "#$00");
                emitter.emit_inst("LDA", "($F0),Y");
                // Store length in temp location for comparison
                emitter.emit_inst("STA", "$F2");
                None // Will compare against $F2
            } else {
                // For arrays, size is known at compile time
                match &iterable_ty {
                    crate::sema::types::Type::Array(_, sz) => Some(*sz),
                    _ => {
                        return Err(CodegenError::UnsupportedOperation(
                            "ForEach requires array or string type".to_string(),
                        ));
                    }
                }
            };

            // Loop start
            emitter.emit_label(&loop_label);

            // Check if counter (X) >= length
            if is_string {
                // Compare X against string length in $F2
                emitter.emit_inst("CPX", "$F2");
            } else if let Some(size) = array_size {
                // Compare X against known array size
                emitter.emit_inst("CPX", &format!("#${:02X}", size));
            }
            emitter.emit_inst("BCS", &end_label); // Branch if X >= size

            // Push loop context for break/continue
            emitter.push_loop(loop_label.clone(), end_label.clone());

            // Store index in index variable if present
            if let Some(idx_var) = index_var
                && let Some(idx_sym) = info.resolved_symbols.get(&idx_var.span) {
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

            // Load iterable[X] into A using indirect indexed: LDA (ptr),Y
            // Transfer X to Y for indexing
            emitter.emit_inst("TXA", "");
            emitter.emit_inst("TAY", "");

            if is_string {
                // For strings, add 1 to skip length byte
                emitter.emit_inst("INY", "");
                emitter.emit_inst("LDA", "($F0),Y");
            } else {
                // For arrays, direct indexed access
                emitter.emit_inst("LDA", &format!("(${:02X}),Y", iterable_base));
            }

            // Store the element in the loop variable
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
                            "ForEach loop variable must have concrete location".to_string(),
                        ));
                    }
                }
            } else {
                return Err(CodegenError::SymbolNotFound(var_name.node.clone()));
            }

            // Restore counter from stack (it's still in X, so no need)
            emitter.reg_state.invalidate_all();

            // Execute loop body
            generate_stmt(body, emitter, info, string_collector)?;

            // Pop loop context
            emitter.pop_loop();

            // Increment counter
            emitter.emit_inst("INX", "");
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

fn generate_index_assignment(
    object: &Spanned<crate::ast::Expr>,
    index: &Spanned<crate::ast::Expr>,
    value: &Spanned<crate::ast::Expr>,
    emitter: &mut Emitter,
    info: &ProgramInfo,
    string_collector: &mut StringCollector,
) -> Result<(), CodegenError> {
    use crate::ast::Expr;
    use crate::ast::PrimitiveType;
    use crate::sema::table::SymbolLocation;
    use crate::sema::types::Type;

    emitter.emit_comment("Array element assignment");

    // Step 1: Get the element type for the array
    let object_type = info.resolved_types.get(&object.span).ok_or_else(|| {
        CodegenError::UnsupportedOperation("Type information not found".to_string())
    })?;

    let element_type = match object_type {
        Type::Array(elem_ty, _size) => elem_ty,
        _ => {
            return Err(CodegenError::UnsupportedOperation(
                "Can only index arrays".to_string(),
            ));
        }
    };

    let is_multibyte = matches!(
        &**element_type,
        Type::Primitive(PrimitiveType::U16)
            | Type::Primitive(PrimitiveType::I16)
            | Type::Primitive(PrimitiveType::B16)
    );

    // Step 2: Evaluate the value expression
    emitter.emit_comment("Evaluate value to assign");
    generate_expr(value, emitter, info, string_collector)?;

    // Step 3: Save value to temp storage
    emitter.emit_comment("Save value to temp");
    emitter.emit_inst("STA", "$20"); // Save low byte
    if is_multibyte {
        emitter.emit_inst("STY", "$21"); // Save high byte for u16
    }

    // Step 4: Evaluate index expression
    emitter.emit_comment("Evaluate index");
    generate_expr(index, emitter, info, string_collector)?;

    // Step 5: Transfer index to Y register
    emitter.emit_inst("TAY", "");

    // Step 6: Get array base address
    // For now, only support simple variable arrays
    if let Expr::Variable(array_name) = &object.node {
        let sym = info
            .resolved_symbols
            .get(&object.span)
            .or_else(|| info.table.lookup(array_name))
            .ok_or_else(|| CodegenError::SymbolNotFound(array_name.clone()))?;

        match sym.location {
            SymbolLocation::ZeroPage(addr) => {
                // For u8 arrays: direct indexed addressing
                if !is_multibyte {
                    // Restore value
                    emitter.emit_inst("LDA", "$20");
                    // Store to array[index]
                    emitter.emit_inst("STA", &format!("(${:02X}),Y", addr));
                } else {
                    // For u16 arrays: need to scale index by 2
                    emitter.emit_comment("Scale index for u16 array (multiply by 2)");
                    emitter.emit_inst("TYA", ""); // Get index back to A
                    emitter.emit_inst("ASL", "A"); // Multiply by 2
                    emitter.emit_inst("TAY", ""); // Back to Y

                    // Restore and store low byte
                    emitter.emit_inst("LDA", "$20");
                    emitter.emit_inst("STA", &format!("(${:02X}),Y", addr));

                    // Store high byte at next position
                    emitter.emit_inst("INY", "");
                    emitter.emit_inst("LDA", "$21");
                    emitter.emit_inst("STA", &format!("(${:02X}),Y", addr));
                }
            }
            SymbolLocation::Absolute(addr) if addr < 256 => {
                let addr_u8 = addr as u8;
                // For u8 arrays: direct indexed addressing
                if !is_multibyte {
                    // Restore value
                    emitter.emit_inst("LDA", "$20");
                    // Store to array[index]
                    emitter.emit_inst("STA", &format!("(${:02X}),Y", addr_u8));
                } else {
                    // For u16 arrays: need to scale index by 2
                    emitter.emit_comment("Scale index for u16 array (multiply by 2)");
                    emitter.emit_inst("TYA", ""); // Get index back to A
                    emitter.emit_inst("ASL", "A"); // Multiply by 2
                    emitter.emit_inst("TAY", ""); // Back to Y

                    // Restore and store low byte
                    emitter.emit_inst("LDA", "$20");
                    emitter.emit_inst("STA", &format!("(${:02X}),Y", addr_u8));

                    // Store high byte at next position
                    emitter.emit_inst("INY", "");
                    emitter.emit_inst("LDA", "$21");
                    emitter.emit_inst("STA", &format!("(${:02X}),Y", addr_u8));
                }
            }
            _ => {
                return Err(CodegenError::UnsupportedOperation(format!(
                    "Array '{}' must be in zero page for indexed assignment",
                    array_name
                )));
            }
        }
    } else {
        return Err(CodegenError::UnsupportedOperation(
            "Can only assign to array variables, not expressions".to_string(),
        ));
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn generate_slice_assignment(
    object: &Spanned<crate::ast::Expr>,
    start: &Spanned<crate::ast::Expr>,
    end: &Spanned<crate::ast::Expr>,
    inclusive: bool,
    value: &Spanned<crate::ast::Expr>,
    emitter: &mut Emitter,
    info: &ProgramInfo,
    string_collector: &mut StringCollector,
) -> Result<(), CodegenError> {
    use crate::ast::{Expr, Literal};
    use crate::sema::const_eval::eval_const_expr_with_env;
    use crate::sema::table::SymbolLocation;

    emitter.emit_comment("Slice assignment");

    // Get the array variable info
    let array_name = if let Expr::Variable(name) = &object.node {
        name
    } else {
        return Err(CodegenError::UnsupportedOperation(
            "Slice assignment only supported on array variables".to_string(),
        ));
    };

    let sym = info
        .resolved_symbols
        .get(&object.span)
        .or_else(|| info.table.lookup(array_name))
        .ok_or_else(|| CodegenError::SymbolNotFound(array_name.clone()))?;

    let addr = match sym.location {
        SymbolLocation::ZeroPage(a) => a,
        SymbolLocation::Absolute(a) if a < 256 => a as u8,
        _ => {
            return Err(CodegenError::UnsupportedOperation(format!(
                "Array '{}' must be in zero page for slice assignment",
                array_name
            )));
        }
    };

    // Try to evaluate slice bounds as constants
    let const_env = HashMap::default();
    let start_val = eval_const_expr_with_env(start, &const_env)
        .ok()
        .and_then(|v| v.as_integer())
        .map(|v| v as usize);
    let end_val = eval_const_expr_with_env(end, &const_env)
        .ok()
        .and_then(|v| v.as_integer())
        .map(|v| v as usize);

    // Get values from RHS (must be an array literal for now)
    let values = match &value.node {
        Expr::Literal(Literal::Array(elems)) => elems,
        _ => {
            return Err(CodegenError::UnsupportedOperation(
                "Slice assignment requires an array literal on the right-hand side".to_string(),
            ));
        }
    };

    // If bounds are constant, we can unroll the assignment
    if let (Some(s), Some(e)) = (start_val, end_val) {
        let actual_end = if inclusive { e + 1 } else { e };
        let slice_len = actual_end - s;

        // Verify slice length matches value array length
        if values.len() != slice_len {
            return Err(CodegenError::UnsupportedOperation(format!(
                "Slice length ({}) does not match value array length ({})",
                slice_len,
                values.len()
            )));
        }

        emitter.emit_comment(&format!(
            "Unrolled slice assignment [{}..{}]",
            s, actual_end
        ));

        // Unroll: generate individual stores for each element
        for (i, val_expr) in values.iter().enumerate() {
            let target_index = s + i;

            // Generate the value expression
            generate_expr(val_expr, emitter, info, string_collector)?;

            // Store to array[target_index] using indirect indexed addressing
            emitter.emit_inst("LDY", &format!("#${:02X}", target_index));
            emitter.emit_inst("STA", &format!("(${:02X}),Y", addr));
        }
    } else {
        // Dynamic bounds - not supported yet
        return Err(CodegenError::UnsupportedOperation(
            "Slice assignment with non-constant bounds is not yet supported".to_string(),
        ));
    }

    Ok(())
}

fn generate_field_assignment(
    object: &Spanned<crate::ast::Expr>,
    field: &Spanned<String>,
    value: &Spanned<crate::ast::Expr>,
    emitter: &mut Emitter,
    info: &ProgramInfo,
    string_collector: &mut StringCollector,
) -> Result<(), CodegenError> {
    use crate::ast::Expr;
    use crate::ast::PrimitiveType;
    use crate::sema::table::SymbolLocation;
    use crate::sema::types::Type;

    // Get the base object (must be a variable for now)
    if let Expr::Variable(var_name) = &object.node {
        // Look up the variable
        let sym = info
            .resolved_symbols
            .get(&object.span)
            .or_else(|| info.table.lookup(var_name))
            .ok_or_else(|| CodegenError::SymbolNotFound(var_name.clone()))?;

        // Get the base address of the struct
        let base_addr = match sym.location {
            SymbolLocation::ZeroPage(addr) => addr as u16,
            SymbolLocation::Absolute(addr) => addr,
            _ => {
                return Err(CodegenError::UnsupportedOperation(format!(
                    "Cannot assign to field of variable with location: {:?}",
                    sym.location
                )));
            }
        };

        // Get the struct type name from the symbol's type
        let struct_name = if let Type::Named(name) = &sym.ty {
            name
        } else {
            return Err(CodegenError::UnsupportedOperation(format!(
                "variable '{}' is not a struct type",
                var_name
            )));
        };

        // Look up the struct definition
        let struct_def = info.type_registry.get_struct(struct_name).ok_or_else(|| {
            CodegenError::UnsupportedOperation(format!(
                "struct '{}' not found in type registry",
                struct_name
            ))
        })?;

        // Find the field and get its offset
        let field_info = struct_def.get_field(&field.node).ok_or_else(|| {
            CodegenError::UnsupportedOperation(format!(
                "field '{}' not found in struct '{}'",
                field.node, struct_name
            ))
        })?;

        // Check if field is multi-byte
        let is_multibyte = matches!(
            &field_info.ty,
            Type::Primitive(PrimitiveType::U16)
                | Type::Primitive(PrimitiveType::I16)
                | Type::Primitive(PrimitiveType::B16)
        );

        emitter.emit_comment(&format!("Field assignment: {}.{}", var_name, field.node));

        // Check if this is a parameter (pass-by-reference)
        // Parameters are in the param region ($80-$BF)
        let param_base = emitter.memory_layout.param_base;
        let param_end = emitter.memory_layout.param_end;
        let is_parameter = base_addr >= param_base as u16 && base_addr <= param_end as u16;

        // Generate value expression (result in A, or A/Y for u16)
        generate_expr(value, emitter, info, string_collector)?;

        if is_parameter {
            // Check if this struct param has a local pointer copy
            // (prevents clobbering on nested calls)
            let local_ptr_addr = emitter
                .current_function()
                .and_then(|fn_name| info.function_metadata.get(fn_name))
                .and_then(|meta| meta.struct_param_locals.get(var_name))
                .copied();

            let ptr_addr = local_ptr_addr.unwrap_or(base_addr as u8);

            // Use indirect indexed addressing: STA ($ptr),Y
            // Need to save A first since we'll need Y for the offset
            let offset = field_info.offset;

            // Save value to temp
            emitter.emit_inst("STA", "$20"); // Save low byte
            if is_multibyte {
                emitter.emit_inst("STY", "$21"); // Save high byte
            }

            // Set Y to field offset and store via indirect
            emitter.emit_inst("LDY", &format!("#${:02X}", offset));
            emitter.emit_inst("LDA", "$20"); // Restore value
            emitter.emit_inst("STA", &format!("(${:02X}),Y", ptr_addr));

            if is_multibyte {
                // Store high byte at next offset
                emitter.emit_inst("INY", "");
                emitter.emit_inst("LDA", "$21");
                emitter.emit_inst("STA", &format!("(${:02X}),Y", ptr_addr));
            }
        } else {
            // Local struct - direct access
            let field_addr = base_addr + field_info.offset as u16;

            if field_addr < 0x100 {
                emitter.emit_inst("STA", &format!("${:02X}", field_addr));
                if is_multibyte {
                    emitter.emit_inst("STY", &format!("${:02X}", field_addr + 1));
                }
            } else {
                emitter.emit_inst("STA", &format!("${:04X}", field_addr));
                if is_multibyte {
                    emitter.emit_inst("STY", &format!("${:04X}", field_addr + 1));
                }
            }
        }

        Ok(())
    } else {
        Err(CodegenError::UnsupportedOperation(
            "Field assignment only supported on variables (not expressions)".to_string(),
        ))
    }
}

fn generate_match(
    expr: &Spanned<crate::ast::Expr>,
    arms: &[crate::ast::MatchArm],
    emitter: &mut Emitter,
    info: &ProgramInfo,
    string_collector: &mut StringCollector,
) -> Result<(), CodegenError> {
    // Determine the best strategy for this match
    let strategy = determine_match_strategy(arms, info);

    match strategy {
        MatchStrategy::Sequential => {
            generate_match_sequential(expr, arms, emitter, info, string_collector)
        }
        MatchStrategy::JumpTable {
            max_tag,
            wildcard_arm_index,
        } => generate_match_jump_table(
            expr,
            arms,
            emitter,
            info,
            string_collector,
            max_tag,
            wildcard_arm_index,
        ),
    }
}

/// Generate match statement using sequential CMP/BEQ comparisons
///
/// Used for small matches (1-2 arms) or non-enum patterns.
/// May fail for large matches if arm bodies exceed BEQ branch distance (127 bytes).
fn generate_match_sequential(
    expr: &Spanned<crate::ast::Expr>,
    arms: &[crate::ast::MatchArm],
    emitter: &mut Emitter,
    info: &ProgramInfo,
    string_collector: &mut StringCollector,
) -> Result<(), CodegenError> {
    use crate::ast::Pattern;

    let match_id = emitter.next_match_id();

    emitter.emit_comment("Match statement (sequential)");

    // Check if we're matching on an enum by looking at the first pattern
    let is_enum_match = arms
        .iter()
        .any(|arm| matches!(arm.pattern.node, Pattern::EnumVariant { .. }));

    // Evaluate the matched expression into accumulator
    generate_expr(expr, emitter, info, string_collector)?;

    // Use pointer ops area for indirect addressing to avoid conflict with temp storage
    let ptr_base = emitter.memory_layout.pointer_ops_start; // $30 by default

    if is_enum_match {
        // For enum matching, expression returns a pointer in A:X
        // Store pointer at pointer ops area (not $20 which is used by temp storage)
        emitter.emit_inst("STA", &format!("${:02X}", ptr_base));
        emitter.emit_inst("STX", &format!("${:02X}", ptr_base + 1));

        // Load the discriminant tag from the enum (first byte)
        emitter.emit_inst("LDY", "#$00");
        emitter.emit_inst("LDA", &format!("(${:02X}),Y", ptr_base));
        emitter.emit_inst("STA", &format!("${:02X}", ptr_base + 2)); // Store tag
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
                if let crate::ast::Expr::Literal(crate::ast::Literal::Integer(val)) = &lit_expr.node
                {
                    // For enum matching, we already have the tag in $22, but this is for literal patterns
                    // which shouldn't mix with enum patterns in the same match
                    emitter.emit_inst("LDA", "$20");
                    emitter.emit_inst("CMP", &format!("#${:02X}", val));
                    emitter.emit_inst("BEQ", &format!("match_{}_arm_{}", match_id, i));
                }
            }
            Pattern::Range {
                start,
                end,
                inclusive,
            } => {
                // Range check: value >= start && value <= end (or < end+1 for inclusive)
                if let (
                    crate::ast::Expr::Literal(crate::ast::Literal::Integer(start_val)),
                    crate::ast::Expr::Literal(crate::ast::Literal::Integer(end_val)),
                ) = (&start.node, &end.node)
                {
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
            Pattern::EnumVariant {
                enum_name,
                variant,
                bindings,
            } => {
                // Look up the enum definition
                let enum_def = info
                    .type_registry
                    .get_enum(&enum_name.node)
                    .ok_or_else(|| {
                        CodegenError::UnsupportedOperation(format!(
                            "enum '{}' not found in type registry",
                            enum_name.node
                        ))
                    })?;

                // Find the variant
                let variant_info = enum_def.get_variant(&variant.node).ok_or_else(|| {
                    CodegenError::UnsupportedOperation(format!(
                        "variant '{}' not found in enum '{}'",
                        variant.node, enum_name.node
                    ))
                })?;

                // Compare the tag with the expected variant tag
                emitter.emit_inst("LDA", &format!("${:02X}", ptr_base + 2)); // Load stored tag
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

    // Generate arm bodies
    generate_match_arm_bodies(arms, emitter, info, string_collector, match_id)?;

    emitter.emit_label(&format!("match_{}_end", match_id));

    Ok(())
}

/// Generate match statement using jump table dispatch
///
/// Used for enum matches with 3+ arms to avoid BEQ branch distance limitations.
/// The jump table allows arm bodies to be arbitrarily large.
fn generate_match_jump_table(
    expr: &Spanned<crate::ast::Expr>,
    arms: &[crate::ast::MatchArm],
    emitter: &mut Emitter,
    info: &ProgramInfo,
    string_collector: &mut StringCollector,
    max_tag: u8,
    wildcard_arm_index: Option<usize>,
) -> Result<(), CodegenError> {
    let match_id = emitter.next_match_id();
    let jump_ptr = emitter.memory_layout.jump_ptr();

    emitter.emit_comment("Match statement (jump table)");

    // Evaluate the matched expression into accumulator
    // For enum matching, expression returns a pointer in A:X
    generate_expr(expr, emitter, info, string_collector)?;

    // Use pointer ops area for indirect addressing
    let ptr_base = emitter.memory_layout.pointer_ops_start;

    // Store pointer at pointer ops area (not $20 which conflicts with temp storage)
    emitter.emit_inst("STA", &format!("${:02X}", ptr_base));
    emitter.emit_inst("STX", &format!("${:02X}", ptr_base + 1));

    // Load the discriminant tag from the enum (first byte)
    emitter.emit_inst("LDY", "#$00");
    emitter.emit_inst("LDA", &format!("(${:02X}),Y", ptr_base));
    emitter.emit_inst("STA", &format!("${:02X}", ptr_base + 2)); // Store tag for binding extraction

    // Jump table dispatch:
    // 1. Double the tag (addresses are 2 bytes)
    // 2. Use as index into jump table
    // 3. Load address and JMP indirect
    emitter.emit_inst("ASL", ""); // tag * 2
    emitter.emit_inst("TAX", ""); // Transfer to X for indexing
    emitter.emit_inst("LDA", &format!("match_{}_jt,X", match_id));
    emitter.emit_inst("STA", &format!("${:02X}", jump_ptr));
    emitter.emit_inst("LDA", &format!("match_{}_jt+1,X", match_id));
    emitter.emit_inst("STA", &format!("${:02X}", jump_ptr + 1));
    emitter.emit_inst("JMP", &format!("(${:02X})", jump_ptr));

    // Emit jump table
    emit_jump_table(emitter, arms, info, match_id, max_tag, wildcard_arm_index)?;

    // Generate arm bodies
    generate_match_arm_bodies(arms, emitter, info, string_collector, match_id)?;

    // Panic handler for non-exhaustive matches (if no wildcard)
    if wildcard_arm_index.is_none() {
        emitter.emit_label(&format!("match_{}_panic", match_id));
        emitter.emit_comment("Unreachable - non-exhaustive match");
        emitter.emit_inst("BRK", "");
    }

    emitter.emit_label(&format!("match_{}_end", match_id));

    Ok(())
}

/// Emit the jump table for a match statement
///
/// The table contains .WORD entries for each tag value from 0 to max_tag.
/// Missing tags are filled with the wildcard arm label (or panic label if no wildcard).
fn emit_jump_table(
    emitter: &mut Emitter,
    arms: &[crate::ast::MatchArm],
    info: &ProgramInfo,
    match_id: u32,
    max_tag: u8,
    wildcard_arm_index: Option<usize>,
) -> Result<(), CodegenError> {
    use crate::ast::Pattern;

    // Build mapping from tag -> arm index
    let mut tag_to_arm: Vec<Option<usize>> = vec![None; (max_tag + 1) as usize];

    for (arm_index, arm) in arms.iter().enumerate() {
        if let Pattern::EnumVariant {
            enum_name, variant, ..
        } = &arm.pattern.node
            && let Some(enum_def) = info.type_registry.get_enum(&enum_name.node)
            && let Some(variant_info) = enum_def.get_variant(&variant.node)
            && (variant_info.tag as usize) < tag_to_arm.len()
        {
            tag_to_arm[variant_info.tag as usize] = Some(arm_index);
        }
    }

    // Emit jump table label
    emitter.emit_label(&format!("match_{}_jt", match_id));

    // Emit .WORD entries for each tag
    for tag in 0..=max_tag {
        let arm_label = if let Some(arm_index) = tag_to_arm[tag as usize] {
            format!("match_{}_arm_{}", match_id, arm_index)
        } else if let Some(wildcard_index) = wildcard_arm_index {
            format!("match_{}_arm_{}", match_id, wildcard_index)
        } else {
            format!("match_{}_panic", match_id)
        };
        emitter.emit_word_label(&arm_label);
    }

    Ok(())
}

/// Generate arm bodies for a match statement
///
/// Shared between sequential and jump table strategies.
fn generate_match_arm_bodies(
    arms: &[crate::ast::MatchArm],
    emitter: &mut Emitter,
    info: &ProgramInfo,
    string_collector: &mut StringCollector,
    match_id: u32,
) -> Result<(), CodegenError> {
    use crate::ast::Pattern;

    for (i, arm) in arms.iter().enumerate() {
        emitter.emit_label(&format!("match_{}_arm_{}", match_id, i));

        // Extract bindings for enum variant patterns
        if let Pattern::EnumVariant {
            enum_name,
            variant,
            bindings,
        } = &arm.pattern.node
            && !bindings.is_empty()
        {
            // Look up the enum definition to get field information
            let enum_def = info
                .type_registry
                .get_enum(&enum_name.node)
                .ok_or_else(|| {
                    CodegenError::UnsupportedOperation(format!(
                        "enum '{}' not found in type registry",
                        enum_name.node
                    ))
                })?;

            let variant_info = enum_def.get_variant(&variant.node).ok_or_else(|| {
                CodegenError::UnsupportedOperation(format!(
                    "variant '{}' not found in enum '{}'",
                    variant.node, enum_name.node
                ))
            })?;

            // Extract field values from enum data
            // Enum layout in memory: [tag: u8][field0][field1]...
            // The pointer in pointer ops area points to the tag byte
            // Field data starts at offset 1
            let ptr_base = emitter.memory_layout.pointer_ops_start;

            match &variant_info.data {
                crate::sema::type_defs::VariantData::Tuple(field_types) => {
                    // Tuple variant: extract each field by position
                    if bindings.len() != field_types.len() {
                        return Err(CodegenError::UnsupportedOperation(format!(
                            "Pattern binding count mismatch: expected {}, got {}",
                            field_types.len(),
                            bindings.len()
                        )));
                    }

                    let mut offset = 1; // Start after the tag byte
                    for (binding, field_type) in bindings.iter().zip(field_types.iter()) {
                        // Load field value using indirect indexed addressing
                        emitter.emit_inst("LDY", &format!("#${:02X}", offset));
                        emitter.emit_inst("LDA", &format!("(${:02X}),Y", ptr_base));

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
                                    return Err(CodegenError::UnsupportedOperation(format!(
                                        "Binding '{}' has unsupported location",
                                        binding.name.node
                                    )));
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
                        "Pattern bindings for struct variants not yet implemented".to_string(),
                    ));
                }
                crate::sema::type_defs::VariantData::Unit => {
                    // Unit variant shouldn't have bindings
                    if !bindings.is_empty() {
                        return Err(CodegenError::UnsupportedOperation(
                            "Unit variant should not have bindings".to_string(),
                        ));
                    }
                }
            }
        }

        generate_stmt(&arm.body, emitter, info, string_collector)?;

        // Only emit JMP if the arm body doesn't already terminate control flow
        // (e.g., return, break, continue) - this eliminates dead code
        if !stmt_terminates(&arm.body.node) {
            emitter.emit_inst("JMP", &format!("match_{}_end", match_id));
        }
    }

    Ok(())
}

/// Substitute {variable} patterns in inline assembly with actual addresses
fn substitute_asm_vars(
    instruction: &str,
    info: &ProgramInfo,
    current_function: Option<&str>,
) -> Result<String, CodegenError> {
    let mut result = instruction.to_string();

    // Find all {var} patterns
    while let Some(start) = result.find('{') {
        if let Some(end) = result[start..].find('}') {
            let end = start + end;
            let var_name = &result[start + 1..end];

            // Look up the variable in resolved_symbols (by name)
            // We search through resolved_symbols because the symbol table's scopes
            // have been exited after semantic analysis
            // Priority: 1) Local variables in current function, 2) Global symbols
            let symbol = info
                .resolved_symbols
                .values()
                .find(|s| {
                    s.name == var_name
                        && (s.containing_function.as_deref() == current_function
                            || s.containing_function.is_none())
                })
                // Prefer local over global if both exist with same name
                .or_else(|| {
                    info.resolved_symbols
                        .values()
                        .find(|s| s.name == var_name && s.containing_function.is_none())
                })
                .ok_or_else(|| CodegenError::SymbolNotFound(var_name.to_string()))?;

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

/// Uniquify assembly labels by appending a suffix
/// This is needed when inlining functions to avoid duplicate label errors
fn uniquify_asm_labels(line: &str, suffix: usize) -> String {
    let trimmed = line.trim();

    // Check if this is a label definition (ends with :)
    if let Some(label_name) = trimmed.strip_suffix(':') {
        // Label definition: append suffix before the colon
        return format!("{}_{}:", label_name, suffix);
    }

    // Check if line contains a label reference
    // Label references are typically in the operand part of an instruction
    let parts: Vec<&str> = trimmed.split_whitespace().collect();
    if parts.len() < 2 {
        // No operand, return as-is
        return line.to_string();
    }

    let mnemonic = parts[0];
    let operand = parts[1..].join(" ");

    // Special case: BBS/BBR instructions have format "BBS0 $20,label"
    // where the label is after a comma
    if (mnemonic.starts_with("BBS") || mnemonic.starts_with("BBR"))
        && let Some(comma_pos) = operand.find(',')
    {
        let addr_part = &operand[..comma_pos];
        let label_part = operand[comma_pos + 1..].trim();
        return format!("{} {},{}_{}", mnemonic, addr_part, label_part, suffix);
    }

    // Check if operand looks like a label reference
    // Labels are alphanumeric/underscore, not registers ($, #, A, X, Y) or numbers
    let is_label_ref = !operand.starts_with('$')  // Not hex address
                    && !operand.starts_with('#')  // Not immediate
                    && operand != "A"              // Not accumulator
                    && !operand.starts_with("A,") // Not indexed
                    && !operand.starts_with("X")  // Not X register
                    && !operand.starts_with("Y")  // Not Y register
                    && operand.chars().next().is_some_and(|c| c.is_alphabetic() || c == '_');

    if is_label_ref {
        // Split operand by comma (for "label,X" style addressing)
        let op_parts: Vec<&str> = operand.split(',').collect();
        let label_part = op_parts[0];
        let rest = if op_parts.len() > 1 {
            format!(",{}", op_parts[1..].join(","))
        } else {
            String::new()
        };

        format!("{} {}_{}{}", mnemonic, label_part, suffix, rest)
    } else {
        line.to_string()
    }
}

/// Generate a normal (non-unrolled) for loop
fn generate_normal_loop(
    var_name: &Spanned<String>,
    range: &crate::ast::Range,
    body: &Spanned<Stmt>,
    emitter: &mut Emitter,
    info: &ProgramInfo,
    string_collector: &mut StringCollector,
) -> Result<(), CodegenError> {
    // For-loops use X register for the counter (fast increment with INX)
    // u16 arithmetic uses Y register for high bytes to avoid conflicts

    let loop_end_temp = emitter.memory_layout.loop_end_temp();
    let loop_label = emitter.next_label("fl");
    let end_label = emitter.next_label("fx");

    // Initialize loop counter with range start
    generate_expr(&range.start, emitter, info, string_collector)?;
    emitter.emit_inst("TAX", ""); // Counter in X register

    // Generate end value and store in temp location
    generate_expr(&range.end, emitter, info, string_collector)?;
    emitter.emit_inst("STA", &format!("${:02X}", loop_end_temp));

    // Store X (loop counter) to the loop variable location
    if let Some(sym) = info
        .resolved_symbols
        .get(&body.span)
        .or_else(|| info.table.lookup(&var_name.node))
    {
        match sym.location {
            crate::sema::table::SymbolLocation::ZeroPage(addr) => {
                emitter.emit_inst("STX", &format!("${:02X}", addr));
            }
            crate::sema::table::SymbolLocation::Absolute(addr) => {
                emitter.emit_inst("STX", &format!("${:04X}", addr));
            }
            _ => {}
        }
    }

    // Loop start
    emitter.emit_label(&loop_label);

    // Check condition: compare counter with end value
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

    // Execute body
    emitter.reg_state.invalidate_all(); // Body might use registers
    generate_stmt(body, emitter, info, string_collector)?;

    // Pop loop context
    emitter.pop_loop();

    // Increment counter
    emitter.emit_inst("INX", "");

    // Update loop variable with new counter value
    if let Some(sym) = info
        .resolved_symbols
        .get(&body.span)
        .or_else(|| info.table.lookup(&var_name.node))
    {
        match sym.location {
            crate::sema::table::SymbolLocation::ZeroPage(addr) => {
                emitter.emit_inst("STX", &format!("${:02X}", addr));
            }
            crate::sema::table::SymbolLocation::Absolute(addr) => {
                emitter.emit_inst("STX", &format!("${:04X}", addr));
            }
            _ => {}
        }
    }

    emitter.emit_inst("JMP", &loop_label);
    emitter.emit_label(&end_label);

    Ok(())
}
