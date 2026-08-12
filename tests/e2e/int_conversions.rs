//! Integer conversions: how a cast extends, and what type a pair of bare
//! literals agrees on.
//!
//! Both were found by `tests/fuzz_exec.rs` once it started generating `i8` and
//! `i16` programs. Neither had a test, and neither is visible in the assembly
//! without knowing what to look for — the wrong extension is one `LDY #$00`
//! where an `LDA #$FF` belonged.

use crate::common::exec::run;

/// Run `body` with the two output bytes at `$0900`/`$0901` and read them as a
/// little-endian 16-bit value.
fn out16(body: &str) -> u32 {
    let mut e = run(&format!(
        "const OUT: addr = 0x0900;\nconst OUT1: addr = 0x0901;\n\
         #[reset]\nfn main() {{\n{body}\n    loop {{}}\n}}\n"
    ));
    e.mem16(0x0900) as u32
}

/// Widen `value` of type `from` to `to`, and report the 16 bits that land in
/// memory.
fn widen(from: &str, to: &str, value: &str) -> u32 {
    out16(&format!(
        "    let x: {from} = {value};\n    let y: {to} = x as {to};\n\
         \x20   let o: u16 = y as u16;\n    OUT = o.low;\n    OUT1 = o.high;"
    ))
}

// ---------------------------------------------------------------------------
// Which extension a widening cast performs
// ---------------------------------------------------------------------------
//
// It is a property of the *source*: a signed source carries its sign into the
// high byte, an unsigned one carries zero. Reading it off the destination
// instead — what the compiler did — gets both mixed cases backwards.

#[test]
fn an_unsigned_source_zero_extends_even_into_a_signed_destination() {
    assert_eq!(widen("u8", "i16", "200"), 200, "200 as i16 is 200, not -56");
    assert_eq!(widen("u8", "i16", "255"), 255);
    assert_eq!(
        widen("u8", "i16", "128"),
        128,
        "the high bit is a value bit"
    );
}

#[test]
fn a_signed_source_sign_extends_even_into_an_unsigned_destination() {
    assert_eq!(
        widen("i8", "u16", "-1"),
        65535,
        "-1 as u16 keeps the value, reinterpreted"
    );
    assert_eq!(widen("i8", "u16", "-56"), 65480);
    assert_eq!(widen("i8", "u16", "-128"), 65408);
}

#[test]
fn matched_signedness_is_unchanged() {
    assert_eq!(widen("u8", "u16", "255"), 255);
    assert_eq!(widen("i8", "i16", "-1"), 65535, "0xFFFF is -1");
    assert_eq!(widen("i8", "i16", "127"), 127);
}

#[test]
fn a_positive_signed_source_extends_with_zeroes() {
    assert_eq!(widen("i8", "u16", "127"), 127);
    assert_eq!(widen("i8", "i16", "0"), 0);
}

/// Narrowing is unaffected: it keeps the low byte whatever the signs are.
#[test]
fn narrowing_keeps_the_low_byte() {
    assert_eq!(widen("u16", "u8", "0x1234"), 0x34);
    assert_eq!(widen("i16", "u8", "-300"), 0xD4, "-300 is 0xFED4");
    assert_eq!(widen("u16", "i8", "200"), 65480, "200 as i8 is -56");
}

/// A round trip through the other signedness at the same width is a pure
/// reinterpretation, so it must come back unchanged.
#[test]
fn a_same_width_round_trip_preserves_the_bits() {
    for v in ["-128", "-1", "0", "127"] {
        assert_eq!(
            out16(&format!(
                "    let x: i8 = {v};\n    let y: i8 = (x as u8) as i8;\n\
                 \x20   let o: u16 = y as u16;\n    OUT = o.low;\n    OUT1 = o.high;"
            )),
            widen("i8", "i16", v),
            "i8 {v} through u8 and back"
        );
    }
}

// ---------------------------------------------------------------------------
// What type two bare literals agree on
// ---------------------------------------------------------------------------
//
// Each literal used to fall back to its own default — `-5` to `i8`, `3` to
// `u8` — and the operator then rejected the pair, even though `i8` holds both.
// A declared type covered it up; a condition, which has no ambient type, did
// not.

#[test]
fn a_negative_and_a_positive_literal_share_a_type_in_a_condition() {
    let src = "    let a: i8 = 1;\n    OUT = 0;\n    OUT1 = 0;\n\
               \x20   if (-5 - 3) < a { OUT = 1; }\n\
               \x20   if a > (-5 - 3) { OUT1 = 1; }";
    assert_eq!(out16(src), 0x0101, "-8 < 1 both ways round");
}

#[test]
fn the_shared_type_is_wide_enough_for_both_literals() {
    let src = "    let a: u16 = 1;\n    OUT = 0;\n    OUT1 = 0;\n\
               \x20   if (300 + 1) > a { OUT = 1; }\n\
               \x20   if (30000 + 2000) > a { OUT1 = 1; }";
    assert_eq!(out16(src), 0x0101);
}

/// A literal pair nested under an operator that only accepts a literal on the
/// right — a shift count — is the shape that first showed this up.
#[test]
fn a_literal_subexpression_adapts_to_its_sibling() {
    let src = "    let v: i8 = -1;\n    OUT = 0;\n\
               \x20   if (v & (37 >> 1)) == 18 { OUT = 1; }\n    OUT1 = 0;";
    assert_eq!(out16(src), 0x0001, "-1 & 18 is 18");
}

/// A declared type still wins when it holds both values, so an expression that
/// used to compute at 16 bits does not silently narrow to 8.
#[test]
fn a_declared_type_still_wins_over_the_literals_own_range() {
    let mut e = run("const OUT: addr = 0x0900;\nconst OUT1: addr = 0x0901;\n\
         #[reset]\nfn main() {\n    let x: i16 = 3 - 5;\n\
         \x20   let o: u16 = x as u16;\n    OUT = o.low;\n    OUT1 = o.high;\n\
         \x20   loop {}\n}\n");
    assert_eq!(e.mem16(0x0900), 0xFFFE, "-2 at 16 bits, not 254 at 8");
}
