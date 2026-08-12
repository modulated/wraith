//! End-to-end tests for CPU flag access.
//!
//! These assert on *behavior*, not on generated assembly. The distinction
//! matters here more than anywhere else: an assembly test for carry handling
//! reads `CLC` and `ADC` and passes, but so does the wrong sequence — `CLC`
//! after the `ADC`, `SEC` where `CLC` belongs, or a carry that survives one
//! statement too long. Only running the program pins down what the flag was.
//!
//! Each test therefore stores the quantity under test to a known address and
//! reads it back from the emulator. The 6502 flag conventions the tests encode:
//!
//! - `ADC` adds the carry in, so an addition must `CLC` first; carry comes out
//!   set when the unsigned sum exceeded 255.
//! - `SBC` subtracts the *borrow*, which is the inverted carry, so a subtraction
//!   must `SEC` first; carry comes out set when no borrow was needed
//!   (i.e. when `a >= b`).
//! - `zero` is set when the last result was 0.

use crate::common::exec::run;

/// Scratch addresses in RAM the programs below write their results to.
const OUT: u16 = 0x0400;
const OUT2: u16 = 0x0401;

// ============================================================
// Carry out of 8-bit arithmetic
// ============================================================

#[test]
fn addition_that_wraps_sets_carry() {
    // 255 + 1 wraps to 0 and carries out. Reading `carry` after the addition
    // must see the carry that addition produced, not a stale one.
    let mut e = run(r#"
        const SUM: addr = 0x0400;
        const CARRY: addr = 0x0401;
        #[reset]
        fn main() {
            let x: u8 = 255;
            let y: u8 = x + 1;
            SUM = y;
            CARRY = carry as u8;
            loop {}
        }
    "#);
    assert_eq!(e.mem(OUT), 0, "255 + 1 wraps to 0");
    assert_eq!(e.mem(OUT2), 1, "255 + 1 must carry out");
}

#[test]
fn addition_that_does_not_wrap_clears_carry() {
    // The companion to the above, and the one that catches a missing `CLC`:
    // if the carry were left set from an earlier operation, 1 + 1 would come
    // out as 3 and would report a carry it never generated.
    let mut e = run(r#"
        const SUM: addr = 0x0400;
        const CARRY: addr = 0x0401;
        #[reset]
        fn main() {
            let big: u8 = 255;
            let wrapped: u8 = big + 1;   // leaves carry set
            let x: u8 = 1;
            let y: u8 = x + 1;           // must CLC, so this is 2 and not 3
            SUM = y;
            CARRY = carry as u8;
            loop {}
        }
    "#);
    assert_eq!(e.mem(OUT), 2, "1 + 1 is 2 — a stale carry would make it 3");
    assert_eq!(e.mem(OUT2), 0, "1 + 1 does not carry out");
}

#[test]
fn subtraction_borrows_through_an_inverted_carry() {
    // On the 6502 carry is the *inverse* borrow: 10 - 5 needs no borrow, so
    // carry comes out set. A missing `SEC` before the `SBC` would subtract an
    // extra 1 and give 4.
    let mut e = run(r#"
        const DIFF: addr = 0x0400;
        const CARRY: addr = 0x0401;
        #[reset]
        fn main() {
            let x: u8 = 10;
            let y: u8 = x - 5;
            DIFF = y;
            CARRY = carry as u8;
            loop {}
        }
    "#);
    assert_eq!(e.mem(OUT), 5, "10 - 5 is 5 — a missing SEC would give 4");
    assert_eq!(e.mem(OUT2), 1, "10 - 5 needs no borrow, so carry stays set");
}

#[test]
fn subtraction_that_underflows_clears_carry() {
    // 5 - 10 wraps to 251 and borrows, clearing carry. This is the branch the
    // "carry is inverted" convention exists for.
    let mut e = run(r#"
        const DIFF: addr = 0x0400;
        const CARRY: addr = 0x0401;
        #[reset]
        fn main() {
            let x: u8 = 5;
            let y: u8 = x - 10;
            DIFF = y;
            CARRY = carry as u8;
            loop {}
        }
    "#);
    assert_eq!(e.mem(OUT), 251, "5 - 10 wraps to 251");
    assert_eq!(e.mem(OUT2), 0, "5 - 10 borrows, so carry is cleared");
}

// ============================================================
// The zero flag
// ============================================================

#[test]
fn equality_comparison_takes_the_branch_it_should() {
    // The old test asserted `CMP` and `BEQ` appeared. That says nothing about
    // polarity: `BNE` in place of `BEQ` produces the same two mnemonics and
    // inverts the program. Running it settles which arm executed.
    let mut e = run(r#"
        const TAKEN: addr = 0x0400;
        const NOT_TAKEN: addr = 0x0401;
        #[reset]
        fn main() {
            let x: u8 = 5;
            if x == 5 { TAKEN = 1; }
            if x == 6 { NOT_TAKEN = 1; }
            loop {}
        }
    "#);
    assert_eq!(e.mem(OUT), 1, "5 == 5 must take the branch");
    assert_eq!(e.mem(OUT2), 0, "5 == 6 must not take the branch");
}

#[test]
fn a_zero_result_sets_the_zero_flag() {
    let mut e = run(r#"
        const DIFF: addr = 0x0400;
        const ZERO: addr = 0x0401;
        #[reset]
        fn main() {
            let x: u8 = 5;
            let y: u8 = x - 5;
            DIFF = y;
            ZERO = zero as u8;
            loop {}
        }
    "#);
    assert_eq!(e.mem(OUT), 0, "5 - 5 is 0");
    assert_eq!(e.mem(OUT2), 1, "a zero result sets the zero flag");
}

#[test]
fn a_nonzero_result_clears_the_zero_flag() {
    let mut e = run(r#"
        const DIFF: addr = 0x0400;
        const ZERO: addr = 0x0401;
        #[reset]
        fn main() {
            let x: u8 = 5;
            let y: u8 = x - 3;
            DIFF = y;
            ZERO = zero as u8;
            loop {}
        }
    "#);
    assert_eq!(e.mem(OUT), 2, "5 - 3 is 2");
    assert_eq!(e.mem(OUT2), 0, "a nonzero result clears the zero flag");
}

// ============================================================
// Flags as values
// ============================================================

#[test]
fn a_set_flag_reads_as_one_not_as_its_status_bit() {
    // `negative as u8` must normalize to 1. `negative` lives in bit 7 of the
    // status register, so an implementation that masks the flag in place
    // without normalizing yields 0x80 here — a value that is still "truthy"
    // and so goes unnoticed by every test that only checks for nonzero.
    // Carry is the case that cannot catch this: its bit is 0x01 already.
    let mut e = run(r#"
        const N: addr = 0x0400;
        const C: addr = 0x0401;
        #[reset]
        fn main() {
            let x: u8 = 100;
            let y: u8 = x + 100;   // 200: bit 7 set, no carry out
            N = negative as u8;
            C = carry as u8;
            loop {}
        }
    "#);
    assert_eq!(
        e.mem(OUT),
        1,
        "200 has bit 7 set, so `negative` is 1 — not 0x80, its status-bit value"
    );
    assert_eq!(e.mem(OUT2), 0, "100 + 100 fits in u8, so no carry out");
}

#[test]
fn a_clear_flag_reads_as_zero() {
    let mut e = run(r#"
        const N: addr = 0x0400;
        const C: addr = 0x0401;
        #[reset]
        fn main() {
            let x: u8 = 200;
            let y: u8 = x + 100;   // wraps to 44: carry out, bit 7 clear
            N = negative as u8;
            C = carry as u8;
            loop {}
        }
    "#);
    assert_eq!(e.mem(OUT), 0, "the wrapped result 44 has bit 7 clear");
    assert_eq!(e.mem(OUT2), 1, "200 + 100 carries out");
}

#[test]
fn a_flag_drives_a_conditional_directly() {
    // `if carry` is the idiomatic use, and exercises the branch rather than the
    // materialize-to-a-byte path.
    let mut e = run(r#"
        const OVERFLOWED: addr = 0x0400;
        const FINE: addr = 0x0401;
        #[reset]
        fn main() {
            let x: u8 = 250;
            let y: u8 = x + 10;
            if carry { OVERFLOWED = 1; }

            let a: u8 = 1;
            let b: u8 = a + 1;
            if carry { FINE = 1; }
            loop {}
        }
    "#);
    assert_eq!(e.mem(OUT), 1, "250 + 10 overflows, so `if carry` runs");
    assert_eq!(e.mem(OUT2), 0, "1 + 1 does not, so `if carry` is skipped");
}

// ============================================================
// 16-bit carry propagation
// ============================================================

#[test]
fn u16_addition_propagates_carry_between_bytes() {
    // 0x00FF + 1 = 0x0100 is the case a dropped inter-byte carry gets wrong:
    // it would produce 0x0000. Asserting `ADC` appears twice cannot see that.
    let mut e = run(r#"
        const LO: addr = 0x0400;
        const HI: addr = 0x0401;
        #[reset]
        fn main() {
            let x: u16 = 0x00FF;
            let one: u16 = 1;
            let y: u16 = x + one;
            LO = y.low;
            HI = y.high;
            loop {}
        }
    "#);
    assert_eq!(
        e.mem16(OUT),
        0x0100,
        "the carry out of the low byte must reach the high byte"
    );
}

#[test]
fn u16_addition_wraps_at_the_type_maximum() {
    let mut e = run(r#"
        const LO: addr = 0x0400;
        const HI: addr = 0x0401;
        #[reset]
        fn main() {
            let x: u16 = 0xFFFF;
            let one: u16 = 1;
            let y: u16 = x + one;
            LO = y.low;
            HI = y.high;
            loop {}
        }
    "#);
    assert_eq!(e.mem16(OUT), 0x0000, "0xFFFF + 1 wraps to 0");
}

#[test]
fn u16_subtraction_borrows_between_bytes() {
    // The mirror of the addition case: 0x0100 - 1 = 0x00FF only if the borrow
    // out of the low byte is applied to the high byte.
    let mut e = run(r#"
        const LO: addr = 0x0400;
        const HI: addr = 0x0401;
        #[reset]
        fn main() {
            let x: u16 = 0x0100;
            let one: u16 = 1;
            let y: u16 = x - one;
            LO = y.low;
            HI = y.high;
            loop {}
        }
    "#);
    assert_eq!(
        e.mem16(OUT),
        0x00FF,
        "the borrow out of the low byte must reach the high byte"
    );
}
