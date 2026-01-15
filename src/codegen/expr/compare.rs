//! Comparison and logical operation code generation
//!
//! This module handles:
//! - All 6 comparison operators (==, !=, <, >, <=, >=)
//! - Logical AND/OR with short-circuit evaluation
//! - Boolean result generation (0 for false, 1 for true)

use crate::ast::{Expr, Spanned};
use crate::codegen::{CodegenError, Emitter, StringCollector};
use crate::sema::ProgramInfo;

// Import generate_expr from parent module for recursive calls
use super::generate_expr;

/// Generate equality comparison (==)
///
/// Compares A register with value at TEMP and sets A to 1 if equal, 0 otherwise
pub(super) fn generate_compare_eq(emitter: &mut Emitter, is_u16: bool) -> Result<(), CodegenError> {
    // A == TEMP: Compare and set A to 1 if equal, 0 otherwise
    let true_label = emitter.next_label("et");
    let end_label = emitter.next_label("ex");
    let temp = emitter.memory_layout.temp_reg();

    if is_u16 {
        // 16-bit comparison
        // Compare low bytes (A vs TEMP)
        emitter.emit_inst("CMP", &format!("${:02X}", temp));
        let false_label = emitter.next_label("ef");
        emitter.emit_inst("BNE", &false_label);

        // Compare high bytes (Y vs TEMP+1)
        emitter.emit_inst("CPY", &format!("${:02X}", temp + 1));
        emitter.emit_inst("BNE", &false_label);

        // Equal
        emitter.emit_inst("LDA", "#$01");
        emitter.emit_inst("JMP", &end_label);

        emitter.emit_label(&false_label);
        emitter.emit_inst("LDA", "#$00");
    } else {
        // 8-bit comparison
        emitter.emit_inst("CMP", &format!("${:02X}", temp));
        emitter.emit_inst("BEQ", &true_label);

        // False case
        emitter.emit_inst("LDA", "#$00");
        emitter.emit_inst("JMP", &end_label);

        // True case
        emitter.emit_label(&true_label);
        emitter.emit_inst("LDA", "#$01");
    }

    emitter.emit_label(&end_label);
    // Comparison modifies A register - invalidate tracking
    emitter.mark_a_unknown();
    Ok(())
}

/// Generate inequality comparison (!=)
///
/// Compares A register with value at TEMP and sets A to 1 if not equal, 0 otherwise
pub(super) fn generate_compare_ne(emitter: &mut Emitter, is_u16: bool) -> Result<(), CodegenError> {
    // A != TEMP: Opposite of equal
    let true_label = emitter.next_label("nt");
    let end_label = emitter.next_label("nx");
    let temp = emitter.memory_layout.temp_reg();

    if is_u16 {
        // 16-bit comparison
        // Compare low bytes
        emitter.emit_inst("CMP", &format!("${:02X}", temp));
        emitter.emit_inst("BNE", &true_label);

        // Compare high bytes
        emitter.emit_inst("CPY", &format!("${:02X}", temp + 1));
        emitter.emit_inst("BNE", &true_label);

        // Equal (False)
        emitter.emit_inst("LDA", "#$00");
        emitter.emit_inst("JMP", &end_label);

        // Not Equal (True)
        emitter.emit_label(&true_label);
        emitter.emit_inst("LDA", "#$01");
    } else {
        // 8-bit comparison
        emitter.emit_inst("CMP", &format!("${:02X}", temp));
        emitter.emit_inst("BNE", &true_label);

        // False case
        emitter.emit_inst("LDA", "#$00");
        emitter.emit_inst("JMP", &end_label);

        // True case
        emitter.emit_label(&true_label);
        emitter.emit_inst("LDA", "#$01");
    }

    emitter.emit_label(&end_label);
    // Comparison modifies A register - invalidate tracking
    emitter.mark_a_unknown();
    Ok(())
}

/// Generate less-than comparison (<)
///
/// Compares A register with value at TEMP and sets A to 1 if less than, 0 otherwise
pub(super) fn generate_compare_lt(emitter: &mut Emitter, is_u16: bool) -> Result<(), CodegenError> {
    // A < TEMP
    let true_label = emitter.next_label("lt");
    let end_label = emitter.next_label("lx");
    let temp = emitter.memory_layout.temp_reg();

    if is_u16 {
        // 16-bit comparison (Unsigned)
        let false_label = emitter.next_label("lf");

        // Compare High Bytes (Y vs TEMP+1)
        emitter.emit_inst("CPY", &format!("${:02X}", temp + 1));
        emitter.emit_inst("BCC", &true_label); // Y < High -> True
        emitter.emit_inst("BNE", &false_label); // Y > High -> False

        // High bytes equal, compare Low Bytes (A vs TEMP)
        emitter.emit_inst("CMP", &format!("${:02X}", temp));
        emitter.emit_inst("BCC", &true_label); // A < Low -> True

        // False
        emitter.emit_label(&false_label);
        emitter.emit_inst("LDA", "#$00");
        emitter.emit_inst("JMP", &end_label);

        // True
        emitter.emit_label(&true_label);
        emitter.emit_inst("LDA", "#$01");
    } else {
        // 8-bit comparison
        emitter.emit_inst("CMP", &format!("${:02X}", temp));
        emitter.emit_inst("BCC", &true_label); // Branch if carry clear (A < TEMP)

        // False case
        emitter.emit_inst("LDA", "#$00");
        emitter.emit_inst("JMP", &end_label);

        // True case
        emitter.emit_label(&true_label);
        emitter.emit_inst("LDA", "#$01");
    }

    emitter.emit_label(&end_label);
    // Comparison modifies A register - invalidate tracking
    emitter.mark_a_unknown();
    Ok(())
}

