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
