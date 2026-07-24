//! Differential sweep over the for-loop strategy matrix.
//!
//! Every combination of {u8, u16} x {constant, runtime} bounds x {inclusive,
//! exclusive} x {counter used, counter unused} selects a different codegen
//! strategy (unrolled, count-down, bottom-test counting, far-branch variants).
//! These tests run a battery of boundary values through each combination on
//! the emulator and compare iteration counts and last-seen counter values
//! against a Rust-computed oracle.

use crate::common::exec::run;

/// Oracle: iteration count and last counter value for a u16-domain range.
fn oracle(start: u32, end: u32, inclusive: bool) -> (u32, Option<u32>) {
    let upper = if inclusive { end + 1 } else { end };
    if upper <= start {
        (0, None)
    } else {
        (upper - start, Some(upper - 1))
    }
}

/// Run a u8 for-loop and return (iteration count, last i seen or 0xAA).
fn u8_case(start: u8, end: u8, inclusive: bool, use_var: bool, runtime: bool) -> (u16, u8) {
    let op = if inclusive { "..=" } else { ".." };
    let body = if use_var {
        "SINK = i; n = n + one;"
    } else {
        "n = n + one;"
    };
    let (decls, range) = if runtime {
        (
            format!("fn s() -> u8 {{ return {start}; }} fn e() -> u8 {{ return {end}; }}"),
            format!("s(){op}e()"),
        )
    } else {
        (String::new(), format!("{start}{op}{end}"))
    };
    let src = format!(
        r#"
        const LO: addr = 0x0400;
        const HI: addr = 0x0401;
        const SINK: addr = 0x0402;
        {decls}
        #[reset]
        fn main() {{
            SINK = 0xAA;
            let n: u16 = 0;
            let one: u16 = 1;
            for i in {range} {{
                {body}
            }}
            LO = n.low;
            HI = n.high;
            loop {{}}
        }}
    "#
    );
    let mut ex = run(&src);
    (ex.mem16(0x0400), ex.mem(0x0402))
}

/// Run a u16 for-loop and return (iteration count, last i seen or 0xAAAA).
fn u16_case(start: u16, end: u16, inclusive: bool, use_var: bool, runtime: bool) -> (u16, u16) {
    let op = if inclusive { "..=" } else { ".." };
    let body = if use_var {
        "SL = i.low; SH = i.high; n = n + one;"
    } else {
        "n = n + one;"
    };
    let (decls, range) = if runtime {
        (
            format!("fn s() -> u16 {{ return {start}; }} fn e() -> u16 {{ return {end}; }}"),
            format!("s(){op}e()"),
        )
    } else {
        (String::new(), format!("{start}{op}{end}"))
    };
    let src = format!(
        r#"
        const LO: addr = 0x0400;
        const HI: addr = 0x0401;
        const SL: addr = 0x0402;
        const SH: addr = 0x0403;
        {decls}
        #[reset]
        fn main() {{
            SL = 0xAA;
            SH = 0xAA;
            let n: u16 = 0;
            let one: u16 = 1;
            for i: u16 in {range} {{
                {body}
            }}
            LO = n.low;
            HI = n.high;
            loop {{}}
        }}
    "#
    );
    let mut ex = run(&src);
    (ex.mem16(0x0400), ex.mem16(0x0402))
}

fn check_u8(start: u8, end: u8, inclusive: bool, use_var: bool, runtime: bool) {
    let (count, expect_last) = oracle(start as u32, end as u32, inclusive);
    let (got_count, got_last) = u8_case(start, end, inclusive, use_var, runtime);
    let label = format!(
        "u8 {start}{}{end} use_var={use_var} runtime={runtime}",
        if inclusive { "..=" } else { ".." }
    );
    assert_eq!(got_count as u32, count, "{label}: iteration count");
    if use_var {
        let expect_sink = expect_last.map(|v| v as u8).unwrap_or(0xAA);
        assert_eq!(got_last, expect_sink, "{label}: last counter value");
    }
}

fn check_u16(start: u16, end: u16, inclusive: bool, use_var: bool, runtime: bool) {
    let (count, expect_last) = oracle(start as u32, end as u32, inclusive);
    let (got_count, got_last) = u16_case(start, end, inclusive, use_var, runtime);
    let label = format!(
        "u16 {start}{}{end} use_var={use_var} runtime={runtime}",
        if inclusive { "..=" } else { ".." }
    );
    assert_eq!(got_count as u32, count, "{label}: iteration count");
    if use_var {
        let expect_sink = expect_last.map(|v| v as u16).unwrap_or(0xAAAA);
        assert_eq!(got_last, expect_sink, "{label}: last counter value");
    }
}

/// Boundary values that select interesting u8 behavior: empty ranges,
/// single iterations, the unroll threshold (8), and the type maximum.
const U8_CASES: &[(u8, u8)] = &[
    (0, 0),
    (0, 1),
    (0, 8),
    (0, 9),
    (0, 10),
    (5, 5),
    (5, 3),
    (0, 255),
    (250, 255),
    (254, 255),
    (255, 255),
    (100, 200),
];

#[test]
fn sweep_u8_exclusive() {
    for &(s, e) in U8_CASES {
        for use_var in [false, true] {
            for runtime in [false, true] {
                check_u8(s, e, false, use_var, runtime);
            }
        }
    }
}