/// Generate greater-than-or-equal comparison (>=)
///
/// Compares A register with value at TEMP and sets A to 1 if greater than or equal, 0 otherwise
pub(super) fn generate_compare_ge(emitter: &mut Emitter, is_u16: bool) -> Result<(), CodegenError> {
    // A >= TEMP: Opposite of <
    // Optimized: Invert logic of LT
    let true_label = emitter.next_label("gt");
    let end_label = emitter.next_label("gx");
    let temp = emitter.memory_layout.temp_reg();

    if is_u16 {
        // 16-bit (Unsigned)
        let false_label = emitter.next_label("gf");

        // Compare High Bytes
        emitter.emit_inst("CPY", &format!("${:02X}", temp + 1));
        emitter.emit_inst("BCC", &false_label); // Y < High -> False
        emitter.emit_inst("BNE", &true_label); // Y > High -> True

        // High equal, check Low
        emitter.emit_inst("CMP", &format!("${:02X}", temp));
        emitter.emit_inst("BCS", &true_label); // A >= Low -> True

        // False
        emitter.emit_label(&false_label);
        emitter.emit_inst("LDA", "#$00");
        emitter.emit_inst("JMP", &end_label);

        // True
        emitter.emit_label(&true_label);
        emitter.emit_inst("LDA", "#$01");
    } else {
        // 8-bit comparison
        emitter.emit_inst("CMP", &format!("${:02X}", temp));
        emitter.emit_inst("BCS", &true_label); // Branch if carry set (A >= TEMP)

        // False case
        emitter.emit_inst("LDA", "#$00");
        emitter.emit_inst("JMP", &end_label);

        // True case
        emitter.emit_label(&true_label);
        emitter.emit_inst("LDA", "#$01");
    }

    emitter.emit_label(&end_label);
    // Comparison modifies A register - invalidate tracking
    emitter.mark_a_unknown();
    Ok(())
}

/// Generate greater-than comparison (>)
///
/// Compares A register with value at TEMP and sets A to 1 if greater than, 0 otherwise
pub(super) fn generate_compare_gt(emitter: &mut Emitter, is_u16: bool) -> Result<(), CodegenError> {
    // A > TEMP
    let true_label = emitter.next_label("gt");
    let end_label = emitter.next_label("gx");
    let temp = emitter.memory_layout.temp_reg();

    if is_u16 {
        // 16-bit (Unsigned)
        let false_label = emitter.next_label("gf");

        // Compare High Bytes
        emitter.emit_inst("CPY", &format!("${:02X}", temp + 1));
        emitter.emit_inst("BCC", &false_label); // Y < High -> False
        emitter.emit_inst("BNE", &true_label); // Y > High -> True

        // High equal, check Low
        // A > Low means A >= Low AND A != Low
        // BCS (>=) and BNE (!=)
        emitter.emit_inst("CMP", &format!("${:02X}", temp));
        emitter.emit_inst("BEQ", &false_label);
        emitter.emit_inst("BCS", &true_label);

        // False
        emitter.emit_label(&false_label);
        emitter.emit_inst("LDA", "#$00");
        emitter.emit_inst("JMP", &end_label);

        // True
        emitter.emit_label(&true_label);
        emitter.emit_inst("LDA", "#$01");
    } else {
        // 8-bit comparison
        emitter.emit_inst("CMP", &format!("${:02X}", temp));
        emitter.emit_inst("BEQ", &end_label); // If equal, result is 0 (false)
        emitter.emit_inst("BCS", &true_label); // If carry set and not equal, A > TEMP

        // False case (carry clear, meaning A < TEMP)
        emitter.emit_inst("LDA", "#$00");
        emitter.emit_inst("JMP", &end_label);

        // True case
        emitter.emit_label(&true_label);
        emitter.emit_inst("LDA", "#$01");
    }

    emitter.emit_label(&end_label);
    // Comparison modifies A register - invalidate tracking
    emitter.mark_a_unknown();
    Ok(())
}

