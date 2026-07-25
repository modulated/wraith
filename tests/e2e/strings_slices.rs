//! Runtime (emulator) tests for string and slice ergonomics: array `.len`,
//! `.len`-driven iteration, and string equality.

use crate::common::exec::run;

// ---------------------------------------------------------------------------
// Array .len (compile-time constant) and idiomatic iteration.
// ---------------------------------------------------------------------------

#[test]
fn array_len_constant() {
    let mut e = run(r#"
        const LEN: addr = 0x0400;
        #[reset]
        fn main() {
            let a: [u8; 5] = [1, 2, 3, 4, 5];
            LEN = a.len as u8;
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0400), 5, "a.len is the array size");
}

#[test]
fn for_index_over_array_len() {
    let mut e = run(r#"
        const OUT: addr = 0x0400;
        #[reset]
        fn main() {
            let a: [u8; 5] = [10, 20, 30, 40, 50];
            let sum: u8 = 0;
            for i in 0..a.len {
                sum = sum + a[i as u8];
            }
            OUT = sum;
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0400), 150, "sum via `for i in 0..a.len`");
}
