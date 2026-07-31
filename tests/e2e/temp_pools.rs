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
fn foreach_over_a_string_survives_a_multiply_in_the_body() {
    // The loop counter was kept only in X, and a u8 multiply in the body uses X
    // as its 8-iteration shift counter — zeroing the loop counter every pass, so
    // the loop never terminated. The counter now lives in a hidden zero-page
    // slot, reloaded into X at each loop head, so the body may clobber X freely.
    // "111" folds to a number: 0*3+1=1, 1*3+1=4, 4*3+1=13.
    let mut e = run(r#"
        const OUT: addr = 0x0900;
        #[reset]
        fn main() {
            let s: str = "111";
            let acc: u8 = 0;
            for c in s {
                acc = acc * 3 + (c as u8 - 48);
            }
            OUT = acc;
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0900), 13);
}

#[test]
fn foreach_over_an_array_survives_a_multiply_in_the_body() {
    // Same corruption, array iterable: the compile-time-sized counter is in X
    // and the body's multiply zeroed it. Sum of 1..=4 each tripled: 3+6+9+12=30.
    let mut e = run(r#"
        const OUT: addr = 0x0900;
        #[reset]
        fn main() {
            let a: [u8; 4] = [1, 2, 3, 4];
            let acc: u8 = 0;
            for v in a {
                acc = acc + v * 3;
            }
            OUT = acc;
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0900), 30);
}

#[test]
fn indexed_foreach_survives_a_multiply_in_the_body() {
    // The `for (i, c)` form stores the index from X at the loop head; the fix
    // reloads X from the counter slot there, so a body multiply that clobbers X
    // does not corrupt either the index or the iteration. dst[i] = i*2 over 4
    // chars writes 0,2,4,6.
    let mut e = run(r#"
        const D0: addr = 0x0900; const D1: addr = 0x0901;
        const D2: addr = 0x0902; const D3: addr = 0x0903;
        #[reset]
        fn main() {
            let s: str = "wxyz";
            let dst: [u8; 4] = [0; 4];
            for (i, c) in s {
                dst[i] = i * 2;
            }
            D0 = dst[0]; D1 = dst[1]; D2 = dst[2]; D3 = dst[3];
            loop {}
        }
    "#);
    assert_eq!(
        (e.mem(0x0900), e.mem(0x0901), e.mem(0x0902), e.mem(0x0903)),
        (0, 2, 4, 6)
    );
}

#[test]
fn nested_foreach_over_strings_keeps_both_counters() {
    // The inner loop's counter is also X; without a per-loop memory counter the
    // inner loop would leave X at the inner length and the outer INX would
    // resume from there. Each of the 2 outer chars pairs with 3 inner chars, so
    // the body runs 6 times.
    let mut e = run(r#"
        const OUT: addr = 0x0900;
        #[reset]
        fn main() {
            let outer: str = "AB";
            let inner: str = "xyz";
            let n: u8 = 0;
            for c in outer {
                for d in inner {
                    n = n + 1;
                }
            }
            OUT = n;
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0900), 6);
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
                arr[1] = c as u8;
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
            OUT = s[i * s.len as u8 - i] as u8;
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
            let c: u8 = t[0] as u8;
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
