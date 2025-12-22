//! Expression Code Generation
//!
//! Compiles expressions into assembly instructions.
//! Result is typically left in the Accumulator (A).

use crate::ast::{Expr, Spanned};
use crate::codegen::{CodegenError, Emitter};
use crate::sema::ProgramInfo;
use crate::sema::table::SymbolLocation;

/// Check if an expression is "simple" - can be re-evaluated cheaply without side effects
/// Simple expressions: literals, variables
/// Complex expressions: function calls, binary ops, array indexing, etc.
fn is_simple_expr(expr: &Expr) -> bool {
    matches!(expr, Expr::Literal(_) | Expr::Variable(_))
}

pub fn generate_expr(
    expr: &Spanned<Expr>,
    emitter: &mut Emitter,
    info: &ProgramInfo,
) -> Result<(), CodegenError> {
    // Check if this expression was constant-folded
    if let Some(const_val) = info.folded_constants.get(&expr.span) {
        match const_val {
            crate::sema::const_eval::ConstValue::Integer(n) => {
                // Load the constant value directly
                let val = (*n as u64) & 0xFF; // Truncate to u8 for now
                emitter.emit_inst("LDA", &format!("#${:02X}", val));
                return Ok(());
            }
            crate::sema::const_eval::ConstValue::Bool(b) => {
                emitter.emit_inst("LDA", if *b { "#$01" } else { "#$00" });
                return Ok(());
            }
            _ => {
                // Fall through to normal codegen for non-integer constants
            }
        }
    }

    match &expr.node {
        Expr::Literal(lit) => generate_literal(lit, emitter),
        Expr::Variable(name) => generate_variable(name, expr.span, emitter, info),
        Expr::Binary { left, op, right } => generate_binary(left, *op, right, emitter, info),
        Expr::Unary { op, operand } => generate_unary(*op, operand, emitter, info),
        Expr::Call { function, args } => generate_call(function, args, emitter, info),
        Expr::Paren(inner) => generate_expr(inner, emitter, info), // Just unwrap
        Expr::Cast { expr: inner, target_type } => {
            generate_type_cast(inner, target_type, emitter, info)
        }
        Expr::Index { object, index } => generate_index(object, index, emitter, info),
        Expr::StructInit { name, fields } => generate_struct_init(name, fields, emitter, info),
        Expr::Field { object, field } => generate_field_access(object, field, emitter, info),
        Expr::EnumVariant {
            enum_name,
            variant,
            data,
        } => generate_enum_variant(enum_name, variant, data, emitter, info),
        _ => Ok(()), // TODO: Implement other expressions
    }
}

fn generate_call(
    function: &Spanned<String>,
    args: &[Spanned<Expr>],
    emitter: &mut Emitter,
    info: &ProgramInfo,
) -> Result<(), CodegenError> {
    // 6502 calling convention: Arguments are passed in zero page locations
    // Parameters are allocated starting at param_base (from memory layout)
    // This avoids using the hardware stack which is limited and slow to access

    let param_base = emitter.memory_layout.param_base;

    emitter.emit_comment(&format!("Call {} with {} args", function.node, args.len()));

    // Store each argument to its corresponding parameter location
    for (i, arg) in args.iter().enumerate() {
        let param_addr = param_base + i as u8;

        // Generate argument expression (result in A)
        generate_expr(arg, emitter, info)?;

        // Store to parameter location
        emitter.emit_inst("STA", &format!("${:02X}", param_addr));
    }

    // Call the function
    emitter.emit_inst("JSR", &function.node);

    // Result is returned in A register (no cleanup needed)

    Ok(())
}

fn generate_unary(
    op: crate::ast::UnaryOp,
    operand: &Spanned<Expr>,
    emitter: &mut Emitter,
    info: &ProgramInfo,
) -> Result<(), CodegenError> {
    match op {
        crate::ast::UnaryOp::Deref => {
            // Dereference: *ptr - Load value from address stored in operand
            generate_expr(operand, emitter, info)?;

            // A now contains the address (low byte for u8 pointers)
            // For 6502, we need to set up indirect addressing
            // Store address in zero page for indirect access
            emitter.emit_inst("STA", "$30"); // Store low byte at $30

            // For u16 pointers, we'd also load high byte:
            // For now, assume u8 addresses or low byte only
            emitter.emit_inst("LDA", "#$00");
            emitter.emit_inst("STA", "$31"); // High byte = 0 for zero page

            // Load value using indirect addressing
            emitter.emit_inst("LDY", "#$00");
            emitter.emit_inst("LDA", "($30),Y"); // Indirect indexed addressing

            return Ok(());
        }
        crate::ast::UnaryOp::AddrOf | crate::ast::UnaryOp::AddrOfMut => {
            // Address-of: &var or &mut var - Get the address of a variable
            // Don't evaluate the operand, just get its address
            if let Expr::Variable(name) = &operand.node {
                if let Some(sym) = info.table.lookup(name) {
                    let addr = match sym.location {
                        crate::sema::table::SymbolLocation::ZeroPage(zp) => zp as u16,
                        crate::sema::table::SymbolLocation::Absolute(abs) => abs,
                        _ => {
                            return Err(CodegenError::UnsupportedOperation(
                                format!("Cannot take address of variable with location: {:?}", sym.location)
                            ));
                        }
                    };

                    // Load the address into A (low byte)
                    emitter.emit_inst("LDA", &format!("#${:02X}", addr & 0xFF));

                    // For u16 addresses, high byte would go in another register
                    // TODO: Handle 16-bit address-of properly

                    return Ok(());
                } else {
                    return Err(CodegenError::SymbolNotFound(name.clone()));
                }
            } else {
                return Err(CodegenError::UnsupportedOperation(
                    "Address-of (&) only supported on variables".to_string()
                ));
            }
        }
        _ => {}
    }

    // For other operations, evaluate operand first
    generate_expr(operand, emitter, info)?;

    // Apply unary operation to A
    match op {
        crate::ast::UnaryOp::Neg => {
            // Two's complement: ~A + 1
            emitter.emit_inst("EOR", "#$FF"); // Bitwise NOT
            emitter.emit_inst("CLC", "");
            emitter.emit_inst("ADC", "#$01"); // Add 1
        }
        crate::ast::UnaryOp::BitNot => {
            // Bitwise NOT
            emitter.emit_inst("EOR", "#$FF");
        }
        crate::ast::UnaryOp::Not => {
            // Logical NOT: convert to boolean (0 or 1) and invert
            let true_label = emitter.next_label("not_true");
            let end_label = emitter.next_label("not_end");

            emitter.emit_inst("CMP", "#$00");
            emitter.emit_inst("BEQ", &true_label); // If zero, result is true (1)

            // False case (input was non-zero)
            emitter.emit_inst("LDA", "#$00");
            emitter.emit_inst("JMP", &end_label);

            // True case (input was zero)
            emitter.emit_label(&true_label);
            emitter.emit_inst("LDA", "#$01");

            emitter.emit_label(&end_label);
        }
        crate::ast::UnaryOp::Deref | crate::ast::UnaryOp::AddrOf | crate::ast::UnaryOp::AddrOfMut => {
            // Already handled above
        }
    }

    Ok(())
}

