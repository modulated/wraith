//! `q8.8` — signed 16-bit fixed-point, 8 integer bits and 8 fraction bits.
//!
//! A value is a two's-complement `i16` scaled by 256, so `1.5` is `0x0180`.
//! Add and subtract are plain 16-bit arithmetic (no decimal mode); a cast to an
//! integer is an arithmetic shift right by 8 — the integer part, rounded toward
//! negative infinity. Multiply and divide are not built yet and are refused.

use crate::common::exec::run;
use crate::common::harness::{assert_error_contains, compile_success};

/// A fractional literal binds and reads back its integer part.
#[test]
fn a_fixed_literal_reads_back_its_integer_part() {
    let mut e = run(r#"
        const OUT: addr = 0x0900;
        #[reset]
        fn main() {
            let pos: q8.8 = 1.5;
            OUT = pos as u8;
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0900), 1, "the integer part of 1.5 is 1");
}

/// Adding fractions carries into the integer byte: 1.5 + 0.5 = 2.0.
#[test]
fn a_fraction_carries_into_the_integer_byte() {
    let mut e = run(r#"
        const OUT: addr = 0x0900;
        #[reset]
        fn main() {
            let a: q8.8 = 1.5;
            let b: q8.8 = 0.5;
            let c: q8.8 = a + b;
            OUT = c as u8;
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0900), 2, "1.5 + 0.5 = 2.0");
}

