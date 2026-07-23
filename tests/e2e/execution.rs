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

// ---------------------------------------------------------------------------
// Shift width correctness (`<<` and `>>` on u16 must move all 16 bits)
// ---------------------------------------------------------------------------

/// Compute `(a << n)` as u16 and return the full 16-bit result.
fn u16_shl(a: u16, n: u8) -> u16 {
    let src = format!(
        r#"
        const LO: addr = 0x0400;
        const HI: addr = 0x0401;
        #[reset]
        fn main() {{
            let x: u16 = {a};
            let s: u8 = {n};
            let r: u16 = x << s;
            LO = r.low;
            HI = r.high;
            loop {{}}
        }}
    "#
    );
    run(&src).mem16(0x0400)
}

/// Compute `(a >> n)` as u16 and return the full 16-bit result.
fn u16_shr(a: u16, n: u8) -> u16 {
    let src = format!(
        r#"
        const LO: addr = 0x0400;
        const HI: addr = 0x0401;
        #[reset]
        fn main() {{
            let x: u16 = {a};
            let s: u8 = {n};
            let r: u16 = x >> s;
            LO = r.low;
            HI = r.high;
            loop {{}}
        }}
    "#
    );
    run(&src).mem16(0x0400)
}

#[test]
fn u16_shl_crosses_byte_boundary() {
    // 1 << 8 = 256: the set bit must move from the low byte into the high byte.
    assert_eq!(u16_shl(1, 8), 256, "1 << 8 should be 256");
    // 3 << 7 = 384 = 0x0180: bits straddle the byte boundary.
    assert_eq!(u16_shl(3, 7), 384, "3 << 7 should be 384");
    // 0x00FF << 1 = 0x01FE: carry out of the low byte into the high byte.
    assert_eq!(u16_shl(0x00FF, 1), 0x01FE, "255 << 1 should be 510");
}

#[test]
fn u16_shl_small_counts() {
    assert_eq!(u16_shl(0x0102, 1), 0x0204, "shift keeps high byte");
    assert_eq!(u16_shl(5, 2), 20, "5 << 2 should be 20");
}

#[test]
fn u16_shr_crosses_byte_boundary() {
    // 0x0180 >> 7 = 3: bits must come down from the high byte.
    assert_eq!(u16_shr(0x0180, 7), 3, "384 >> 7 should be 3");
    // 0x0100 >> 1 = 0x0080: carry out of the high byte into the low byte.
    assert_eq!(u16_shr(0x0100, 1), 0x0080, "256 >> 1 should be 128");
    // 0xFF00 >> 8 = 0x00FF (existing special case).
    assert_eq!(u16_shr(0xFF00, 8), 0x00FF, "0xFF00 >> 8 should be 0x00FF");
}

#[test]
fn u16_shr_preserves_high_byte() {
    // 0x0402 >> 1 = 0x0201: the high byte must shift, not be dropped.
    assert_eq!(u16_shr(0x0402, 1), 0x0201, "0x0402 >> 1 should be 0x0201");
}

// ---------------------------------------------------------------------------
// u16 array element reads (index must scale ×2 and load both bytes)
// ---------------------------------------------------------------------------

/// Read `data[i]` from a u16 array with a runtime index and return the full u16.
fn u16_array_read(index: u8) -> u16 {
    let src = format!(
        r#"
        const LO: addr = 0x0400;
        const HI: addr = 0x0401;
        #[reset]
        fn main() {{
            let data: [u16; 4] = [0x1122, 0x3344, 0x5566, 0x7788];
            let i: u8 = {index};
            let v: u16 = data[i];
            LO = v.low;
            HI = v.high;
            loop {{}}
        }}
    "#
    );
    run(&src).mem16(0x0400)
}

#[test]
fn u16_array_read_scales_index() {
    // Element 0 is the only one a low-byte-only read could get right.
    assert_eq!(u16_array_read(0), 0x1122, "data[0] should be 0x1122");
    // These require scaling the index and loading the high byte.
    assert_eq!(u16_array_read(1), 0x3344, "data[1] should be 0x3344");
    assert_eq!(u16_array_read(2), 0x5566, "data[2] should be 0x5566");
    assert_eq!(u16_array_read(3), 0x7788, "data[3] should be 0x7788");
}