/// Generate less-than-or-equal comparison (<=)
///
/// Compares A register with value at TEMP and sets A to 1 if less than or equal, 0 otherwise
pub(super) fn generate_compare_le(emitter: &mut Emitter, is_u16: bool) -> Result<(), CodegenError> {
    // A <= TEMP
    let end_label = emitter.next_label("lx");
    let temp = emitter.memory_layout.temp_reg();

    if is_u16 {
        // 16-bit (Unsigned)
        let true_label = emitter.next_label("lt");

        // Compare High Bytes
        emitter.emit_inst("CPY", &format!("${:02X}", temp + 1));
        emitter.emit_inst("BCC", &true_label); // Y < High -> True
        emitter.emit_inst("BNE", &end_label); // Y > High -> False (LDA 0 below)

        // High equal, check Low
        emitter.emit_inst("CMP", &format!("${:02X}", temp));
        emitter.emit_inst("BEQ", &true_label); // Equal -> True
        emitter.emit_inst("BCC", &true_label); // A < Low -> True

        // False (Default fallthrough state needs to be loaded with 0? No wait)
        emitter.emit_inst("LDA", "#$00");
        emitter.emit_inst("JMP", &end_label);

        // True
        emitter.emit_label(&true_label);
        emitter.emit_inst("LDA", "#$01");
    } else {
        // 8-bit comparison
        emitter.emit_inst("CMP", &format!("${:02X}", temp));
        emitter.emit_inst("BEQ", &end_label); // If equal, keep A as is (wait, A is matched val, non-zero? No A==TEMP. BEQ taken means A==TEMP.)
        // Bug in original: "If equal, keep A as is, set to 1 after".
        // If A=5, TEMP=5. CMP -> Z=1. BEQ end_label.
        // At end_label, A is still 5!
        // But we want boolean logic 0/1.
        // If A=0, TEMP=0. A=0.
        // Incorrect logic in original?
        // Let's fix 8-bit too.

        // Original logic:
        // BEQ end_label
        // BCS false_label
        // LDA 1
        // JMP end
        // false: LDA 0
        // end:

        // If EQ: Jumps to END with A unchanged.
        // If A was 5. A is 5 (True in C bool, but we want 1).
        // If A was 0. A is 0 (False).
        // So `0 <= 0` returns 0 (False)? Wrong.

        // Fix 8-bit logic:
        emitter.emit_inst("CMP", &format!("${:02X}", temp));
        let true_label = emitter.next_label("lt");
        emitter.emit_inst("BEQ", &true_label);
        emitter.emit_inst("BCC", &true_label);

        // False
        emitter.emit_inst("LDA", "#$00");
        emitter.emit_inst("JMP", &end_label);

        // True
        emitter.emit_label(&true_label);
        emitter.emit_inst("LDA", "#$01");
    }

    emitter.emit_label(&end_label);
    // Comparison modifies A register - invalidate tracking
    emitter.mark_a_unknown();
    Ok(())
}

/// Generate logical AND with short-circuit evaluation
///
/// Evaluates left operand first. If false, skips right operand and returns false.
/// Only evaluates right operand if left is true.
pub(super) fn generate_logical_and(
    left: &Spanned<Expr>,
    right: &Spanned<Expr>,
    emitter: &mut Emitter,
    info: &ProgramInfo,
    string_collector: &mut StringCollector,
) -> Result<(), CodegenError> {
    // Short-circuit AND: if left is false, skip right and return false
    let end_label = emitter.next_label("ax");

    // Evaluate left operand
    generate_expr(left, emitter, info, string_collector)?;

    // If left is false (0), result is false
    emitter.emit_inst("CMP", "#$00");
    emitter.emit_inst("BEQ", &end_label); // If zero, A is already 0, done

    // Left was true, evaluate right
    generate_expr(right, emitter, info, string_collector)?;

    // Convert right to boolean (0 or 1)
    let true_label = emitter.next_label("at");
    emitter.emit_inst("CMP", "#$00");
    emitter.emit_inst("BNE", &true_label);

    // Right is false
    emitter.emit_inst("LDA", "#$00");
    emitter.emit_inst("JMP", &end_label);

    // Right is true
    emitter.emit_label(&true_label);
    emitter.emit_inst("LDA", "#$01");

    emitter.emit_label(&end_label);
    // Comparison modifies A register - invalidate tracking
    emitter.mark_a_unknown();
    Ok(())
}

/// Generate logical OR with short-circuit evaluation
///
/// Evaluates left operand first. If true, skips right operand and returns true.
/// Only evaluates right operand if left is false.
pub(super) fn generate_logical_or(
    left: &Spanned<Expr>,
    right: &Spanned<Expr>,
    emitter: &mut Emitter,
    info: &ProgramInfo,
    string_collector: &mut StringCollector,
) -> Result<(), CodegenError> {
    // Short-circuit OR: if left is true, skip right and return true
    let true_label = emitter.next_label("ot");
    let eval_right_label = emitter.next_label("or");
    let end_label = emitter.next_label("ox");

    // Evaluate left operand
    generate_expr(left, emitter, info, string_collector)?;

    // If left is true (non-zero), result is true
    emitter.emit_inst("CMP", "#$00");
    emitter.emit_inst("BEQ", &eval_right_label); // If zero, evaluate right
    emitter.emit_inst("JMP", &true_label); // Non-zero, result is true

    // Left was false, evaluate right
    emitter.emit_label(&eval_right_label);
    generate_expr(right, emitter, info, string_collector)?;

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
    // Comparison modifies A register - invalidate tracking
    emitter.mark_a_unknown();
    Ok(())
}
