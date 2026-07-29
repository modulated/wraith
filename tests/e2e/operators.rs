//! Operator tests, verified by execution.
//!
//! These assert the *value* an operator produces, not that some opcode appears
//! in the output. Asserting that `x += 5` emits an `ADC` says nothing about
//! whether `x` ends up as 15; running it does — and it catches the wrong-result
//! miscompiles that instruction-presence checks miss entirely.
//!
//! Two optimisation shapes are still checked against the assembly, because they
//! are about *which* code is emitted rather than what it computes: `x += 1`
//! becoming `INC`, and `x -= 1` becoming `DEC`.

use crate::common::exec::run;
use crate::common::{assert_asm_contains, compile_success};

/// Run a program that computes a u8 into `OUT` ($0900) and return it.
fn eval_u8(body: &str) -> u8 {
    let src = format!(
        r#"
        const OUT: addr = 0x0900;
        #[reset]
        fn main() {{
            {body}
            loop {{}}
        }}
    "#
    );
    run(&src).mem(0x0900)
}

/// Run a program that computes a u16 into LO/HI ($0900/$0901) and return it.
fn eval_u16(body: &str) -> u16 {
    let src = format!(
        r#"
        const LO: addr = 0x0900;
        const HI: addr = 0x0901;
        #[reset]
        fn main() {{
            {body}
            loop {{}}
        }}
    "#
    );
    run(&src).mem16(0x0900)
}

/// Evaluate a bool condition, returning 1 when true and 2 when false.
fn eval_cond(decls: &str, cond: &str) -> u8 {
    eval_u8(&format!(
        "{decls} if {cond} {{ OUT = 1; }} else {{ OUT = 2; }}"
    ))
}

// ============================================================================
// Compound assignment
// ============================================================================

#[test]
fn compound_add_assign() {
    assert_eq!(eval_u8("let x: u8 = 10; x += 5; OUT = x;"), 15);
}

#[test]
fn compound_sub_assign() {
    assert_eq!(eval_u8("let x: u8 = 10; x -= 3; OUT = x;"), 7);
}

#[test]
fn compound_mul_assign() {
    assert_eq!(eval_u8("let x: u8 = 10; x *= 3; OUT = x;"), 30);
}

#[test]
fn compound_div_assign() {
    assert_eq!(eval_u8("let x: u8 = 10; x /= 2; OUT = x;"), 5);
}

#[test]
fn compound_bitwise_and_assign() {
    assert_eq!(eval_u8("let x: u8 = 0xFF; x &= 0x0F; OUT = x;"), 0x0F);
}

#[test]
fn compound_bitwise_or_assign() {
    assert_eq!(eval_u8("let x: u8 = 0x0F; x |= 0xF0; OUT = x;"), 0xFF);
}

#[test]
fn compound_bitwise_xor_assign() {
    assert_eq!(eval_u8("let x: u8 = 0xFF; x ^= 0xAA; OUT = x;"), 0x55);
}

#[test]
fn compound_shift_left_assign() {
    assert_eq!(eval_u8("let x: u8 = 1; x <<= 3; OUT = x;"), 8);
}

#[test]
fn compound_shift_right_assign() {
    assert_eq!(eval_u8("let x: u8 = 8; x >>= 2; OUT = x;"), 2);
}

// ============================================================================
// Increment / decrement, including wrap-around
// ============================================================================

#[test]
fn increment_variable() {
    assert_eq!(eval_u8("let x: u8 = 41; x += 1; OUT = x;"), 42);
}

#[test]
fn decrement_variable() {
    assert_eq!(eval_u8("let x: u8 = 43; x -= 1; OUT = x;"), 42);
}

#[test]
fn increment_wraps_at_255() {
    // Nothing traps on a 6502: 255 + 1 wraps to 0.
    assert_eq!(eval_u8("let x: u8 = 255; x += 1; OUT = x;"), 0);
}

#[test]
fn decrement_wraps_at_zero() {
    assert_eq!(eval_u8("let x: u8 = 0; x -= 1; OUT = x;"), 255);
}

#[test]
fn increment_by_one_uses_inc() {
    // A codegen-shape check, not a value check: ±1 must become INC/DEC rather
    // than a load/add/store sequence.
    let asm = compile_success("fn main() { let x: u8 = 10; x += 1; }");
    assert_asm_contains(&asm, "INC");
}

#[test]
fn decrement_by_one_uses_dec() {
    let asm = compile_success("fn main() { let x: u8 = 10; x -= 1; }");
    assert_asm_contains(&asm, "DEC");
}

// ============================================================================
// Bitwise operators
// ============================================================================

#[test]
fn bitwise_and() {
    assert_eq!(
        eval_u8("let a: u8 = 0xCC; let b: u8 = 0x0F; OUT = a & b;"),
        0x0C
    );
}

#[test]
fn bitwise_or() {
    assert_eq!(
        eval_u8("let a: u8 = 0xC0; let b: u8 = 0x0F; OUT = a | b;"),
        0xCF
    );
}

#[test]
fn bitwise_xor() {
    assert_eq!(
        eval_u8("let a: u8 = 0xCC; let b: u8 = 0xFF; OUT = a ^ b;"),
        0x33
    );
}

#[test]
fn bitwise_not() {
    assert_eq!(eval_u8("let a: u8 = 0x0F; OUT = ~a;"), 0xF0);
}

// ============================================================================
// Logical operators
// ============================================================================

#[test]
fn logical_and() {
    assert_eq!(
        eval_cond("let a: u8 = 1; let b: u8 = 1;", "a == 1 && b == 1"),
        1
    );
    assert_eq!(
        eval_cond("let a: u8 = 1; let b: u8 = 0;", "a == 1 && b == 1"),
        2
    );
}

