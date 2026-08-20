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
fn the_address_of_an_element_at_a_run_time_index() {
    // This used to be refused — the address of an element needed a constant
    // index, because the offset was folded into the base at compile time.
    // It is the general chain now: the base is computed, the scaled index is
    // added to it, and a constant chain still folds to two immediate loads.
    let mut e = run(r#"
        const R0: addr = 0x0900;
        const R1: addr = 0x0901;
        #[reset]
        fn main() {
            let buf: [u8; 4] = [0; 4];
            let i: u8 = 2;
            let p: &u8 = &buf[i];
            *p = 7;
            R0 = buf[1];
            R1 = buf[2];
            loop {}
        }
    "#);
    assert_eq!(
        (e.mem(0x0900), e.mem(0x0901)),
        (0, 7),
        "the write lands on the element the index named, and nowhere else"
    );
}

// ============================================================================
// `p[i]` — indexing through a pointer
// ============================================================================

#[test]
fn indexing_a_pointer_reads_the_element() {
    // `&buf` decays to a pointer at the first element, so `p[2]` and `buf[2]`
    // must name the same byte. The pointer carries no length; that is the whole
    // difference from a slice.
    let mut e = run(r#"
        const R0: addr = 0x0900;
        const R1: addr = 0x0901;
        #[reset]
        fn main() {
            let buf: [u8; 4] = [0; 4];
            buf[0] = 10;
            buf[2] = 30;
            let p: &u8 = &buf;
            R0 = p[0];
            R1 = p[2];
            loop {}
        }
    "#);
    assert_eq!((e.mem(0x0900), e.mem(0x0901)), (10, 30));
}

#[test]
fn indexing_a_pointer_with_a_runtime_index_works() {
    // The index is an ordinary expression evaluated into Y — this is the shape
    // `&buf[i]` could not express, and the reason `p + n` is not needed.
    let mut e = run(r#"
        const R0: addr = 0x0900;
        #[reset]
        fn main() {
            let buf: [u8; 4] = [0; 4];
            buf[3] = 77;
            let p: &u8 = &buf;
            let i: u8 = 3;
            R0 = p[i];
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0900), 77);
}

#[test]
fn writing_through_a_pointer_index_lands_in_the_array() {
    let mut e = run(r#"
        const R0: addr = 0x0900;
        const R1: addr = 0x0901;
        #[reset]
        fn main() {
            let buf: [u8; 4] = [0; 4];
            let p: &u8 = &buf;
            p[1] = 0xAB;
            R0 = buf[0];
            R1 = buf[1];
            loop {}
        }
    "#);
    assert_eq!(
        (e.mem(0x0900), e.mem(0x0901)),
        (0, 0xAB),
        "only element 1 changed"
    );
}

#[test]
fn a_u16_pointer_index_is_scaled_by_the_element_size() {
    // `p[2]` on a `&u16` is base + 4. Forgetting to scale writes over element 1
    // and still looks plausible in a hex dump.
    let mut e = run(r#"
        const R0: addr = 0x0900;
        const R1: addr = 0x0901;
        const R2: addr = 0x0902;
        #[reset]
        fn main() {
            let w: [u16; 4] = [0; 4];
            let p: &u16 = &w;
            p[2] = 0xBEEF;
            R0 = w[1].low;
            R1 = w[2].low;
            R2 = w[2].high;
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0900), 0x00, "element 1 is untouched");
    assert_eq!((e.mem(0x0901), e.mem(0x0902)), (0xEF, 0xBE));
}

#[test]
fn a_callee_can_fill_a_buffer_it_was_handed() {
    // The shape the whole feature exists for: a driver writing into a caller's
    // buffer. Before pointers this had to own a `static` or take a bare `u16`
    // the compiler could not check.
    let mut e = run(r#"
        const R0: addr = 0x0900;
        const R1: addr = 0x0901;
        const R2: addr = 0x0902;
        fn fill(dest: &u8, n: u8, v: u8) {
            let i: u8 = 0;
            while i < n {
                dest[i] = v;
                i = i + 1;
            }
        }
        #[reset]
        fn main() {
            let buf: [u8; 8] = [0; 8];
            fill(&buf, 3, 0x5A);
            R0 = buf[0];
            R1 = buf[2];
            R2 = buf[3];
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0900), 0x5A);
    assert_eq!(e.mem(0x0901), 0x5A);
    assert_eq!(e.mem(0x0902), 0x00, "the fill stopped at n");
}

#[test]
fn a_pointer_into_a_static_array_indexes_correctly() {
    // A `static` array lives inline at its own label, so `&ARR` is a
    // compile-time constant rather than a slot to load.
    let mut e = run(r#"
        const R0: addr = 0x0900;
        static ARR: [u8; 4] = [0; 4];
        #[reset]
        fn main() {
            let p: &u8 = &ARR;
            p[2] = 0x11;
            R0 = ARR[2];
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0900), 0x11);
}

// ============================================================================
// `p.field` — auto-deref on a pointer to a struct
// ============================================================================

#[test]
fn a_field_can_be_read_through_a_pointer() {
    // A struct *parameter* already holds an address in its slot, so `&Struct`
    // takes exactly the same indirect path. Making them differ would be
    // gratuitous, which is why `p.field` auto-derefs rather than needing
    // `(*p).field`.
    let mut e = run(r#"
        const R0: addr = 0x0900;
        const R1: addr = 0x0901;
        struct Dev { status: u8, count: u16 }
        #[reset]
        fn main() {
            let d: Dev = Dev { status: 7, count: 0x1234 };
            let p: &Dev = &d;
            R0 = p.status;
            R1 = p.count.high;
            loop {}
        }
    "#);
    assert_eq!((e.mem(0x0900), e.mem(0x0901)), (7, 0x12));
}

#[test]
fn a_field_can_be_written_through_a_pointer() {
    let mut e = run(r#"
        const R0: addr = 0x0900;
        const R1: addr = 0x0901;
        struct Dev { status: u8, count: u16 }
        #[reset]
        fn main() {
            let d: Dev = Dev { status: 0, count: 0 };
            let p: &Dev = &d;
            p.status = 9;
            p.count = 0xBEEF;
            R0 = d.status;
            R1 = d.count.high;
            loop {}
        }
    "#);
    assert_eq!((e.mem(0x0900), e.mem(0x0901)), (9, 0xBE));
}

#[test]
fn a_struct_pointer_survives_being_passed_down() {
    // The frame-colouring invariant again, this time for a pointer to an
    // aggregate: the callee's frame cannot overlap the caller's local.
    let mut e = run(r#"
        const R0: addr = 0x0900;
        struct Dev { status: u8 }
        fn bump(p: &Dev) { p.status = p.status + 1; }
        #[reset]
        fn main() {
            let d: Dev = Dev { status: 40 };
            bump(&d);
            bump(&d);
            R0 = d.status;
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0900), 42);
}

#[test]
fn a_pointer_to_a_static_struct_reads_and_writes_the_static() {
    let mut e = run(r#"
        const R0: addr = 0x0900;
        struct Dev { status: u8 }
        static D: Dev = Dev { status: 3 };
        #[reset]
        fn main() {
            let p: &Dev = &D;
            p.status = p.status + 4;
            R0 = D.status;
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0900), 7);
}

#[test]
fn taking_the_address_of_a_field_through_a_pointer_still_works() {
    // `&p.field` composes: the pointer is the base, the offset is added to it.
    let mut e = run(r#"
        const R0: addr = 0x0900;
        struct Dev { status: u8, flag: u8 }
        fn set_flag(d: &Dev) { let f: &u8 = &d.flag; *f = 1; }
        #[reset]
        fn main() {
            let d: Dev = Dev { status: 0, flag: 0 };
            set_flag(&d);
            R0 = d.flag;
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0900), 1);
}

#[test]
fn a_field_chain_follows_through_a_pointer() {
    // This was refused, and the refusal named the pointer and told you to bind
    // an intermediate: `resolve_static_struct_lvalue` computes a compile-time
    // address and there is none behind a pointer. The chain is computed at run
    // time now — the base of what the object names, plus the offsets along the
    // way — so one level of auto-deref is no longer the limit.
    let mut e = run(r#"
        const R0: addr = 0x0900;
        const R1: addr = 0x0901;
        struct Inner { pad: u8, v: u8 }
        struct Outer { tag: u8, inner: Inner }
        fn peek(p: &Outer) -> u8 { return p.inner.v; }
        fn poke(p: &Outer, x: u8) { p.inner.v = x; }
        #[reset]
        fn main() {
            let o: Outer = Outer { tag: 9, inner: Inner { pad: 0, v: 1 } };
            R0 = peek(&o);
            poke(&o, 42);
            R1 = o.inner.v;
            loop {}
        }
    "#);
    assert_eq!(
        (e.mem(0x0900), e.mem(0x0901)),
        (1, 42),
        "two levels deep, read and written, with the offsets summed"
    );
}

// ============================================================================
// Codegen shape
// ============================================================================

#[test]
fn indexing_a_pointer_uses_indirect_indexed_addressing() {
    // The load-bearing detail: `p[i]` must go through `(zp),Y` on the *slot*,
    // reading the pointer it holds. Absolute-indexed on the slot's address
    // would read the pointer bytes themselves as data.
    let asm = compile_success(
        r#"
        const R0: addr = 0x0900;
        fn fetch(p: &u8, i: u8) -> u8 { return p[i]; }
        #[reset]
        fn main() { let b: [u8; 2] = [0; 2]; R0 = fetch(&b, 1); loop {} }
    "#,
    );
    assert!(
        asm.contains("),Y"),
        "expected an indirect-indexed load:\n{asm}"
    );
}

// ============================================================================
// No arithmetic on pointers
// ============================================================================

#[test]
fn adding_two_pointers_is_rejected() {
    // This has to be checked explicitly. `check_binary`'s compatibility gate is
    // `left_ty == right_ty`, so two `&u8`s pass it and a 16-bit add is emitted
    // using the A:Y convention on values that arrived in A:X — a wrong answer
    // with no diagnostic anywhere.
    let err = expect_error(
        r#"
        const R0: addr = 0x0900;
        static A: u8 = 0;
        static B: u8 = 0;
        #[reset]
        fn main() { let p: &u8 = &A; let q: &u8 = &B; let r: &u8 = p + q; R0 = *r; loop {} }
    "#,
    );
    assert!(
        err.contains("p[i]"),
        "the alternative should be named: {err}"
    );
}

#[test]
fn adding_an_integer_to_a_pointer_is_rejected() {
    // `p + 1` is the C spelling and the one most likely to be reached for.
    // Indexing is the supported form because it scales by the element width,
    // which a raw byte offset does not.
    let err = expect_error(
        r#"
        const R0: addr = 0x0900;
        static A: u8 = 0;
        #[reset]
        fn main() { let p: &u8 = &A; let q: &u8 = p + 1; R0 = *q; loop {} }
    "#,
    );
    assert!(err.contains("p[i]"), "{err}");
}

#[test]
fn pointers_compare_directly_and_through_a_u16_cast() {
    // `p == q` now compares the two-byte address directly (a dedicated A:X
    // sequence); the older `p as u16 == q as u16` form still works too.
    assert_eq!(
        run(r#"
            const R0: addr = 0x0900;
            static A: u8 = 0;
            static B: u8 = 0;
            #[reset]
            fn main() {
                let p: &u8 = &A;
                let q: &u8 = &A;
                let r: &u8 = &B;
                let same: u8 = 0;
                let diff: u8 = 0;
                if p == q { same = 1; }               // direct
                if p as u16 == r as u16 { diff = 1; } // via cast
                R0 = same * 2 + diff;
                loop {}
            }
        "#)
        .mem(0x0900),
        2
    );
}

#[test]
fn a_read_after_an_indirect_store_is_not_elided() {
    // An indirect store is opaque: `STA ($36),Y` can land on any zero-page
    // byte, so it invalidates every cached register belief. Without that, the
    // second read of `x` would be optimised away as "already in A" and report
    // the value from before the write.
    let mut e = run(r#"
        const R0: addr = 0x0900;
        const R1: addr = 0x0901;
        #[reset]
        fn main() {
            let x: u8 = 5;
            let p: &u8 = &x;
            R0 = x;
            *p = 9;
            R1 = x;
            loop {}
        }
    "#);
    assert_eq!((e.mem(0x0900), e.mem(0x0901)), (5, 9));
}

#[test]
fn a_read_after_an_indirect_indexed_store_is_not_elided() {
    // The same for `p[i] = v`, which goes through the same opaque store.
    let mut e = run(r#"
        const R0: addr = 0x0900;
        const R1: addr = 0x0901;
        #[reset]
        fn main() {
            let buf: [u8; 2] = [0; 2];
            buf[0] = 3;
            let p: &u8 = &buf;
            R0 = buf[0];
            p[0] = 8;
            R1 = buf[0];
            loop {}
        }
    "#);
    assert_eq!((e.mem(0x0900), e.mem(0x0901)), (3, 8));
}

#[test]
fn a_pointer_store_after_a_call_reloads_y() {
    // `STA (zp),Y` needs Y at the byte offset, and the peephole used to assume
    // Y survived a `JSR` — "JSR/RTS don't necessarily change Y on 6502", which
    // is true of the instruction and false of every function this compiler
    // emits. The second store's `LDY #$00` was dropped and the write landed at
    // the callee's leftover offset instead.
    let mut e = run(r#"
        const R0: addr = 0x0900;
        const R1: addr = 0x0901;
        static A: u8 = 0;
        static B: u8 = 0;
        fn scribble(v: u8) -> u8 {
            let buf: [u8; 8] = [0; 8];
            buf[7] = v;
            return buf[7];
        }
        #[reset]
        fn main() {
            let p: &u8 = &A;
            *p = 0x11;
            R1 = scribble(9);
            *p = 0x33;
            R0 = A;
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0900), 0x33, "the second store reached A, not A+Y");
    assert_eq!(e.mem(0x0901), 9);
}

// ---------------------------------------------------------------------------
// Pointer equality: == / != compare the two-byte address
// ---------------------------------------------------------------------------

#[test]
fn two_pointers_to_the_same_variable_are_equal() {
    let mut e = run(r#"
        const OUT: addr = 0x0900;
        #[reset]
        fn main() {
            let x: u8 = 7;
            let p: &u8 = &x;
            let q: &u8 = &x;
            OUT = 0;
            if p == q { OUT = 1; }
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0900), 1, "&x == &x");
}

#[test]
fn pointers_to_different_variables_are_not_equal() {
    let mut e = run(r#"
        const OUT: addr = 0x0900;
        #[reset]
        fn main() {
            let x: u8 = 1;
            let y: u8 = 2;
            let p: &u8 = &x;
            let q: &u8 = &y;
            OUT = 5;
            if p != q { OUT = 9; }
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0900), 9, "&x != &y");
}

#[test]
fn a_pointer_can_be_null_checked_against_a_cast_zero() {
    // The linked-list null check the feature is for: `p == 0 as &u8`.
    let eval = |init: &str, expect: u8| {
        let src = format!(
            r#"
            const OUT: addr = 0x0900;
            #[reset]
            fn main() {{
                let x: u8 = 42;
                {init}
                OUT = 0;
                if p == 0 as &u8 {{ OUT = 1; }} else {{ OUT = 2; }}
                loop {{}}
            }}
        "#
        );
        assert_eq!(run(&src).mem(0x0900), expect);
    };
    eval("let p: &u8 = 0 as &u8;", 1); // null -> then branch
    eval("let p: &u8 = &x;", 2); // non-null -> else branch
}

#[test]
fn pointer_ordering_and_arithmetic_stay_rejected() {
    // Only == / != opened up; `<` and `+` on pointers remain errors.
    let lt = expect_error(
        r#"
        const OUT: addr = 0x0900;
        #[reset]
        fn main() { let x: u8 = 1; let p: &u8 = &x; let q: &u8 = &x; if p < q { OUT = 1; } loop {} }
    "#,
    );
    assert!(lt.contains("Lt") || lt.contains("compare"), "{lt}");
    let add = expect_error(
        r#"
        const OUT: addr = 0x0900;
        #[reset]
        fn main() { let x: u8 = 1; let p: &u8 = &x; let q: &u8 = &x; let r: &u8 = p + q; loop {} }
    "#,
    );
    assert!(add.contains("Add") || add.contains("index"), "{add}");
}

#[test]
fn a_write_through_a_pointer_to_a_pointer_keeps_both_address_bytes() {
    // `*pp = q` writes a two-byte pointer through `pp: &&u8`. The store used to
    // treat only a u16 pointee as two bytes, so a pointer pointee dropped its
    // high byte — and a pointer's high byte lives in X, not Y. B sits at an
    // address whose high byte differs from A's, so a lost byte reads the wrong
    // static.
    let mut e = run(r#"
        const OUT: addr = 0x0900;
        static A: u8 = 10;
        static B: u8 = 20;
        #[reset]
        fn main() {
            let p: &u8 = &A;
            let pp: &&u8 = &p;
            *pp = &B;      // redirect p to B, through pp
            OUT = *p;      // reads B (20) only if the full address survived
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0900), 20, "the redirected pointer must reach B");
}
