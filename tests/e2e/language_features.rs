//! Runtime (emulator) tests for language features not otherwise exercised on
//! the emulator: casts (widen/narrow/sign-extend), compound assignment,
//! enum dispatch, array-fill init, nested calls, mutable globals, inclusive
//! ranges, string iteration, and division/modulo specifics.

use crate::common::exec::run;

// ---------------------------------------------------------------------------
// Casts.
// ---------------------------------------------------------------------------

#[test]
fn cast_widen_u8_to_u16() {
    let mut e = run(r#"
        const LO: addr = 0x0400;
        const HI: addr = 0x0401;
        #[reset]
        fn main() {
            let a: u8 = 200;
            let b: u16 = a as u16;
            LO = b.low;
            HI = b.high;
            loop {}
        }
    "#);
    assert_eq!(e.mem16(0x0400), 200, "u8 200 widened to u16 (high byte 0)");
}

#[test]
fn cast_narrow_u16_to_u8() {
    let mut e = run(r#"
        const OUT: addr = 0x0400;
        #[reset]
        fn main() {
            let a: u16 = 0x1234;
            let b: u8 = a as u8;
            OUT = b;
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0400), 0x34, "u16 0x1234 narrowed to u8 keeps low byte");
}

#[test]
fn cast_sign_extend_i8_to_i16() {
    let mut e = run(r#"
        const LO: addr = 0x0400;
        const HI: addr = 0x0401;
        #[reset]
        fn main() {
            let a: i8 = -5;
            let b: i16 = a as i16;
            LO = b.low;
            HI = b.high;
            loop {}
        }
    "#);
    // -5 sign-extends to 0xFFFB (high byte 0xFF, not 0x00).
    assert_eq!(e.mem16(0x0400), 0xFFFB, "i8 -5 sign-extended to i16");
}

// ---------------------------------------------------------------------------
// Compound assignment operators.
// ---------------------------------------------------------------------------

#[test]
fn compound_assignment_operators() {
    let mut e = run(r#"
        const OUT: addr = 0x0400;
        #[reset]
        fn main() {
            let x: u8 = 10;
            x += 5;   // 15
            x -= 3;   // 12
            x *= 2;   // 24
            x <<= 1;  // 48
            x |= 1;   // 49
            x &= 0xF7;// 49 & 0xF7 = 0x31 = 49 (bit 3 clear; 49 = 0x31)
            OUT = x;
            loop {}
        }
    "#);
    // 10 +5 -3 *2 <<1 = 48; |1 = 49 (0x31); &0xF7 leaves 0x31 (bit3 already 0).
    assert_eq!(e.mem(0x0400), 0x31, "chained compound assignments");
}

#[test]
fn compound_assignment_u16() {
    let mut e = run(r#"
        const LO: addr = 0x0400;
        const HI: addr = 0x0401;
        #[reset]
        fn main() {
            let x: u16 = 1000;
            x += 500;  // 1500
            x -= 100;  // 1400
            LO = x.low;
            HI = x.high;
            loop {}
        }
    "#);
    assert_eq!(e.mem16(0x0400), 1400, "u16 compound assignment");
}

// ---------------------------------------------------------------------------
// Enum dispatch over unit variants.
// ---------------------------------------------------------------------------

#[test]
fn enum_unit_variant_dispatch() {
    let classify = |v: &str| {
        let src = format!(
            r#"
            enum Dir {{ North, South, East, West }}
            const OUT: addr = 0x0400;
            #[reset]
            fn main() {{
                let d: Dir = Dir::{v};
                match d {{
                    Dir::North => {{ OUT = 1; }}
                    Dir::South => {{ OUT = 2; }}
                    Dir::East  => {{ OUT = 3; }}
                    Dir::West  => {{ OUT = 4; }}
                }}
                loop {{}}
            }}
        "#
        );
        run(&src).mem(0x0400)
    };
    assert_eq!(classify("North"), 1);
    assert_eq!(classify("East"), 3);
    assert_eq!(classify("West"), 4);
}

// ---------------------------------------------------------------------------
// Array-fill initializer and read.
// ---------------------------------------------------------------------------

#[test]
fn array_fill_init() {
    let mut e = run(r#"
        const AV: addr = 0x0400;
        const BV: addr = 0x0401;
        #[reset]
        fn main() {
            let buf: [u8; 8] = [7; 8];
            let i: u8 = 3;
            AV = buf[0];
            BV = buf[i];
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0400), 7, "buf[0] from fill");
    assert_eq!(e.mem(0x0401), 7, "buf[i] from fill");
}

// ---------------------------------------------------------------------------
// Nested / composed function calls.
// ---------------------------------------------------------------------------

#[test]
fn nested_function_calls() {
    let mut e = run(r#"
        const OUT: addr = 0x0400;
        fn inc2(n: u8) -> u8 { return n + 2; }
        fn dbl(n: u8) -> u8 { return n + n; }
        #[reset]
        fn main() {
            OUT = dbl(inc2(inc2(5)));
            loop {}
        }
    "#);
    // inc2(inc2(5)) = 9; dbl(9) = 18.
    assert_eq!(e.mem(0x0400), 18, "dbl(inc2(inc2(5))) = 18");
}

// ---------------------------------------------------------------------------
// u8 binary op whose right operand clobbers Y.
// ---------------------------------------------------------------------------

#[test]
fn u8_binary_right_operand_clobbers_y() {
    // The left operand `(a + 1)` is complex, so codegen parks it in Y while it
    // evaluates the right operand. `arr[i]` scales its index through Y, which
    // destroyed the parked value before the fix (result came out as the index,
    // not the sum).
    let mut e = run(r#"
        const OUT: addr = 0x0400;
        #[reset]
        fn main() {
            let a: u8 = 10;
            let arr: [u8; 4] = [1, 2, 3, 5];
            let i: u8 = 3;
            let r: u8 = (a + 1) + arr[i];
            OUT = r;
            loop {}
        }
    "#);
    // (10 + 1) + arr[3](=5) = 16.
    assert_eq!(e.mem(0x0400), 16, "left operand must survive a Y-clobbering right");
}

// ---------------------------------------------------------------------------
// foreach over a b16 (BCD) array reserves a full 2-byte loop-variable slot.
// ---------------------------------------------------------------------------

#[test]
fn foreach_b16_loop_var_full_width() {
    // b16 was omitted from the loop-variable width check, so the loop var got
    // only 1 byte and the next body-local (`t`) overlapped its high byte. The
    // cast reads the high byte back, which `t` had clobbered.
    let mut e = run(r#"
        const LO: addr = 0x0400;
        const HI: addr = 0x0401;
        #[reset]
        fn main() {
            let arr: [b16; 2] = [1234 as b16, 5678 as b16];
            for x in arr {
                let t: u8 = 0x99;
                let xu: u16 = x as u16;
                LO = xu.low;
                HI = xu.high;
                LO = t;
            }
            loop {}
        }
    "#);
    // Last element 5678 packs to BCD 0x5678; its high byte must be 0x56, not
    // the 0x99 written into the overlapping slot.
    assert_eq!(e.mem(0x0401), 0x56, "b16 loop var high byte must survive");
}

// ---------------------------------------------------------------------------
// Inclusive for-range.
// ---------------------------------------------------------------------------

#[test]
fn inclusive_for_range() {
    let mut e = run(r#"
        const OUT: addr = 0x0400;
        #[reset]
        fn main() {
            let s: u8 = 0;
            for i in 1..=5 {
                s = s + i;
            }
            OUT = s;
            loop {}
        }
    "#);
    // 1 + 2 + 3 + 4 + 5 = 15.
    assert_eq!(e.mem(0x0400), 15, "inclusive range 1..=5 sums to 15");
}

// ---------------------------------------------------------------------------
// String iteration (ForEach over a string's characters).
// ---------------------------------------------------------------------------

#[test]
fn string_foreach_sum() {
    let mut e = run(r#"
        const OUT: addr = 0x0400;
        #[reset]
        fn main() {
            let s: str = "AB";
            let sum: u8 = 0;
            for c in s {
                sum = sum + c;
            }
            OUT = sum;
            loop {}
        }
    "#);
    // 'A'(65) + 'B'(66) = 131.
    assert_eq!(e.mem(0x0400), 131, "sum of 'A' + 'B'");
}

// ---------------------------------------------------------------------------
// Division and modulo specifics.
// ---------------------------------------------------------------------------

#[test]
fn division_and_modulo() {
    let calc = |expr: &str| {
        let src = format!(
            r#"
            const OUT: addr = 0x0400;
            #[reset]
            fn main() {{
                let a: u8 = 23;
                let b: u8 = 5;
                OUT = {expr};
                loop {{}}
            }}
        "#
        );
        run(&src).mem(0x0400)
    };
    assert_eq!(calc("a / b"), 4, "23 / 5 = 4");
    assert_eq!(calc("a % b"), 3, "23 % 5 = 3");
}

// ---------------------------------------------------------------------------
// else-if chain.
// ---------------------------------------------------------------------------

#[test]
fn else_if_chain() {
    let grade = |score: u8| {
        let src = format!(
            r#"
            const OUT: addr = 0x0400;
            #[reset]
            fn main() {{
                let s: u8 = {score};
                if s >= 90 {{
                    OUT = 4;
                }} else if s >= 80 {{
                    OUT = 3;
                }} else if s >= 70 {{
                    OUT = 2;
                }} else {{
                    OUT = 1;
                }}
                loop {{}}
            }}
        "#
        );
        run(&src).mem(0x0400)
    };
    assert_eq!(grade(95), 4);
    assert_eq!(grade(85), 3);
    assert_eq!(grade(72), 2);
    assert_eq!(grade(50), 1);
}
