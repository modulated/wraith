//! Control-flow tests, verified by execution.
//!
//! Control flow decides *which* code runs, so the meaningful assertion is the
//! value a program arrives at — not that a `BEQ` appears somewhere. A loop that
//! runs one iteration too many, or a `break` that escapes the wrong loop, emits
//! perfectly plausible assembly; only running it tells you.

use crate::common::assert_error_contains;
use crate::common::exec::run;

/// Run a program body that leaves a u8 in `OUT` ($0900).
fn eval(body: &str) -> u8 {
    let src = format!(
        r#"
        const OUT: addr = 0x0900;
        #[reset]
        fn main() {{
            {body}
            loop {{}}
        }}
    "#
    );
    run(&src).mem(0x0900)
}

// ============================================================================
// Conditionals
// ============================================================================

#[test]
fn if_statement() {
    assert_eq!(eval("let x: u8 = 10; if x > 5 { OUT = 1; }"), 1);
    assert_eq!(
        eval("let x: u8 = 3; if x > 5 { OUT = 1; }"),
        0,
        "body must be skipped when the condition is false"
    );
}

#[test]
fn if_else() {
    assert_eq!(
        eval("let x: u8 = 10; if x > 5 { OUT = 1; } else { OUT = 2; }"),
        1
    );
    assert_eq!(
        eval("let x: u8 = 1; if x > 5 { OUT = 1; } else { OUT = 2; }"),
        2
    );
}

#[test]
fn else_if_chain_picks_first_match() {
    let grade = |s: u8| {
        eval(&format!(
            "let s: u8 = {s};
             if s >= 90 {{ OUT = 4; }} else if s >= 80 {{ OUT = 3; }}
             else if s >= 70 {{ OUT = 2; }} else {{ OUT = 1; }}"
        ))
    };
    assert_eq!(grade(95), 4);
    assert_eq!(grade(85), 3);
    assert_eq!(grade(70), 2, "boundary is inclusive");
    assert_eq!(grade(69), 1);
}

// ============================================================================
// Loops — the assertion is the iteration count
// ============================================================================

#[test]
fn while_loop() {
    // 0+1+2+3+4: proves the loop runs exactly five times.
    assert_eq!(
        eval("let i: u8 = 0; let s: u8 = 0; while i < 5 { s = s + i; i = i + 1; } OUT = s;"),
        10
    );
}

#[test]
fn while_loop_zero_iterations() {
    assert_eq!(
        eval("let i: u8 = 9; let n: u8 = 0; while i < 5 { n = n + 1; i = i + 1; } OUT = n;"),
        0,
        "a condition false on entry runs the body zero times"
    );
}

#[test]
fn for_range_loop() {
    // 0..5 is half-open: 0+1+2+3+4 = 10, not 15.
    assert_eq!(
        eval("let s: u8 = 0; for i in 0..5 { s = s + i; } OUT = s;"),
        10
    );
}

#[test]
fn for_range_inclusive() {
    assert_eq!(
        eval("let s: u8 = 0; for i in 0..=5 { s = s + i; } OUT = s;"),
        15
    );
}

#[test]
fn while_loop_with_break() {
    assert_eq!(
        eval("let i: u8 = 0; while i < 100 { if i == 7 { break; } i = i + 1; } OUT = i;"),
        7
    );
}

#[test]
fn while_loop_with_continue() {
    // Count only the even values in 1..=10.
    assert_eq!(
        eval(
            "let i: u8 = 0; let n: u8 = 0;
             while i < 10 { i = i + 1; if (i & 1) == 1 { continue; } n = n + 1; }
             OUT = n;"
        ),
        5
    );
}

#[test]
fn for_loop_with_break() {
    assert_eq!(
        eval("let last: u8 = 0; for i in 0..20 { if i == 4 { break; } last = i; } OUT = last;"),
        3
    );
}

#[test]
fn for_loop_with_continue() {
    // `continue` must still advance the loop variable, or this never terminates.
    assert_eq!(
        eval("let n: u8 = 0; for i in 0..10 { if i < 5 { continue; } n = n + 1; } OUT = n;"),
        5
    );
}

#[test]
fn for_loop_bound_beyond_the_counter_type_is_rejected() {
    // This compiled and ran 44 iterations: 300 truncated to its low byte at
    // the compare.
    assert_error_contains(
        r#"
        const OUT: addr = 0x0900;
        #[reset]
        fn main() { for i: u8 in 0..300 { OUT = i; } loop {} }
    "#,
        "does not fit in counter type u8",
    );
}

#[test]
fn for_loop_runtime_bound_wider_than_the_counter_is_rejected() {
    // A u16 bound expression against a u8 counter compared only the low byte.
    assert_error_contains(
        r#"
        const OUT: addr = 0x0900;
        fn top() -> u16 { return 300; }
        #[reset]
        fn main() { for i: u8 in 0..top() { OUT = i; } loop {} }
    "#,
        "does not fit in counter type u8",
    );
}

#[test]
fn for_loop_with_a_wide_counter_counts_past_255() {
    // The u16 form of the same loop, which is the fix the error points at.
    assert_eq!(
        eval("let n: u16 = 0; for i: u16 in 0..300 { n = i; } OUT = n.low;"),
        43,
        "ran to 299 (low byte 43) — counted, not truncated"
    );
}

#[test]
fn nested_loop_break_exits_inner_only() {
    // 3 outer iterations x 2 inner (break at j == 2) = 6.
    assert_eq!(
        eval(
            "let n: u8 = 0;
             for i in 0..3 { for j in 0..10 { if j == 2 { break; } n = n + 1; } }
             OUT = n;"
        ),
        6
    );
}

#[test]
fn loop_with_break() {
    assert_eq!(
        eval("let n: u8 = 0; loop { n = n + 1; if n == 3 { break; } } OUT = n;"),
        3
    );
}

