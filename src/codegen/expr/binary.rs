//! Binary operation code generation
//!
//! This module handles:
//! - All binary operations (arithmetic, bitwise, shifts)
//! - Strength reduction optimizations
//! - BCD mode handling (SED/CLD)
//! - Helper functions for multiply, divide, modulo

use crate::ast::{Expr, Spanned};
use crate::codegen::{CodegenError, Emitter, StringCollector};
use crate::sema::ProgramInfo;
use crate::sema::types::Type;

// Import comparison and logical functions from sibling modules
use super::{
    generate_compare_eq, generate_compare_ge, generate_compare_gt, generate_compare_le,
    generate_compare_lt, generate_compare_ne, generate_logical_and, generate_logical_or,
};

// Import generate_expr from parent module for recursive calls
use super::generate_expr;

/// Check if an expression is "simple" - can be re-evaluated cheaply without side effects
fn is_simple_expr(expr: &Expr) -> bool {
    matches!(expr, Expr::Literal(_) | Expr::Variable(_))
}

/// Does evaluating this expression leave the Y register untouched?
///
/// The u8 binary fast path parks the left operand in Y while the right operand
/// is evaluated, so any right operand that writes Y — array indexing (`arr[i]`
/// scales the index through Y), casts and byte extraction (`.low`/`.high` load
/// the u16 source's high byte into Y), nested binary ops (their own u8 save uses
/// Y), field/pointer derefs, and calls — would silently corrupt it. Only
/// literals and direct scalar variable reads are guaranteed to touch A alone.
fn is_y_preserving(expr: &Expr) -> bool {
    match expr {
        Expr::Literal(_) | Expr::Variable(_) => true,
        Expr::Paren(inner) => is_y_preserving(&inner.node),
        _ => false,
    }
}

