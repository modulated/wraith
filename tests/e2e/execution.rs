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
// u16 for-loop counters (16-bit compare + increment)
// ---------------------------------------------------------------------------

/// Count iterations of `for i: u16 in start..end` (or `..=`) into a u16.
fn u16_for_iterations(start: u16, end: u16, inclusive: bool) -> u16 {
    let op = if inclusive { "..=" } else { ".." };
    let src = format!(
        r#"
        const LO: addr = 0x0400;
        const HI: addr = 0x0401;
        #[reset]
        fn main() {{
            let n: u16 = 0;
            let one: u16 = 1;
            for i: u16 in {start}{op}{end} {{
                n = n + one;
            }}
            LO = n.low;
            HI = n.high;
            loop {{}}
        }}
    "#
    );
    run(&src).mem16(0x0400)
}

#[test]
fn u16_for_loop_delay_range() {
    // The historical bug: `for i: u16 in 0..30000` compiled to 8-bit loop
    // machinery that compared only the low byte of the end ($7530 -> $30),
    // running 48 iterations instead of 30000.
    assert_eq!(u16_for_iterations(0, 30000, false), 30000);
}

#[test]
fn u16_for_loop_end_low_byte_small() {
    // End 0x0130: low byte 0x30 = 48 < 304, the truncated compare exited early.
    assert_eq!(u16_for_iterations(0, 0x0130, false), 304);
}

#[test]
fn u16_for_loop_end_low_byte_zero() {
    // End 0x0100: low byte 0 made the old code run 0 iterations.
    assert_eq!(u16_for_iterations(0, 0x0100, false), 256);
}

#[test]
fn u16_for_loop_inclusive_range() {
    assert_eq!(u16_for_iterations(0, 0x0100, true), 257);
}

#[test]
fn u16_for_loop_nonzero_start() {
    assert_eq!(u16_for_iterations(0x00F0, 0x0110, false), 32);
}

