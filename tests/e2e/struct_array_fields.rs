//! Array fields inside structs.
//!
//! `struct S { a: [u8; 4], b: u8 }` could not be constructed at all — a field
//! wider than two bytes was rejected with "struct field type with size 4 not
//! yet supported" — and `s.a[i]` could not be read, because indexing only
//! understood a bare variable.
//!
//! A two-byte array field was worse than rejected: it fell through to the
//! scalar path and the *pointer* to the literal's data was stored into the
//! field, with a garbage high byte (the scalar path reads Y while an address
//! arrives in X). It compiled, and the field held nonsense.
//!
//! An array field is laid out inline in its owner like any other field, so it
//! has a fixed base address; these check the bytes that base actually reaches.

use crate::common::exec::run;

const OUT: u16 = 0x0900;

#[test]
fn a_struct_with_an_array_field_can_be_built_and_read() {
    let mut e = run(r#"
        const O0: addr = 0x0900;
        const O1: addr = 0x0901;
        const O2: addr = 0x0902;
        const O3: addr = 0x0903;
        struct S { a: [u8; 4], b: u8 }
        #[reset]
        fn main() {
            let s: S = S { a: [10, 20, 30, 40], b: 99 };
            O0 = s.a[0];
            O1 = s.a[2];
            O2 = s.a[3];
            O3 = s.b;
            loop {}
        }
    "#);
    assert_eq!(e.mem(OUT), 10);
    assert_eq!(e.mem(0x0901), 30);
    assert_eq!(e.mem(0x0902), 40);
    assert_eq!(
        e.mem(0x0903),
        99,
        "the scalar after the array is undisturbed"
    );
}

/// The two-byte case, which used to compile and store a pointer.
#[test]
fn a_two_byte_array_field_holds_its_elements_not_a_pointer() {
    let mut e = run(r#"
        const O0: addr = 0x0900;
        const O1: addr = 0x0901;
        const O2: addr = 0x0902;
        struct S { a: [u8; 2], b: u8 }
        #[reset]
        fn main() {
            let s: S = S { a: [17, 34], b: 99 };
            O0 = s.a[0];
            O1 = s.a[1];
            O2 = s.b;
            loop {}
        }
    "#);
    assert_eq!(e.mem(OUT), 17, "not the low byte of a pointer");
    assert_eq!(e.mem(0x0901), 34);
    assert_eq!(e.mem(0x0902), 99);
}

#[test]
fn an_array_field_is_readable_at_a_runtime_index() {
    let mut e = run(r#"
        const O0: addr = 0x0900;
        const O1: addr = 0x0901;
        struct S { a: [u8; 4], b: u8 }
        #[reset]
        fn main() {
            let s: S = S { a: [10, 20, 30, 40], b: 99 };
            let i: u8 = 2;
            O0 = s.a[i];
            let j: u8 = 0;
            O1 = s.a[j];
            loop {}
        }
    "#);
    assert_eq!(e.mem(OUT), 30);
    assert_eq!(e.mem(0x0901), 10);
}

#[test]
fn an_array_field_is_writable() {
    let mut e = run(r#"
        const O0: addr = 0x0900;
        const O1: addr = 0x0901;
        const O2: addr = 0x0902;
        const O3: addr = 0x0903;
        struct S { a: [u8; 4], b: u8 }
        #[reset]
        fn main() {
            let s: S = S { a: [10, 20, 30, 40], b: 99 };
            s.a[1] = 77;
            let i: u8 = 3;
            s.a[i] = 88;
            O0 = s.a[0];
            O1 = s.a[1];
            O2 = s.a[3];
            O3 = s.b;
            loop {}
        }
    "#);
    assert_eq!(e.mem(OUT), 10, "untouched element");
    assert_eq!(e.mem(0x0901), 77, "constant index write");
    assert_eq!(e.mem(0x0902), 88, "runtime index write");
    assert_eq!(e.mem(0x0903), 99, "the write did not run past the array");
}

/// A field before the array shifts its offset, so a base computed without the
/// offset reads the wrong bytes.
#[test]
fn an_array_field_after_a_scalar_is_at_the_right_offset() {
    let mut e = run(r#"
        const O0: addr = 0x0900;
        const O1: addr = 0x0901;
        const O2: addr = 0x0902;
        struct S { lead: u8, a: [u8; 3], trail: u8 }
        #[reset]
        fn main() {
            let s: S = S { lead: 7, a: [10, 20, 30], trail: 9 };
            O0 = s.lead;
            O1 = s.a[0];
            O2 = s.trail;
            loop {}
        }
    "#);
    assert_eq!(e.mem(OUT), 7, "field before the array");
    assert_eq!(e.mem(0x0901), 10, "first array element, not `lead`");
    assert_eq!(e.mem(0x0902), 9, "field after the array");
}

#[test]
fn two_array_fields_in_one_struct_stay_separate() {
    let mut e = run(r#"
        const O0: addr = 0x0900;
        const O1: addr = 0x0901;
        const O2: addr = 0x0902;
        struct S { a: [u8; 3], b: [u8; 3], c: u8 }
        #[reset]
        fn main() {
            let s: S = S { a: [1, 2, 3], b: [11, 12, 13], c: 9 };
            s.a[1] = 77;
            O0 = s.a[1];
            O1 = s.b[1];
            O2 = s.c;
            loop {}
        }
    "#);
    assert_eq!(e.mem(OUT), 77);
    assert_eq!(e.mem(0x0901), 12, "writing `a` must not reach `b`");
    assert_eq!(e.mem(0x0902), 9);
}

/// Two-byte elements are index-scaled, so a missing `ASL` reads half-way into
/// the wrong element.
#[test]
fn a_u16_array_field_scales_its_index() {
    let mut e = run(r#"
        const LO: addr = 0x0900;
        const HI: addr = 0x0901;
        const B: addr = 0x0902;
        struct S { a: [u16; 3], b: u8 }
        #[reset]
        fn main() {
            let s: S = S { a: [100, 300, 500], b: 9 };
            let i: u8 = 1;
            let v: u16 = s.a[i];
            LO = v.low;
            HI = v.high;
            B = s.b;
            loop {}
        }
    "#);
    assert_eq!(e.mem16(OUT), 300, "element 1, whole value");
    assert_eq!(e.mem(0x0902), 9);
}

#[test]
fn a_u16_array_field_is_writable() {
    let mut e = run(r#"
        const LO: addr = 0x0900;
        const HI: addr = 0x0901;
        const B: addr = 0x0902;
        struct S { a: [u16; 3], b: u8 }
        #[reset]
        fn main() {
            let s: S = S { a: [100, 300, 500], b: 9 };
            s.a[1] = 1000 as u16;
            let v: u16 = s.a[1];
            LO = v.low;
            HI = v.high;
            B = s.b;
            loop {}
        }
    "#);
    assert_eq!(e.mem16(OUT), 1000);
    assert_eq!(
        e.mem(0x0902),
        9,
        "the two-byte store stayed inside the array"
    );
}

/// A nested struct whose inner one has the array field: the base is a chain of
/// two offsets.
#[test]
fn an_array_field_of_a_nested_struct_is_reachable() {
    let mut e = run(r#"
        const O0: addr = 0x0900;
        const O1: addr = 0x0901;
        struct Inner { a: [u8; 3], t: u8 }
        struct Outer { lead: u8, inner: Inner }
        #[reset]
        fn main() {
            let s: Outer = Outer { lead: 5, inner: Inner { a: [10, 20, 30], t: 9 } };
            O0 = s.inner.a[2];
            s.inner.a[0] = 77;
            O1 = s.inner.a[0];
            loop {}
        }
    "#);
    assert_eq!(e.mem(OUT), 30);
    assert_eq!(e.mem(0x0901), 77);
}

/// The fill form has its own initializer path.
#[test]
fn an_array_field_accepts_the_fill_initializer() {
    let mut e = run(r#"
        const O0: addr = 0x0900;
        const O1: addr = 0x0901;
        const O2: addr = 0x0902;
        struct S { a: [u8; 4], b: u8 }
        #[reset]
        fn main() {
            let s: S = S { a: [7; 4], b: 99 };
            O0 = s.a[0];
            O1 = s.a[3];
            O2 = s.b;
            loop {}
        }
    "#);
    assert_eq!(e.mem(OUT), 7);
    assert_eq!(e.mem(0x0901), 7);
    assert_eq!(e.mem(0x0902), 99);
}

/// A struct with an array field, summed through a loop — the shape this exists
/// for (driver state, a sprite row, a small ring buffer).
#[test]
fn an_array_field_can_be_looped_over() {
    let mut e = run(r#"
        const O0: addr = 0x0900;
        struct S { a: [u8; 4], b: u8 }
        #[reset]
        fn main() {
            let s: S = S { a: [1, 2, 3, 4], b: 99 };
            let acc: u8 = 0;
            for i in 0..4 { acc = acc + s.a[i]; }
            O0 = acc;
            loop {}
        }
    "#);
    assert_eq!(e.mem(OUT), 10);
}

/// A `static` with an array field already worked through the const-data path;
/// pinned so the new inline path does not displace it.
#[test]
fn a_static_struct_with_an_array_field_still_works() {
    let mut e = run(r#"
        const O0: addr = 0x0900;
        const O1: addr = 0x0901;
        struct S { a: [u8; 4], b: u8 }
        static G: S = S { a: [10, 20, 30, 40], b: 99 };
        #[reset]
        fn main() {
            O0 = G.a[2];
            O1 = G.b;
            loop {}
        }
    "#);
    assert_eq!(e.mem(OUT), 30);
    assert_eq!(e.mem(0x0901), 99);
}
