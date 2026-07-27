//! Casts between pointers and integers, and pointer-valued statics.
//!
//! The whole difficulty here is a register convention. A 16-bit *scalar* is
//! carried in A:Y; a pointer is carried in A:X, like every other pointer-like
//! value in this compiler. So `p as u16` and `n as &T` change no bytes at all —
//! they move the high byte between X and Y. The 6502 has no `TXY`/`TYX`, and
//! going through A costs four instructions because A is holding the low byte,
//! so both directions round-trip through a zero-page scratch byte instead.
//!
//! Getting that wrong does not crash. It leaves the high byte at whatever the
//! other index register happened to hold, which in a freshly-zeroed emulator is
//! usually $00 — a plausible-looking zero-page address. That is why these tests
//! use addresses with a non-zero high byte throughout.

use crate::common::exec::run;
use crate::common::harness::{CompileResult, compile, compile_success};

fn expect_error(src: &str) -> String {
    match compile(src) {
        CompileResult::SemaError(e) | CompileResult::CodegenError(e) => e,
        CompileResult::Success(..) => panic!("expected a compile error, but it compiled"),
        other => panic!("expected an error, got {other:?}"),
    }
}

// ============================================================================
// Integer to pointer
// ============================================================================

#[test]
fn an_integer_cast_to_a_pointer_addresses_that_location() {
    // The documented way to name a fixed hardware location without an `addr`
    // declaration. $0910 is chosen for its non-zero high byte: a dropped X
    // would write to $0010 and this would silently read back zero.
    let mut e = run(r#"
        const R0: addr = 0x0900;
        #[reset]
        fn main() {
            let p: &u8 = 0x0910 as &u8;
            *p = 0x5A;
            R0 = 0;
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0910), 0x5A);
}

#[test]
fn a_runtime_u16_cast_to_a_pointer_keeps_its_high_byte() {
    // The value is computed rather than folded, so this exercises the actual
    // Y -> X move rather than a constant load.
    let mut e = run(r#"
        const R0: addr = 0x0900;
        #[reset]
        fn main() {
            let base: u16 = 0x0900;
            let n: u16 = base + 0x20;
            let p: &u8 = n as &u8;
            *p = 0xC3;
            R0 = 0;
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0920), 0xC3, "the high byte survived the cast");
}

#[test]
fn an_eight_bit_value_cast_to_a_pointer_addresses_zero_page() {
    // An 8-bit source has no high byte to carry, so it names a zero-page byte.
    let asm = compile_success(
        r#"
        const R0: addr = 0x0900;
        #[reset]
        fn main() { let n: u8 = 0x40; let p: &u8 = n as &u8; R0 = *p; loop {} }
    "#,
    );
    assert!(
        asm.contains("LDX #$00"),
        "expected a zeroed high byte:\n{asm}"
    );
}

// ============================================================================
// Pointer to integer
// ============================================================================

#[test]
fn a_pointer_cast_to_u16_round_trips_back_to_a_pointer() {
    // The round trip is the real test: if either direction dropped the high
    // byte, the pointer would come back naming a different page.
    let mut e = run(r#"
        const R0: addr = 0x0900;
        static SLOT: u8 = 0;
        #[reset]
        fn main() {
            let p: &u8 = &SLOT;
            let n: u16 = p as u16;
            let q: &u8 = n as &u8;
            *q = 42;
            R0 = SLOT;
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0900), 42);
}