fn generate_binary(
    left: &Spanned<Expr>,
    op: crate::ast::BinaryOp,
    right: &Spanned<Expr>,
    emitter: &mut Emitter,
    info: &ProgramInfo,
) -> Result<(), CodegenError> {
    // Handle short-circuit logical operations specially
    match op {
        crate::ast::BinaryOp::And => return generate_logical_and(left, right, emitter, info),
        crate::ast::BinaryOp::Or => return generate_logical_or(left, right, emitter, info),
        _ => {}
    }

    // Optimization: Avoid stack if left operand is simple (variable or literal)
    let use_stack = !is_simple_expr(&left.node);

    if use_stack {
        // Complex left expression: use stack
        // 1. Generate left operand -> A
        generate_expr(left, emitter, info)?;

        // 2. Push A to stack to save it
        emitter.emit_inst("PHA", "");

        // 3. Generate right operand -> A
        generate_expr(right, emitter, info)?;

        // 4. Store right operand in TEMP
        emitter.emit_inst("STA", &format!("${:02X}", emitter.memory_layout.temp_reg()));

        // 5. Restore left operand -> A
        emitter.emit_inst("PLA", "");
    } else {
        // Simple left expression: evaluate right first, store in temp, then eval left
        // This saves PHA/PLA instructions (4 cycles)

        // 1. Generate right operand -> A
        generate_expr(right, emitter, info)?;

        // 2. Store right operand in TEMP
        emitter.emit_inst("STA", &format!("${:02X}", emitter.memory_layout.temp_reg()));

        // 3. Generate left operand -> A (simple, no side effects)
        generate_expr(left, emitter, info)?;
    }

    // 6. Perform operation
    match op {
        crate::ast::BinaryOp::Add => {
            emitter.emit_inst("CLC", "");
            emitter.emit_inst("ADC", &format!("${:02X}", emitter.memory_layout.temp_reg()));
        }
        crate::ast::BinaryOp::Sub => {
            emitter.emit_inst("SEC", "");
            emitter.emit_inst("SBC", &format!("${:02X}", emitter.memory_layout.temp_reg()));
        }
        crate::ast::BinaryOp::BitAnd => {
            emitter.emit_inst("AND", &format!("${:02X}", emitter.memory_layout.temp_reg()));
        }
        crate::ast::BinaryOp::BitOr => {
            emitter.emit_inst("ORA", &format!("${:02X}", emitter.memory_layout.temp_reg()));
        }
        crate::ast::BinaryOp::BitXor => {
            emitter.emit_inst("EOR", &format!("${:02X}", emitter.memory_layout.temp_reg()));
        }
        crate::ast::BinaryOp::Shl => {
            generate_shift_left(emitter)?;
        }
        crate::ast::BinaryOp::Shr => {
            generate_shift_right(emitter)?;
        }
        crate::ast::BinaryOp::Mul => {
            generate_multiply(emitter)?;
        }
        crate::ast::BinaryOp::Div => {
            generate_divide(emitter)?;
        }
        crate::ast::BinaryOp::Mod => {
            generate_modulo(emitter)?;
        }
        // Comparison operations - result is boolean (0 or 1)
        crate::ast::BinaryOp::Eq => {
            generate_compare_eq(emitter)?;
        }
        crate::ast::BinaryOp::Ne => {
            generate_compare_ne(emitter)?;
        }
        crate::ast::BinaryOp::Lt => {
            generate_compare_lt(emitter)?;
        }
        crate::ast::BinaryOp::Ge => {
            generate_compare_ge(emitter)?;
        }
        crate::ast::BinaryOp::Gt => {
            // A > B is same as B < A, so swap and use Lt
            // We already have A in accumulator and B in TEMP
            // Just swap them conceptually
            generate_compare_gt(emitter)?;
        }
        crate::ast::BinaryOp::Le => {
            // A <= B is same as B >= A
            generate_compare_le(emitter)?;
        }
        // TODO: Implement other ops (Mul, Div, Shl, Shr, etc.)
        _ => return Err(CodegenError::UnsupportedOperation(format!("Binary operator: {:?}", op))),
    }

    Ok(())
}