#[test]
fn logical_or() {
    assert_eq!(
        eval_cond("let a: u8 = 0; let b: u8 = 1;", "a == 1 || b == 1"),
        1
    );
    assert_eq!(
        eval_cond("let a: u8 = 0; let b: u8 = 0;", "a == 1 || b == 1"),
        2
    );
}

#[test]
fn logical_not() {
    assert_eq!(eval_cond("let a: u8 = 0;", "!(a == 1)"), 1);
    assert_eq!(eval_cond("let a: u8 = 1;", "!(a == 1)"), 2);
}

// ============================================================================
// Comparisons
// ============================================================================

#[test]
fn comparison_equal() {
    assert_eq!(eval_cond("let a: u8 = 5; let b: u8 = 5;", "a == b"), 1);
    assert_eq!(eval_cond("let a: u8 = 5; let b: u8 = 6;", "a == b"), 2);
}

#[test]
fn comparison_not_equal() {
    assert_eq!(eval_cond("let a: u8 = 5; let b: u8 = 6;", "a != b"), 1);
    assert_eq!(eval_cond("let a: u8 = 5; let b: u8 = 5;", "a != b"), 2);
}

#[test]
fn comparison_less_than() {
    assert_eq!(eval_cond("let a: u8 = 4; let b: u8 = 5;", "a < b"), 1);
    assert_eq!(eval_cond("let a: u8 = 5; let b: u8 = 5;", "a < b"), 2);
}

#[test]
fn comparison_greater_than() {
    assert_eq!(eval_cond("let a: u8 = 6; let b: u8 = 5;", "a > b"), 1);
    assert_eq!(eval_cond("let a: u8 = 5; let b: u8 = 5;", "a > b"), 2);
}

#[test]
fn comparison_less_equal() {
    assert_eq!(eval_cond("let a: u8 = 5; let b: u8 = 5;", "a <= b"), 1);
    assert_eq!(eval_cond("let a: u8 = 6; let b: u8 = 5;", "a <= b"), 2);
}

#[test]
fn comparison_greater_equal() {
    assert_eq!(eval_cond("let a: u8 = 5; let b: u8 = 5;", "a >= b"), 1);
    assert_eq!(eval_cond("let a: u8 = 4; let b: u8 = 5;", "a >= b"), 2);
}

#[test]
fn comparison_is_unsigned_at_the_boundary() {
    // 0 and 255 are exactly where an accidental signed comparison shows up: as
    // i8, 255 is -1 and would compare *less* than 1.
    assert_eq!(
        eval_cond("let a: u8 = 255; let b: u8 = 1;", "a > b"),
        1,
        "255 > 1 must be an unsigned comparison"
    );
    assert_eq!(eval_cond("let a: u8 = 0; let b: u8 = 255;", "a < b"), 1);
}

// ============================================================================
// Shifts
// ============================================================================

#[test]
fn shift_left() {
    assert_eq!(eval_u8("let a: u8 = 0x03; OUT = a << 2;"), 0x0C);
}

#[test]
fn shift_right() {
    assert_eq!(eval_u8("let a: u8 = 0x0C; OUT = a >> 2;"), 0x03);
}

#[test]
fn shift_left_variable() {
    assert_eq!(eval_u8("let a: u8 = 1; let n: u8 = 5; OUT = a << n;"), 0x20);
}

#[test]
fn shift_right_variable() {
    assert_eq!(
        eval_u8("let a: u8 = 0x80; let n: u8 = 3; OUT = a >> n;"),
        0x10
    );
}

// ============================================================================
// 16-bit arithmetic: the byte boundary is where these break
// ============================================================================

#[test]
fn u16_add_carries_between_bytes() {
    assert_eq!(
        eval_u16("let a: u16 = 0x00FF; a = a + 1; LO = a.low; HI = a.high;"),
        0x0100
    );
}

#[test]
fn u16_sub_borrows_between_bytes() {
    assert_eq!(
        eval_u16("let a: u16 = 0x0100; a = a - 1; LO = a.low; HI = a.high;"),
        0x00FF
    );
}

#[test]
fn u16_comparison_uses_both_bytes() {
    // 0x0100 and 0x0001 differ only in which byte is set; a low-byte-only
    // comparison gets this backwards.
    assert_eq!(
        eval_cond("let a: u16 = 0x0100; let b: u16 = 0x0001;", "a > b"),
        1
    );
}

// ============================================================================
// Compound assignment targets
//
// `x += y` desugars to `x = x + y`, which evaluates the target twice. A pure
// target only costs cycles, but a call in the target ran its side effects
// three times in the emitted code — `arr[idx()] += 5` called idx() for the
// load, the add, and the store. That shape is a parse error now.
// ============================================================================

#[test]
fn compound_assignment_with_a_call_in_the_target_is_rejected() {
    crate::common::assert_error_contains(
        r#"
        fn idx() -> u8 { return 1; }
        #[reset]
        fn main() {
            let arr: [u8; 4] = [0; 4];
            arr[idx()] += 5;
            loop {}
        }
    "#,
        "compound assignment would evaluate this call twice",
    );
}

#[test]
fn compound_assignment_through_a_pure_index_still_works() {
    let mut e = run(r#"
        const OUT: addr = 0x0900;
        #[reset]
        fn main() {
            let arr: [u8; 4] = [10; 4];
            let i: u8 = 2;
            arr[i] += 5;
            arr[i + 1] += 1;
            OUT = arr[2] + arr[3];
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0900), 26);
}
