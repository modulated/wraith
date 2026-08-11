//! `&&` and `||` as *values*, not just as conditions.
//!
//! Short-circuit `&&` used to jump to its exit "with A already zero" — true of
//! the code as emitted, false after the peephole collapsed the left comparison
//! into a bare branch, which leaves the comparison's intermediate in A instead.
//! `if (v < -102) && …` with `v == 0` then read `0 - 0x9A = 0x66` as a truthy
//! "false" and ran the body.
//!
//! Found by `tests/fuzz_exec.rs`. The pair of fixes is a codegen one (load the
//! zero rather than assume it) and a peephole one (do not collapse a boolean
//! whose value is still live).

use crate::common::compile_success;
use crate::common::exec::run;

/// Evaluate `cond` with `v` bound to `value` at type `ty`, reporting 1 or 0.
fn cond(ty: &str, value: &str, cond: &str) -> u8 {
    let mut e = run(&format!(
        "const OUT: addr = 0x0900;\n#[reset]\nfn main() {{\n\
         \x20   let v: {ty} = {value};\n    OUT = 0;\n\
         \x20   if {cond} {{ OUT = 1; }}\n    loop {{}}\n}}\n"
    ));
    e.mem(0x0900)
}

/// The exact shape the fuzzer reduced to: a false signed comparison whose
/// arithmetic leaves a nonzero byte in A.
#[test]
fn a_false_left_operand_makes_the_whole_and_false() {
    assert_eq!(
        cond("i8", "0", "(v < (-102)) && (v <= 0)"),
        0,
        "0 < -102 is false, so the && is false however the compare was emitted"
    );
}

/// The same, swept over the operand values where the subtraction's residue is
/// nonzero — one value passing by luck would prove nothing.
#[test]
fn a_false_left_operand_is_false_at_every_operand_value() {
    for v in ["-128", "-103", "-100", "-1", "0", "1", "126", "127"] {
        let expected = u8::from(v.parse::<i32>().unwrap() < -102);
        assert_eq!(
            cond("i8", v, "(v < (-102)) && (v <= 127)"),
            expected,
            "v = {v}"
        );
    }
}

#[test]
fn and_is_false_unless_both_sides_are_true() {
    for (a, b) in [(0, 0), (0, 1), (1, 0), (1, 1)] {
        assert_eq!(
            cond("u8", &a.to_string(), &format!("(v == 1) && ({b} == 1)")),
            u8::from(a == 1 && b == 1),
            "{a} && {b}"
        );
    }
}

#[test]
fn or_is_true_unless_both_sides_are_false() {
    for (a, b) in [(0, 0), (0, 1), (1, 0), (1, 1)] {
        assert_eq!(
            cond("u8", &a.to_string(), &format!("(v == 1) || ({b} == 1)")),
            u8::from(a == 1 || b == 1),
            "{a} || {b}"
        );
    }
}

/// `&&` and `||` nest, and the fix must not disturb the polarity of either.
#[test]
fn nested_connectives_agree_with_the_truth_table() {
    for v in 0u8..8 {
        let (x, y, z) = (v & 1 == 1, v & 2 == 2, v & 4 == 4);
        let src = "((v & 1) == 1) && (((v & 2) == 2) || (!((v & 4) == 4)))";
        assert_eq!(
            cond("u8", &v.to_string(), src),
            u8::from(x && (y || !z)),
            "v = {v}"
        );
    }
}

/// The right operand of `&&` is skipped when the left is false — the point of
/// short-circuiting. A division by a value that is zero on the skipped path is
/// the classic witness; here the witness is a side effect on a `static`.
#[test]
fn the_right_operand_is_not_evaluated_when_the_left_is_false() {
    let mut e = run("const OUT: addr = 0x0900;\nstatic TOUCHED: u8 = 0;\n\
         fn touch() -> u8 { TOUCHED = 1; return 1; }\n\
         #[reset]\nfn main() {\n    let v: u8 = 0;\n\
         \x20   if (v == 1) && (touch() == 1) { }\n\
         \x20   OUT = TOUCHED;\n    loop {}\n}\n");
    assert_eq!(e.mem(0x0900), 0, "touch() must not run");
}

/// A large right operand puts the `&&`'s exit far away. A conditional branch
/// reaches ±127 bytes, so this has to assemble without one spanning it — the
/// harness assembles to a byte image, which is where the range is checked.
#[test]
fn a_large_right_operand_still_assembles() {
    let bulk: String = (0..40)
        .map(|i| format!("(v % {}) ", i + 3))
        .collect::<Vec<_>>()
        .join("+ ");
    let mut e = run(&format!(
        "const OUT: addr = 0x0900;\n#[reset]\nfn main() {{\n\
         \x20   let v: u16 = 1;\n    OUT = 0;\n\
         \x20   if (v == 1) && (({bulk}) == 0) {{ OUT = 1; }}\n    loop {{}}\n}}\n"
    ));
    // Every `1 % n` with n >= 3 is 1, so the sum is 40, not 0.
    assert_eq!(e.mem(0x0900), 0);
}

/// The collapse that caused this is still worth having where it is sound, and
/// the accumulator-liveness guard must not switch it off: a plain `if x < n`
/// carries no materialized boolean at all. (`>` compiles to a two-branch
/// sequence that this collapse has never matched, so `<` is the case to pin.)
#[test]
fn a_plain_comparison_still_collapses_into_its_branch() {
    let asm = compile_success(
        "const OUT: addr = 0x0900;\n#[reset]\nfn main() {\n\
         \x20   let v: u8 = 5;\n    if v < 3 { OUT = 1; }\n    loop {}\n}\n",
    );
    // The only `LDA #$01` left is the assignment in the body; the boolean's
    // own `LDA #$00` / `LDA #$01` pair and its `CMP #$00` are gone.
    assert_eq!(
        asm.matches("LDA #$01").count(),
        1,
        "expected the comparison to fold into a branch, got:\n{asm}"
    );
    assert!(
        !asm.contains("CMP #$00"),
        "the re-test of the boolean should be gone:\n{asm}"
    );
}
