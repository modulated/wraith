//! Range patterns in `match`, across every width and signedness.
//!
//! Range arms used to compare only the low byte of the scrutinee, whatever its
//! type. On a 16-bit value that is a silent wrong branch — `300` is `0x012C`,
//! whose low byte is 44, so it matched `0..=100` — and when a bound exceeded
//! 255 the compiler emitted `CMP #$0100`, which is not an instruction and so
//! failed at assembly time rather than in the compiler.
//!
//! Literal arms were already width-correct, and `if n <= 100` on a `u16` was
//! always right, which is why this survived: every neighbouring construct
//! behaved, and only ranges were wrong.
//!
//! The cases below are chosen so that a low-byte-only comparison gives the
//! opposite answer to the correct one wherever possible.

use crate::common::exec::run;

/// Run `body` and return the byte it left at $0900.
fn arm_taken(body: &str) -> u8 {
    let src = format!("const OUT: addr = 0x0900;\n#[reset]\nfn main() {{ {body} loop {{}} }}\n");
    let mut e = run(&src);
    e.mem(0x0900)
}

// ---------------------------------------------------------------------------
// u8 — the path that always worked; kept so a 16-bit fix cannot break it
// ---------------------------------------------------------------------------

#[test]
fn u8_range_arms() {
    assert_eq!(
        arm_taken("let n: u8 = 50; match n { 0..=100 => { OUT=1; } _ => { OUT=2; } }"),
        1
    );
    assert_eq!(
        arm_taken("let n: u8 = 200; match n { 0..=100 => { OUT=1; } _ => { OUT=2; } }"),
        2
    );
    assert_eq!(
        arm_taken("let n: u8 = 0; match n { 0..=0 => { OUT=1; } _ => { OUT=2; } }"),
        1
    );
    assert_eq!(
        arm_taken("let n: u8 = 5; match n { 10..=255 => { OUT=1; } _ => { OUT=2; } }"),
        2
    );
}

/// `0..=255` has no representable `end + 1`. The bound used to be formatted as
/// `CMP #$100`; everything that clears the lower bound is in range, so the arm
/// is taken outright.
#[test]
fn u8_range_to_the_type_maximum() {
    assert_eq!(
        arm_taken("let n: u8 = 200; match n { 0..=255 => { OUT=1; } _ => { OUT=2; } }"),
        1
    );
    assert_eq!(
        arm_taken("let n: u8 = 255; match n { 200..=255 => { OUT=1; } _ => { OUT=2; } }"),
        1
    );
    assert_eq!(
        arm_taken("let n: u8 = 199; match n { 200..=255 => { OUT=1; } _ => { OUT=2; } }"),
        2
    );
}

// ---------------------------------------------------------------------------
// u16 — the silent miscompile
// ---------------------------------------------------------------------------

/// Each of these has a low byte that lands inside the range while the full
/// value lies outside it, so a low-byte comparison takes the wrong arm.
#[test]
fn u16_range_arms_compare_the_whole_value() {
    // 300 = 0x012C, low byte 44 — inside 0..=100.
    assert_eq!(
        arm_taken("let n: u16 = 300; match n { 0..=100 => { OUT=1; } _ => { OUT=2; } }"),
        2
    );
    // 256 = 0x0100, low byte 0 — inside 0..=10.
    assert_eq!(
        arm_taken("let n: u16 = 256; match n { 0..=10 => { OUT=1; } _ => { OUT=2; } }"),
        2
    );
    // 513 = 0x0201, low byte 1 — inside 1..=5.
    assert_eq!(
        arm_taken("let n: u16 = 513; match n { 1..=5 => { OUT=1; } _ => { OUT=2; } }"),
        2
    );
}

/// Bounds above 255 are the case that produced un-assemblable output.
#[test]
fn u16_range_bounds_above_a_byte() {
    assert_eq!(
        arm_taken("let n: u16 = 300; match n { 256..=400 => { OUT=1; } _ => { OUT=2; } }"),
        1
    );
    assert_eq!(
        arm_taken("let n: u16 = 255; match n { 256..=400 => { OUT=1; } _ => { OUT=2; } }"),
        2
    );
    assert_eq!(
        arm_taken("let n: u16 = 401; match n { 256..=400 => { OUT=1; } _ => { OUT=2; } }"),
        2
    );
    assert_eq!(
        arm_taken("let n: u16 = 1000; match n { 999..=1001 => { OUT=1; } _ => { OUT=2; } }"),
        1
    );
}