#[test]
fn u16_for_loop_var_carries_into_high_byte() {
    // The counter itself must propagate its carry: record the last value of i.
    let mut e = run(r#"
        const LO: addr = 0x0400;
        const HI: addr = 0x0401;
        #[reset]
        fn main() {
            let last: u16 = 0;
            for i: u16 in 0..0x0110 {
                last = i;
            }
            LO = last.low;
            HI = last.high;
            loop {}
        }
    "#);
    assert_eq!(e.mem16(0x0400), 0x010F, "last i should be end - 1");
}

#[test]
fn u16_for_loop_runtime_end() {
    // A non-constant range end takes the temp-pair path instead of immediates.
    let mut e = run(r#"
        const LO: addr = 0x0400;
        const HI: addr = 0x0401;
        fn limit() -> u16 {
            return 0x0120;
        }
        #[reset]
        fn main() {
            let n: u16 = 0;
            let one: u16 = 1;
            for i: u16 in 0..limit() {
                n = n + one;
            }
            LO = n.low;
            HI = n.high;
            loop {}
        }
    "#);
    assert_eq!(
        e.mem16(0x0400),
        0x0120,
        "runtime end must compare both bytes"
    );
}

#[test]
fn u16_for_loop_break_and_continue() {
    // `continue` must jump to the increment (not the head) and `break` must
    // exit; both must work above 255.
    let mut e = run(r#"
        const LO: addr = 0x0400;
        const HI: addr = 0x0401;
        #[reset]
        fn main() {
            let n: u16 = 0;
            let one: u16 = 1;
            for i: u16 in 0..0x0200 {
                if i == 0x0105 {
                    break;
                }
                if i < 0x0100 {
                    continue;
                }
                n = n + one;
            }
            LO = n.low;
            HI = n.high;
            loop {}
        }
    "#);
    // Only i in 0x0100..0x0105 increments n.
    assert_eq!(
        e.mem16(0x0400),
        5,
        "break/continue must respect 16-bit counter"
    );
}

#[test]
fn u16_for_loop_unrolled_sets_high_byte() {
    // Small constant ranges unroll; a 16-bit counter above 255 must have its
    // high byte written each iteration (it was left uninitialized before).
    let mut e = run(r#"
        const LO: addr = 0x0400;
        const HI: addr = 0x0401;
        #[reset]
        fn main() {
            let last: u16 = 0;
            for i: u16 in 0x0205..0x0208 {
                last = i;
            }
            LO = last.low;
            HI = last.high;
            loop {}
        }
    "#);
    assert_eq!(
        e.mem16(0x0400),
        0x0207,
        "unrolled u16 loop must set high byte"
    );
}

// ---------------------------------------------------------------------------
// Loop bounds must survive the loop body (hidden frame slot, not scratch)
// ---------------------------------------------------------------------------

#[test]
fn u16_for_loop_nested_runtime_bounds() {
    // Both loops have non-constant ends; with a shared scratch pair the inner
    // loop's bound would overwrite the outer loop's live bound.
    let mut e = run(r#"
        const LO: addr = 0x0400;
        const HI: addr = 0x0401;
        fn outer_lim() -> u16 {
            return 3;
        }
        fn inner_lim() -> u16 {
            return 0x0102;
        }
        #[reset]
        fn main() {
            let n: u16 = 0;
            let one: u16 = 1;
            for i: u16 in 0..outer_lim() {
                for j: u16 in 0..inner_lim() {
                    n = n + one;
                }
            }
            LO = n.low;
            HI = n.high;
            loop {}
        }
    "#);
    // 3 * 258 iterations.
    assert_eq!(
        e.mem16(0x0400),
        774,
        "nested loops must not share bound storage"
    );
}

#[test]
fn u16_for_loop_bound_survives_shift_in_body() {
    // A u16 shift with a runtime count parks its high byte in the $22 scratch
    // byte; the loop's bound must live elsewhere.
    let mut e = run(r#"
        const LO: addr = 0x0400;
        const HI: addr = 0x0401;
        fn lim() -> u16 {
            return 0x0110;
        }
        fn one_bit() -> u8 {
            return 1;
        }
        #[reset]
        fn main() {
            let n: u16 = 0;
            let one: u16 = 1;
            for i: u16 in 0..lim() {
                let x: u16 = one << one_bit();
                n = n + x;
            }
            LO = n.low;
            HI = n.high;
            loop {}
        }
    "#);
    // 272 iterations, each adding 2.
    assert_eq!(
        e.mem16(0x0400),
        544,
        "u16 shift in body must not corrupt bound"
    );
}

#[test]
fn u16_for_loop_bound_survives_call_with_loop() {
    // The body calls a function that runs its own runtime-bounded loop. Frame
    // coloring keeps the callee's bound slot disjoint from the caller's.
    let mut e = run(r#"
        const LO: addr = 0x0400;
        const HI: addr = 0x0401;
        fn inner_lim() -> u16 {
            return 0x0101;
        }
        fn outer_lim() -> u16 {
            return 3;
        }
        fn count_inner() -> u16 {
            let m: u16 = 0;
            let one: u16 = 1;
            for j: u16 in 0..inner_lim() {
                m = m + one;
            }
            return m;
        }
        #[reset]
        fn main() {
            let n: u16 = 0;
            for i: u16 in 0..outer_lim() {
                n = n + count_inner();
            }
            LO = n.low;
            HI = n.high;
            loop {}
        }
    "#);
    // 3 * 257 iterations counted.
    assert_eq!(
        e.mem16(0x0400),
        771,
        "callee loop must not corrupt caller bound"
    );
}

#[test]
fn u8_for_loop_nested_runtime_bounds() {
    // The 8-bit path had the same shared-scratch defect.
    let mut e = run(r#"
        const OUT: addr = 0x0400;
        fn outer_lim() -> u8 {
            return 3;
        }
        fn inner_lim() -> u8 {
            return 7;
        }
        #[reset]
        fn main() {
            let n: u8 = 0;
            for i in 0..outer_lim() {
                for j in 0..inner_lim() {
                    n = n + 1;
                }
            }
            OUT = n;
            loop {}
        }
    "#);
    assert_eq!(
        e.mem(0x0400),
        21,
        "nested u8 loops must not share bound storage"
    );
}

// ---------------------------------------------------------------------------
// Inclusive loops must not wrap the counter at the type maximum
// ---------------------------------------------------------------------------

#[test]
fn u16_for_loop_inclusive_to_max() {
    // `..=0xFFFF`: the endpoint passes the head check, and an unconditional
    // increment would wrap the counter to zero and loop forever. Termination
    // itself is the regression check (the harness panics on budget exhaustion).
    let mut e = run(r#"
        const LO: addr = 0x0400;
        const HI: addr = 0x0401;
        #[reset]
        fn main() {
            let n: u16 = 0;
            let one: u16 = 1;
            for i: u16 in 0xFFF0..=0xFFFF {
                n = n + one;
            }
            LO = n.low;
            HI = n.high;
            loop {}
        }
    "#);
    assert_eq!(
        e.mem16(0x0400),
        16,
        "0xFFF0..=0xFFFF must run exactly 16 times"
    );
}

#[test]
fn u8_for_loop_inclusive_to_max() {
    // Same wrap hazard for the 8-bit path at `..=0xFF`.
    let mut e = run(r#"
        const OUT: addr = 0x0400;
        #[reset]
        fn main() {
            let n: u8 = 0;
            for i in 0xF0..=0xFF {
                n = n + 1;
            }
            OUT = n;
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0400), 16, "0xF0..=0xFF must run exactly 16 times");
}

#[test]
fn u8_for_loop_continue_reaches_increment() {
    // `continue` must jump to the increment, not the head; jumping to the head
    // both skipped the increment (infinite loop) and compared a clobbered X.
    let mut e = run(r#"
        const OUT: addr = 0x0400;
        #[reset]
        fn main() {
            let n: u8 = 0;
            for i in 0..10 {
                if i < 5 {
                    continue;
                }
                n = n + 1;
            }
            OUT = n;
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0400), 5, "continue must not skip the increment");
}
