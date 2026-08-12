//! Struct literals in expression position, where the fields are computed.
//!
//! A constant struct literal is emitted as bytes in the CODE section and
//! evaluates to a pointer at them. A computed one has nothing to point at until
//! it runs, so sema reserves a block of RAM per literal site and codegen
//! assembles the fields into it. These check the values that come out, because
//! the failure mode of getting it wrong is a plausible-looking pointer at the
//! wrong bytes.

use crate::common::exec::run;

#[test]
fn a_returned_struct_with_computed_fields_carries_its_values() {
    // The spec's `move_point`, which codegen used to reject outright.
    let mut e = run(r#"
        const OX: addr = 0x0900;
        const OY: addr = 0x0901;
        struct Point { x: u8, y: u8 }
        fn move_point(p: Point, dx: u8, dy: u8) -> Point {
            return Point { x: p.x + dx, y: p.y + dy };
        }
        #[reset]
        fn main() {
            let a: Point = Point { x: 10, y: 20 };
            let b: Point = move_point(a, 3, 4);
            OX = b.x;
            OY = b.y;
            loop {}
        }
    "#);
    assert_eq!((e.mem(0x0900), e.mem(0x0901)), (13, 24));
}

#[test]
fn a_returned_struct_does_not_disturb_its_source() {
    // `move_point` reads `p` while building the result; if the result block
    // aliased the parameter the reads would see half-updated values.
    let mut e = run(r#"
        const AX: addr = 0x0900;
        const AY: addr = 0x0901;
        const BX: addr = 0x0902;
        const BY: addr = 0x0903;
        struct Point { x: u8, y: u8 }
        fn move_point(p: Point, dx: u8, dy: u8) -> Point {
            return Point { x: p.x + dx, y: p.y + dy };
        }
        #[reset]
        fn main() {
            let a: Point = Point { x: 10, y: 20 };
            let b: Point = move_point(a, 3, 4);
            AX = a.x; AY = a.y;
            BX = b.x; BY = b.y;
            loop {}
        }
    "#);
    assert_eq!((e.mem(0x0900), e.mem(0x0901)), (10, 20), "source unchanged");
    assert_eq!((e.mem(0x0902), e.mem(0x0903)), (13, 24), "result correct");
}

#[test]
fn a_computed_struct_literal_works_as_a_call_argument() {
    let mut e = run(r#"
        const OUT: addr = 0x0900;
        struct P { x: u8, y: u8 }
        fn sum(p: P) -> u8 { return p.x + p.y; }
        #[reset]
        fn main() {
            let a: u8 = 5;
            OUT = sum(P { x: a + 1, y: a + 2 });
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0900), 13, "6 + 7");
}

#[test]
fn two_distinct_literal_sites_do_not_share_a_block() {
    // Each literal site gets its own reservation, so two live results must not
    // alias — the classic failure of a single shared scratch buffer.
    let mut e = run(r#"
        const AX: addr = 0x0900;
        const BX: addr = 0x0901;
        struct P { x: u8, y: u8 }
        fn one(n: u8) -> P { return P { x: n + 1, y: 0 }; }
        fn two(n: u8) -> P { return P { x: n + 100, y: 0 }; }
        #[reset]
        fn main() {
            let a: P = one(1);
            let b: P = two(1);
            AX = a.x;
            BX = b.x;
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0900), 2, "one(1).x");
    assert_eq!(e.mem(0x0901), 101, "two(1).x");
}

#[test]
fn a_reassignment_from_a_computed_literal_updates_every_field() {
    let mut e = run(r#"
        const OX: addr = 0x0900;
        const OY: addr = 0x0901;
        struct P { x: u8, y: u8 }
        #[reset]
        fn main() {
            let a: u8 = 7;
            let p: P = P { x: 0, y: 0 };
            p = P { x: a + 1, y: a + 2 };
            OX = p.x;
            OY = p.y;
            loop {}
        }
    "#);
    assert_eq!((e.mem(0x0900), e.mem(0x0901)), (8, 9));
}

#[test]
fn a_computed_u16_field_keeps_both_bytes() {
    // The two-byte store path writes A then the high register; a computed
    // literal must use the same convention as a constant one.
    let mut e = run(r#"
        const LO: addr = 0x0900;
        const HI: addr = 0x0901;
        const B: addr = 0x0902;
        struct W { a: u16, b: u8 }
        fn mk(n: u16) -> W { return W { a: n + 1, b: 9 }; }
        #[reset]
        fn main() {
            let w: W = mk(300);
            LO = w.a.low;
            HI = w.a.high;
            B = w.b;
            loop {}
        }
    "#);
    assert_eq!(e.mem16(0x0900), 301, "300 + 1 keeps its high byte");
    assert_eq!(e.mem(0x0902), 9);
}

#[test]
fn a_constant_literal_still_goes_to_rom() {
    // The optimization that makes constant literals free must survive: no RAM
    // block, bytes in the code stream. Asserted on the emitted assembly because
    // that is the property — the values are checked by the tests above.
    let asm = crate::common::compile_success(
        r#"
        struct P { x: u8, y: u8 }
        fn mk() -> P { return P { x: 7, y: 9 }; }
        #[reset]
        fn main() { let p: P = mk(); loop {} }
        "#,
    );
    crate::common::assert_asm_contains(&asm, ".BYTE $07");
    crate::common::assert_asm_contains(&asm, ".BYTE $09");
}

#[test]
fn a_recursive_struct_return_unwinds_correctly() {
    // Each level builds its result in the same site's block, but copies the
    // callee's result into a frame local before overwriting it. Frames are
    // saved across recursion; this checks the struct block's reuse does not
    // break the same way.
    let mut e = run(r#"
        const OUT: addr = 0x0900;
        struct P { x: u8, y: u8 }
        fn countdown(n: u8) -> P {
            if n == 0 { return P { x: 0, y: 0 }; }
            let inner: P = countdown(n - 1);
            return P { x: inner.x + 1, y: 0 };
        }
        #[reset]
        fn main() {
            let p: P = countdown(5);
            OUT = p.x;
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0900), 5, "five levels each add 1");
}
