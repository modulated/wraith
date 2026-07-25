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
// Pointer/string parameter passed after other parameters.
// ---------------------------------------------------------------------------

#[test]
fn string_param_after_u16_params() {
    // A str (pointer) parameter positioned after u16 params must receive the
    // correct pointer. A stale register belief used to elide the pointer's
    // low-byte load, passing the previous argument's value instead.
    let mut e = run(r#"
        const OUT: addr = 0x0400;
        fn pick(a: u16, b: u16, s: str) -> u8 { return s[1]; }
        #[reset]
        fn main() {
            let s: str = "XYZ";
            OUT = pick(1, 2, s);
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0400), 0x59, "s[1] == 'Y' when str is the 3rd arg");
}

// ---------------------------------------------------------------------------
// First-class slices: `let s: &[T] = arr[a..b]`, `s.len`, `s[i]`.
// ---------------------------------------------------------------------------

#[test]
fn slice_of_u8_array_index_and_len() {
    let mut e = run(r#"
        const E0: addr = 0x0400;
        const E1: addr = 0x0401;
        const E2: addr = 0x0402;
        const LEN: addr = 0x0403;
        #[reset]
        fn main() {
            let a: [u8; 5] = [10, 20, 30, 40, 50];
            let s: &[u8] = a[1..4];   // elements 20, 30, 40
            E0 = s[0 as u8];
            E1 = s[1 as u8];
            E2 = s[2 as u8];
            LEN = s.len as u8;
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0400), 20, "s[0] == a[1]");
    assert_eq!(e.mem(0x0401), 30, "s[1] == a[2]");
    assert_eq!(e.mem(0x0402), 40, "s[2] == a[3]");
    assert_eq!(e.mem(0x0403), 3, "s.len == 3");
}

#[test]
fn slice_iteration_with_len() {
    // Sum a slice's elements using its runtime length.
    let mut e = run(r#"
        const OUT: addr = 0x0400;
        #[reset]
        fn main() {
            let a: [u8; 6] = [1, 2, 3, 4, 5, 6];
            let s: &[u8] = a[2..5];   // 3, 4, 5
            let sum: u8 = 0;
            for i in 0..s.len {
                sum = sum + s[i as u8];
            }
            OUT = sum;
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0400), 12, "3 + 4 + 5 = 12");
}

#[test]
fn slice_of_u16_array_scales_index() {
    // u16-element slice: each element is 2 bytes, index must scale.
    let mut e = run(r#"
        const LO: addr = 0x0400;
        const HI: addr = 0x0401;
        const LEN: addr = 0x0402;
        #[reset]
        fn main() {
            let a: [u16; 4] = [0x1111, 0x2222, 0x3333, 0x4444];
            let s: &[u16] = a[1..3];  // 0x2222, 0x3333
            let v: u16 = s[1 as u8];  // 0x3333
            LO = v.low;
            HI = v.high;
            LEN = s.len as u8;
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0400), 0x33, "s[1] low byte");
    assert_eq!(e.mem(0x0401), 0x33, "s[1] high byte");
    assert_eq!(e.mem(0x0402), 2, "s.len == 2");
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