fn generate_literal(lit: &crate::ast::Literal, emitter: &mut Emitter) -> Result<(), CodegenError> {
    match lit {
        crate::ast::Literal::Integer(val) => {
            // TODO: Handle values > 255
            // Use optimized load that skips if value already in A
            emitter.emit_lda_immediate(*val);
            Ok(())
        }
        crate::ast::Literal::Bool(val) => {
            let v = if *val { 1 } else { 0 };
            emitter.emit_lda_immediate(v);
            Ok(())
        }
        crate::ast::Literal::String(s) => {
            // Generate string literal with length prefix
            // Create unique label for this string
            let str_label = emitter.next_label("str");
            let skip_label = emitter.next_label("str_skip");

            // Jump over the string data
            emitter.emit_inst("JMP", &skip_label);

            // Emit string data with label
            emitter.emit_label(&str_label);

            // Emit length as u16 (little-endian)
            let len = s.len() as u16;
            emitter.emit_word(len);

            // Emit string bytes
            if !s.is_empty() {
                emitter.emit_bytes(s.as_bytes());
            }

            // Skip label
            emitter.emit_label(&skip_label);

            // Load address of string into A (low byte) and X (high byte)
            // For now, we'll use a comment since we don't have real address resolution
            emitter.emit_comment(&format!("Load address of string: \"{}\"", s));
            emitter.emit_inst("LDA", &format!("#<{}", str_label));
            emitter.emit_inst("LDX", &format!("#>{}", str_label));

            // Result: A = low byte of address, X = high byte of address
            Ok(())
        }
        crate::ast::Literal::Array(elements) => {
            // Generate array literal - store data in memory and return address
            let arr_label = emitter.next_label("arr");
            let skip_label = emitter.next_label("arr_skip");

            // Jump over the array data
            emitter.emit_inst("JMP", &skip_label);

            // Emit array data with label
            emitter.emit_label(&arr_label);

            // Emit each element (assuming u8 for now)
            for elem in elements {
                // For simple integer literals, emit directly
                if let crate::ast::Expr::Literal(crate::ast::Literal::Integer(val)) = &elem.node {
                    emitter.emit_byte(*val as u8);
                } else {
                    // TODO: Support complex expressions in array literals
                    return Err(CodegenError::UnsupportedOperation(
                        "Only integer literals supported in array literals".to_string()
                    ));
                }
            }

            // Skip label
            emitter.emit_label(&skip_label);

            // Load address of array into A (low byte) and X (high byte)
            emitter.emit_comment(&format!("Load address of array ({} elements)", elements.len()));
            emitter.emit_inst("LDA", &format!("#<{}", arr_label));
            emitter.emit_inst("LDX", &format!("#>{}", arr_label));

            Ok(())
        }
        crate::ast::Literal::ArrayFill { value, count } => {
            // Generate array filled with repeated value
            let arr_label = emitter.next_label("arr_fill");
            let skip_label = emitter.next_label("arr_fill_skip");

            // Jump over the array data
            emitter.emit_inst("JMP", &skip_label);

            // Emit array data with label
            emitter.emit_label(&arr_label);

            // Get the fill value (must be a simple literal)
            if let crate::ast::Expr::Literal(crate::ast::Literal::Integer(val)) = &value.node {
                let byte_val = *val as u8;
                // Emit the value 'count' times
                for _ in 0..*count {
                    emitter.emit_byte(byte_val);
                }
            } else {
                return Err(CodegenError::UnsupportedOperation(
                    "Only integer literals supported in array fill".to_string()
                ));
            }

            // Skip label
            emitter.emit_label(&skip_label);

            // Load address of array into A (low byte) and X (high byte)
            emitter.emit_comment(&format!("Load address of filled array ({} elements)", count));
            emitter.emit_inst("LDA", &format!("#<{}", arr_label));
            emitter.emit_inst("LDX", &format!("#>{}", arr_label));

            Ok(())
        }
    }
}

fn generate_variable(
    name: &str,
    span: crate::ast::Span,
    emitter: &mut Emitter,
    info: &ProgramInfo,
) -> Result<(), CodegenError> {
    if let Some(sym) = info.resolved_symbols.get(&span) {
        match sym.location {
            SymbolLocation::Absolute(addr) => {
                // Use optimized load that skips if value already in A
                emitter.emit_lda_abs(addr);
                Ok(())
            }
            SymbolLocation::ZeroPage(addr) => {
                // Use optimized load that skips if value already in A
                emitter.emit_lda_zp(addr);
                Ok(())
            }
            SymbolLocation::Stack(offset) => {
                // Stack relative addressing: LDA $offset, X (assuming X is frame pointer)
                // For now, we don't have a frame pointer setup, so this is a placeholder
                // TODO: Implement stack frame access
                emitter.emit_comment(&format!("Load local {} (offset {})", name, offset));
                Ok(())
            }
            _ => Err(CodegenError::UnsupportedOperation(format!(
                "Variable '{}' has unsupported location type: {:?}",
                name, sym.location
            ))),
        }
    } else {
        // Fallback to global lookup if not found in resolved (shouldn't happen if analyzed correctly)
        if let Some(sym) = info.table.lookup(name) {
            match sym.location {
                SymbolLocation::Absolute(addr) => {
                    emitter.emit_inst("LDA", &format!("${:04X}", addr));
                    Ok(())
                }
                _ => Err(CodegenError::UnsupportedOperation(format!(
                    "Variable '{}' has unsupported location type: {:?}",
                    name, sym.location
                ))),
            }
        } else {
            Err(CodegenError::SymbolNotFound(name.to_string()))
        }
    }
}

// Comparison helper functions
// All assume: A contains left operand, emitter.memory_layout.temp_reg() contains right operand
// Result is left in A as 0 (false) or 1 (true)

