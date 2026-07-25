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

#[test]
fn slice_with_runtime_bounds() {
    // Slice bounds computed from variables at runtime.
    let mut e = run(r#"
        const E0: addr = 0x0400;
        const E1: addr = 0x0401;
        const LEN: addr = 0x0402;
        #[reset]
        fn main() {
            let a: [u8; 6] = [10, 20, 30, 40, 50, 60];
            let i: u8 = 2;
            let j: u8 = 5;
            let s: &[u8] = a[i..j];   // 30, 40, 50
            E0 = s[0 as u8];
            E1 = s[2 as u8];
            LEN = s.len as u8;
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0400), 30, "s[0] == a[2]");
    assert_eq!(e.mem(0x0401), 50, "s[2] == a[4]");
    assert_eq!(e.mem(0x0402), 3, "s.len == j - i == 3");
}

#[test]
fn slice_with_runtime_bounds_u16() {
    // Runtime bounds with u16 elements (base offset must scale by 2).
    let mut e = run(r#"
        const LO: addr = 0x0400;
        const HI: addr = 0x0401;
        #[reset]
        fn main() {
            let a: [u16; 5] = [0x1111, 0x2222, 0x3333, 0x4444, 0x5555];
            let i: u8 = 1;
            let j: u8 = 4;
            let s: &[u16] = a[i..j];  // 0x2222, 0x3333, 0x4444
            let v: u16 = s[2 as u8];  // 0x4444
            LO = v.low;
            HI = v.high;
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0400), 0x44, "s[2] low byte");
    assert_eq!(e.mem(0x0401), 0x44, "s[2] high byte");
}

#[test]
fn slice_of_slice() {
    // Re-slicing a slice narrows the view further; offsets compose.
    let mut e = run(r#"
        const E0: addr = 0x0400;
        const E1: addr = 0x0401;
        const LEN: addr = 0x0402;
        #[reset]
        fn main() {
            let a: [u8; 8] = [10, 20, 30, 40, 50, 60, 70, 80];
            let s: &[u8] = a[1..7];   // 20,30,40,50,60,70
            let s2: &[u8] = s[2..5];  // 40,50,60
            E0 = s2[0 as u8];
            E1 = s2[2 as u8];
            LEN = s2.len as u8;
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0400), 40, "s2[0] == a[3]");
    assert_eq!(e.mem(0x0401), 60, "s2[2] == a[5]");
    assert_eq!(e.mem(0x0402), 3, "s2.len == 3");
}

#[test]
fn slice_passed_to_function() {
    // A slice passed to a function: the callee reads its length and elements
    // from the descriptor copied into its parameter slot.
    let mut e = run(r#"
        const OUT: addr = 0x0400;
        fn sum(s: &[u8]) -> u8 {
            let acc: u8 = 0;
            for i in 0..s.len {
                acc = acc + s[i as u8];
            }
            return acc;
        }
        #[reset]
        fn main() {
            let a: [u8; 6] = [1, 2, 3, 4, 5, 6];
            let s: &[u8] = a[1..5];   // 2, 3, 4, 5
            OUT = sum(s);
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0400), 14, "2 + 3 + 4 + 5 = 14");
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
