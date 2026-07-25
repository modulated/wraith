//! Runtime (emulator) tests for standard-library primitives that require real
//! execution: block memory copy/fill (std/mem.wr) and the math helpers
//! widening-multiply, divmod, and the PRNG (std/math.wr).

use crate::common::exec::run;

// ---------------------------------------------------------------------------
// std/mem.wr — memcpy / memcpy16 / memset / memset16
// ---------------------------------------------------------------------------

#[test]
fn memcpy_copies_bytes_indirect() {
    // Fill a source region, copy it elsewhere, and confirm the destination
    // holds the same bytes — i.e. the copy dereferences the pointers.
    let mut e = run(r#"
        import { memcpy } from "std/mem.wr";
        const S0: addr = 0x0500;
        const S1: addr = 0x0501;
        const S2: addr = 0x0502;
        const S3: addr = 0x0503;
        #[reset]
        fn main() {
            S0 = 0x11; S1 = 0x22; S2 = 0x33; S3 = 0x44;
            memcpy(0x0600, 0x0500, 4);
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0600), 0x11);
    assert_eq!(e.mem(0x0601), 0x22);
    assert_eq!(e.mem(0x0602), 0x33);
    assert_eq!(e.mem(0x0603), 0x44);
}

#[test]
fn memcpy16_copies_across_page_boundary() {
    // Copy a run that straddles a page boundary (0x05FE..0x0601 -> 0x06FE..),
    // which only works if the 16-bit pointer/counter logic carries correctly.
    let mut e = run(r#"
        import { memcpy16 } from "std/mem.wr";
        const P0: addr = 0x05FE;
        const P1: addr = 0x05FF;
        const P2: addr = 0x0600;
        const P3: addr = 0x0601;
        #[reset]
        fn main() {
            P0 = 0xDE; P1 = 0xAD; P2 = 0xBE; P3 = 0xEF;
            memcpy16(0x06FE, 0x05FE, 4);
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x06FE), 0xDE, "byte before page boundary");
    assert_eq!(e.mem(0x06FF), 0xAD, "last byte of page");
    assert_eq!(e.mem(0x0700), 0xBE, "first byte of next page");
    assert_eq!(e.mem(0x0701), 0xEF, "second byte of next page");
}

#[test]
fn memset_fills_region() {
    let mut e = run(r#"
        import { memset } from "std/mem.wr";
        #[reset]
        fn main() {
            memset(0x0600, 0x5A, 5);
            loop {}
        }
    "#);
    for i in 0..5u16 {
        assert_eq!(e.mem(0x0600 + i), 0x5A, "byte {i}");
    }
    // One past the end must be untouched (still 0).
    assert_eq!(e.mem(0x0605), 0x00, "one past end untouched");
}

#[test]
fn memset16_fills_across_page_boundary() {
    let mut e = run(r#"
        import { memset16 } from "std/mem.wr";
        #[reset]
        fn main() {
            memset16(0x05FE, 0x99, 4);
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x05FE), 0x99);
    assert_eq!(e.mem(0x05FF), 0x99);
    assert_eq!(e.mem(0x0600), 0x99);
    assert_eq!(e.mem(0x0601), 0x99);
    assert_eq!(e.mem(0x0602), 0x00, "one past end untouched");
}
