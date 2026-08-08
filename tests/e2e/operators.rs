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
fn u8_divide_and_modulo_by_zero_are_defined() {
    // The zero path used to read an uninitialized temp ("leave A as-is" was a
    // lie). Both now yield a defined 0xFF, matching div16/mod16's 0xFFFF.
    assert_eq!(
        eval_u8("let a: u8 = 42; let b: u8 = 0; OUT = a / b;"),
        0xFF,
        "u8 x / 0 -> 0xFF"
    );
    assert_eq!(
        eval_u8("let a: u8 = 42; let b: u8 = 0; OUT = a % b;"),
        0xFF,
        "u8 x % 0 -> 0xFF"
    );
}

#[test]
fn u8_divide_and_modulo_still_correct_for_nonzero() {
    // The div-by-zero restructure must not disturb the normal path.
    assert_eq!(eval_u8("let a: u8 = 23; let b: u8 = 5; OUT = a / b;"), 4);
    assert_eq!(eval_u8("let a: u8 = 23; let b: u8 = 5; OUT = a % b;"), 3);
    assert_eq!(eval_u8("let a: u8 = 100; let b: u8 = 10; OUT = a / b;"), 10);
    assert_eq!(eval_u8("let a: u8 = 7; let b: u8 = 8; OUT = a % b;"), 7);
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

#[test]
fn unary_minus_binds_postfix_field_access() {
    // `-p.x` failed to parse as a field negation ("cannot apply '-' to type
    // P") because unary minus never consumed postfix suffixes.
    let mut e = run(r#"
        struct P { x: i8 }
        const OUT: addr = 0x0900;
        #[reset]
        fn main() {
            let p: P = P { x: 5 };
            let v: i8 = -p.x;
            OUT = v as u8;
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0900), 0xFB, "-5 as two's complement");
}

#[test]
fn unary_bitnot_binds_postfix_index() {
    let mut e = run(r#"
        const OUT: addr = 0x0900;
        #[reset]
        fn main() {
            let a: [u8; 2] = [0x0F, 0];
            OUT = ~a[0];
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0900), 0xF0);
}

// ---------------------------------------------------------------------------
// Division / modulo by zero is defined (0xFF), not undefined behavior
// ---------------------------------------------------------------------------

#[test]
fn u8_divide_by_zero_yields_a_defined_sentinel() {
    // The zero path once fell through and loaded an uninitialized temp ("leave A
    // as-is"); it now yields 0xFF, matching div16's 0xFFFF all-ones sentinel.
    assert_eq!(eval_u8("let a: u8 = 42; let b: u8 = 0; OUT = a / b;"), 0xFF);
}

#[test]
fn u8_modulo_by_zero_yields_a_defined_sentinel() {
    assert_eq!(eval_u8("let a: u8 = 42; let b: u8 = 0; OUT = a % b;"), 0xFF);
}

// ---------------------------------------------------------------------------
// Literal and precedence details (regression locks; both already correct)
// ---------------------------------------------------------------------------

#[test]
fn underscores_in_a_numeric_literal_are_digit_separators() {
    // `1_000` is 1000, not `1` followed by an identifier `_000`.
    assert_eq!(
        eval_u16("let x: u16 = 1_000; LO = x.low; HI = x.high;"),
        1000
    );
    assert_eq!(eval_u8("let x: u8 = 0b1010_0101; OUT = x;"), 0xA5);
    assert_eq!(eval_u8("let x: u8 = 0x1_F; OUT = x;"), 0x1F);
}

#[test]
fn a_prefix_operator_binds_tighter_than_a_cast() {
    // `&x as u16` is `(&x) as u16` (the address as a number), matching Rust —
    // not `&(x as u16)`, which would take the address of a temporary and fail.
    // The same rule gives `-x as i16` = `(-x) as i16`.
    let _asm = compile_success(
        "#[reset]\nfn main() { let x: u8 = 5; let a: u16 = &x as u16; let n: i16 = -x as i16; loop {} }",
    );
}

#[test]
fn u16_bitwise_ops_combine_both_bytes() {
    // Regression: u16 &/|/^ only combined the low byte, leaving the high byte
    // of the result equal to the left operand's high byte.
    assert_eq!(
        eval_u16("let a: u16 = 0x0001; LO = (a | 0x8000).low; HI = (a | 0x8000).high;"),
        0x8001
    );
    assert_eq!(
        eval_u16("let a: u16 = 0xFFFF; LO = (a & 0x0F0F).low; HI = (a & 0x0F0F).high;"),
        0x0F0F
    );
    assert_eq!(
        eval_u16("let a: u16 = 0xFF00; LO = (a ^ 0x0FF0).low; HI = (a ^ 0x0FF0).high;"),
        0xF0F0
    );
}