#[test]
fn u16_range_to_the_type_maximum() {
    assert_eq!(
        arm_taken("let n: u16 = 65535; match n { 0..=65535 => { OUT=1; } _ => { OUT=2; } }"),
        1
    );
    assert_eq!(
        arm_taken("let n: u16 = 0; match n { 0..=65535 => { OUT=1; } _ => { OUT=2; } }"),
        1
    );
}

/// A `u16` scrutinee still has to match on the correct side of the boundary.
#[test]
fn u16_range_boundaries_are_inclusive() {
    assert_eq!(
        arm_taken("let n: u16 = 300; match n { 300..=400 => { OUT=1; } _ => { OUT=2; } }"),
        1
    );
    assert_eq!(
        arm_taken("let n: u16 = 400; match n { 300..=400 => { OUT=1; } _ => { OUT=2; } }"),
        1
    );
    assert_eq!(
        arm_taken("let n: u16 = 299; match n { 300..=400 => { OUT=1; } _ => { OUT=2; } }"),
        2
    );
}

// ---------------------------------------------------------------------------
// signed — the sign of the whole value, not of the low byte
// ---------------------------------------------------------------------------

#[test]
fn i8_range_arms() {
    assert_eq!(
        arm_taken("let n: i8 = -5; match n { 0..=100 => { OUT=1; } _ => { OUT=2; } }"),
        2
    );
    assert_eq!(
        arm_taken("let n: i8 = -50; match n { -100..=-1 => { OUT=1; } _ => { OUT=2; } }"),
        1
    );
    assert_eq!(
        arm_taken("let n: i8 = 5; match n { -100..=-1 => { OUT=1; } _ => { OUT=2; } }"),
        2
    );
}

/// The lower-bound test against `0` is a bare sign-bit check (`BMI`), which
/// reads the N flag left by the load of the scrutinee. When the scrutinee is a
/// computed value the compiler stores it (`STA $20`) and reloads it (`LDA $20`)
/// to set N — but a peephole once dropped the reload as redundant, so `BMI`
/// read a stale flag from whatever produced the value (here the shift routine's
/// `CPX #$00`, which left N clear). A negative scrutinee then passed the "≥ 0"
/// test and took the arm. The scrutinee must be *computed*, not a bare variable,
/// to force the store/reload the bug depended on.
#[test]
fn a_computed_negative_scrutinee_fails_the_low_bound() {
    // `arr[0] >> 0` is -51: negative, so outside `0..105`, so the default runs.
    assert_eq!(
        arm_taken(
            "let arr: [i8; 2] = [-51, 7]; \
             match ((arr[0] >> 0) as i8) { 0..105 => { OUT = 1; } _ => { OUT = 2; } }"
        ),
        2,
        "a negative computed scrutinee is below the range and must miss the arm"
    );
    // The same shape with a value that *is* in range still matches.
    assert_eq!(
        arm_taken(
            "let arr: [i8; 2] = [40, 7]; \
             match ((arr[0] >> 0) as i8) { 0..105 => { OUT = 1; } _ => { OUT = 2; } }"
        ),
        1,
        "an in-range computed scrutinee must still take the arm"
    );
}

/// `0..=127` on an `i8` asks "is it < 128", which is not representable — the
/// truncated bound reads as -128 and the arm matched nothing.
#[test]
fn i8_range_to_the_signed_maximum() {
    assert_eq!(
        arm_taken("let n: i8 = 50; match n { 0..=127 => { OUT=1; } _ => { OUT=2; } }"),
        1
    );
    assert_eq!(
        arm_taken("let n: i8 = 127; match n { 0..=127 => { OUT=1; } _ => { OUT=2; } }"),
        1
    );
    assert_eq!(
        arm_taken("let n: i8 = -1; match n { 0..=127 => { OUT=1; } _ => { OUT=2; } }"),
        2
    );
}

