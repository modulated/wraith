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
