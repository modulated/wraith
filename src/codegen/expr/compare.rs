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

/// Emit `LDA $20` and set N so that a following `BMI` is taken iff the signed
/// value at `$20` is `< bound`.
///
/// For `bound == 0` this is just the sign-bit test (no subtraction). Otherwise
/// it computes `value - bound` and folds `N ⊕ V` into N via `EOR #$80` on
/// overflow, so a single `BMI` decides the signed comparison. `nov_label` is a
/// caller-unique label for the overflow-correction skip. The caller emits the
/// branch (near `BMI`, or a far branch) itself.
///
/// Shared by the match-range codegen in both the statement and expression paths
/// (`stmt::match_stmt` and `expr`), which differ only in how they branch.
pub(crate) fn emit_signed_lt_flag(emitter: &mut Emitter, bound: i64, nov_label: &str) {
    emitter.emit_inst("LDA", "$20");
    if bound != 0 {
        emitter.emit_inst("SEC", "");
        emitter.emit_inst("SBC", &format!("#${:02X}", bound as u8));
        emitter.emit_inst("BVC", nov_label);
        emitter.emit_inst("EOR", "#$80");
        emitter.emit_label(nov_label);
    }
}

/// Emit a *signed* ordering comparison, leaving a 0/1 boolean in A.
///
/// Left is in A (low) / Y (high for u16); right is at TEMP (/ TEMP+1). After a
/// signed subtraction `left - right`, the mathematical "less than" condition is
/// `N ⊕ V`; flipping bit 7 on overflow (`EOR #$80`) folds that into N alone so a
/// single `BMI`/`BPL` decides the result. `>` and `<=` reuse the same core by
/// swapping the operands first (`a > b` ⟺ `b < a`).
///
/// - `swap`     : exchange left/right before subtracting (for GT/LE).
/// - `true_on_neg`: branch to "true" on `BMI` (N set) when true, else `BPL`.
fn emit_signed_compare(
    emitter: &mut Emitter,
    is_u16: bool,
    temp: u8,
    swap: bool,
    true_on_neg: bool,
) -> Result<(), CodegenError> {
    let true_label = emitter.next_label("st");
    let end_label = emitter.next_label("sx");
    let nov_label = emitter.next_label("sv");

    if swap {
        if is_u16 {
            // Exchange the 16-bit operands via two scratch bytes ($22/$23 by
            // default) so `left - right` below actually computes `right - left`.
            let s = temp + 2;
            emitter.emit_inst("STA", &format!("${:02X}", s)); // save left low
            emitter.emit_inst("STY", &format!("${:02X}", s + 1)); // save left high
            emitter.emit_inst("LDA", &format!("${:02X}", temp)); // A = right low
            emitter.emit_inst("LDY", &format!("${:02X}", temp + 1)); // Y = right high
            emitter.emit_inst("LDX", &format!("${:02X}", s));
            emitter.emit_inst("STX", &format!("${:02X}", temp)); // TEMP = left low
            emitter.emit_inst("LDX", &format!("${:02X}", s + 1));
            emitter.emit_inst("STX", &format!("${:02X}", temp + 1)); // TEMP+1 = left high
        } else {
            emitter.emit_inst("LDX", &format!("${:02X}", temp)); // X = right
            emitter.emit_inst("STA", &format!("${:02X}", temp)); // TEMP = left
            emitter.emit_inst("TXA", ""); // A = right
        }
    }

    // Signed subtraction (8- or 16-bit).
    emitter.emit_inst("SEC", "");
    emitter.emit_inst("SBC", &format!("${:02X}", temp));
    if is_u16 {
        emitter.emit_inst("TYA", "");
        emitter.emit_inst("SBC", &format!("${:02X}", temp + 1));
    }

    // Fold (N ⊕ V) into N: on signed overflow the sign bit is inverted.
    emitter.emit_inst("BVC", &nov_label);
    emitter.emit_inst("EOR", "#$80");
    emitter.emit_label(&nov_label);

    if true_on_neg {
        emitter.emit_inst("BMI", &true_label);
    } else {
        emitter.emit_inst("BPL", &true_label);
    }

    emitter.emit_inst("LDA", "#$00");
    emitter.emit_inst("JMP", &end_label);
    emitter.emit_label(&true_label);
    emitter.emit_inst("LDA", "#$01");
    emitter.emit_label(&end_label);
    emitter.mark_a_unknown();
    Ok(())
}

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

        emit_bool_tail(emitter, &true_label, None, &end_label);
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

        emit_bool_tail(emitter, &true_label, None, &end_label);
    }

    emitter.emit_label(&end_label);
    // Comparison modifies A register - invalidate tracking
    emitter.mark_a_unknown();
    Ok(())
}

/// Generate less-than comparison (<)
///
/// Compares A register with value at TEMP and sets A to 1 if less than, 0 otherwise
/// Emit the shared boolean-result tail of an unsigned comparison: the false
/// path loads `#$00` and jumps to `end`, the true path (at `true_label`) loads
/// `#$01` and falls through. `false_label` is `Some` when the false path is a
/// branch target that needs its own label, `None` when control falls into it.
///
/// The caller emits `end` and invalidates A afterwards (shared by both the
/// 8- and 16-bit arms, so it stays out of this helper to avoid a duplicate
/// label).
fn emit_bool_tail(
    emitter: &mut Emitter,
    true_label: &str,
    false_label: Option<&str>,
    end_label: &str,
) {
    if let Some(f) = false_label {
        emitter.emit_label(f);
    }
    emitter.emit_inst("LDA", "#$00");
    emitter.emit_inst("JMP", end_label);
    emitter.emit_label(true_label);
    emitter.emit_inst("LDA", "#$01");
}

