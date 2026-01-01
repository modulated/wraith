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
pub(super) fn generate_compare_eq(emitter: &mut Emitter) -> Result<(), CodegenError> {
    // A == TEMP: Compare and set A to 1 if equal, 0 otherwise
    let true_label = emitter.next_label("et");
    let end_label = emitter.next_label("ex");

    emitter.emit_inst("CMP", &format!("${:02X}", emitter.memory_layout.temp_reg()));
    emitter.emit_inst("BEQ", &true_label);

    // False case
    emitter.emit_inst("LDA", "#$00");
    emitter.emit_inst("JMP", &end_label);

    // True case
    emitter.emit_label(&true_label);
    emitter.emit_inst("LDA", "#$01");

    emitter.emit_label(&end_label);
    // Comparison modifies A register - invalidate tracking
    emitter.mark_a_unknown();
    Ok(())
}

/// Generate inequality comparison (!=)
///
/// Compares A register with value at TEMP and sets A to 1 if not equal, 0 otherwise
pub(super) fn generate_compare_ne(emitter: &mut Emitter) -> Result<(), CodegenError> {
    // A != TEMP: Opposite of equal
    let true_label = emitter.next_label("nt");
    let end_label = emitter.next_label("nx");

    emitter.emit_inst("CMP", &format!("${:02X}", emitter.memory_layout.temp_reg()));
    emitter.emit_inst("BNE", &true_label);

    // False case
    emitter.emit_inst("LDA", "#$00");
    emitter.emit_inst("JMP", &end_label);

    // True case
    emitter.emit_label(&true_label);
    emitter.emit_inst("LDA", "#$01");

    emitter.emit_label(&end_label);
    // Comparison modifies A register - invalidate tracking
    emitter.mark_a_unknown();
    Ok(())
}

/// Generate less-than comparison (<)
///
/// Compares A register with value at TEMP and sets A to 1 if less than, 0 otherwise
pub(super) fn generate_compare_lt(emitter: &mut Emitter) -> Result<(), CodegenError> {
    // A < TEMP: Use CMP which sets carry flag if A >= TEMP
    let true_label = emitter.next_label("lt");
    let end_label = emitter.next_label("lx");

    emitter.emit_inst("CMP", &format!("${:02X}", emitter.memory_layout.temp_reg()));
    emitter.emit_inst("BCC", &true_label); // Branch if carry clear (A < TEMP)

    // False case
    emitter.emit_inst("LDA", "#$00");
    emitter.emit_inst("JMP", &end_label);

    // True case
    emitter.emit_label(&true_label);
    emitter.emit_inst("LDA", "#$01");

    emitter.emit_label(&end_label);
    // Comparison modifies A register - invalidate tracking
    emitter.mark_a_unknown();
    Ok(())
}

/// Generate greater-than-or-equal comparison (>=)
///
/// Compares A register with value at TEMP and sets A to 1 if greater than or equal, 0 otherwise
pub(super) fn generate_compare_ge(emitter: &mut Emitter) -> Result<(), CodegenError> {
    // A >= TEMP: Opposite of <
    let true_label = emitter.next_label("gt");
    let end_label = emitter.next_label("gx");

    emitter.emit_inst("CMP", &format!("${:02X}", emitter.memory_layout.temp_reg()));
    emitter.emit_inst("BCS", &true_label); // Branch if carry set (A >= TEMP)

    // False case
    emitter.emit_inst("LDA", "#$00");
    emitter.emit_inst("JMP", &end_label);

    // True case
    emitter.emit_label(&true_label);
    emitter.emit_inst("LDA", "#$01");

    emitter.emit_label(&end_label);
    // Comparison modifies A register - invalidate tracking
    emitter.mark_a_unknown();
    Ok(())
}

/// Generate greater-than comparison (>)
///
/// Compares A register with value at TEMP and sets A to 1 if greater than, 0 otherwise
pub(super) fn generate_compare_gt(emitter: &mut Emitter) -> Result<(), CodegenError> {
    // A > TEMP: Same as TEMP < A
    // We need to swap: load TEMP into A, then compare with original A
    // But original A is gone. Alternative: A > B is equivalent to NOT(A <= B)
    // A <= B means A < B OR A == B
    // So A > B means A >= B AND A != B
    // Or simply: CMP sets flags, if carry set AND not equal, then A > B

    let true_label = emitter.next_label("gt");
    let end_label = emitter.next_label("gx");

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
    // Comparison modifies A register - invalidate tracking
    emitter.mark_a_unknown();
    Ok(())
}

/// Generate less-than-or-equal comparison (<=)
///
/// Compares A register with value at TEMP and sets A to 1 if less than or equal, 0 otherwise
pub(super) fn generate_compare_le(emitter: &mut Emitter) -> Result<(), CodegenError> {
    // A <= TEMP: Same as NOT(A > TEMP)
    // A <= B means A < B OR A == B
    // CMP: if carry clear OR equal, then A <= TEMP

    let false_label = emitter.next_label("lf");
    let end_label = emitter.next_label("lx");

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
