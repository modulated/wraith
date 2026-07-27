//! Pointers: `&x`, `*p`, and `*p = v`.
//!
//! A pointer is two bytes carried in A (low) : X (high) — the same convention
//! arrays, strings and enums already use. That matters more than it sounds:
//! a 16-bit *scalar* uses A:Y, so anything that stores a pointer has to pick
//! `STX` rather than `STY` for the high byte. Get it wrong and only the low
//! byte lands, leaving a pointer into whatever page the slot happened to hold —
//! which, in a freshly-zeroed emulator, still works by accident.
//!
//! The other thing worth stating: passing `&local` down into a callee is safe
//! *because* frames are colour-allocated so a callee's frame never overlaps a
//! live caller's. That is not a new guarantee — struct arguments have been
//! passed by reference this way all along — but these tests are the first to
//! depend on it explicitly.

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
// Reading and writing through a pointer
// ============================================================================

#[test]
fn a_write_through_a_pointer_lands_in_the_variable() {
    assert_eq!(
        run(r#"
            const OUT: addr = 0x0900;
            #[reset]
            fn main() {
                let x: u8 = 1;
                let p: &u8 = &x;
                *p = 42;
                OUT = x;
                loop {}
            }
        "#)
        .mem(0x0900),
        42
    );
}

#[test]
fn a_read_through_a_pointer_sees_the_variable() {
    assert_eq!(
        run(r#"
            const OUT: addr = 0x0900;
            #[reset]
            fn main() {
                let x: u8 = 77;
                let p: &u8 = &x;
                OUT = *p;
                loop {}
            }
        "#)
        .mem(0x0900),
        77
    );
}

#[test]
fn a_write_through_a_pointer_is_seen_by_a_later_read_through_it() {
    assert_eq!(
        run(r#"
            const OUT: addr = 0x0900;
            #[reset]
            fn main() {
                let x: u8 = 1;
                let p: &u8 = &x;
                *p = 9;
                OUT = *p;
                loop {}
            }
        "#)
        .mem(0x0900),
        9
    );
}

#[test]
fn two_pointers_to_the_same_variable_alias() {
    // The test that catches a missing `invalidate_registers` after an indirect
    // store: a cached belief about `x` would survive the write through `p` and
    // make the read through `q` return the stale value.
    assert_eq!(
        run(r#"
            const OUT: addr = 0x0900;
            #[reset]
            fn main() {
                let x: u8 = 0;
                let p: &u8 = &x;
                let q: &u8 = &x;
                *p = 1;
                *q = 2;
                OUT = *p;
                loop {}
            }
        "#)
        .mem(0x0900),
        2
    );
}

#[test]
fn a_pointer_to_a_u16_carries_both_bytes() {
    // The pointee's width decides whether the deref moves one byte or two.
    assert_eq!(
        run(r#"
            const LO: addr = 0x0900;
            const HI: addr = 0x0901;
            #[reset]
            fn main() {
                let x: u16 = 1;
                let p: &u16 = &x;
                *p = 0x1234;
                LO = x.low;
                HI = x.high;
                loop {}
            }
        "#)
        .mem16(0x0900),
        0x1234
    );
}

// ============================================================================
// Passing a pointer down the call chain
// ============================================================================

#[test]
fn a_callee_can_write_through_a_pointer_to_the_callers_local() {
    // The motivating shape, in miniature: hand a callee somewhere to put its
    // answer. Safe because a callee's frame never overlaps a live caller's.
    assert_eq!(
        run(r#"
            const OUT: addr = 0x0900;
            fn put(dest: &u8, v: u8) { *dest = v; }
            #[reset]
            fn main() {
                let slot: u8 = 0;
                put(&slot, 55);
                OUT = slot;
                loop {}
            }
        "#)
        .mem(0x0900),
        55
    );
}

#[test]
fn a_pointer_survives_two_levels_of_call() {
    assert_eq!(
        run(r#"
            const OUT: addr = 0x0900;
            fn inner(d: &u8) { *d = 7; }
            fn outer(d: &u8) { inner(d); }
            #[reset]
            fn main() {
                let slot: u8 = 0;
                outer(&slot);
                OUT = slot;
                loop {}
            }
        "#)
        .mem(0x0900),
        7
    );
}

#[test]
fn a_callee_reads_through_a_pointer_it_was_given() {
    assert_eq!(
        run(r#"
            const OUT: addr = 0x0900;
            fn twice(src: &u8) -> u8 { return *src + *src; }
            #[reset]
            fn main() {
                let n: u8 = 21;
                OUT = twice(&n);
                loop {}
            }
        "#)
        .mem(0x0900),
        42
    );
}

// ============================================================================
// Pointers to storage that is not in zero page
// ============================================================================

#[test]
fn a_pointer_to_a_static_reads_and_writes_it() {
    // A static lives in BSS, so this exercises the absolute-address form of
    // `&` rather than the zero-page one.
    let mut e = run(r#"
        const OUT: addr = 0x0900;
        static COUNT: u8 = 5;
        #[reset]
        fn main() {
            let p: &u8 = &COUNT;
            *p = 99;
            OUT = COUNT;
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0900), 99);
}

#[test]
fn a_pointer_to_a_local_array_element_writes_the_array() {
    assert_eq!(
        run(r#"
            const OUT: addr = 0x0900;
            #[reset]
            fn main() {
                let buf: [u8; 4] = [0; 4];
                let p: &u8 = &buf[2];
                *p = 0x5A;
                OUT = buf[2];
                loop {}
            }
        "#)
        .mem(0x0900),
        0x5A
    );
}

// ============================================================================
// The A:X convention
// ============================================================================

#[test]
fn storing_a_pointer_writes_both_of_its_bytes() {
    // A pointer uses A:X, unlike a 16-bit scalar's A:Y, so the store has to
    // pick STX. With only the low byte written the high byte is whatever the
    // slot held — which is zero in a fresh emulator, so this passes by luck
    // unless the emitted code is checked.
    let asm = compile_success(
        r#"
        const OUT: addr = 0x0900;
        #[reset]
        fn main() {
            let x: u8 = 1;
            let p: &u8 = &x;
            *p = 2;
            OUT = x;
            loop {}
        }
    "#,
    );
    assert!(
        asm.contains("LDX #$00"),
        "the high byte of a zero-page address is loaded:\n{asm}"
    );
    assert!(
        asm.lines().any(|l| l.trim().starts_with("STX $")),
        "and stored with STX, not STY:\n{asm}"
    );
}

// ============================================================================
// What `&` refuses
// ============================================================================

#[test]
fn the_address_of_a_temporary_is_rejected() {
    for expr in ["&5", "&(1 + 2)"] {
        let err = expect_error(&format!(
            "const OUT: addr = 0x0900; #[reset] fn main() {{ let p: &u8 = {expr}; loop {{}} }}"
        ));
        assert!(err.contains("temporary"), "for `{expr}`: {err}");
    }
}

#[test]
fn the_address_of_a_constant_is_rejected() {
    // A `const` is recorded at the sentinel address zero and referenced by ROM
    // label, so `&LIMIT` would quietly mean a pointer to $0000.
    let err = expect_error(
        r#"
        const LIMIT: u8 = 10;
        const OUT: addr = 0x0900;
        #[reset]
        fn main() { let p: &u8 = &LIMIT; loop {} }
    "#,
    );
    assert!(err.contains("constant"), "{err}");
    assert!(err.contains("ROM"), "the reason should be given: {err}");
}

#[test]
fn the_address_of_an_addr_declaration_is_rejected() {
    // Its read/write access mode is enforced at the name; a pointer would
    // launder that away.
    let err = expect_error(
        r#"
        const PORT: write addr = 0x6000;
        const OUT: addr = 0x0900;
        #[reset]
        fn main() { let p: &u8 = &PORT; loop {} }
    "#,
    );
    assert!(err.contains("access mode"), "{err}");
}

#[test]
fn the_address_of_a_function_is_rejected() {
    let err = expect_error(
        r#"
        const OUT: addr = 0x0900;
        fn helper() -> u8 { return 1; }
        #[reset]
        fn main() { let p: &u8 = &helper; loop {} }
    "#,
    );
    assert!(err.contains("already its address"), "{err}");
}

#[test]
fn dereferencing_a_non_pointer_is_rejected() {
    let err = expect_error(
        r#"
        const OUT: addr = 0x0900;
        #[reset]
        fn main() { let x: u8 = 1; OUT = *x; loop {} }
    "#,
    );
    assert!(err.contains('*'), "{err}");
}

#[test]
fn a_pointer_does_not_implicitly_become_an_integer() {
    let err = expect_error(
        r#"
        const OUT: addr = 0x0900;
        #[reset]
        fn main() { let x: u8 = 1; let n: u16 = &x; loop {} }
    "#,
    );
    assert!(err.contains("u16") && err.contains("&u8"), "{err}");
}

#[test]
fn a_pointer_to_the_wrong_type_is_rejected() {
    let err = expect_error(
        r#"
        const OUT: addr = 0x0900;
        #[reset]
        fn main() { let y: u16 = 1; let p: &u8 = &y; loop {} }
    "#,
    );
    assert!(err.contains("&u8") && err.contains("&u16"), "{err}");
}

// ============================================================================
// Addresses of parts of things
// ============================================================================

#[test]
fn a_pointer_to_a_struct_field_writes_that_field() {
    // A local struct is stored inline in the frame, so the field's address is
    // just the slot plus the field's offset.
    let mut e = run(r#"
        const R0: addr = 0x0900;
        const R1: addr = 0x0901;
        struct Point { x: u8, y: u8 }
        #[reset]
        fn main() {
            let pt: Point = Point { x: 1, y: 2 };
            let py: &u8 = &pt.y;
            *py = 9;
            R0 = pt.x;
            R1 = pt.y;
            loop {}
        }
    "#);
    assert_eq!(
        (e.mem(0x0900), e.mem(0x0901)),
        (1, 9),
        "only the addressed field changes"
    );
}

#[test]
fn a_pointer_to_a_static_array_element_writes_that_element() {
    // A static array lives at its own label, so the element address is a
    // compile-time constant rather than a runtime add.
    let mut e = run(r#"
        const R0: addr = 0x0900;
        const R1: addr = 0x0901;
        static BUF: [u8; 4] = [0; 4];
        #[reset]
        fn main() {
            let p: &u8 = &BUF[2];
            *p = 0xC3;
            R0 = BUF[1];
            R1 = BUF[2];
            loop {}
        }
    "#);
    assert_eq!(
        (e.mem(0x0900), e.mem(0x0901)),
        (0, 0xC3),
        "the neighbour is untouched"
    );
}

#[test]
fn a_pointer_to_a_u16_array_element_is_scaled_by_the_element_size() {
    // `&w[2]` on a u16 array is base + 4, not base + 2. An unscaled offset
    // would land halfway through element 1 and still look plausible.
    let mut e = run(r#"
        const R0: addr = 0x0900;
        const R1: addr = 0x0901;
        #[reset]
        fn main() {
            let w: [u16; 4] = [0; 4];
            let p: &u16 = &w[2];
            *p = 0xBEEF;
            R0 = w[1].low;
            R1 = w[2].low;
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0900), 0x00, "element 1 is untouched");
    assert_eq!(e.mem(0x0901), 0xEF, "element 2 got the value");
}

#[test]
fn a_runtime_index_into_address_of_is_rejected_for_now() {
    // Pointer arithmetic on a computed index is what `p[i]` will provide;
    // until then this must say so rather than emit something wrong.
    let err = expect_error(
        r#"
        const OUT: addr = 0x0900;
        #[reset]
        fn main() {
            let buf: [u8; 4] = [0; 4];
            let i: u8 = 2;
            let p: &u8 = &buf[i];
            *p = 1;
            loop {}
        }
    "#,
    );
    assert!(err.contains("constant index"), "{err}");
}