fn generate_compare_eq(emitter: &mut Emitter) -> Result<(), CodegenError> {
    // A == TEMP: Compare and set A to 1 if equal, 0 otherwise
    let true_label = emitter.next_label("eq_true");
    let end_label = emitter.next_label("eq_end");

    emitter.emit_inst("CMP", &format!("${:02X}", emitter.memory_layout.temp_reg()));
    emitter.emit_inst("BEQ", &true_label);

    // False case
    emitter.emit_inst("LDA", "#$00");
    emitter.emit_inst("JMP", &end_label);

    // True case
    emitter.emit_label(&true_label);
    emitter.emit_inst("LDA", "#$01");

    emitter.emit_label(&end_label);
    Ok(())
}

fn generate_compare_ne(emitter: &mut Emitter) -> Result<(), CodegenError> {
    // A != TEMP: Opposite of equal
    let true_label = emitter.next_label("ne_true");
    let end_label = emitter.next_label("ne_end");

    emitter.emit_inst("CMP", &format!("${:02X}", emitter.memory_layout.temp_reg()));
    emitter.emit_inst("BNE", &true_label);

    // False case
    emitter.emit_inst("LDA", "#$00");
    emitter.emit_inst("JMP", &end_label);

    // True case
    emitter.emit_label(&true_label);
    emitter.emit_inst("LDA", "#$01");

    emitter.emit_label(&end_label);
    Ok(())
}

fn generate_compare_lt(emitter: &mut Emitter) -> Result<(), CodegenError> {
    // A < TEMP: Use CMP which sets carry flag if A >= TEMP
    let true_label = emitter.next_label("lt_true");
    let end_label = emitter.next_label("lt_end");

    emitter.emit_inst("CMP", &format!("${:02X}", emitter.memory_layout.temp_reg()));
    emitter.emit_inst("BCC", &true_label); // Branch if carry clear (A < TEMP)

    // False case
    emitter.emit_inst("LDA", "#$00");
    emitter.emit_inst("JMP", &end_label);

    // True case
    emitter.emit_label(&true_label);
    emitter.emit_inst("LDA", "#$01");

    emitter.emit_label(&end_label);
    Ok(())
}

fn generate_compare_ge(emitter: &mut Emitter) -> Result<(), CodegenError> {
    // A >= TEMP: Opposite of <
    let true_label = emitter.next_label("ge_true");
    let end_label = emitter.next_label("ge_end");

    emitter.emit_inst("CMP", &format!("${:02X}", emitter.memory_layout.temp_reg()));
    emitter.emit_inst("BCS", &true_label); // Branch if carry set (A >= TEMP)

    // False case
    emitter.emit_inst("LDA", "#$00");
    emitter.emit_inst("JMP", &end_label);

    // True case
    emitter.emit_label(&true_label);
    emitter.emit_inst("LDA", "#$01");

    emitter.emit_label(&end_label);
    Ok(())
}

fn generate_compare_gt(emitter: &mut Emitter) -> Result<(), CodegenError> {
    // A > TEMP: Same as TEMP < A
    // We need to swap: load TEMP into A, then compare with original A
    // But original A is gone. Alternative: A > B is equivalent to NOT(A <= B)
    // A <= B means A < B OR A == B
    // So A > B means A >= B AND A != B
    // Or simply: CMP sets flags, if carry set AND not equal, then A > B

    let true_label = emitter.next_label("gt_true");
    let end_label = emitter.next_label("gt_end");

    emitter.emit_inst("CMP", &format!("${:02X}", emitter.memory_layout.temp_reg()));
    emitter.emit_inst("BEQ", &end_label); // If equal, result is 0 (false)
    emitter.emit_inst("BCS", &true_label); // If carry set and not equal, A > TEMP

    // False case (carry clear, meaning A < TEMP)
    emitter.emit_inst("LDA", "#$00");
    emitter.emit_inst("JMP", &end_label);

    // True case
    emitter.emit_label(&true_label);
    emitter.emit_inst("LDA", "#$01");

    emitter.emit_label(&end_label);
    Ok(())
}

fn generate_compare_le(emitter: &mut Emitter) -> Result<(), CodegenError> {
    // A <= TEMP: Same as NOT(A > TEMP)
    // A <= B means A < B OR A == B
    // CMP: if carry clear OR equal, then A <= TEMP

    let false_label = emitter.next_label("le_false");
    let end_label = emitter.next_label("le_end");

    emitter.emit_inst("CMP", &format!("${:02X}", emitter.memory_layout.temp_reg()));
    emitter.emit_inst("BEQ", &end_label); // If equal, keep A as is, set to 1 after
    emitter.emit_inst("BCS", &false_label); // If A >= TEMP and not equal, A > TEMP (false)

    // True case (carry clear, meaning A < TEMP, or was equal)
    emitter.emit_inst("LDA", "#$01");
    emitter.emit_inst("JMP", &end_label);

    // False case
    emitter.emit_label(&false_label);
    emitter.emit_inst("LDA", "#$00");

    emitter.emit_label(&end_label);
    Ok(())
}

// Shift helper functions
// A contains value to shift, emitter.memory_layout.temp_reg() contains shift amount

fn generate_shift_left(emitter: &mut Emitter) -> Result<(), CodegenError> {
    // Shift A left by emitter.memory_layout.temp_reg() bits
    // Use X register as loop counter

    let loop_label = emitter.next_label("shl_loop");
    let end_label = emitter.next_label("shl_end");

    // Load shift count into X
    emitter.emit_inst("LDX", &format!("${:02X}", emitter.memory_layout.temp_reg()));

    // Check if count is zero
    emitter.emit_inst("CPX", "#$00");
    emitter.emit_inst("BEQ", &end_label);

    // Loop: shift left once per iteration
    emitter.emit_label(&loop_label);
    emitter.emit_inst("ASL", "A"); // Arithmetic shift left
    emitter.emit_inst("DEX", "");
    emitter.emit_inst("BNE", &loop_label);

    emitter.emit_label(&end_label);
    Ok(())
}