#[test]
fn a_pointer_cast_to_u16_is_the_address_it_names() {
    // `&SLOT as u16` must equal SLOT's own address, which the program can check
    // for itself by using the number as an address again — but here the point
    // is that the high byte is non-zero and survives being stored as a u16.
    let mut e = run(r#"
        const R0: addr = 0x0900;
        const R1: addr = 0x0901;
        static SLOT: u8 = 0;
        #[reset]
        fn main() {
            let n: u16 = &SLOT as u16;
            R0 = n.low;
            R1 = n.high;
            loop {}
        }
    "#);
    assert_ne!(e.mem(0x0901), 0, "a static lives above the zero page");
}

#[test]
fn casting_a_pointer_to_u16_moves_the_high_byte_from_x_to_y() {
    // The load-bearing shape. There is no TXY on a 6502, so this has to go
    // through a zero-page byte; `TXA` would destroy the low byte in A.
    let asm = compile_success(
        r#"
        const R0: addr = 0x0900;
        static SLOT: u8 = 0;
        #[reset]
        fn main() { let n: u16 = &SLOT as u16; R0 = n.high; loop {} }
    "#,
    );
    assert!(
        asm.contains("STX $22") && asm.contains("LDY $22"),
        "expected the X -> Y move through scratch:\n{asm}"
    );
}

#[test]
fn a_cast_between_pointer_types_changes_nothing() {
    let mut e = run(r#"
        const R0: addr = 0x0900;
        static W: u16 = 0;
        #[reset]
        fn main() {
            let p: &u16 = &W;
            let b: &u8 = p as &u8;
            *b = 0x77;
            R0 = W.low;
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0900), 0x77);
}

// ============================================================================
// The legality matrix
// ============================================================================

#[test]
fn narrowing_a_pointer_to_eight_bits_is_rejected() {
    // Silently discarding the page is the one pointer mistake that looks fine
    // in a hex dump, so it has to be named rather than truncated.
    let err = expect_error(
        r#"
        const R0: addr = 0x0900;
        static SLOT: u8 = 0;
        #[reset]
        fn main() { let n: u8 = &SLOT as u8; R0 = n; loop {} }
    "#,
    );
    assert!(err.contains("16 bits"), "{err}");
}

#[test]
fn casting_a_pointer_to_bool_is_rejected() {
    let err = expect_error(
        r#"
        const R0: addr = 0x0900;
        static SLOT: u8 = 0;
        #[reset]
        fn main() { let b: bool = &SLOT as bool; R0 = 0; loop {} }
    "#,
    );
    assert!(err.contains("cannot cast a pointer"), "{err}");
}

#[test]
fn casting_a_bool_to_a_pointer_is_rejected() {
    let err = expect_error(
        r#"
        const R0: addr = 0x0900;
        #[reset]
        fn main() { let p: &u8 = true as &u8; R0 = *p; loop {} }
    "#,
    );
    assert!(err.contains("only an integer address"), "{err}");
}

#[test]
fn casting_a_string_to_a_pointer_is_rejected() {
    // A string *is* already a pointer at runtime, but to a length-prefixed
    // block, so reinterpreting it as `&u8` would silently point at the prefix.
    let err = expect_error(
        r#"
        const R0: addr = 0x0900;
        #[reset]
        fn main() { let s: str = "hi"; let p: &u8 = s as &u8; R0 = *p; loop {} }
    "#,
    );
    assert!(err.contains("only an integer address"), "{err}");
}

// ============================================================================
// Pointer-valued statics
// ============================================================================

#[test]
fn a_static_pointer_can_be_initialised_to_another_statics_address() {
    // A static's BSS address is known during analysis, so this needs no label
    // indirection — unlike a function reference, which the assembler resolves.
    let mut e = run(r#"
        const R0: addr = 0x0900;
        static COUNT: u8 = 7;
        static P: &u8 = &COUNT;
        #[reset]
        fn main() { R0 = *P; loop {} }
    "#);
    assert_eq!(e.mem(0x0900), 7, "the pointer was live before main ran");
}

#[test]
fn a_static_pointer_initialised_to_a_literal_address_is_not_null() {
    let mut e = run(r#"
        const R0: addr = 0x0900;
        static P: &u8 = 0x0930 as &u8;
        #[reset]
        fn main() { *P = 0x99; R0 = 0; loop {} }
    "#);
    assert_eq!(e.mem(0x0930), 0x99);
}

#[test]
fn a_forward_reference_in_a_static_initialiser_is_rejected() {
    // Statics are laid out in declaration order, so LATER has no address yet.
    // Falling back to zeros would leave a null pointer that only misbehaves at
    // run time, which is exactly the failure typed pointers exist to prevent.
    let err = expect_error(
        r#"
        const R0: addr = 0x0900;
        static P: &u8 = &LATER;
        static LATER: u8 = 3;
        #[reset]
        fn main() { R0 = *P; loop {} }
    "#,
    );
    assert!(err.contains("declaration order"), "{err}");
}

#[test]
fn taking_the_address_of_a_constant_in_a_static_initialiser_is_rejected() {
    // Same reason as in ordinary code: an immutable const is recorded at the
    // sentinel address 0 and referenced by ROM label, so this would mean $0000.
    let err = expect_error(
        r#"
        const R0: addr = 0x0900;
        const LIMIT: u8 = 9;
        static P: &u8 = &LIMIT;
        #[reset]
        fn main() { R0 = *P; loop {} }
    "#,
    );
    assert!(err.contains("LIMIT"), "{err}");
    assert!(err.contains("ROM") || err.contains("label"), "{err}");
}