#[test]
fn sweep_u8_inclusive() {
    for &(s, e) in U8_CASES {
        for use_var in [false, true] {
            for runtime in [false, true] {
                check_u8(s, e, true, use_var, runtime);
            }
        }
    }
}

/// u16 boundaries: byte-carry edges, empty ranges, and the type maximum.
const U16_CASES: &[(u16, u16)] = &[
    (0, 0),
    (0, 1),
    (0, 0x0100),
    (0x00FF, 0x0101),
    (0xFFF0, 0xFFFF),
    (0xFFFF, 0xFFFF),
    (0x0200, 0x0100),
    (0x0100, 0x00FF),
];

#[test]
fn sweep_u16_exclusive() {
    for &(s, e) in U16_CASES {
        for use_var in [false, true] {
            for runtime in [false, true] {
                check_u16(s, e, false, use_var, runtime);
            }
        }
    }
}

#[test]
fn sweep_u16_inclusive() {
    for &(s, e) in U16_CASES {
        // Skip inclusive-to-0xFFFF for the giant range only; the wrap edge is
        // covered by (0xFFF0, 0xFFFF) and (0xFFFF, 0xFFFF).
        for use_var in [false, true] {
            for runtime in [false, true] {
                check_u16(s, e, true, use_var, runtime);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Adversarial interactions per strategy
// ---------------------------------------------------------------------------

#[test]
fn countdown_loop_break_and_continue() {
    // Count-down strategy (counter unused): break must exit with the right
    // side effects; continue must reach the DEC (not skip it).
    let mut e = run(r#"
        const OUT: addr = 0x0400;
        #[reset]
        fn main() {
            let n: u8 = 0;
            for i in 0..100 {
                n = n + 1;
                if n == 7 {
                    break;
                }
            }
            let m: u8 = 0;
            let k: u8 = 0;
            for j in 0..20 {
                k = k + 1;
                if k < 15 {
                    continue;
                }
                m = m + 1;
            }
            OUT = n + m;
            loop {}
        }
    "#);
    // n = 7 (break), m = 6 (iterations 15..=20 of k)
    assert_eq!(e.mem(0x0400), 13, "break/continue in count-down loops");
}

#[test]
fn counting_loop_body_clobbers_x_via_runtime_shift() {
    // A runtime-count shift uses X as its scratch counter; the loop must
    // reload its own counter afterwards.
    let mut e = run(r#"
        const OUT: addr = 0x0400;
        fn one_bit() -> u8 { return 1; }
        #[reset]
        fn main() {
            let n: u8 = 0;
            for i in 0..10 {
                let d: u8 = i << one_bit();
                n = n + d;
            }
            OUT = n;
            loop {}
        }
    "#);
    // sum of 2*i for i in 0..10 = 90
    assert_eq!(e.mem(0x0400), 90, "shift in body must not corrupt counter");
}

#[test]
fn counting_loop_body_mutates_counter() {
    // The body writes the loop variable; the increment must resume from the
    // written value (i = 0, 3, 6, 9 -> 4 iterations).
    let mut e = run(r#"
        const OUT: addr = 0x0400;
        #[reset]
        fn main() {
            let n: u8 = 0;
            for i in 0..10 {
                i = i + 2;
                n = n + 1;
            }
            OUT = n;
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0400), 4, "body counter mutation must be honored");
}

#[test]
fn countdown_detection_defeated_by_shadowing() {
    // The body declares its own `i`; the reference walk conservatively treats
    // that as a use, so the loop takes the counting shape - and must still be
    // correct.
    let mut e = run(r#"
        const OUT: addr = 0x0400;
        #[reset]
        fn main() {
            let n: u8 = 0;
            for i in 0..10 {
                let i: u8 = 5;
                n = n + i;
            }
            OUT = n;
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0400), 50, "shadowed counter loop still correct");
}

#[test]
fn recursive_function_with_runtime_loop_bound() {
    // The hidden bound slot lives in the frame, so recursion's frame
    // save/restore must preserve it across the self-call.
    let mut e = run(r#"
        const OUT: addr = 0x0400;
        fn lim() -> u8 { return 3; }
        fn tri(depth: u8) -> u8 {
            if depth == 0 {
                return 0;
            }
            let acc: u8 = 0;
            for i in 0..lim() {
                acc = acc + depth;
            }
            return acc + tri(depth - 1);
        }
        #[reset]
        fn main() {
            OUT = tri(4);
            loop {}
        }
    "#);
    // 3*(4+3+2+1) = 30
    assert_eq!(e.mem(0x0400), 30, "bound slot must survive recursion");
}

#[test]
fn u16_return_of_u8_literal_zero_extends() {
    // Regression for the bug this sweep exposed: a `-> u16` function
    // returning an 8-bit-typed expression must zero the high byte, not leak
    // whatever Y held at the call site.
    let mut e = run(r#"
        const LO: addr = 0x0400;
        const HI: addr = 0x0401;
        fn poison_y() -> u16 { return 0x0700; }
        fn small() -> u16 { return 255; }
        #[reset]
        fn main() {
            let p: u16 = poison_y();
            let v: u16 = small();
            LO = v.low;
            HI = v.high;
            loop {}
        }
    "#);
    assert_eq!(
        e.mem16(0x0400),
        255,
        "u16 return of u8 expr must zero-extend"
    );
}