fn generate_shift_right(emitter: &mut Emitter) -> Result<(), CodegenError> {
    // Shift A right by emitter.memory_layout.temp_reg() bits
    // Use X register as loop counter

    let loop_label = emitter.next_label("shr_loop");
    let end_label = emitter.next_label("shr_end");

    // Load shift count into X
    emitter.emit_inst("LDX", &format!("${:02X}", emitter.memory_layout.temp_reg()));

    // Check if count is zero
    emitter.emit_inst("CPX", "#$00");
    emitter.emit_inst("BEQ", &end_label);

    // Loop: shift right once per iteration
    emitter.emit_label(&loop_label);
    emitter.emit_inst("LSR", "A"); // Logical shift right
    emitter.emit_inst("DEX", "");
    emitter.emit_inst("BNE", &loop_label);

    emitter.emit_label(&end_label);
    Ok(())
}

// Logical operation helper functions with short-circuit evaluation

fn generate_logical_and(
    left: &Spanned<Expr>,
    right: &Spanned<Expr>,
    emitter: &mut Emitter,
    info: &ProgramInfo,
) -> Result<(), CodegenError> {
    // Short-circuit AND: if left is false, skip right and return false
    let end_label = emitter.next_label("and_end");

    // Evaluate left operand
    generate_expr(left, emitter, info)?;

    // If left is false (0), result is false
    emitter.emit_inst("CMP", "#$00");
    emitter.emit_inst("BEQ", &end_label); // If zero, A is already 0, done

    // Left was true, evaluate right
    generate_expr(right, emitter, info)?;

    // Convert right to boolean (0 or 1)
    let true_label = emitter.next_label("and_true");
    emitter.emit_inst("CMP", "#$00");
    emitter.emit_inst("BNE", &true_label);

    // Right is false
    emitter.emit_inst("LDA", "#$00");
    emitter.emit_inst("JMP", &end_label);

    // Right is true
    emitter.emit_label(&true_label);
    emitter.emit_inst("LDA", "#$01");

    emitter.emit_label(&end_label);
    Ok(())
}

fn generate_logical_or(
    left: &Spanned<Expr>,
    right: &Spanned<Expr>,
    emitter: &mut Emitter,
    info: &ProgramInfo,
) -> Result<(), CodegenError> {
    // Short-circuit OR: if left is true, skip right and return true
    let true_label = emitter.next_label("or_true");
    let eval_right_label = emitter.next_label("or_eval_right");
    let end_label = emitter.next_label("or_end");

    // Evaluate left operand
    generate_expr(left, emitter, info)?;

    // If left is true (non-zero), result is true
    emitter.emit_inst("CMP", "#$00");
    emitter.emit_inst("BEQ", &eval_right_label); // If zero, evaluate right
    emitter.emit_inst("JMP", &true_label); // Non-zero, result is true

    // Left was false, evaluate right
    emitter.emit_label(&eval_right_label);
    generate_expr(right, emitter, info)?;

    // Convert right to boolean
    emitter.emit_inst("CMP", "#$00");
    emitter.emit_inst("BNE", &true_label);

    // Result is false
    emitter.emit_inst("LDA", "#$00");
    emitter.emit_inst("JMP", &end_label);

    // Result is true
    emitter.emit_label(&true_label);
    emitter.emit_inst("LDA", "#$01");

    emitter.emit_label(&end_label);
    Ok(())
}

// Arithmetic helper functions for multiply, divide, modulo
// These require software implementation on 6502

fn generate_multiply(emitter: &mut Emitter) -> Result<(), CodegenError> {
    // Multiply A * TEMP using repeated addition
    // Result in A (will overflow for results > 255)
    const RESULT_REG: u8 = 0x22;

    let loop_label = emitter.next_label("mul_loop");
    let end_label = emitter.next_label("mul_end");

    // Save multiplicand (A) to X
    emitter.emit_inst("TAX", "");

    // Initialize result to 0
    emitter.emit_inst("LDA", "#$00");
    emitter.emit_inst("STA", &format!("${:02X}", RESULT_REG));

    // Check if multiplier is zero
    emitter.emit_inst("LDA", &format!("${:02X}", emitter.memory_layout.temp_reg()));
    emitter.emit_inst("CMP", "#$00");
    emitter.emit_inst("BEQ", &end_label);

    // Store multiplier count in Y
    emitter.emit_inst("TAY", "");

    // Loop: add X to result Y times
    emitter.emit_label(&loop_label);
    emitter.emit_inst("TXA", ""); // Load multiplicand
    emitter.emit_inst("CLC", "");
    emitter.emit_inst("ADC", &format!("${:02X}", RESULT_REG));
    emitter.emit_inst("STA", &format!("${:02X}", RESULT_REG));
    emitter.emit_inst("DEY", "");
    emitter.emit_inst("BNE", &loop_label);

    // Load result into A
    emitter.emit_label(&end_label);
    emitter.emit_inst("LDA", &format!("${:02X}", RESULT_REG));

    Ok(())
}