/// An integration loop: add a velocity each step, the common game-math use.
#[test]
fn an_integration_loop_accumulates_a_velocity() {
    let mut e = run(r#"
        const OUT: addr = 0x0900;
        #[reset]
        fn main() {
            let pos: q8.8 = 0.0;
            let vel: q8.8 = 0.25;
            let i: u8 = 0;
            while i < 16 {
                pos = pos + vel;
                i = i + 1;
            }
            OUT = pos as u8;
            loop {}
        }
    "#);
    // 16 * 0.25 = 4.0
    assert_eq!(e.mem(0x0900), 4, "sixteen steps of 0.25 reach 4.0");
}

/// Subtraction, and a value that lands on a whole number.
#[test]
fn subtraction_works() {
    let mut e = run(r#"
        const OUT: addr = 0x0900;
        #[reset]
        fn main() {
            let a: q8.8 = 10.5;
            let b: q8.8 = 3.5;
            let c: q8.8 = a - b;
            OUT = c as u8;
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0900), 7, "10.5 - 3.5 = 7.0");
}

/// A whole number reaches fixed-point through `.0` or a cast, not bare.
#[test]
fn a_whole_number_literal_is_whole_units() {
    let mut e = run(r#"
        const OUT: addr = 0x0900;
        #[reset]
        fn main() {
            let a: q8.8 = 3.0;
            let b: q8.8 = 0.5;
            OUT = (a + b) as u8;
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0900), 3, "3.0 + 0.5 = 3.5, integer part 3");
}

/// A bare integer is refused in a fixed-point context — say `3.0` or `3 as q8.8`.
#[test]
fn a_bare_integer_is_not_adopted_as_fixed() {
    assert_error_contains(
        r#"
        fn f() {
            let a: q8.8 = 3;
        }
        "#,
        "type",
    );
}

/// Comparisons are signed 16-bit and respect the fraction.
#[test]
fn comparison_respects_the_fraction() {
    let mut e = run(r#"
        const OUT: addr = 0x0900;
        #[reset]
        fn main() {
            let a: q8.8 = 1.25;
            let b: q8.8 = 1.75;
            if a < b {
                OUT = 1;
            } else {
                OUT = 0;
            }
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0900), 1, "1.25 < 1.75");
}

/// An integer scales up to fixed-point and back down.
#[test]
fn int_to_fixed_and_back() {
    let mut e = run(r#"
        const OUT: addr = 0x0900;
        #[reset]
        fn main() {
            let n: u8 = 5;
            let f: q8.8 = n as q8.8;
            let back: u8 = f as u8;
            OUT = back;
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0900), 5, "5 -> 5.0 -> 5");
}

/// A negative fixed value casts to an integer by flooring (arithmetic shift).
#[test]
fn a_negative_value_floors_toward_negative_infinity() {
    let mut e = run(r#"
        const LO: addr = 0x0900;
        const HI: addr = 0x0901;
        #[reset]
        fn main() {
            let a: q8.8 = 0.0;
            let b: q8.8 = 1.5;
            let n: q8.8 = a - b;      // -1.5
            let whole: i16 = n as i16; // floor(-1.5) = -2
            LO = whole.low;
            HI = whole.high;
            loop {}
        }
    "#);
    assert_eq!(e.mem16(0x0900), 0xFFFE, "floor(-1.5) is -2");
}

/// A `q8.8` survives a function boundary — passed in and returned.
#[test]
fn fixed_crosses_a_call() {
    let mut e = run(r#"
        const OUT: addr = 0x0900;
        fn bump(x: q8.8) -> q8.8 {
            return x + 1.0;
        }
        #[reset]
        fn main() {
            let a: q8.8 = 2.5;
            let b: q8.8 = bump(a);
            OUT = b as u8;
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0900), 3, "bump(2.5) = 3.5, integer part 3");
}

/// A mutable `static` holds a `q8.8` across the reset write and a store.
#[test]
fn a_static_holds_a_fixed_value() {
    let mut e = run(r#"
        const OUT: addr = 0x0900;
        static P: q8.8 = 0.0;
        #[reset]
        fn main() {
            P = 1.5;
            P = P + 2.0;
            OUT = P as u8;
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0900), 3, "1.5 + 2.0 = 3.5, integer part 3");
}

/// Add is a plain 16-bit add, not the decimal-mode path BCD uses.
#[test]
fn add_uses_no_decimal_mode() {
    let asm = compile_success(
        r#"
        const OUT: addr = 0x0900;
        #[reset]
        fn main() {
            let a: q8.8 = 1.5;
            let b: q8.8 = 0.5;
            OUT = (a + b) as u8;
            loop {}
        }
    "#,
    );
    assert!(
        !asm.contains("SED"),
        "fixed-point add must not set decimal mode:\n{asm}"
    );
}

/// Multiply: 1.5 * 2.0 = 3.0.
#[test]
fn multiply_whole_result() {
    let mut e = run(r#"
        const OUT: addr = 0x0900;
        #[reset]
        fn main() {
            let a: q8.8 = 1.5;
            let b: q8.8 = 2.0;
            OUT = (a * b) as u8;
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0900), 3, "1.5 * 2.0 = 3.0");
}

/// Multiply produces a fractional result: 0.5 * 0.5 = 0.25.
#[test]
fn multiply_fractional_result() {
    let mut e = run(r#"
        const LO: addr = 0x0900;
        const HI: addr = 0x0901;
        #[reset]
        fn main() {
            let a: q8.8 = 0.5;
            let b: q8.8 = 0.5;
            let c: q8.8 = a * b;   // 0.25 = 0x0040
            let raw: i16 = c as i16;
            // 0.25 floors to 0 as an int, so check the encoding via + itself * 4
            let four: q8.8 = c + c + c + c;  // 1.0
            LO = four as u8;
            HI = raw as u8;
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0900), 1, "0.25 * 4 = 1.0");
    assert_eq!(e.mem(0x0901), 0, "0.25 floors to 0");
}

/// A negative operand carries its sign through the product.
#[test]
fn multiply_signed() {
    let mut e = run(r#"
        const LO: addr = 0x0900;
        const HI: addr = 0x0901;
        #[reset]
        fn main() {
            let a: q8.8 = 0.0;
            let b: q8.8 = 1.5;
            let neg: q8.8 = a - b;        // -1.5
            let two: q8.8 = 2.0;
            let r: q8.8 = neg * two;      // -3.0
            let whole: i16 = r as i16;    // -3
            LO = whole.low;
            HI = whole.high;
            loop {}
        }
    "#);
    assert_eq!(
        e.mem16(0x0900),
        0xFFFD,
        "-1.5 * 2.0 = -3.0, integer part -3"
    );
}

/// Two negatives multiply to a positive.
#[test]
fn multiply_two_negatives() {
    let mut e = run(r#"
        const OUT: addr = 0x0900;
        #[reset]
        fn main() {
            let z: q8.8 = 0.0;
            let a: q8.8 = z - 2.0;   // -2.0
            let b: q8.8 = z - 1.5;   // -1.5
            OUT = (a * b) as u8;     // 3.0
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0900), 3, "-2.0 * -1.5 = 3.0");
}

/// Multiply is commutative — the routine handles either operand order.
#[test]
fn multiply_is_commutative() {
    let mut e = run(r#"
        const OUT: addr = 0x0900;
        #[reset]
        fn main() {
            let a: q8.8 = 2.5;
            let b: q8.8 = 3.0;
            let ab: q8.8 = a * b;
            let ba: q8.8 = b * a;
            if ab == ba { OUT = 1; } else { OUT = 0; }
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0900), 1, "a*b == b*a");
}

/// Multiplying by 1.0 is the identity.
#[test]
fn multiply_by_one_is_identity() {
    let mut e = run(r#"
        const OUT: addr = 0x0900;
        #[reset]
        fn main() {
            let a: q8.8 = 12.0;
            let one: q8.8 = 1.0;
            OUT = (a * one) as u8;
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0900), 12, "12.0 * 1.0 = 12.0");
}

/// Overflow past the q8.8 range wraps (truncate and wrap, as documented).
#[test]
fn multiply_overflow_wraps() {
    let mut e = run(r#"
        const LO: addr = 0x0900;
        const HI: addr = 0x0901;
        #[reset]
        fn main() {
            let a: q8.8 = 100.0;
            let b: q8.8 = 2.0;
            let r: q8.8 = a * b;   // 200.0 overflows signed q8.8 -> wraps
            let raw: i16 = r as i16;
            LO = raw.low;
            HI = raw.high;
            loop {}
        }
    "#);
    // 200.0 = 0xC800 as a bit pattern; as a *signed* q8.8 that is negative.
    // >>8 integer part = 0xC8 sign-extended = -56.
    assert_eq!(
        e.mem16(0x0900),
        0xFFC8,
        "200.0 wraps to -56.0 in signed q8.8"
    );
}

/// Divide is deferred.
#[test]
fn divide_is_refused_for_now() {
    assert_error_contains(
        r#"
        fn f() {
            let a: q8.8 = 3.0;
            let b: q8.8 = 2.0;
            let c: q8.8 = a / b;
        }
        "#,
        "fixed-point",
    );
}

/// A fractional literal with no fixed-point context has no type to adopt.
#[test]
fn a_bare_fraction_needs_a_fixed_context() {
    assert_error_contains(
        r#"
        fn f() {
            let x: u16 = 1.5;
        }
        "#,
        "fixed-point",
    );
}

/// Mixing a `q8.8` with a plain integer type is a type error.
#[test]
fn mixing_fixed_with_int_is_rejected() {
    assert_error_contains(
        r#"
        fn f() {
            let a: q8.8 = 1.5;
            let b: u8 = 2;
            let c: q8.8 = a + b;
        }
        "#,
        "type",
    );
}