#[test]
fn u16_array_roundtrip() {
    // Write through the (already-correct) store path, then read back.
    let mut e = run(r#"
        const LO: addr = 0x0400;
        const HI: addr = 0x0401;
        #[reset]
        fn main() {
            let data: [u16; 3] = [0x1000, 0x2000, 0x3000];
            let i: u8 = 1;
            let w: u16 = 0xBEEF;
            data[i] = w;
            let v: u16 = data[i];
            LO = v.low;
            HI = v.high;
            loop {}
        }
    "#);
    assert_eq!(
        e.mem16(0x0400),
        0xBEEF,
        "written u16 should read back intact"
    );
}

// ---------------------------------------------------------------------------
// match / pattern binding correctness
// ---------------------------------------------------------------------------

/// `match x { 5 => 50, n => n }` for a u8 scrutinee.
fn u8_match_var_binding(x: u8) -> u8 {
    let src = format!(
        r#"
        const OUT: addr = 0x0400;
        #[reset]
        fn main() {{
            let x: u8 = {x};
            match x {{
                5 => {{ OUT = 50; }}
                n => {{ OUT = n; }}
            }}
            loop {{}}
        }}
    "#
    );
    run(&src).mem(0x0400)
}

#[test]
fn match_variable_pattern_binds_scrutinee() {
    // The variable arm must observe the actual matched value, not garbage.
    assert_eq!(u8_match_var_binding(7), 7, "n should bind to 7");
    assert_eq!(u8_match_var_binding(200), 200, "n should bind to 200");
    // The literal arm still wins when it matches.
    assert_eq!(u8_match_var_binding(5), 50, "5 => 50");
}

#[test]
fn match_u16_literal_compares_full_value() {
    // 256 and 0 share a low byte; an 8-bit-only compare would confuse them.
    let src = |v: u16| {
        format!(
            r#"
            const OUT: addr = 0x0400;
            #[reset]
            fn main() {{
                let x: u16 = {v};
                match x {{
                    256 => {{ OUT = 1; }}
                    n => {{ OUT = 2; }}
                }}
                loop {{}}
            }}
        "#
        )
    };
    assert_eq!(
        run(&src(256)).mem(0x0400),
        1,
        "256 should match the 256 arm"
    );
    assert_eq!(
        run(&src(0)).mem(0x0400),
        2,
        "0 must NOT match the 256 arm (differs in high byte)"
    );
}

#[test]
fn match_u16_variable_binding_keeps_high_byte() {
    let src = r#"
        const LO: addr = 0x0400;
        const HI: addr = 0x0401;
        #[reset]
        fn main() {
            let x: u16 = 0x0305;
            match x {
                256 => { LO = 0; HI = 0; }
                n => { LO = n.low; HI = n.high; }
            }
            loop {}
        }
    "#;
    assert_eq!(
        run(src).mem16(0x0400),
        0x0305,
        "u16 binding must keep both bytes"
    );
}

#[test]
fn match_tuple_variant_u16_field_not_truncated() {
    // Extracting a u16 tuple field must copy both bytes, not just the low byte.
    let mut e = run(r#"
        enum Result {
            Ok(u16),
            Err(u8),
        }
        const LO: addr = 0x0400;
        const HI: addr = 0x0401;
        #[reset]
        fn main() {
            let res: Result = Result::Ok(1000);
            match res {
                Result::Ok(value) => { LO = value.low; HI = value.high; }
                Result::Err(code) => { LO = code; HI = 0; }
            }
            loop {}
        }
    "#);
    assert_eq!(
        e.mem16(0x0400),
        1000,
        "u16 tuple field should extract as 1000"
    );
}

// ---------------------------------------------------------------------------
// Strength reduction must not double-evaluate the left operand
// ---------------------------------------------------------------------------

#[test]
fn mul_pow2_evaluates_left_once() {
    // `tick() * 2` is strength-reduced to a shift. The left operand has a side
    // effect (increments CTR), so it must run exactly once.
    let mut e = run(r#"
        const CTR: addr = 0x0400;
        const OUT: addr = 0x0401;
        fn tick() -> u8 {
            CTR = CTR + 1;
            return 3;
        }
        #[reset]
        fn main() {
            CTR = 0;
            OUT = tick() * 2;
            loop {}
        }
    "#);
    assert_eq!(
        e.mem(0x0400),
        1,
        "left operand must be evaluated exactly once"
    );
    assert_eq!(e.mem(0x0401), 6, "3 * 2 should be 6");
}

// ---------------------------------------------------------------------------
// Register state must be invalidated at loop back-edge targets
// ---------------------------------------------------------------------------

#[test]
fn loop_carried_var_reloaded_at_head() {
    // The head of a `loop` is a back-edge target: register contents cached
    // before the loop are stale on iteration 2+. Reading a loop-carried
    // variable at the loop head must reload it from memory rather than reuse
    // whatever the previous iteration's body left in A. Here `OUT = x` is the
    // first body statement, and the trailing `if x == N` comparison clobbers A
    // before jumping back, so without a reload the store writes garbage.
    let mut e = run(r#"
        const OUT: addr = 0x0400;
        #[reset]
        fn main() {
            let x: u8 = 1;
            loop {
                OUT = x;
                x = x + 1;
                if x == 5 {
                    break;
                }
            }
            loop {}
        }
    "#);
    // Iterations write x = 1, 2, 3, 4 to OUT; the last committed value is 4.
    assert_eq!(e.mem(0x0400), 4, "loop head must reload x each iteration");
}

#[test]
fn while_carried_var_survives_iterations() {
    // The `while` condition check is the sibling back-edge target of the `loop`
    // head and gets the same invalidation. Current codegen already reloads the
    // condition's operands from memory, so this exercises correctness rather
    // than reproducing the bare-STA bug directly; it guards the while path
    // against future register-state regressions.
    let mut e = run(r#"
        const OUT: addr = 0x0400;
        #[reset]
        fn main() {
            let x: u8 = 1;
            while x < 5 {
                OUT = x;
                x = x + 1;
            }
            loop {}
        }
    "#);
    // Iterations write x = 1, 2, 3, 4 to OUT; the last committed value is 4.
    assert_eq!(e.mem(0x0400), 4, "while head must reload x each iteration");
}