fn generate_divide(emitter: &mut Emitter) -> Result<(), CodegenError> {
    // Divide A / TEMP using repeated subtraction
    // Result (quotient) in A
    const QUOTIENT_REG: u8 = 0x22;
    const DIVIDEND_REG: u8 = 0x23;

    let loop_label = emitter.next_label("div_loop");
    let end_label = emitter.next_label("div_end");

    // Check for division by zero
    emitter.emit_inst("LDX", &format!("${:02X}", emitter.memory_layout.temp_reg()));
    emitter.emit_inst("CPX", "#$00");
    emitter.emit_inst("BEQ", &end_label); // Result undefined, leave A as-is

    // Initialize quotient to 0
    emitter.emit_inst("LDX", "#$00");
    emitter.emit_inst("STX", &format!("${:02X}", QUOTIENT_REG));

    // Store dividend
    emitter.emit_inst("STA", &format!("${:02X}", DIVIDEND_REG));

    // Loop: subtract divisor from dividend until dividend < divisor
    emitter.emit_label(&loop_label);
    emitter.emit_inst("LDA", &format!("${:02X}", DIVIDEND_REG));
    emitter.emit_inst("CMP", &format!("${:02X}", emitter.memory_layout.temp_reg()));
    emitter.emit_inst("BCC", &end_label); // If dividend < divisor, done

    // Subtract divisor
    emitter.emit_inst("SEC", "");
    emitter.emit_inst("SBC", &format!("${:02X}", emitter.memory_layout.temp_reg()));
    emitter.emit_inst("STA", &format!("${:02X}", DIVIDEND_REG));

    // Increment quotient
    emitter.emit_inst("INC", &format!("${:02X}", QUOTIENT_REG));
    emitter.emit_inst("JMP", &loop_label);

    emitter.emit_label(&end_label);
    emitter.emit_inst("LDA", &format!("${:02X}", QUOTIENT_REG));

    Ok(())
}

fn generate_modulo(emitter: &mut Emitter) -> Result<(), CodegenError> {
    // Modulo A % TEMP using repeated subtraction
    // Result (remainder) in A
    const DIVIDEND_REG: u8 = 0x23;

    let loop_label = emitter.next_label("mod_loop");
    let end_label = emitter.next_label("mod_end");

    // Check for division by zero
    emitter.emit_inst("LDX", &format!("${:02X}", emitter.memory_layout.temp_reg()));
    emitter.emit_inst("CPX", "#$00");
    emitter.emit_inst("BEQ", &end_label); // Result undefined, leave A as-is

    // Store dividend
    emitter.emit_inst("STA", &format!("${:02X}", DIVIDEND_REG));

    // Loop: subtract divisor from dividend until dividend < divisor
    emitter.emit_label(&loop_label);
    emitter.emit_inst("LDA", &format!("${:02X}", DIVIDEND_REG));
    emitter.emit_inst("CMP", &format!("${:02X}", emitter.memory_layout.temp_reg()));
    emitter.emit_inst("BCC", &end_label); // If dividend < divisor, done (A has remainder)

    // Subtract divisor
    emitter.emit_inst("SEC", "");
    emitter.emit_inst("SBC", &format!("${:02X}", emitter.memory_layout.temp_reg()));
    emitter.emit_inst("STA", &format!("${:02X}", DIVIDEND_REG));
    emitter.emit_inst("JMP", &loop_label);

    emitter.emit_label(&end_label);
    emitter.emit_inst("LDA", &format!("${:02X}", DIVIDEND_REG));

    Ok(())
}

// Array indexing helper
fn generate_index(
    _object: &Spanned<Expr>,
    index: &Spanned<Expr>,
    emitter: &mut Emitter,
    info: &ProgramInfo,
) -> Result<(), CodegenError> {
    // For array indexing: array[index]
    // We need to calculate the address and load from it
    // This is a simplified implementation that assumes:
    // - Array base address is known
    // - Index fits in a single byte

    // Generate index expression -> A
    generate_expr(index, emitter, info)?;

    // Save index in X register
    emitter.emit_inst("TAX", "");

    // For now, emit a comment about array access
    // Full implementation would need:
    // 1. Get base address of array
    // 2. Add index (with proper scaling for element size)
    // 3. Load from calculated address
    emitter.emit_comment("Array index access (TODO: implement proper addressing)");

    // Placeholder: just load zero for now
    emitter.emit_inst("LDA", "#$00");

    Ok(())
}

fn generate_struct_init(
    name: &Spanned<String>,
    fields: &[crate::ast::FieldInit],
    emitter: &mut Emitter,
    info: &ProgramInfo,
) -> Result<(), CodegenError> {
    emitter.emit_comment(&format!("Struct init: {}", name.node));

    // Look up the struct definition
    let struct_def = info.type_registry.get_struct(&name.node)
        .ok_or_else(|| CodegenError::UnsupportedOperation(
            format!("struct '{}' not found in type registry", name.node)
        ))?;

    // Generate labels for struct data
    let struct_label = emitter.next_label(&format!("struct_{}", name.node));
    let skip_label = emitter.next_label("struct_skip");

    // Jump over the data
    emitter.emit_inst("JMP", &skip_label);

    // Emit struct data
    emitter.emit_label(&struct_label);

    // Create a map of field values for quick lookup
    let field_values: std::collections::HashMap<String, &Spanned<crate::ast::Expr>> =
        fields.iter()
            .map(|f| (f.name.node.clone(), &f.value))
            .collect();

    // Initialize each field in order (respecting struct layout)
    for field_info in &struct_def.fields {
        if let Some(value_expr) = field_values.get(&field_info.name) {
            // Evaluate the field value expression and emit as data
            // For now, we only support constant expressions
            if let crate::ast::Expr::Literal(lit) = &value_expr.node {
                match lit {
                    crate::ast::Literal::Integer(val) => {
                        // Emit the appropriate number of bytes based on field type
                        let size = field_info.ty.size();
                        if size == 1 {
                            emitter.emit_byte(*val as u8);
                        } else if size == 2 {
                            // Emit as little-endian u16
                            emitter.emit_byte((*val & 0xFF) as u8);
                            emitter.emit_byte(((*val >> 8) & 0xFF) as u8);
                        } else {
                            return Err(CodegenError::UnsupportedOperation(
                                format!("struct field type with size {} not yet supported", size)
                            ));
                        }
                    }
                    crate::ast::Literal::Bool(b) => {
                        emitter.emit_byte(if *b { 1 } else { 0 });
                    }
                    _ => {
                        return Err(CodegenError::UnsupportedOperation(
                            "only integer and bool literals supported in struct initialization".to_string()
                        ));
                    }
                }
            } else {
                return Err(CodegenError::UnsupportedOperation(
                    "only constant expressions supported in struct initialization".to_string()
                ));
            }
        } else {
            // Field not provided - initialize to zero
            for _ in 0..field_info.ty.size() {
                emitter.emit_byte(0);
            }
        }
    }

    emitter.emit_label(&skip_label);

    // Load the address of the struct into A (low byte) and X (high byte)
    emitter.emit_inst("LDA", &format!("#<{}", struct_label));
    emitter.emit_inst("LDX", &format!("#>{}", struct_label));

    Ok(())
}

