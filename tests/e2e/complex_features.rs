//! Runtime (emulator) tests for the more complex language features that the
//! rest of the suite only checks at the assembly-string level: non-tail
//! recursion, pass-by-reference struct params, bitwise ops, short-circuit
//! evaluation, nested loops with break/continue, BCD arithmetic, strings,
//! multi-argument calls, and constant lookup tables.

use crate::common::exec::run;

// ---------------------------------------------------------------------------
// Non-tail recursion — exercises the software-stack frame save/restore and the
// `f(a) + f(b)` left-operand spill.
// ---------------------------------------------------------------------------

#[test]
fn recursive_fibonacci() {
    let mut e = run(r#"
        const OUT: addr = 0x0400;
        #[reset]
        fn main() {
            OUT = fib(9);
            loop {}
        }
        fn fib(n: u8) -> u8 {
            if n < 2 {
                return n;
            }
            return fib(n - 1) + fib(n - 2);
        }
    "#);
    // fib(9) = 34.
    assert_eq!(e.mem(0x0400), 34, "fib(9) via non-tail recursion");
}

#[test]
fn mutual_recursion_is_even() {
    let mut e = run(r#"
        const EV: addr = 0x0400;
        const OD: addr = 0x0401;
        #[reset]
        fn main() {
            EV = is_even(10) as u8;
            OD = is_even(7) as u8;
            loop {}
        }
        fn is_even(n: u8) -> bool {
            if n == 0 { return true; }
            return is_odd(n - 1);
        }
        fn is_odd(n: u8) -> bool {
            if n == 0 { return false; }
            return is_even(n - 1);
        }
    "#);
    assert_eq!(e.mem(0x0400), 1, "is_even(10) = true");
    assert_eq!(e.mem(0x0401), 0, "is_even(7) = false");
}

// ---------------------------------------------------------------------------
// Pass-by-reference struct parameters.
// ---------------------------------------------------------------------------

#[test]
fn struct_passed_by_reference() {
    let mut e = run(r#"
        struct Point { x: u8, y: u16 }
        const X: addr = 0x0400;
        const YLO: addr = 0x0401;
        const YHI: addr = 0x0402;
        #[reset]
        fn main() {
            let p: Point = Point { x: 7, y: 0x1234 };
            X = get_x(p);
            let yv: u16 = get_y(p);
            YLO = yv.low;
            YHI = yv.high;
            loop {}
        }
        fn get_x(p: Point) -> u8 { return p.x; }
        fn get_y(p: Point) -> u16 { return p.y; }
    "#);
    assert_eq!(
        e.mem(0x0400),
        7,
        "get_x reads the struct field by reference"
    );
    assert_eq!(e.mem16(0x0401), 0x1234, "get_y reads the u16 field");
}

// ---------------------------------------------------------------------------
// Bitwise operators.
// ---------------------------------------------------------------------------

#[test]
fn bitwise_operators() {
    let bit = |expr: &str| {
        let src = format!(
            r#"
            const OUT: addr = 0x0400;
            #[reset]
            fn main() {{
                let a: u8 = 0xF0;
                let b: u8 = 0x3C;
                OUT = {expr};
                loop {{}}
            }}
        "#
        );
        run(&src).mem(0x0400)
    };
    assert_eq!(bit("a & b"), 0x30, "0xF0 & 0x3C");
    assert_eq!(bit("a | b"), 0xFC, "0xF0 | 0x3C");
    assert_eq!(bit("a ^ b"), 0xCC, "0xF0 ^ 0x3C");
    assert_eq!(bit("~a"), 0x0F, "~0xF0");
}

// ---------------------------------------------------------------------------
// Short-circuit evaluation: the right operand must not run when the left
// already decides the result.
// ---------------------------------------------------------------------------

#[test]
fn short_circuit_and_skips_rhs() {
    let mut e = run(r#"
        const CTR: addr = 0x0400;
        const OUT: addr = 0x0401;
        fn touch() -> bool { CTR = CTR + 1; return true; }
        #[reset]
        fn main() {
            CTR = 0;
            let flag: bool = false;
            if flag && touch() {
                OUT = 1;
            } else {
                OUT = 0;
            }
            loop {}
        }
    "#);
    assert_eq!(
        e.mem(0x0400),
        0,
        "&& must not evaluate touch() when lhs is false"
    );
    assert_eq!(e.mem(0x0401), 0, "branch not taken");
}

#[test]
fn short_circuit_or_skips_rhs() {
    let mut e = run(r#"
        const CTR: addr = 0x0400;
        const OUT: addr = 0x0401;
        fn touch() -> bool { CTR = CTR + 1; return false; }
        #[reset]
        fn main() {
            CTR = 0;
            let flag: bool = true;
            if flag || touch() {
                OUT = 1;
            } else {
                OUT = 0;
            }
            loop {}
        }
    "#);
    assert_eq!(
        e.mem(0x0400),
        0,
        "|| must not evaluate touch() when lhs is true"
    );
    assert_eq!(e.mem(0x0401), 1, "branch taken");
}

// ---------------------------------------------------------------------------
// Nested loops and break/continue.
// ---------------------------------------------------------------------------

#[test]
fn nested_for_loops_sum() {
    let mut e = run(r#"
        const OUT: addr = 0x0400;
        #[reset]
        fn main() {
            let s: u8 = 0;
            for i in 0..4 {
                for j in 0..3 {
                    s = s + 1;
                }
            }
            OUT = s;
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0400), 12, "4 * 3 iterations");
}

#[test]
fn while_loop_with_break() {
    let mut e = run(r#"
        const OUT: addr = 0x0400;
        #[reset]
        fn main() {
            let i: u8 = 0;
            let s: u8 = 0;
            while i < 100 {
                if i == 5 {
                    break;
                }
                s = s + i;
                i = i + 1;
            }
            OUT = s;
            loop {}
        }
    "#);
    // 0 + 1 + 2 + 3 + 4 = 10, then break at i == 5.
    assert_eq!(e.mem(0x0400), 10, "sum until break at 5");
}

// ---------------------------------------------------------------------------
// BCD (binary-coded decimal) arithmetic.
// ---------------------------------------------------------------------------

#[test]
fn bcd_addition_decimal() {
    let mut e = run(r#"
        const OUT: addr = 0x0400;
        #[reset]
        fn main() {
            let a: b8 = 25 as b8;
            let b: b8 = 17 as b8;
            let c: b8 = a + b;
            OUT = c as u8;
            loop {}
        }
    "#);
    // 25 + 17 = 42 in decimal, stored packed as 0x42.
    assert_eq!(e.mem(0x0400), 0x42, "BCD 25 + 17 = 42 (packed 0x42)");
}

// ---------------------------------------------------------------------------
// Strings: length prefix and indexing.
// ---------------------------------------------------------------------------

#[test]
fn string_len_and_index() {
    let mut e = run(r#"
        const LEN: addr = 0x0400;
        const C0: addr = 0x0401;
        const C4: addr = 0x0402;
        #[reset]
        fn main() {
            let s: str = "hello";
            LEN = s.len as u8;
            C0 = s[0];
            C4 = s[4];
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0400), 5, "\"hello\".len == 5");
    assert_eq!(e.mem(0x0401), b'h', "s[0] == 'h'");
    assert_eq!(e.mem(0x0402), b'o', "s[4] == 'o'");
}

// ---------------------------------------------------------------------------
// Multi-argument functions.
// ---------------------------------------------------------------------------

#[test]
fn three_argument_call() {
    let mut e = run(r#"
        const OUT: addr = 0x0400;
        fn mix(a: u8, b: u8, c: u8) -> u8 { return a + b + c; }
        #[reset]
        fn main() {
            OUT = mix(10, 20, 12);
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0400), 42, "10 + 20 + 12 = 42");
}

// ---------------------------------------------------------------------------
// Constant lookup table (global const array indexed at a constant).
// ---------------------------------------------------------------------------

#[test]
fn const_lookup_table() {
    let mut e = run(r#"
        const SQUARES: [u8; 6] = [0, 1, 4, 9, 16, 25];
        const OUT: addr = 0x0400;
        #[reset]
        fn main() {
            OUT = SQUARES[5];
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0400), 25, "SQUARES[5] == 25");
}

#[test]
fn const_lookup_table_runtime_index() {
    let read = |i: u8| {
        let src = format!(
            r#"
            const CUBES: [u8; 5] = [0, 1, 8, 27, 64];
            const OUT: addr = 0x0400;
            #[reset]
            fn main() {{
                let i: u8 = {i};
                OUT = CUBES[i];
                loop {{}}
            }}
        "#
        );
        run(&src).mem(0x0400)
    };
    assert_eq!(read(0), 0, "CUBES[0]");
    assert_eq!(read(3), 27, "CUBES[3] (runtime index)");
    assert_eq!(read(4), 64, "CUBES[4]");
}

#[test]
fn const_lookup_table_u16() {
    let mut e = run(r#"
        const TABLE: [u16; 4] = [0x1000, 0x2000, 0x3000, 0xBEEF];
        const LO: addr = 0x0400;
        const HI: addr = 0x0401;
        #[reset]
        fn main() {
            let i: u8 = 3;
            let v: u16 = TABLE[i];
            LO = v.low;
            HI = v.high;
            loop {}
        }
    "#);
    assert_eq!(
        e.mem16(0x0400),
        0xBEEF,
        "TABLE[3] (u16 const lookup) == 0xBEEF"
    );
}
