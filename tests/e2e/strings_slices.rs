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
fn slice_with_complex_runtime_bounds_keeps_both() {
    // The runtime path parked `end` at $21 while `start` was still being
    // evaluated — and a u16 subexpression inside `start` stages its operands
    // at exactly $20/$21. `end` came back as 0, so the length wrapped.
    let mut e = run(r#"
        const E0: addr = 0x0400;
        const LEN: addr = 0x0401;
        #[reset]
        fn main() {
            let a: [u8; 8] = [10, 11, 12, 13, 14, 15, 16, 17];
            let w: u16 = 2;
            let s: &[u8] = a[(w + 1) as u8..(w + 4) as u8];   // 13, 14, 15
            E0 = s[0 as u8];
            LEN = s.len as u8;
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0400), 13, "s[0] == a[3]");
    assert_eq!(e.mem(0x0401), 3, "s.len == 6 - 3 == 3");
}

#[test]
fn slice_reassignment() {
    // `s = arr[a..b];` retargets an existing slice variable.
    let mut e = run(r#"
        const E0: addr = 0x0400;
        const LEN: addr = 0x0401;
        #[reset]
        fn main() {
            let a: [u8; 6] = [1, 2, 3, 4, 5, 6];
            let s: &[u8] = a[0..2];   // 1, 2
            s = a[3..6];              // 4, 5, 6
            E0 = s[0 as u8];
            LEN = s.len as u8;
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0400), 4, "s[0] == a[3] after reassignment");
    assert_eq!(e.mem(0x0401), 3, "s.len == 3 after reassignment");
}

#[test]
fn foreach_over_slice() {
    // Iterate a slice directly with `for x in s`.
    let mut e = run(r#"
        const OUT: addr = 0x0400;
        #[reset]
        fn main() {
            let a: [u8; 6] = [10, 20, 30, 40, 50, 60];
            let s: &[u8] = a[1..5];   // 20, 30, 40, 50
            let sum: u8 = 0;
            for x in s {
                sum = sum + x;
            }
            OUT = sum;
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0400), 140, "20 + 30 + 40 + 50 = 140");
}

#[test]
fn slice_returned_from_function() {
    // A function computes a sub-slice and returns it; the caller reads through
    // the returned descriptor (base still points into the original array).
    let mut e = run(r#"
        const E0: addr = 0x0400;
        const E1: addr = 0x0401;
        const LEN: addr = 0x0402;
        fn middle(a: &[u8]) -> &[u8] {
            let m: &[u8] = a[1..4];
            return m;
        }
        #[reset]
        fn main() {
            let arr: [u8; 6] = [10, 20, 30, 40, 50, 60];
            let s: &[u8] = arr[0..6];
            let r: &[u8] = middle(s);   // 20, 30, 40
            E0 = r[0 as u8];
            E1 = r[2 as u8];
            LEN = r.len as u8;
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0400), 20, "r[0] == arr[1]");
    assert_eq!(e.mem(0x0401), 40, "r[2] == arr[3]");
    assert_eq!(e.mem(0x0402), 3, "r.len == 3");
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

#[test]
fn slice_inclusive_runtime_end() {
    let mut e = run(r#"
        const E0: addr = 0x0400;
        const LEN: addr = 0x0401;
        #[reset]
        fn main() {
            let a: [u8; 6] = [10, 20, 30, 40, 50, 60];
            let i: u8 = 1;
            let j: u8 = 4;
            let s: &[u8] = a[i..=j];   // a[1..=4] -> 20,30,40,50
            E0 = s[3 as u8];
            LEN = s.len as u8;
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0400), 50, "inclusive end includes a[4]");
    assert_eq!(e.mem(0x0401), 4, "len == j - i + 1 == 4");
}

#[test]
fn foreach_slice_over_255_elements() {
    // Iterate a slice with 300 elements; count in a u16 to prove the 16-bit
    // counter iterates past 255.
    let mut e = run(r#"
        const LO: addr = 0x0400;
        const HI: addr = 0x0401;
        #[reset]
        fn main() {
            let big: [u8; 300] = [1; 300];
            let s: &[u8] = big[0..300];
            let count: u16 = 0;
            for x in s {
                count = count + (x as u16);
            }
            LO = count.low;
            HI = count.high;
            loop {}
        }
    "#);
    // 300 elements each == 1 -> sum 300 == 0x012C.
    assert_eq!(e.mem16(0x0400), 300, "iterated all 300 elements");
}

// ============================================================================
// Multi-byte characters in constant slices
//
// Constant string slices index by byte, but the value is held as a Rust
// String: slicing inside a multi-byte character used to *panic* the compiler
// ("byte index 2 is not a char boundary"). It is a clean error now, and a
// boundary-aligned slice of the same string compiles and reads back.
// ============================================================================

#[test]
fn a_const_slice_inside_a_multibyte_character_is_an_error_not_a_panic() {
    let result = crate::common::harness::compile(
        r#"
        const S: str = "héllo";
        const T: str = S[0..2];
        #[reset]
        fn main() { loop {} }
    "#,
    );
    match result {
        crate::common::harness::CompileResult::SemaError(e) => {
            assert!(e.contains("multi-byte character"), "{e}")
        }
        other => panic!("expected a sema error, got {other:?}"),
    }
}

#[test]
fn a_const_slice_at_a_character_boundary_works() {
    // "héllo" is six bytes; S[0..3] is "hé", whose second byte is é's first
    // (0xC3).
    let mut e = run(r#"
        const S: str = "héllo";
        const T: str = S[0..3];
        const OUT: addr = 0x0900;
        #[reset]
        fn main() { OUT = T[1]; loop {} }
    "#);
    assert_eq!(e.mem(0x0900), 0xC3);
}