fn generate_field_access(
    object: &Spanned<crate::ast::Expr>,
    field: &Spanned<String>,
    emitter: &mut Emitter,
    info: &ProgramInfo,
) -> Result<(), CodegenError> {
    use crate::ast::Expr;

    // Get the base object (must be a variable for now)
    if let Expr::Variable(var_name) = &object.node {
        // Look up the variable in the symbol table
        if let Some(sym) = info.table.lookup(var_name) {
            // Get the base address of the struct
            let base_addr = match sym.location {
                crate::sema::table::SymbolLocation::ZeroPage(addr) => addr as u16,
                crate::sema::table::SymbolLocation::Absolute(addr) => addr,
                _ => {
                    return Err(CodegenError::UnsupportedOperation(
                        format!("Cannot access field of variable with location: {:?}", sym.location)
                    ));
                }
            };

            emitter.emit_comment(&format!("Field access: {}.{}", var_name, field.node));

            // Get the struct type name from the symbol's type
            let struct_name = if let crate::sema::types::Type::Named(name) = &sym.ty {
                name
            } else {
                return Err(CodegenError::UnsupportedOperation(
                    format!("variable '{}' is not a struct type", var_name)
                ));
            };

            // Look up the struct definition
            let struct_def = info.type_registry.get_struct(struct_name)
                .ok_or_else(|| CodegenError::UnsupportedOperation(
                    format!("struct '{}' not found in type registry", struct_name)
                ))?;

            // Find the field and get its offset
            let field_info = struct_def.get_field(&field.node)
                .ok_or_else(|| CodegenError::UnsupportedOperation(
                    format!("field '{}' not found in struct '{}'", field.node, struct_name)
                ))?;

            let field_addr = base_addr + field_info.offset as u16;

            // Load the field value into accumulator
            if field_addr < 0x100 {
                emitter.emit_inst("LDA", &format!("${:02X}", field_addr));
            } else {
                emitter.emit_inst("LDA", &format!("${:04X}", field_addr));
            }

            Ok(())
        } else {
            Err(CodegenError::SymbolNotFound(var_name.clone()))
        }
    } else {
        Err(CodegenError::UnsupportedOperation(
            "Field access only supported on variables (not expressions)".to_string()
        ))
    }
}

fn generate_enum_variant(
    enum_name: &Spanned<String>,
    variant: &Spanned<String>,
    data: &crate::ast::VariantData,
    emitter: &mut Emitter,
    info: &ProgramInfo,
) -> Result<(), CodegenError> {
    emitter.emit_comment(&format!("Enum variant: {}::{}", enum_name.node, variant.node));

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

    // Generate labels for enum data
    let enum_label = emitter.next_label(&format!("enum_{}_{}", enum_name.node, variant.node));
    let skip_label = emitter.next_label("enum_skip");

    // Jump over the data
    emitter.emit_inst("JMP", &skip_label);

    // Emit enum data
    emitter.emit_label(&enum_label);

    // Emit discriminant tag
    emitter.emit_byte(variant_info.tag);

    // Emit variant data based on type
    match (&variant_info.data, data) {
        (crate::sema::type_defs::VariantData::Unit, crate::ast::VariantData::Unit) => {
            // Unit variant - just the tag, no data
        }
        (crate::sema::type_defs::VariantData::Tuple(field_types), crate::ast::VariantData::Tuple(values)) => {
            // Tuple variant - emit each value
            if values.len() != field_types.len() {
                return Err(CodegenError::UnsupportedOperation(
                    format!("variant '{}' expects {} fields, got {}", variant.node, field_types.len(), values.len())
                ));
            }

            for (value_expr, field_type) in values.iter().zip(field_types.iter()) {
                // For now, only support constant expressions
                if let crate::ast::Expr::Literal(lit) = &value_expr.node {
                    match lit {
                        crate::ast::Literal::Integer(val) => {
                            let size = field_type.size();
                            if size == 1 {
                                emitter.emit_byte(*val as u8);
                            } else if size == 2 {
                                // Emit as little-endian u16
                                emitter.emit_byte((*val & 0xFF) as u8);
                                emitter.emit_byte(((*val >> 8) & 0xFF) as u8);
                            } else {
                                return Err(CodegenError::UnsupportedOperation(
                                    format!("field type with size {} not yet supported", size)
                                ));
                            }
                        }
                        crate::ast::Literal::Bool(b) => {
                            emitter.emit_byte(if *b { 1 } else { 0 });
                        }
                        _ => {
                            return Err(CodegenError::UnsupportedOperation(
                                "only integer and bool literals supported in enum variant data".to_string()
                            ));
                        }
                    }
                } else {
                    return Err(CodegenError::UnsupportedOperation(
                        "only constant expressions supported in enum variant construction".to_string()
                    ));
                }
            }
        }
        (crate::sema::type_defs::VariantData::Struct(field_infos), crate::ast::VariantData::Struct(field_inits)) => {
            // Struct variant - similar to struct initialization
            let field_values: std::collections::HashMap<String, &Spanned<crate::ast::Expr>> =
                field_inits.iter()
                    .map(|f| (f.name.node.clone(), &f.value))
                    .collect();

            for field_info in field_infos {
                if let Some(value_expr) = field_values.get(&field_info.name) {
                    if let crate::ast::Expr::Literal(lit) = &value_expr.node {
                        match lit {
                            crate::ast::Literal::Integer(val) => {
                                let size = field_info.ty.size();
                                if size == 1 {
                                    emitter.emit_byte(*val as u8);
                                } else if size == 2 {
                                    emitter.emit_byte((*val & 0xFF) as u8);
                                    emitter.emit_byte(((*val >> 8) & 0xFF) as u8);
                                } else {
                                    return Err(CodegenError::UnsupportedOperation(
                                        format!("field type with size {} not yet supported", size)
                                    ));
                                }
                            }
                            crate::ast::Literal::Bool(b) => {
                                emitter.emit_byte(if *b { 1 } else { 0 });
                            }
                            _ => {
                                return Err(CodegenError::UnsupportedOperation(
                                    "only integer and bool literals supported in enum variant".to_string()
                                ));
                            }
                        }
                    } else {
                        return Err(CodegenError::UnsupportedOperation(
                            "only constant expressions supported in enum variant construction".to_string()
                        ));
                    }
                } else {
                    // Field not provided - initialize to zero
                    for _ in 0..field_info.ty.size() {
                        emitter.emit_byte(0);
                    }
                }
            }
        }
        _ => {
            return Err(CodegenError::UnsupportedOperation(
                format!("variant data mismatch for '{}'", variant.node)
            ));
        }
    }

    emitter.emit_label(&skip_label);

    // Load the address of the enum into A (low byte) and X (high byte)
    emitter.emit_inst("LDA", &format!("#<{}", enum_label));
    emitter.emit_inst("LDX", &format!("#>{}", enum_label));

    Ok(())
}