pub(super) fn generate_compare_lt(
    emitter: &mut Emitter,
    is_u16: bool,
    is_signed: bool,
) -> Result<(), CodegenError> {
    // A < TEMP
    let temp = emitter.memory_layout.temp_reg();
    if is_signed {
        return emit_signed_compare(emitter, is_u16, temp, false, true);
    }
    let true_label = emitter.next_label("lt");
    let end_label = emitter.next_label("lx");

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

        emit_bool_tail(emitter, &true_label, Some(&false_label), &end_label);
    } else {
        // 8-bit comparison
        emitter.emit_inst("CMP", &format!("${:02X}", temp));
        emitter.emit_inst("BCC", &true_label); // Branch if carry clear (A < TEMP)

        emit_bool_tail(emitter, &true_label, None, &end_label);
    }

    emitter.emit_label(&end_label);
    // Comparison modifies A register - invalidate tracking
    emitter.mark_a_unknown();
    Ok(())
}

/// Generate greater-than-or-equal comparison (>=)
///
/// Compares A register with value at TEMP and sets A to 1 if greater than or equal, 0 otherwise
pub(super) fn generate_compare_ge(
    emitter: &mut Emitter,
    is_u16: bool,
    is_signed: bool,
) -> Result<(), CodegenError> {
    // A >= TEMP: Opposite of <
    // Optimized: Invert logic of LT
    let temp = emitter.memory_layout.temp_reg();
    if is_signed {
        return emit_signed_compare(emitter, is_u16, temp, false, false);
    }
    let true_label = emitter.next_label("gt");
    let end_label = emitter.next_label("gx");

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

        emit_bool_tail(emitter, &true_label, Some(&false_label), &end_label);
    } else {
        // 8-bit comparison
        emitter.emit_inst("CMP", &format!("${:02X}", temp));
        emitter.emit_inst("BCS", &true_label); // Branch if carry set (A >= TEMP)

        emit_bool_tail(emitter, &true_label, None, &end_label);
    }

    emitter.emit_label(&end_label);
    // Comparison modifies A register - invalidate tracking
    emitter.mark_a_unknown();
    Ok(())
}

/// Generate greater-than comparison (>)
///
/// Compares A register with value at TEMP and sets A to 1 if greater than, 0 otherwise
pub(super) fn generate_compare_gt(
    emitter: &mut Emitter,
    is_u16: bool,
    is_signed: bool,
) -> Result<(), CodegenError> {
    // A > TEMP
    let temp = emitter.memory_layout.temp_reg();
    if is_signed {
        // a > b  ⟺  b < a: swap operands and use the less-than core.
        return emit_signed_compare(emitter, is_u16, temp, true, true);
    }
    let true_label = emitter.next_label("gt");
    let end_label = emitter.next_label("gx");

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

        emit_bool_tail(emitter, &true_label, Some(&false_label), &end_label);
    } else {
        // 8-bit comparison. The equal case must fall into the false path and
        // load 0: branching straight to the end would leave the left operand in
        // A, which a following `CMP #$00 / BNE` reads as *true*, so `a > b`
        // would wrongly hold for a == b.
        let false_label = emitter.next_label("gf");
        emitter.emit_inst("CMP", &format!("${:02X}", temp));
        emitter.emit_inst("BEQ", &false_label); // equal -> not greater
        emitter.emit_inst("BCS", &true_label); // carry set and not equal -> A > TEMP

        emit_bool_tail(emitter, &true_label, Some(&false_label), &end_label);
    }

    emitter.emit_label(&end_label);
    // Comparison modifies A register - invalidate tracking
    emitter.mark_a_unknown();
    Ok(())
}

/// Generate less-than-or-equal comparison (<=)
///
/// Compares A register with value at TEMP and sets A to 1 if less than or equal, 0 otherwise
pub(super) fn generate_compare_le(
    emitter: &mut Emitter,
    is_u16: bool,
    is_signed: bool,
) -> Result<(), CodegenError> {
    // A <= TEMP
    let temp = emitter.memory_layout.temp_reg();
    if is_signed {
        // a <= b  ⟺  b >= a: swap operands and use the >= core.
        return emit_signed_compare(emitter, is_u16, temp, true, false);
    }
    let end_label = emitter.next_label("lx");

    if is_u16 {
        // 16-bit unsigned: left (A=low, Y=high) <= right (TEMP=low, TEMP+1=high)
        let true_label = emitter.next_label("lt");
        let false_label = emitter.next_label("lf");

        // Compare high bytes first
        emitter.emit_inst("CPY", &format!("${:02X}", temp + 1));
        emitter.emit_inst("BCC", &true_label); // left.high < right.high -> true
        emitter.emit_inst("BNE", &false_label); // left.high > right.high -> false

        // High bytes equal: compare low bytes
        emitter.emit_inst("CMP", &format!("${:02X}", temp));
        emitter.emit_inst("BEQ", &true_label); // equal -> true
        emitter.emit_inst("BCC", &true_label); // left.low < right.low -> true

        emit_bool_tail(emitter, &true_label, Some(&false_label), &end_label);
    } else {
        // 8-bit unsigned: A <= TEMP
        let true_label = emitter.next_label("lt");
        emitter.emit_inst("CMP", &format!("${:02X}", temp));
        emitter.emit_inst("BEQ", &true_label); // equal -> true
        emitter.emit_inst("BCC", &true_label); // A < TEMP -> true

        emit_bool_tail(emitter, &true_label, None, &end_label);
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