// ============================================================================
// match
// ============================================================================

#[test]
fn match_literal_patterns() {
    let pick = |v: u8| {
        eval(&format!(
            "let x: u8 = {v};
             match x {{ 1 => {{ OUT = 10; }} 2 => {{ OUT = 20; }} _ => {{ OUT = 99; }} }}"
        ))
    };
    assert_eq!(pick(1), 10);
    assert_eq!(pick(2), 20);
    assert_eq!(pick(7), 99, "wildcard catches the rest");
}

#[test]
fn match_multiple_arms() {
    let pick = |v: u8| {
        eval(&format!(
            "let x: u8 = {v};
             match x {{
                1 => {{ OUT = 10; }} 2 => {{ OUT = 20; }}
                3 => {{ OUT = 30; }} _ => {{ OUT = 0; }}
             }}"
        ))
    };
    assert_eq!(pick(1), 10);
    assert_eq!(pick(3), 30);
    assert_eq!(pick(9), 0);
}

#[test]
fn match_enum_variants() {
    let pick = |v: &str| {
        let src = format!(
            r#"
            enum Color {{ Red, Green, Blue }}
            const OUT: addr = 0x0900;
            #[reset]
            fn main() {{
                let c: Color = Color::{v};
                match c {{
                    Color::Red   => {{ OUT = 1; }}
                    Color::Green => {{ OUT = 2; }}
                    Color::Blue  => {{ OUT = 3; }}
                }}
                loop {{}}
            }}
        "#
        );
        run(&src).mem(0x0900)
    };
    assert_eq!(pick("Red"), 1);
    assert_eq!(pick("Green"), 2);
    assert_eq!(pick("Blue"), 3);
}

#[test]
fn match_expression_yields_a_value() {
    assert_eq!(
        eval("let x: u8 = 2; let r: u8 = match x { 1 => 10, 2 => 20, _ => 99 }; OUT = r;"),
        20
    );
}

#[test]
fn match_range_arm() {
    let pick = |v: u8| {
        eval(&format!(
            "let x: u8 = {v};
             match x {{ 0..=9 => {{ OUT = 1; }} 10..=19 => {{ OUT = 2; }} _ => {{ OUT = 3; }} }}"
        ))
    };
    assert_eq!(pick(5), 1);
    assert_eq!(pick(15), 2);
    assert_eq!(pick(50), 3);
}

// ============================================================================
// Type errors remain compile-time checks, not runtime behaviour
// ============================================================================

#[test]
fn match_expression_incompatible_arms_error() {
    // Arms with no common type are rejected; previously the first arm's type
    // won and later arms were ignored.
    assert_error_contains(
        r#"
        fn main() {
            let k: u8 = 1;
            let flag: bool = match k { 0 => true, _ => 5 };
        }
    "#,
        "type mismatch",
    );
}

#[test]
fn match_expression_compatible_arms_unify() {
    // A u8 arm and a u16 arm unify to u16 — and the wider value survives.
    assert_eq!(
        run(r#"
            const LO: addr = 0x0900;
            const HI: addr = 0x0901;
            #[reset]
            fn main() {
                let x: u8 = 2;
                let r: u16 = match x { 1 => 5, _ => 300 };
                LO = r.low;
                HI = r.high;
                loop {}
            }
        "#)
        .mem16(0x0900),
        300
    );
}

// ============================================================================
// ForEach size limits
//
// ForEach lowers to an 8-bit counter and offset: past 255 elements (or 127
// for u16 elements, whose byte offset is the index doubled) the bound or the
// offset silently wrapped — a 300-element array iterated 44 times, a
// 200-element u16 array re-reading elements 0..71. Both are errors now.
// ============================================================================

#[test]
fn foreach_over_an_array_past_255_elements_is_rejected() {
    assert_error_contains(
        r#"
        static big: [u8; 300] = [0; 300];
        #[reset]
        fn main() {
            let sum: u8 = 0;
            for x in big { sum = sum + x; }
            loop {}
        }
    "#,
        "cannot iterate",
    );
}

#[test]
fn foreach_over_a_wide_array_past_127_elements_is_rejected() {
    assert_error_contains(
        r#"
        static big: [u16; 200] = [0; 200];
        #[reset]
        fn main() {
            let sum: u8 = 0;
            for x in big { sum = sum + x.low; }
            loop {}
        }
    "#,
        "cannot iterate",
    );
}

#[test]
fn foreach_at_the_limits_still_works() {
    // 255 u8 elements and 127 u16 elements are the largest that fit.
    let mut e = run(r#"
        const OUT: addr = 0x0900;
        #[reset]
        fn main() {
            let a: [u8; 255] = [2; 255];
            let count: u8 = 0;
            for x in a { count = count + 1; }
            OUT = count;
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0900), 255);
}

#[test]
fn a_large_match_assembles_and_picks_the_right_arm() {
    // Sequential matches branched to arm bodies with BEQ/BCC: past ~127 bytes
    // of compare section the branch overflowed and flatasm failed. Arm-ward
    // branches now invert over a JMP, so any arm count assembles — and the
    // first and last arms must still be the ones that run.
    let arms: String = (0..20)
        .map(|k| format!("{} => {{ OUT = {}; }}", k, k + 100))
        .collect::<Vec<_>>()
        .join("\n");
    let pick = |v: u8| {
        eval(&format!(
            "let x: u8 = {v};
             match x {{
                 {arms}
                 _ => {{ OUT = 0; }}
             }}"
        ))
    };
    assert_eq!(pick(0), 100, "first arm, the furthest branch");
    assert_eq!(pick(19), 119, "last literal arm");
    assert_eq!(pick(25), 0, "wildcard");
}
