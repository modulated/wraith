//! Zero-page temp-pool discipline.
//!
//! The $F0-$F3 "high pool" was both allocated through `TempAllocator` and
//! written directly by string/struct/index paths that never reserved it, and
//! every pool-exhaustion path fell back to a hardcoded address — which was by
//! construction a slot some live value already occupied. Each test below is a
//! miscompile from that family: a value silently replaced by another live
//! value's bytes.
//!
//! The conventions are now enforced: staging goes through the allocator,
//! exhaustion is a compile error or a software-stack spill, and the ForEach
//! string/slice staging is re-done at every loop head so it never lives
//! across the body.

use crate::common::exec::run;

#[test]
fn nested_u16_adds_do_not_reuse_a_live_save_slot() {
    // Two live u16 left-operand saves fill the 4-byte high pool; the third
    // nested add used to fall back to a hardcoded $F2 — a live ancestor's own
    // slot — and evaluated (e+f) twice, (c+d) never: 291 instead of 255.
    let mut e = run(r#"
        const LO: addr = 0x0900;
        const HI: addr = 0x0901;
        #[reset]
        fn main() {
            let a: u16 = 1;
            let b: u16 = 2;
            let c: u16 = 4;
            let d: u16 = 8;
            let f: u16 = 16;
            let g: u16 = 32;
            let h: u16 = 64;
            let i: u16 = 128;
            let r: u16 = (a + b) + ((c + d) + ((f + g) + (h + i)));
            LO = r.low;
            HI = r.high;
            loop {}
        }
    "#);
    assert_eq!(e.mem16(0x0900), 255);
}

#[test]
fn foreach_over_a_string_survives_an_index_assignment_in_the_body() {
    // The string pointer was staged at $F0/$F1 for the whole loop, and the
    // body's `arr[1] = c` parked its value at exactly $F0 — iteration 2 read
    // through a wild pointer. Staging now happens at every loop head.
    let mut e = run(r#"
        const OUT: addr = 0x0900;
        #[reset]
        fn main() {
            let s: str = "ABC";
            let arr: [u8; 2] = [0, 0];
            let sum: u8 = 0;
            for c in s {
                arr[1] = c;
                sum = sum + arr[1];
            }
            OUT = sum;
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0900), 65 + 66 + 67);
}

#[test]
fn string_index_with_len_and_multiply_in_the_index() {
    // `s[i * s.len as u8 - i]`: the string pointer staged for `s[...]` was
    // overwritten by `.len`'s own staging and by the u8 multiply's
    // multiplicand, both hardcoded at $F0. 1 * 4 - 1 = 3, and s[3] of "cdef"
    // is 'f'.
    let mut e = run(r#"
        const OUT: addr = 0x0900;
        #[reset]
        fn main() {
            let s: str = "cdef";
            let i: u8 = 1;
            OUT = s[i * s.len as u8 - i];
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0900), b'f');
}

#[test]
fn index_assignment_with_len_in_the_index() {
    // The value (42) was parked at $F0 while the index evaluated, and `.len`'s
    // hardcoded staging overwrote it with the string pointer's low byte:
    // arr[3] got $D0-something instead of 42.
    let mut e = run(r#"
        const OUT: addr = 0x0900;
        #[reset]
        fn main() {
            let s: str = "hey";
            let arr: [u8; 5] = [0; 5];
            arr[s.len as u8] = 42;
            OUT = arr[3];
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0900), 42);
}

#[test]
fn array_of_struct_field_write_with_a_call_in_the_index() {
    // The value (42) was parked at $F4 without reserving it, and `ident`'s
    // argument staging was handed the same $F4: ps[1].x got 1.
    let mut e = run(r#"
        const OUT: addr = 0x0900;
        struct P { x: u8, y: u8 }
        static ps: [P; 3] = [P { x: 0, y: 0 }, P { x: 0, y: 0 }, P { x: 0, y: 0 }];
        fn ident(n: u8) -> u8 { return n; }
        #[reset]
        fn main() {
            ps[ident(1)].x = 42;
            OUT = ps[1].x;
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0900), 42);
}

#[test]
fn string_equality_with_a_call_on_the_right() {
    // The left pointer was staged at $F0/$F1 across the right-hand call, and
    // the callee's own string indexing rewrote $F0 (incremented, even): the
    // comparison then read the callee's leftover pointer, and "AB" != "AB".
    // The left pointer now crosses the call on the software stack.
    let mut e = run(r#"
        const OUT: addr = 0x0900;
        fn mk() -> str {
            let t: str = "AB";
            let c: u8 = t[0];
            return t;
        }
        #[reset]
        fn main() {
            let s: str = "AB";
            if s == mk() {
                OUT = 1;
            }
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0900), 1);
}