fn generate_type_cast(
    expr: &Spanned<crate::ast::Expr>,
    target_type: &Spanned<crate::ast::TypeExpr>,
    emitter: &mut Emitter,
    info: &ProgramInfo,
) -> Result<(), CodegenError> {
    use crate::ast::{PrimitiveType, TypeExpr};

    // Evaluate the source expression
    generate_expr(expr, emitter, info)?;

    // Determine target type
    match &target_type.node {
        TypeExpr::Primitive(target_prim) => {
            match target_prim {
                PrimitiveType::U16 | PrimitiveType::I16 => {
                    // Casting to 16-bit: Need to handle high byte
                    // For u8 -> u16: zero-extend (A has low byte, X should be 0)
                    // For i8 -> i16: sign-extend (A has low byte, X should be sign-extended)

                    emitter.emit_comment(&format!("Cast to {:?}", target_prim));

                    if matches!(target_prim, PrimitiveType::I16) {
                        // Sign extension: if bit 7 of A is set, X = $FF, else X = $00
                        emitter.emit_inst("TAX", ""); // Save value in X temporarily
                        emitter.emit_inst("AND", "#$80"); // Check sign bit
                        let neg_label = emitter.next_label("sign_neg");
                        let end_label = emitter.next_label("sign_end");

                        emitter.emit_inst("BEQ", &neg_label); // If zero (positive), use 0
                        emitter.emit_inst("LDA", "#$FF"); // Negative: high byte = $FF
                        emitter.emit_inst("JMP", &end_label);
                        emitter.emit_label(&neg_label);
                        emitter.emit_inst("LDA", "#$00"); // Positive: high byte = $00
                        emitter.emit_label(&end_label);

                        // Now A has high byte, X has low byte - swap them
                        emitter.emit_inst("PHA", ""); // Push high byte
                        emitter.emit_inst("TXA", ""); // A = low byte
                        emitter.emit_inst("TAX", ""); // X = low byte
                        emitter.emit_inst("PLA", ""); // A = high byte (for now)
                        // Result: A = low byte, X = high byte (but we want X = high byte)
                        // Actually for most operations we just use A, so leave low byte in A
                        emitter.emit_inst("TXA", ""); // A = low byte
                    } else {
                        // Zero extension: X = 0
                        emitter.emit_inst("LDX", "#$00");
                        // A already has the low byte
                    }
                }
                PrimitiveType::U8 | PrimitiveType::I8 => {
                    // Casting to 8-bit: Just truncate (A already has the value)
                    emitter.emit_comment(&format!("Cast to {:?} (truncate)", target_prim));
                    // For u16/i16 -> u8, we just keep A (low byte), discard high byte
                    // A already contains the result
                }
                PrimitiveType::Bool => {
                    // Cast to bool: 0 = false, non-zero = true
                    // Convert to canonical boolean (0 or 1)
                    emitter.emit_comment("Cast to bool");
                    let true_label = emitter.next_label("bool_true");
                    let end_label = emitter.next_label("bool_end");

                    emitter.emit_inst("CMP", "#$00");
                    emitter.emit_inst("BNE", &true_label);
                    // False case
                    emitter.emit_inst("LDA", "#$00");
                    emitter.emit_inst("JMP", &end_label);
                    // True case
                    emitter.emit_label(&true_label);
                    emitter.emit_inst("LDA", "#$01");
                    emitter.emit_label(&end_label);
                }
            }
        }
        TypeExpr::Pointer { .. } => {
            // Pointer cast: for now, just treat as address value
            emitter.emit_comment("Cast to pointer (no conversion)");
        }
        _ => {
            // For complex types, we don't know how to cast yet
            emitter.emit_comment("Cast to complex type (TODO)");
        }
    }

    Ok(())
}