#[test]
fn i16_range_arms_compare_the_whole_value() {
    // 300's low byte is 44, inside 0..=100; the full value is not.
    assert_eq!(
        arm_taken("let n: i16 = 300; match n { 0..=100 => { OUT=1; } _ => { OUT=2; } }"),
        2
    );
    assert_eq!(
        arm_taken("let n: i16 = 50; match n { 0..=100 => { OUT=1; } _ => { OUT=2; } }"),
        1
    );
    assert_eq!(
        arm_taken("let n: i16 = -300; match n { 0..=100 => { OUT=1; } _ => { OUT=2; } }"),
        2
    );
    assert_eq!(
        arm_taken("let n: i16 = -300; match n { -400..=-200 => { OUT=1; } _ => { OUT=2; } }"),
        1
    );
}

#[test]
fn i16_range_to_the_signed_maximum() {
    assert_eq!(
        arm_taken("let n: i16 = 1000; match n { 0..=32767 => { OUT=1; } _ => { OUT=2; } }"),
        1
    );
    assert_eq!(
        arm_taken("let n: i16 = -1; match n { 0..=32767 => { OUT=1; } _ => { OUT=2; } }"),
        2
    );
}

// ---------------------------------------------------------------------------
// The neighbours that were already correct, pinned so a range fix cannot
// regress them.
// ---------------------------------------------------------------------------

#[test]
fn u16_literal_arms_stay_width_correct() {
    assert_eq!(
        arm_taken("let n: u16 = 300; match n { 300 => { OUT=1; } _ => { OUT=2; } }"),
        1
    );
    // 44 is 300's low byte: a literal arm must not match on it alone.
    assert_eq!(
        arm_taken("let n: u16 = 300; match n { 44 => { OUT=1; } _ => { OUT=2; } }"),
        2
    );
}

#[test]
fn u16_relational_operators_stay_width_correct() {
    assert_eq!(
        arm_taken("let n: u16 = 300; if n <= 100 { OUT = 1; } else { OUT = 2; }"),
        2
    );
    assert_eq!(
        arm_taken("let n: u16 = 300; if n >= 256 { OUT = 1; } else { OUT = 2; }"),
        1
    );
}

// ---------------------------------------------------------------------------
// Exclusive ranges
// ---------------------------------------------------------------------------
//
// `0..300` used to be a parse error ("expected FatArrow, found '..'"), so a
// pattern accepted only `..=` while `for i in 0..n` accepted both. Codegen
// already handled the exclusive form — it computes the same upper bound either
// way — so what was missing was the parse and the bounds check.

#[test]
fn an_exclusive_range_excludes_its_end() {
    assert_eq!(
        arm_taken("let n: u8 = 99; match n { 0..100 => { OUT=1; } _ => { OUT=2; } }"),
        1
    );
    assert_eq!(
        arm_taken("let n: u8 = 100; match n { 0..100 => { OUT=1; } _ => { OUT=2; } }"),
        2,
        "100 is one past the last value `0..100` covers"
    );
}

#[test]
fn an_exclusive_range_includes_its_start() {
    assert_eq!(
        arm_taken("let n: u8 = 10; match n { 10..20 => { OUT=1; } _ => { OUT=2; } }"),
        1
    );
    assert_eq!(
        arm_taken("let n: u8 = 9; match n { 10..20 => { OUT=1; } _ => { OUT=2; } }"),
        2
    );
}

/// The two spellings of one range must select the same arm at every boundary.
#[test]
fn the_two_spellings_agree() {
    for n in [0u8, 9, 10, 19, 20, 21, 255] {
        let exclusive = arm_taken(&format!(
            "let n: u8 = {n}; match n {{ 10..20 => {{ OUT=1; }} _ => {{ OUT=2; }} }}"
        ));
        let inclusive = arm_taken(&format!(
            "let n: u8 = {n}; match n {{ 10..=19 => {{ OUT=1; }} _ => {{ OUT=2; }} }}"
        ));
        assert_eq!(exclusive, inclusive, "n = {n}");
    }
}

