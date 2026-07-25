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

// ---------------------------------------------------------------------------
// String equality / inequality.
// ---------------------------------------------------------------------------

#[test]
fn string_equality() {
    // Returns 1 when the `==` branch is taken, else 2.
    let eq = |a: &str, b: &str| {
        let src = format!(
            r#"
            const OUT: addr = 0x0400;
            #[reset]
            fn main() {{
                let a: str = "{a}";
                let b: str = "{b}";
                if a == b {{ OUT = 1; }} else {{ OUT = 2; }}
                loop {{}}
            }}
        "#
        );
        run(&src).mem(0x0400)
    };
    assert_eq!(eq("hello", "hello"), 1, "identical strings are equal");
    assert_eq!(eq("hello", "world"), 2, "same length, different bytes");
    assert_eq!(eq("hi", "hiya"), 2, "different lengths are not equal");
    assert_eq!(eq("", ""), 1, "empty strings are equal");
    assert_eq!(eq("abc", "abd"), 2, "differ in last byte");
}

#[test]
fn string_inequality() {
    let ne = |a: &str, b: &str| {
        let src = format!(
            r#"
            const OUT: addr = 0x0400;
            #[reset]
            fn main() {{
                let a: str = "{a}";
                let b: str = "{b}";
                if a != b {{ OUT = 1; }} else {{ OUT = 2; }}
                loop {{}}
            }}
        "#
        );
        run(&src).mem(0x0400)
    };
    assert_eq!(ne("cat", "dog"), 1, "different strings differ");
    assert_eq!(ne("cat", "cat"), 2, "identical strings do not differ");
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
