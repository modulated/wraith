//! Runtime execution tests: compile a Wraith program, assemble it, run it on a
//! 6502 emulator, and assert on the resulting memory. Unlike the string-based
//! codegen tests, these catch *semantic* miscompilation (wrong runtime results).
//!
//! This first block validates the harness itself on known-correct operations;
//! later blocks exercise specific arithmetic/comparison behavior.

use crate::common::exec::{assemble, run};

// ---------------------------------------------------------------------------
// Assembler self-validation (canonical 6502 encodings)
// ---------------------------------------------------------------------------

#[test]
fn assembler_encodes_canonical_instructions() {
    let img = assemble(
        r#"
.ORG $8000
start:
    LDA #$01
    STA $40
    LDA $40
    JMP start
.ORG $FFFC
.WORD start
"#,
    );
    // LDA #$01 = A9 01 ; STA $40 = 85 40 ; LDA $40 = A5 40 ; JMP $8000 = 4C 00 80
    assert_eq!(
        &img[0x8000..0x8009],
        &[0xA9, 0x01, 0x85, 0x40, 0xA5, 0x40, 0x4C, 0x00, 0x80]
    );
    // reset vector little-endian = 00 80
    assert_eq!(&img[0xFFFC..0xFFFE], &[0x00, 0x80]);
}

#[test]
fn assembler_encodes_branch_offset() {
    // BEQ to a label 2 bytes ahead of the following instruction.
    let img = assemble(
        r#"
.ORG $8000
    BEQ skip
    NOP
skip:
    NOP
"#,
    );
    // BEQ = F0, offset = target(skip=$8003) - next($8002) = 1
    assert_eq!(&img[0x8000..0x8003], &[0xF0, 0x01, 0xEA]);
}

// ---------------------------------------------------------------------------
// End-to-end pipeline smoke tests (known-correct results)
// ---------------------------------------------------------------------------

#[test]
fn smoke_u8_addition() {
    let mut e = run(r#"
        const OUT: addr = 0x0400;
        #[reset]
        fn main() {
            let a: u8 = 3;
            let b: u8 = 5;
            OUT = a + b;
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0400), 8, "3 + 5 should be 8");
}

#[test]
fn smoke_u8_subtraction_is_not_reversed() {
    // Non-commutative: verifies operand order (a - b, not b - a).
    let mut e = run(r#"
        const OUT: addr = 0x0400;
        #[reset]
        fn main() {
            let a: u8 = 10;
            let b: u8 = 3;
            OUT = a - b;
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0400), 7, "10 - 3 should be 7");
}

#[test]
fn smoke_u16_addition_carry() {
    // 300 + 7 = 307 = 0x0133 (exercises 16-bit carry propagation).
    let mut e = run(r#"
        const LO: addr = 0x0400;
        const HI: addr = 0x0401;
        #[reset]
        fn main() {
            let a: u16 = 300;
            let b: u16 = 7;
            let s: u16 = a + b;
            LO = s.low;
            HI = s.high;
            loop {}
        }
    "#);
    assert_eq!(e.mem16(0x0400), 307, "300 + 7 should be 307");
}

// ---------------------------------------------------------------------------
// `<=` comparison correctness (u8 and u16)
// ---------------------------------------------------------------------------

/// Store `(a <= b) as u8` at OUT for two u8 values.
fn u8_le(a: u8, b: u8) -> u8 {
    let src = format!(
        r#"
        const OUT: addr = 0x0400;
        #[reset]
        fn main() {{
            let x: u8 = {a};
            let y: u8 = {b};
            let r: bool = x <= y;
            OUT = r as u8;
            loop {{}}
        }}
    "#
    );
    run(&src).mem(0x0400)
}

/// Store `(a <= b) as u8` at OUT for two u16 values.
fn u16_le(a: u16, b: u16) -> u8 {
    let src = format!(
        r#"
        const OUT: addr = 0x0400;
        #[reset]
        fn main() {{
            let x: u16 = {a};
            let y: u16 = {b};
            let r: bool = x <= y;
            OUT = r as u8;
            loop {{}}
        }}
    "#
    );
    run(&src).mem(0x0400)
}

#[test]
fn u8_le_equal_is_true() {
    assert_eq!(u8_le(0, 0), 1, "0 <= 0 should be true");
    assert_eq!(u8_le(3, 3), 1, "3 <= 3 should be true");
    assert_eq!(u8_le(200, 200), 1, "200 <= 200 should be true");
}

#[test]
fn u8_le_ordering() {
    assert_eq!(u8_le(2, 5), 1, "2 <= 5 should be true");
    assert_eq!(u8_le(5, 2), 0, "5 <= 2 should be false");
    assert_eq!(u8_le(0, 1), 1, "0 <= 1 should be true");
    assert_eq!(u8_le(1, 0), 0, "1 <= 0 should be false");
}

#[test]
fn u16_le_high_byte_ordering() {
    // left.high > right.high must be false (the historical bug returned garbage).
    assert_eq!(u16_le(0x0305, 0x0102), 0, "773 <= 258 should be false");
    assert_eq!(u16_le(0x0102, 0x0305), 1, "258 <= 773 should be true");
}

#[test]
fn u16_le_equal_and_low_byte() {
    assert_eq!(u16_le(0x0300, 0x0300), 1, "768 <= 768 should be true");
    assert_eq!(u16_le(0x0300, 0x0301), 1, "768 <= 769 should be true");
    assert_eq!(u16_le(0x0301, 0x0300), 0, "769 <= 768 should be false");
}