/// The 16-bit case, where a low-byte-only comparison would give the opposite
/// answer: 300 is `0x012C`, low byte 44.
#[test]
fn u16_exclusive_ranges_compare_both_bytes() {
    assert_eq!(
        arm_taken("let n: u16 = 299; match n { 0..300 => { OUT=1; } _ => { OUT=2; } }"),
        1
    );
    assert_eq!(
        arm_taken("let n: u16 = 300; match n { 0..300 => { OUT=1; } _ => { OUT=2; } }"),
        2
    );
    assert_eq!(
        arm_taken("let n: u16 = 300; match n { 0..100 => { OUT=1; } _ => { OUT=2; } }"),
        2,
        "a low-byte comparison would see 44 and take the arm"
    );
}

#[test]
fn signed_exclusive_ranges() {
    assert_eq!(
        arm_taken("let n: i8 = -1; match n { -10..0 => { OUT=1; } _ => { OUT=2; } }"),
        1
    );
    assert_eq!(
        arm_taken("let n: i8 = 0; match n { -10..0 => { OUT=1; } _ => { OUT=2; } }"),
        2
    );
    assert_eq!(
        arm_taken("let n: i16 = -300; match n { -1000..-299 => { OUT=1; } _ => { OUT=2; } }"),
        1
    );
    assert_eq!(
        arm_taken("let n: i16 = -299; match n { -1000..-299 => { OUT=1; } _ => { OUT=2; } }"),
        2
    );
}

/// An exclusive end is one past the last value, so it may name the number just
/// beyond the type. `0..256` covers a whole `u8` and must not be rejected as an
/// out-of-range bound.
#[test]
fn an_exclusive_end_may_sit_one_past_the_type_maximum() {
    assert_eq!(
        arm_taken("let n: u8 = 255; match n { 0..256 => { OUT=1; } _ => { OUT=2; } }"),
        1
    );
    assert_eq!(
        arm_taken("let n: i8 = 127; match n { 0..128 => { OUT=1; } _ => { OUT=2; } }"),
        1
    );
    assert_eq!(
        arm_taken("let n: u16 = 65535; match n { 0..65536 => { OUT=1; } _ => { OUT=2; } }"),
        1
    );
}

/// Two past the end is still out of range, so the check has not simply been
/// relaxed.
#[test]
fn a_bound_beyond_that_is_still_rejected() {
    for src in [
        "let n: u8 = 0; match n { 0..257 => { OUT=1; } _ => { OUT=2; } }",
        "let n: u8 = 0; match n { 0..=256 => { OUT=1; } _ => { OUT=2; } }",
        "let n: i8 = 0; match n { 0..129 => { OUT=1; } _ => { OUT=2; } }",
    ] {
        let full =
            format!("const OUT: addr = 0x0900;\n#[reset]\nfn main() {{ {src} loop {{}} }}\n");
        crate::common::assert_error_contains(&full, "cannot match a value of type");
    }
}

/// A range that covers nothing is an arm that can never run — easy to write by
/// accident now that `..` is accepted, since `5..5` looks like it covers 5.
#[test]
fn an_empty_range_is_rejected() {
    for src in [
        "let n: u8 = 0; match n { 5..5 => { OUT=1; } _ => { OUT=2; } }",
        "let n: u8 = 0; match n { 9..3 => { OUT=1; } _ => { OUT=2; } }",
        "let n: u8 = 0; match n { 9..=3 => { OUT=1; } _ => { OUT=2; } }",
    ] {
        let full =
            format!("const OUT: addr = 0x0900;\n#[reset]\nfn main() {{ {src} loop {{}} }}\n");
        crate::common::assert_error_contains(&full, "matches no value");
    }
}

/// A single-value inclusive range is not empty and must keep working.
#[test]
fn a_single_value_range_is_not_empty() {
    assert_eq!(
        arm_taken("let n: u8 = 5; match n { 5..=5 => { OUT=1; } _ => { OUT=2; } }"),
        1
    );
    assert_eq!(
        arm_taken("let n: u8 = 5; match n { 5..6 => { OUT=1; } _ => { OUT=2; } }"),
        1
    );
}