/// Does evaluating this expression involve a function call (direct or nested)?
/// A call clobbers Y and the $F0 scratch pool, so a live left operand held there
/// must instead be spilled across the call.
fn contains_call(expr: &Expr) -> bool {
    match expr {
        Expr::Call { .. } => true,
        Expr::Binary { left, right, .. } => contains_call(&left.node) || contains_call(&right.node),
        Expr::Unary { operand, .. } => contains_call(&operand.node),
        Expr::Paren(inner) => contains_call(&inner.node),
        Expr::Cast { expr: inner, .. } => contains_call(&inner.node),
        Expr::Index { object, index } => contains_call(&object.node) || contains_call(&index.node),
        Expr::Field { object, .. } => contains_call(&object.node),
        Expr::SliceLen(inner) | Expr::U16Low(inner) | Expr::U16High(inner) => {
            contains_call(&inner.node)
        }
        _ => false,
    }
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
        crate::ast::BinaryOp::And => {
            return generate_logical_and(left, right, emitter, info, string_collector);
        }
        crate::ast::BinaryOp::Or => {
            return generate_logical_or(left, right, emitter, info, string_collector);
        }
        _ => {}
    }

    // Determine operand width/signedness before generating code.
    //
    // Arithmetic and comparison operands share a type (Wraith has no implicit
    // conversions), but a complex operand's span may be absent from
    // resolved_types; deriving purely from the left then silently defaults to
    // 8-bit unsigned and miscompiles a wide/signed comparison. So consult the
    // right operand as a fallback. Shifts are the exception: `u16 >> u8` is
    // allowed, so the value's width/signedness must come from the left operand
    // (the value), never the right (a count that may be narrower).
    let left_type = info.resolved_types.get(&left.span);
    let right_type = info.resolved_types.get(&right.span);
    let is_shift = matches!(op, crate::ast::BinaryOp::Shl | crate::ast::BinaryOp::Shr);
    let op_type = if is_shift {
        left_type
    } else {
        left_type.or(right_type)
    };

    let is_u16 = op_type.is_some_and(|ty| {
        matches!(
            ty,
            Type::Primitive(crate::ast::PrimitiveType::U16)
                | Type::Primitive(crate::ast::PrimitiveType::I16)
                | Type::Primitive(crate::ast::PrimitiveType::B16)
        )
    });

    // Whether the operands are signed (i8/i16). Comparisons, `>>`, and division
    // need signed-specific sequences; other ops are bit-identical either way.
    let is_signed = op_type.is_some_and(|ty| ty.is_signed());

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

                    // Generate left operand (result in A, and Y if u16).
                    generate_expr(left, emitter, info, string_collector)?;

                    // Store the shift amount in temp via X so the left operand in
                    // A/Y is preserved — reloading it with a second generate_expr
                    // would double-evaluate a side-effecting left (e.g. a call).
                    let temp_reg = emitter.memory_layout.temp_reg();
                    emitter.emit_inst("LDX", &format!("#${:02X}", shift_amount));
                    emitter.emit_inst("STX", &format!("${:02X}", temp_reg));
                    emitter.mark_x_unknown();

                    // Perform shift
                    generate_shift_left(emitter, is_u16)?;
                    return Ok(());
                }
            }

            // Optimization: x / 256 → x.high (unsigned u16 only; for signed the
            // high byte is not the truncated quotient).
            crate::ast::BinaryOp::Div => {
                if is_u16 && !is_signed && val_u64 == 256 {
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

            // Optimization: x % 256 → x.low (unsigned u16 only; a signed
            // remainder takes the dividend's sign).
            crate::ast::BinaryOp::Mod => {
                if is_u16 && !is_signed && val_u64 == 256 {
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

    // If the right operand contains a call, the fast left-operand saves (Y for u8,
    // the $F0 pool for u16) would be clobbered by the JSR. In that case spill the
    // left operand to the software stack instead — no hardware stack, and it nests
    // correctly (LIFO) with the recursion frame saves in generate_call.
    let right_has_call = contains_call(&right.node);

    // The u8 fast path additionally parks the left operand in Y, which any
    // Y-writing right operand (even a call-free one, e.g. `arr[i]` or a cast)
    // destroys. Route those through the same memory spill. The u16 fast path
    // saves to a temp_alloc slot that nested ops never reuse, so it only needs
    // the call guard.
    let needs_spill = right_has_call || (!is_u16 && !is_y_preserving(&right.node));

    // Allocate temp storage for the u16 fast-path left-operand save (if needed)
    let left_save_addr = if use_stack && is_u16 && !needs_spill {
        emitter.temp_alloc.alloc_high(2)
    } else {
        None
    };

    if use_stack && needs_spill {
        // Right operand contains a call: spill the left operand across it.
        let spill_size = if is_u16 { 2 } else { 1 };

        // 1. Generate left operand -> A (and Y if u16), then spill it.
        generate_expr(left, emitter, info, string_collector)?;
        emitter.spill_scalar(spill_size);

        // 2. Generate right operand -> A (and Y if u16).
        generate_expr(right, emitter, info, string_collector)?;

        // 3. Store right operand in TEMP (both bytes if u16).
        let temp_reg = emitter.memory_layout.temp_reg();
        emitter.emit_sta_zp(temp_reg);
        if is_u16 {
            emitter.emit_inst("STY", &format!("${:02X}", temp_reg + 1));
        }

        // 4. Reload the left operand into A (and Y if u16).
        emitter.reload_scalar(spill_size);
    } else if use_stack {
        // Complex left expression, call-free right operand: use the fast saves
        // (Y for u8, the $F0 pool for u16).

        // 1. Generate left operand -> A (and Y if u16)
        generate_expr(left, emitter, info, string_collector)?;

        if is_u16 {
            // 2a. For u16: Save BOTH bytes to allocated temp storage
            let save_addr = left_save_addr.unwrap_or(0xF2); // Fallback to hardcoded if alloc failed
            emitter.emit_inst("STA", &format!("${:02X}", save_addr));
            emitter.emit_inst("STY", &format!("${:02X}", save_addr + 1));
        } else {
            // 2b. For u8: Save A to Y register (faster)
            emitter.emit_inst("TAY", "");
            emitter.reg_state.transfer_a_to_y();
        }

        // 3. Generate right operand -> A (and Y if u16)
        generate_expr(right, emitter, info, string_collector)?;

        // 4. Store right operand in TEMP (both bytes if u16)
        let temp_reg = emitter.memory_layout.temp_reg();
        emitter.emit_sta_zp(temp_reg);
        if is_u16 {
            emitter.emit_inst("STY", &format!("${:02X}", temp_reg + 1));
        }

        // 5. Restore left operand
        if is_u16 {
            // 5a. For u16: Load BOTH bytes from allocated temp storage
            let save_addr = left_save_addr.unwrap_or(0xF2);
            emitter.emit_inst("LDA", &format!("${:02X}", save_addr));
            emitter.emit_inst("LDY", &format!("${:02X}", save_addr + 1));
            emitter.reg_state.invalidate_all();
            // Free the temp storage
            if left_save_addr.is_some() {
                emitter.temp_alloc.free_high(save_addr, 2);
            }
        } else {
            // 5b. For u8: Transfer from Y -> A
            emitter.emit_inst("TYA", "");
            emitter.reg_state.transfer_y_to_a();
        }
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
    let is_bcd = left_type.is_some_and(|ty| {
        matches!(
            ty,
            crate::sema::types::Type::Primitive(crate::ast::PrimitiveType::B8)
                | crate::sema::types::Type::Primitive(crate::ast::PrimitiveType::B16)
        )
    });

    // For BCD arithmetic, enter decimal mode
    if is_bcd && matches!(op, crate::ast::BinaryOp::Add | crate::ast::BinaryOp::Sub) {
        emitter.emit_comment("Enter BCD mode");
        emitter.emit_inst("SED", "");
    }

    // Reserve temp_reg area ($20-$21) in allocator so helpers don't conflict
    // The right operand is stored there and must not be overwritten
    let temp_reserve = emitter.temp_alloc.alloc_primary(2);

    // 6. Perform operation
    match op {
        crate::ast::BinaryOp::Add => {
            if is_u16 {
                // 16-bit addition: add low bytes, then high bytes with carry
                let temp_low = emitter.memory_layout.temp_reg();
                let temp_high = temp_low + 1;

                emitter.emit_inst("CLC", "");
                emitter.emit_inst("ADC", &format!("${:02X}", temp_low)); // Add low bytes
                emitter.emit_inst("PHA", ""); // Save low byte result on stack
                emitter.emit_inst("TYA", ""); // Get left high byte from Y
                emitter.emit_inst("ADC", &format!("${:02X}", temp_high)); // Add high bytes with carry
                emitter.emit_inst("TAY", ""); // Store high byte result in Y
                emitter.emit_inst("PLA", ""); // Restore low byte result to A
            } else {
                // 8-bit addition
                emitter.emit_inst("CLC", "");
                emitter.emit_inst("ADC", &format!("${:02X}", emitter.memory_layout.temp_reg()));
            }
            // Arithmetic modifies A, invalidate register tracking
            emitter.mark_a_unknown();
        }
        crate::ast::BinaryOp::Sub => {
            if is_u16 {
                // 16-bit subtraction: sub low bytes, then high bytes with borrow
                let temp_low = emitter.memory_layout.temp_reg();
                let temp_high = temp_low + 1;

                emitter.emit_inst("SEC", "");
                emitter.emit_inst("SBC", &format!("${:02X}", temp_low)); // Subtract low bytes
                emitter.emit_inst("PHA", ""); // Save low byte result on stack
                emitter.emit_inst("TYA", ""); // Get left high byte from Y
                emitter.emit_inst("SBC", &format!("${:02X}", temp_high)); // Subtract high bytes with borrow
                emitter.emit_inst("TAY", ""); // Store high byte result in Y
                emitter.emit_inst("PLA", ""); // Restore low byte result to A
            } else {
                // 8-bit subtraction
                emitter.emit_inst("SEC", "");
                emitter.emit_inst("SBC", &format!("${:02X}", emitter.memory_layout.temp_reg()));
            }
            // Arithmetic modifies A, invalidate register tracking
            emitter.mark_a_unknown();
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
            generate_shift_right(emitter, is_u16, is_signed)?;
        }
        crate::ast::BinaryOp::Mul => {
            generate_multiply(emitter, is_u16)?;
        }
        crate::ast::BinaryOp::Div => {
            generate_divide(emitter, is_u16, is_signed)?;
        }
        crate::ast::BinaryOp::Mod => {
            generate_modulo(emitter, is_u16, is_signed)?;
        }
        // Comparison operations - result is boolean (0 or 1)
        crate::ast::BinaryOp::Eq => {
            generate_compare_eq(emitter, is_u16)?;
        }
        crate::ast::BinaryOp::Ne => {
            generate_compare_ne(emitter, is_u16)?;
        }
        crate::ast::BinaryOp::Lt => {
            generate_compare_lt(emitter, is_u16, is_signed)?;
        }
        crate::ast::BinaryOp::Ge => {
            generate_compare_ge(emitter, is_u16, is_signed)?;
        }
        crate::ast::BinaryOp::Gt => {
            // A > B is same as B < A, so swap and use Lt
            // We already have A in accumulator and B in TEMP
            // Just swap them conceptually
            generate_compare_gt(emitter, is_u16, is_signed)?;
        }
        crate::ast::BinaryOp::Le => {
            // A <= B is same as B >= A
            generate_compare_le(emitter, is_u16, is_signed)?;
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
            BinaryOp::Eq
            | BinaryOp::Ne
            | BinaryOp::Lt
            | BinaryOp::Gt
            | BinaryOp::Le
            | BinaryOp::Ge => {
                emitter.emit_comment("Result: A=boolean (0=false, 1=true)");
            }
            _ => {}
        }
    }

    // Free the temp_reg reservation
    if let Some(addr) = temp_reserve {
        emitter.temp_alloc.free_primary(addr, 2);
    }

    Ok(())
}
// Shift helper functions
// A contains value to shift, emitter.memory_layout.temp_reg() contains shift amount

fn generate_shift_left(emitter: &mut Emitter, is_u16: bool) -> Result<(), CodegenError> {
    // Shift the left operand left by emitter.memory_layout.temp_reg() bits.
    // u8:  value in A.
    // u16: low byte in A, high byte in Y.
    // Uses X as the loop counter.

    if !emitter.is_minimal() {
        emitter.emit_comment("Shift left (multiply by power of 2)");
    }

    let temp_reg = emitter.memory_layout.temp_reg();
    let loop_label = emitter.next_label("sl");
    let end_label = emitter.next_label("sx");

    if is_u16 {
        // Park the high byte in scratch so ASL A / ROL <scratch> shift all 16 bits.
        let high_tmp = emitter.memory_layout.loop_end_temp(); // $22 by default
        emitter.emit_inst("STY", &format!("${:02X}", high_tmp));

        // Load shift count into X
        emitter.emit_inst("LDX", &format!("${:02X}", temp_reg));
        emitter.emit_inst("CPX", "#$00");
        emitter.emit_inst("BEQ", &end_label);

        // Loop: low byte shifts bit 7 into carry, high byte rotates it in.
        emitter.emit_label(&loop_label);
        emitter.emit_inst("ASL", "A");
        emitter.emit_inst("ROL", &format!("${:02X}", high_tmp));
        emitter.emit_inst("DEX", "");
        emitter.emit_inst("BNE", &loop_label);

        emitter.emit_label(&end_label);
        // Restore the shifted high byte to Y.
        emitter.emit_inst("LDY", &format!("${:02X}", high_tmp));
    } else {
        // Load shift count into X
        emitter.emit_inst("LDX", &format!("${:02X}", temp_reg));

        // Check if count is zero
        emitter.emit_inst("CPX", "#$00");
        emitter.emit_inst("BEQ", &end_label);

        // Loop: shift left once per iteration
        emitter.emit_label(&loop_label);
        emitter.emit_inst("ASL", "A"); // Arithmetic shift left
        emitter.emit_inst("DEX", "");
        emitter.emit_inst("BNE", &loop_label);

        emitter.emit_label(&end_label);
    }

    // Shift modifies A (and Y for u16) - invalidate tracking
    emitter.mark_a_unknown();
    Ok(())
}

fn generate_shift_right(
    emitter: &mut Emitter,
    is_u16: bool,
    is_signed: bool,
) -> Result<(), CodegenError> {
    // Shift value right by emitter.memory_layout.temp_reg() bits.
    // For u8:  shift A right. For u16: shift A (low) and Y (high) together.
    // Signed operands use an arithmetic shift (sign bit replicated); unsigned
    // use a logical shift (zero filled).

    if !emitter.is_minimal() {
        emitter.emit_comment(if is_signed {
            "Arithmetic shift right (signed divide by power of 2)"
        } else {
            "Shift right (divide by power of 2)"
        });
    }

    let temp_reg = emitter.memory_layout.temp_reg();

    if is_u16 {
        let loop_label = emitter.next_label("sr");
        let end_label = emitter.next_label("sx");

        if is_signed {
            // Arithmetic: park both bytes in scratch. Each iteration seeds the
            // carry with the high byte's sign bit (ASL of a copy), then rotates
            // it back in so the sign is replicated.
            let low_tmp = emitter.memory_layout.loop_end_temp(); // $22
            let high_tmp = low_tmp + 1; // $23
            emitter.emit_inst("STA", &format!("${:02X}", low_tmp));
            emitter.emit_inst("STY", &format!("${:02X}", high_tmp));
            emitter.emit_inst("LDX", &format!("${:02X}", temp_reg));
            emitter.emit_inst("CPX", "#$00");
            emitter.emit_inst("BEQ", &end_label);

            emitter.emit_label(&loop_label);
            emitter.emit_inst("LDA", &format!("${:02X}", high_tmp));
            emitter.emit_inst("ASL", "A"); // carry = sign bit of high
            emitter.emit_inst("ROR", &format!("${:02X}", high_tmp)); // sign back into bit 7
            emitter.emit_inst("ROR", &format!("${:02X}", low_tmp));
            emitter.emit_inst("DEX", "");
            emitter.emit_inst("BNE", &loop_label);

            emitter.emit_label(&end_label);
            emitter.emit_inst("LDA", &format!("${:02X}", low_tmp)); // A = low
            emitter.emit_inst("LDY", &format!("${:02X}", high_tmp)); // Y = high
        } else {
            // Logical: park the high byte in scratch so LSR <hi> / ROR A shift
            // all 16 bits, count in X.
            let high_tmp = emitter.memory_layout.loop_end_temp(); // $22 by default
            emitter.emit_inst("STY", &format!("${:02X}", high_tmp));
            emitter.emit_inst("LDX", &format!("${:02X}", temp_reg));
            emitter.emit_inst("CPX", "#$00");
            emitter.emit_inst("BEQ", &end_label);

            // Loop: high byte shifts bit 0 into carry, low byte rotates it in.
            emitter.emit_label(&loop_label);
            emitter.emit_inst("LSR", &format!("${:02X}", high_tmp));
            emitter.emit_inst("ROR", "A");
            emitter.emit_inst("DEX", "");
            emitter.emit_inst("BNE", &loop_label);

            emitter.emit_label(&end_label);
            // Restore the shifted high byte to Y.
            emitter.emit_inst("LDY", &format!("${:02X}", high_tmp));
        }
    } else {
        // u8 shift
        let loop_label = emitter.next_label("sr");
        let end_label = emitter.next_label("sx");

        // Load shift count into X
        emitter.emit_inst("LDX", &format!("${:02X}", temp_reg));

        // Check if count is zero
        emitter.emit_inst("CPX", "#$00");
        emitter.emit_inst("BEQ", &end_label);

        // Loop: shift right once per iteration.
        emitter.emit_label(&loop_label);
        if is_signed {
            // Arithmetic: seed carry with the sign bit (A >= $80) then rotate in.
            emitter.emit_inst("CMP", "#$80");
            emitter.emit_inst("ROR", "A");
        } else {
            emitter.emit_inst("LSR", "A"); // Logical shift right
        }
        emitter.emit_inst("DEX", "");
        emitter.emit_inst("BNE", &loop_label);

        emitter.emit_label(&end_label);
    }

    // Shift modifies A (and Y for u16) - invalidate tracking
    emitter.mark_a_unknown();
    Ok(())
}

// Arithmetic helper functions for multiply, divide, modulo
// These require software implementation on 6502

fn generate_multiply(emitter: &mut Emitter, is_u16: bool) -> Result<(), CodegenError> {
    if is_u16 {
        generate_multiply_u16(emitter)
    } else {
        generate_multiply_u8(emitter)
    }
}

fn generate_multiply_u8(emitter: &mut Emitter) -> Result<(), CodegenError> {
    // Multiply u8 * u8 -> u8 using shift-and-add algorithm
    // Input: Left operand in A, Right operand in TEMP ($20)
    // Output: Result in A
    //
    // Algorithm:
    //   result = 0
    //   multiplicand = A (saved to high pool)
    //   multiplier = TEMP ($20)
    //   for 8 iterations:
    //     if (multiplier & 1): result += multiplicand
    //     multiplicand <<= 1
    //     multiplier >>= 1

    // Allocate temp storage
    let multiplicand = emitter.temp_alloc.alloc_high(1).unwrap_or(0xF0);
    let result_addr = emitter.temp_alloc.alloc_primary(1).unwrap_or(0x22);
    let temp = emitter.memory_layout.temp_reg();

    let loop_label = emitter.next_label("ml");
    let skip_add = emitter.next_label("ms");

    // Save multiplicand to memory
    emitter.emit_inst("STA", &format!("${:02X}", multiplicand));

    // Initialize result = 0
    emitter.emit_inst("LDA", "#$00");
    emitter.emit_inst("STA", &format!("${:02X}", result_addr));

    // Initialize loop counter (8 bits)
    emitter.emit_inst("LDX", "#$08");

    // Loop for each bit
    emitter.emit_label(&loop_label);

    // Check if multiplier bit 0 is set
    emitter.emit_inst("LDA", &format!("${:02X}", temp));
    emitter.emit_inst("LSR", ""); // Shift right, bit 0 -> carry
    emitter.emit_inst("STA", &format!("${:02X}", temp)); // Save shifted multiplier
    emitter.emit_inst("BCC", &skip_add); // If bit was 0, skip addition

    // Add multiplicand to result
    emitter.emit_inst("LDA", &format!("${:02X}", result_addr));
    emitter.emit_inst("CLC", "");
    emitter.emit_inst("ADC", &format!("${:02X}", multiplicand));
    emitter.emit_inst("STA", &format!("${:02X}", result_addr));

    emitter.emit_label(&skip_add);

    // Shift multiplicand left
    emitter.emit_inst("ASL", &format!("${:02X}", multiplicand));

    // Decrement counter and loop
    emitter.emit_inst("DEX", "");
    emitter.emit_inst("BNE", &loop_label);

    // Load result into A
    emitter.emit_inst("LDA", &format!("${:02X}", result_addr));

    // Free temp storage
    emitter.temp_alloc.free_high(multiplicand, 1);
    emitter.temp_alloc.free_primary(result_addr, 1);

    Ok(())
}

fn generate_multiply_u16(emitter: &mut Emitter) -> Result<(), CodegenError> {
    // For u16 * u16, call the stdlib mul16 function
    // Input: Left operand in A:Y, Right operand in TEMP:TEMP+1 ($20:$21)
    // Output: Result in A:Y

    if emitter.is_verbose() {
        emitter.emit_comment("Call stdlib mul16 for u16 multiplication");
    }

    // Mark that we need mul16 function
    emitter.needs_mul16 = true;

    // mul16 expects parameters at $D9-$DC
    // Store left operand (A:Y) to $D9-$DA
    emitter.emit_inst("STA", "$D9"); // Store low byte
    emitter.emit_inst("STY", "$DA"); // Store high byte

    // Store right operand (TEMP:TEMP+1) to $DB-$DC
    let temp = emitter.memory_layout.temp_reg();
    emitter.emit_inst("LDA", &format!("${:02X}", temp)); // Load right.low
    emitter.emit_inst("STA", "$DB");
    emitter.emit_inst("LDA", &format!("${:02X}", temp + 1)); // Load right.high
    emitter.emit_inst("STA", "$DC");

    // Call mul16
    emitter.emit_inst("JSR", "mul16");

    if emitter.is_verbose() {
        emitter.emit_comment("Returns: A=result_low, Y=result_high (u16)");
    }

    Ok(())
}

/// Negate the 16-bit value at zero-page `[lo, lo+1]` in place (0 - value).
fn emit_negate16_zp(emitter: &mut Emitter, lo: u8) {
    emitter.emit_inst("SEC", "");
    emitter.emit_inst("LDA", "#$00");
    emitter.emit_inst("SBC", &format!("${:02X}", lo));
    emitter.emit_inst("STA", &format!("${:02X}", lo));
    emitter.emit_inst("LDA", "#$00");
    emitter.emit_inst("SBC", &format!("${:02X}", lo + 1));
    emitter.emit_inst("STA", &format!("${:02X}", lo + 1));
}

/// Replace the 16-bit value at `[lo, lo+1]` with its absolute value.
fn emit_abs16_zp(emitter: &mut Emitter, lo: u8) {
    let done = emitter.next_label("ab");
    emitter.emit_inst("LDA", &format!("${:02X}", lo + 1)); // high byte
    emitter.emit_inst("BPL", &done); // sign clear -> already non-negative
    emit_negate16_zp(emitter, lo);
    emitter.emit_label(&done);
}

/// Conditionally two's-complement negate A (8-bit) when N of a prior `BIT` on
/// the sign byte is set. `sign` holds the result sign in bit 7.
fn emit_negate8_if_sign(emitter: &mut Emitter, sign: u8) {
    let done = emitter.next_label("ng");
    emitter.emit_inst("BIT", &format!("${:02X}", sign)); // N = sign bit 7 (leaves A intact)
    emitter.emit_inst("BPL", &done);
    emitter.emit_inst("EOR", "#$FF");
    emitter.emit_inst("CLC", "");
    emitter.emit_inst("ADC", "#$01");
    emitter.emit_label(&done);
}

/// Two's-complement negate A (8-bit) when it is negative. Assumes N already
/// reflects A's sign (i.e. A was just loaded via LDA), leaving abs(A) in A.
fn emit_abs8_a(emitter: &mut Emitter) {
    let done = emitter.next_label("ab");
    emitter.emit_inst("BPL", &done); // N clear (from the preceding LDA) -> non-negative
    emitter.emit_inst("EOR", "#$FF");
    emitter.emit_inst("CLC", "");
    emitter.emit_inst("ADC", "#$01");
    emitter.emit_label(&done);
}

fn generate_divide(
    emitter: &mut Emitter,
    is_u16: bool,
    is_signed: bool,
) -> Result<(), CodegenError> {
    if is_u16 {
        if is_signed {
            return generate_divide_i16(emitter);
        }
        return generate_divide_u16(emitter);
    }
    if is_signed {
        return generate_divide_i8(emitter);
    }

    // Divide u8 A / TEMP using repeated subtraction
    // Result (quotient) in A

    // Allocate temp storage
    let quotient_addr = emitter.temp_alloc.alloc_primary(2).unwrap_or(0x22);
    let dividend_addr = quotient_addr + 1;

    let loop_label = emitter.next_label("dl");
    let end_label = emitter.next_label("dx");

    // Check for division by zero
    emitter.emit_inst("LDX", &format!("${:02X}", emitter.memory_layout.temp_reg()));
    emitter.emit_inst("CPX", "#$00");
    emitter.emit_inst("BEQ", &end_label); // Result undefined, leave A as-is

    // Initialize quotient to 0
    emitter.emit_inst("LDX", "#$00");
    emitter.emit_inst("STX", &format!("${:02X}", quotient_addr));

    // Store dividend
    emitter.emit_inst("STA", &format!("${:02X}", dividend_addr));

    // Loop: subtract divisor from dividend until dividend < divisor
    emitter.emit_label(&loop_label);
    emitter.emit_inst("LDA", &format!("${:02X}", dividend_addr));
    emitter.emit_inst("CMP", &format!("${:02X}", emitter.memory_layout.temp_reg()));
    emitter.emit_inst("BCC", &end_label); // If dividend < divisor, done

    // Subtract divisor
    emitter.emit_inst("SEC", "");
    emitter.emit_inst("SBC", &format!("${:02X}", emitter.memory_layout.temp_reg()));
    emitter.emit_inst("STA", &format!("${:02X}", dividend_addr));

    // Increment quotient
    emitter.emit_inst("INC", &format!("${:02X}", quotient_addr));
    emitter.emit_inst("JMP", &loop_label);

    emitter.emit_label(&end_label);
    emitter.emit_inst("LDA", &format!("${:02X}", quotient_addr));

    // Free temp storage
    emitter.temp_alloc.free_primary(quotient_addr, 2);

    Ok(())
}

fn generate_divide_u16(emitter: &mut Emitter) -> Result<(), CodegenError> {
    // For u16 / u16, call the stdlib div16 function
    // Input: Left operand in A:Y, Right operand in TEMP:TEMP+1 ($20:$21)
    // Output: Result in A:Y

    if emitter.is_verbose() {
        emitter.emit_comment("Call stdlib div16 for u16 division");
    }

    // Mark that we need div16 function
    emitter.needs_div16 = true;

    // div16 expects parameters at $D9-$DC
    // Store left operand (A:Y) to $D9-$DA
    emitter.emit_inst("STA", "$D9"); // Store low byte
    emitter.emit_inst("STY", "$DA"); // Store high byte

    // Store right operand (TEMP:TEMP+1) to $DB-$DC
    let temp = emitter.memory_layout.temp_reg();
    emitter.emit_inst("LDA", &format!("${:02X}", temp)); // Load right.low
    emitter.emit_inst("STA", "$DB");
    emitter.emit_inst("LDA", &format!("${:02X}", temp + 1)); // Load right.high
    emitter.emit_inst("STA", "$DC");

    // Call div16
    emitter.emit_inst("JSR", "div16");

    if emitter.is_verbose() {
        emitter.emit_comment("Returns: A=quotient_low, Y=quotient_high (u16)");
    }

    Ok(())
}

fn generate_modulo(
    emitter: &mut Emitter,
    is_u16: bool,
    is_signed: bool,
) -> Result<(), CodegenError> {
    if is_u16 {
        if is_signed {
            return generate_modulo_i16(emitter);
        }
        return generate_modulo_u16(emitter);
    }
    if is_signed {
        return generate_modulo_i8(emitter);
    }

    // Modulo A % TEMP using repeated subtraction
    // Result (remainder) in A

    // Allocate temp storage
    let dividend_addr = emitter.temp_alloc.alloc_primary(1).unwrap_or(0x23);

    let loop_label = emitter.next_label("md");
    let end_label = emitter.next_label("mx");

    // Check for division by zero
    emitter.emit_inst("LDX", &format!("${:02X}", emitter.memory_layout.temp_reg()));
    emitter.emit_inst("CPX", "#$00");
    emitter.emit_inst("BEQ", &end_label); // Result undefined, leave A as-is

    // Store dividend
    emitter.emit_inst("STA", &format!("${:02X}", dividend_addr));

    // Loop: subtract divisor from dividend until dividend < divisor
    emitter.emit_label(&loop_label);
    emitter.emit_inst("LDA", &format!("${:02X}", dividend_addr));
    emitter.emit_inst("CMP", &format!("${:02X}", emitter.memory_layout.temp_reg()));
    emitter.emit_inst("BCC", &end_label); // If dividend < divisor, done (A has remainder)

    // Subtract divisor
    emitter.emit_inst("SEC", "");
    emitter.emit_inst("SBC", &format!("${:02X}", emitter.memory_layout.temp_reg()));
    emitter.emit_inst("STA", &format!("${:02X}", dividend_addr));
    emitter.emit_inst("JMP", &loop_label);

    emitter.emit_label(&end_label);
    emitter.emit_inst("LDA", &format!("${:02X}", dividend_addr));

    // Free temp storage
    emitter.temp_alloc.free_primary(dividend_addr, 1);

    Ok(())
}

fn generate_modulo_u16(emitter: &mut Emitter) -> Result<(), CodegenError> {
    // For u16 % u16, call the stdlib mod16 function
    // Input: Left operand in A:Y, Right operand in TEMP:TEMP+1 ($20:$21)
    // Output: Remainder in A:Y

    if emitter.is_verbose() {
        emitter.emit_comment("Call stdlib mod16 for u16 modulo");
    }

    // Mark that we need mod16 function
    emitter.needs_mod16 = true;

    // mod16 expects parameters at $D9-$DC
    // Store left operand (A:Y) to $D9-$DA
    emitter.emit_inst("STA", "$D9"); // Store low byte
    emitter.emit_inst("STY", "$DA"); // Store high byte

    // Store right operand (TEMP:TEMP+1) to $DB-$DC
    let temp = emitter.memory_layout.temp_reg();
    emitter.emit_inst("LDA", &format!("${:02X}", temp)); // Load right.low
    emitter.emit_inst("STA", "$DB");
    emitter.emit_inst("LDA", &format!("${:02X}", temp + 1)); // Load right.high
    emitter.emit_inst("STA", "$DC");

    // Call mod16
    emitter.emit_inst("JSR", "mod16");

    if emitter.is_verbose() {
        emitter.emit_comment("Returns: A=remainder_low, Y=remainder_high (u16)");
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Signed division and modulo (sign-magnitude wrappers around the unsigned
// routines). Truncated (round-toward-zero) semantics: the quotient's sign is
// sign(a) ⊕ sign(b), and the remainder takes the sign of the dividend `a`.
// ---------------------------------------------------------------------------

fn generate_divide_i8(emitter: &mut Emitter) -> Result<(), CodegenError> {
    let temp = emitter.memory_layout.temp_reg(); // divisor
    let divd = emitter.temp_alloc.alloc_primary(1).unwrap_or(0x26);
    let sign = emitter.temp_alloc.alloc_primary(1).unwrap_or(0x27);

    // Save dividend, then compute result sign = dividend ⊕ divisor (bit 7).
    emitter.emit_inst("STA", &format!("${:02X}", divd));
    emitter.emit_inst("EOR", &format!("${:02X}", temp));
    emitter.emit_inst("STA", &format!("${:02X}", sign));

    // abs(divisor) in place.
    let vpos = emitter.next_label("dv");
    emitter.emit_inst("LDA", &format!("${:02X}", temp));
    emitter.emit_inst("BPL", &vpos);
    emitter.emit_inst("EOR", "#$FF");
    emitter.emit_inst("CLC", "");
    emitter.emit_inst("ADC", "#$01");
    emitter.emit_inst("STA", &format!("${:02X}", temp));
    emitter.emit_label(&vpos);

    // abs(dividend) into A (LDA sets N to the dividend's sign for emit_abs8_a).
    emitter.emit_inst("LDA", &format!("${:02X}", divd));
    emit_abs8_a(emitter);

    // Unsigned quotient in A, then apply the result sign.
    generate_divide(emitter, false, false)?;
    emit_negate8_if_sign(emitter, sign);

    emitter.temp_alloc.free_primary(divd, 1);
    emitter.temp_alloc.free_primary(sign, 1);
    emitter.mark_a_unknown();
    Ok(())
}

fn generate_modulo_i8(emitter: &mut Emitter) -> Result<(), CodegenError> {
    let temp = emitter.memory_layout.temp_reg(); // divisor
    let divd = emitter.temp_alloc.alloc_primary(1).unwrap_or(0x26);

    // Save the dividend; its sign becomes the remainder's sign.
    emitter.emit_inst("STA", &format!("${:02X}", divd));

    // abs(divisor) in place.
    let vpos = emitter.next_label("mv");
    emitter.emit_inst("LDA", &format!("${:02X}", temp));
    emitter.emit_inst("BPL", &vpos);
    emitter.emit_inst("EOR", "#$FF");
    emitter.emit_inst("CLC", "");
    emitter.emit_inst("ADC", "#$01");
    emitter.emit_inst("STA", &format!("${:02X}", temp));
    emitter.emit_label(&vpos);

    // abs(dividend) into A.
    emitter.emit_inst("LDA", &format!("${:02X}", divd));
    emit_abs8_a(emitter);

    // Unsigned remainder in A, then apply the dividend's sign.
    generate_modulo(emitter, false, false)?;
    emit_negate8_if_sign(emitter, divd);

    emitter.temp_alloc.free_primary(divd, 1);
    emitter.mark_a_unknown();
    Ok(())
}

fn generate_divide_i16(emitter: &mut Emitter) -> Result<(), CodegenError> {
    // Dividend in A/Y, divisor at TEMP/TEMP+1. div16 clobbers $20-$27, so the
    // result sign is parked at $28 to survive the call; $22/$23 hold the
    // dividend only before the call.
    let temp = emitter.memory_layout.temp_reg();
    emitter.emit_inst("STA", "$22");
    emitter.emit_inst("STY", "$23");
    // Result sign = dividend.high ⊕ divisor.high (bit 7).
    emitter.emit_inst("LDA", "$23");
    emitter.emit_inst("EOR", &format!("${:02X}", temp + 1));
    emitter.emit_inst("STA", "$28");

    emit_abs16_zp(emitter, temp); // abs divisor
    emit_abs16_zp(emitter, 0x22); // abs dividend

    emitter.emit_inst("LDA", "$22");
    emitter.emit_inst("LDY", "$23");
    generate_divide_u16(emitter)?; // quotient in A/Y

    // Negate quotient if the result should be negative.
    let done = emitter.next_label("dn");
    emitter.emit_inst("BIT", "$28");
    emitter.emit_inst("BPL", &done);
    emitter.emit_inst("STA", "$22");
    emitter.emit_inst("STY", "$23");
    emit_negate16_zp(emitter, 0x22);
    emitter.emit_inst("LDA", "$22");
    emitter.emit_inst("LDY", "$23");
    emitter.emit_label(&done);
    emitter.mark_a_unknown();
    Ok(())
}

fn generate_modulo_i16(emitter: &mut Emitter) -> Result<(), CodegenError> {
    // Remainder takes the dividend's sign. Park it at $28 to survive mod16.
    let temp = emitter.memory_layout.temp_reg();
    emitter.emit_inst("STA", "$22");
    emitter.emit_inst("STY", "$23");
    emitter.emit_inst("LDA", "$23"); // dividend high
    emitter.emit_inst("STA", "$28"); // sign source (bit 7)

    emit_abs16_zp(emitter, temp); // abs divisor
    emit_abs16_zp(emitter, 0x22); // abs dividend

    emitter.emit_inst("LDA", "$22");
    emitter.emit_inst("LDY", "$23");
    generate_modulo_u16(emitter)?; // remainder in A/Y

    let done = emitter.next_label("mn");
    emitter.emit_inst("BIT", "$28");
    emitter.emit_inst("BPL", &done);
    emitter.emit_inst("STA", "$22");
    emitter.emit_inst("STY", "$23");
    emit_negate16_zp(emitter, 0x22);
    emitter.emit_inst("LDA", "$22");
    emitter.emit_inst("LDY", "$23");
    emitter.emit_label(&done);
    emitter.mark_a_unknown();
    Ok(())
}
