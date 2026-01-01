//! Binary operation code generation
//!
//! This module handles:
//! - All binary operations (arithmetic, bitwise, shifts)
//! - Strength reduction optimizations
//! - BCD mode handling (SED/CLD)
//! - Helper functions for multiply, divide, modulo

use crate::ast::{BinaryOp, Expr, Spanned};
use crate::codegen::{CodegenError, Emitter, StringCollector};
use crate::sema::types::Type;
use crate::sema::ProgramInfo;

// Import comparison and logical functions from sibling modules
use super::{generate_compare_eq, generate_compare_ne, generate_compare_lt, generate_compare_gt, generate_compare_le, generate_compare_ge, generate_logical_and, generate_logical_or};

// Import generate_expr from parent module for recursive calls
use super::generate_expr;

/// Check if an expression is "simple" - can be re-evaluated cheaply without side effects
fn is_simple_expr(expr: &Expr) -> bool {
    matches!(expr, Expr::Literal(_) | Expr::Variable(_))
}

/// Check if a value is a power of 2, return the shift amount (exponent) if it is
fn is_power_of_2(val: u64) -> Option<u8> {
    if val == 0 || (val & (val - 1)) != 0 {
        None
    } else {
        Some(val.trailing_zeros() as u8)
    }
}

pub(super) fn generate_binary(
    left: &Spanned<Expr>,
    op: crate::ast::BinaryOp,
    right: &Spanned<Expr>,
    emitter: &mut Emitter,
    info: &ProgramInfo,
    string_collector: &mut StringCollector,
) -> Result<(), CodegenError> {
    // Handle short-circuit logical operations specially
    match op {
        crate::ast::BinaryOp::And => return generate_logical_and(left, right, emitter, info, string_collector),
        crate::ast::BinaryOp::Or => return generate_logical_or(left, right, emitter, info, string_collector),
        _ => {}
    }

    // Check if we're doing 16-bit arithmetic (check before generating expressions)
    let left_type = info.resolved_types.get(&left.span);
    let is_u16 = left_type.is_some_and(|ty| matches!(ty,
        Type::Primitive(crate::ast::PrimitiveType::U16) |
        Type::Primitive(crate::ast::PrimitiveType::I16) |
        Type::Primitive(crate::ast::PrimitiveType::B16)
    ));

    // === STRENGTH REDUCTION OPTIMIZATIONS ===
    // Transform expensive operations into cheaper equivalents

    // Check if right operand is a literal for optimization
    if let Expr::Literal(crate::ast::Literal::Integer(val)) = &right.node {
        let val_u64 = *val as u64;

        match op {
            // Optimization: x * (power of 2) → x << n
            crate::ast::BinaryOp::Mul => {
                if let Some(shift_amount) = is_power_of_2(val_u64) {
                    if emitter.is_verbose() {
                        emitter.emit_comment(&format!(
                            "Strength reduction: x * {} → x << {}",
                            val_u64, shift_amount
                        ));
                    }

                    // Generate left operand
                    generate_expr(left, emitter, info, string_collector)?;

                    // Store shift amount in temp
                    let temp_reg = emitter.memory_layout.temp_reg();
                    emitter.emit_inst("LDA", &format!("#${:02X}", shift_amount));
                    emitter.emit_sta_zp(temp_reg);

                    // Reload left operand
                    generate_expr(left, emitter, info, string_collector)?;

                    // Perform shift
                    generate_shift_left(emitter, is_u16)?;
                    return Ok(());
                }
            }

            // Optimization: x / 256 → x.high (for u16 only)
            crate::ast::BinaryOp::Div => {
                if is_u16 && val_u64 == 256 {
                    if emitter.is_verbose() {
                        emitter.emit_comment("Strength reduction: x / 256 → x.high");
                    }

                    // Generate left operand (result in A=low, Y=high)
                    generate_expr(left, emitter, info, string_collector)?;

                    // Move high byte to A
                    emitter.emit_inst("TYA", "");
                    if emitter.is_verbose() {
                        emitter.emit_comment("Extract high byte");
                    }
                    return Ok(());
                }
            }

            // Optimization: x % 256 → x.low (for u16 only)
            crate::ast::BinaryOp::Mod => {
                if is_u16 && val_u64 == 256 {
                    if emitter.is_verbose() {
                        emitter.emit_comment("Strength reduction: x % 256 → x.low");
                    }

                    // Generate left operand (result in A=low, Y=high)
                    generate_expr(left, emitter, info, string_collector)?;

                    // Low byte is already in A, just clear Y to indicate u8 result
                    if emitter.is_verbose() {
                        emitter.emit_comment("Low byte already in A");
                    }
                    return Ok(());
                }
            }

            _ => {}
        }
    }

    // Optimization: Avoid stack if left operand is simple (variable or literal)
    let use_stack = !is_simple_expr(&left.node);

    if use_stack {
        // Complex left expression: use Y register for temporary storage
        // This is faster than PHA/PLA (2+3=5 cycles vs 3+4=7 cycles)
        // 1. Generate left operand -> A (and X if u16)
        generate_expr(left, emitter, info, string_collector)?;

        // 2. Save A to Y register
        emitter.emit_inst("TAY", "");
        emitter.reg_state.transfer_a_to_y();

        // 3. Generate right operand -> A (and X if u16)
        generate_expr(right, emitter, info, string_collector)?;

        // 4. Store right operand in TEMP (both bytes if u16)
        let temp_reg = emitter.memory_layout.temp_reg();
        emitter.emit_sta_zp(temp_reg);
        if is_u16 {
            emitter.emit_inst("STY", &format!("${:02X}", temp_reg + 1));
        }

        // 5. Restore left operand from Y -> A
        emitter.emit_inst("TYA", "");
        emitter.reg_state.transfer_y_to_a();
    } else {
        // Simple left expression: evaluate right first, store in temp, then eval left
        // This saves PHA/PLA instructions (4 cycles)

        // 1. Generate right operand -> A (and X if u16)
        generate_expr(right, emitter, info, string_collector)?;

        // 2. Store right operand in TEMP (both bytes if u16)
        let temp_reg = emitter.memory_layout.temp_reg();
        emitter.emit_sta_zp(temp_reg);
        if is_u16 {
            emitter.emit_inst("STY", &format!("${:02X}", temp_reg + 1));
        }

        // 3. Generate left operand -> A (and X if u16) (simple, no side effects)
        generate_expr(left, emitter, info, string_collector)?;
    }

    // Check if we're doing BCD arithmetic
    let is_bcd = left_type.is_some_and(|ty| matches!(ty,
        crate::sema::types::Type::Primitive(crate::ast::PrimitiveType::B8) |
        crate::sema::types::Type::Primitive(crate::ast::PrimitiveType::B16)
    ));

    // For BCD arithmetic, enter decimal mode
    if is_bcd && matches!(op, crate::ast::BinaryOp::Add | crate::ast::BinaryOp::Sub) {
        emitter.emit_comment("Enter BCD mode");
        emitter.emit_inst("SED", "");
    }

    // 6. Perform operation
    match op {
        crate::ast::BinaryOp::Add => {
            if is_u16 {
                // 16-bit addition: add low bytes, then high bytes with carry
                let temp_low = emitter.memory_layout.temp_reg();
                let temp_high = temp_low + 1;

                emitter.emit_inst("CLC", "");
                emitter.emit_inst("ADC", &format!("${:02X}", temp_low));  // Add low bytes
                emitter.emit_inst("PHA", "");  // Save low byte result on stack
                emitter.emit_inst("TYA", "");  // Get left high byte from Y
                emitter.emit_inst("ADC", &format!("${:02X}", temp_high)); // Add high bytes with carry
                emitter.emit_inst("TAY", "");  // Store high byte result in Y
                emitter.emit_inst("PLA", "");  // Restore low byte result to A
            } else {
                // 8-bit addition
                emitter.emit_inst("CLC", "");
                emitter.emit_inst("ADC", &format!("${:02X}", emitter.memory_layout.temp_reg()));
            }
        }
        crate::ast::BinaryOp::Sub => {
            if is_u16 {
                // 16-bit subtraction: sub low bytes, then high bytes with borrow
                let temp_low = emitter.memory_layout.temp_reg();
                let temp_high = temp_low + 1;

                emitter.emit_inst("SEC", "");
                emitter.emit_inst("SBC", &format!("${:02X}", temp_low));  // Subtract low bytes
                emitter.emit_inst("PHA", "");  // Save low byte result on stack
                emitter.emit_inst("TYA", "");  // Get left high byte from Y
                emitter.emit_inst("SBC", &format!("${:02X}", temp_high)); // Subtract high bytes with borrow
                emitter.emit_inst("TAY", "");  // Store high byte result in Y
                emitter.emit_inst("PLA", "");  // Restore low byte result to A
            } else {
                // 8-bit subtraction
                emitter.emit_inst("SEC", "");
                emitter.emit_inst("SBC", &format!("${:02X}", emitter.memory_layout.temp_reg()));
            }
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
            generate_shift_left(emitter, is_u16)?;
        }
        crate::ast::BinaryOp::Shr => {
            generate_shift_right(emitter, is_u16)?;
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
        // And/Or are handled earlier with short-circuit evaluation
        crate::ast::BinaryOp::And | crate::ast::BinaryOp::Or => {
            unreachable!("And/Or should be handled earlier in generate_binary")
        }
    }

    // Exit decimal mode after BCD operations
    if is_bcd && matches!(op, crate::ast::BinaryOp::Add | crate::ast::BinaryOp::Sub) {
        emitter.emit_comment("Exit BCD mode");
        emitter.emit_inst("CLD", "");
    }

    // Add register state comment based on operation type
    if emitter.is_verbose() {
        use crate::ast::BinaryOp;
        match op {
            BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Mod => {
                emitter.emit_comment("Result: A=result, flags set by operation");
            }
            BinaryOp::BitAnd | BinaryOp::BitOr | BinaryOp::BitXor => {
                emitter.emit_comment("Result: A=bitwise_result");
            }
            BinaryOp::Shl | BinaryOp::Shr => {
                emitter.emit_comment("Result: A=shifted_value");
            }
            BinaryOp::Eq | BinaryOp::Ne | BinaryOp::Lt | BinaryOp::Gt | BinaryOp::Le | BinaryOp::Ge => {
                emitter.emit_comment("Result: A=boolean (0=false, 1=true)");
            }
            _ => {}
        }
    }

    Ok(())
}
// Shift helper functions
// A contains value to shift, emitter.memory_layout.temp_reg() contains shift amount

fn generate_shift_left(emitter: &mut Emitter, _is_u16: bool) -> Result<(), CodegenError> {
    // Shift A left by emitter.memory_layout.temp_reg() bits
    // Use X register as loop counter

    if !emitter.is_minimal() {
        emitter.emit_comment("Shift left (multiply by power of 2)");
    }

    let loop_label = emitter.next_label("sl");
    let end_label = emitter.next_label("sx");

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
    // Comparison modifies A register - invalidate tracking
    emitter.mark_a_unknown();
    Ok(())
}

fn generate_shift_right(emitter: &mut Emitter, is_u16: bool) -> Result<(), CodegenError> {
    // Shift value right by emitter.memory_layout.temp_reg() bits
    // For u8: Shift A right
    // For u16: Shift A (low) and Y (high) right together

    if !emitter.is_minimal() {
        emitter.emit_comment("Shift right (divide by power of 2)");
    }

    let temp_reg = emitter.memory_layout.temp_reg();

    if is_u16 {
        // For u16, optimize common case of >> 8 (just take high byte)
        // Check if shift count == 8
        emitter.emit_inst("LDX", &format!("${:02X}", temp_reg));
        emitter.emit_inst("CPX", "#$08");
        let not_eight_label = emitter.next_label("sn");
        let end_label = emitter.next_label("sx");
        emitter.emit_inst("BNE", &not_eight_label);

        // Shift by exactly 8: move high byte to low byte
        emitter.emit_inst("TYA", ""); // A = high byte
        emitter.emit_inst("LDY", "#$00"); // Y = 0
        emitter.emit_inst("JMP", &end_label);

        emitter.emit_label(&not_eight_label);
        // For other shift counts, need multi-byte shift (not yet implemented)
        // For now, just do single-byte shift as before
        emitter.emit_inst("CPX", "#$00");
        emitter.emit_inst("BEQ", &end_label);

        let loop_label = emitter.next_label("sr");
        emitter.emit_label(&loop_label);
        emitter.emit_inst("LSR", "A");
        emitter.emit_inst("DEX", "");
        emitter.emit_inst("BNE", &loop_label);

        emitter.emit_label(&end_label);
    } else {
        // u8 shift
        let loop_label = emitter.next_label("sr");
        let end_label = emitter.next_label("sx");

        // Load shift count into X
        emitter.emit_inst("LDX", &format!("${:02X}", temp_reg));

        // Check if count is zero
        emitter.emit_inst("CPX", "#$00");
        emitter.emit_inst("BEQ", &end_label);

        // Loop: shift right once per iteration
        emitter.emit_label(&loop_label);
        emitter.emit_inst("LSR", "A"); // Logical shift right
        emitter.emit_inst("DEX", "");
        emitter.emit_inst("BNE", &loop_label);

        emitter.emit_label(&end_label);
    }

    // Comparison modifies A register - invalidate tracking
    emitter.mark_a_unknown();
    Ok(())
}

// Arithmetic helper functions for multiply, divide, modulo
// These require software implementation on 6502

fn generate_multiply(emitter: &mut Emitter) -> Result<(), CodegenError> {
    // Multiply A * TEMP using repeated addition
    // Result in A (will overflow for results > 255)
    const RESULT_REG: u8 = 0x22;

    let loop_label = emitter.next_label("ml");
    let end_label = emitter.next_label("mx");

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

    let loop_label = emitter.next_label("dl");
    let end_label = emitter.next_label("dx");

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

    let loop_label = emitter.next_label("md");
    let end_label = emitter.next_label("mx");

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
