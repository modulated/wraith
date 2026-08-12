//! End-to-end tests for functions

use crate::common::exec::run;
use crate::common::*;

#[test]
fn function_call_no_args() {
    let asm = compile_success(
        r#"
        fn foo() {
        }
        fn main() {
            // Address-taken keeps foo out-of-line under the auto-inliner, so the
            // direct-call convention (JSR/label) is what's exercised here.
            let _keep: fn() = foo;
            foo();
        }
    "#,
    );

    // Tail call optimization may convert JSR+RTS to JMP
    assert!(
        asm.contains("JSR foo") || asm.contains("JMP foo"),
        "Expected JSR or JMP foo (tail-call optimized)"
    );
    assert_asm_contains(&asm, "foo:");
}

#[test]
fn function_call_with_args() {
    let asm = compile_success(
        r#"
        fn add(a: u8, b: u8) -> u8 {
            return a + b;
        }
        fn main() {
            let _keep: fn(u8, u8) -> u8 = add; // keep add out-of-line
            let result: u8 = add(5, 10);
        }
    "#,
    );

    assert_asm_contains(&asm, "JSR add");
    assert_asm_contains(&asm, "add:");
}

#[test]
fn function_return_value() {
    let asm = compile_success(
        r#"
        fn get_value() -> u8 {
            return 42;
        }
        fn main() {
            let _keep: fn() -> u8 = get_value; // keep get_value out-of-line
            let x: u8 = get_value();
        }
    "#,
    );

    assert_asm_contains(&asm, "JSR get_value");
    assert_asm_contains(&asm, "RTS");
}

#[test]
fn a_tail_recursive_function_can_be_address_taken() {
    // The tail-call loop label used to sit *before* the function-pointer
    // prologue, so each "iteration" re-copied the $E0 staging block over the
    // freshly updated parameter: the loop reloaded the original argument
    // forever. Called through a pointer, count(5) must actually reach 0.
    let mut e = run(r#"
        const OUT: addr = 0x0900;
        fn count(n: u8) -> u8 {
            if n == 0 { return 0; }
            return count(n - 1);
        }
        #[reset]
        fn main() {
            let f: fn(u8) -> u8 = count;
            OUT = f(5);
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0900), 0);
}

// ===========================================================================
// Return-type enforcement
//
// A declared return type is a promise to every caller, and on a 6502 breaking
// it is silent: the caller reads the accumulator either way, so a function that
// falls off its end hands back whatever the last statement happened to leave
// there. These pin both directions — a value returned where none was declared,
// and a path that reaches the end of a value-returning function.
// ===========================================================================

/// Returning a value from a function declared without a return type.
#[test]
fn a_void_function_cannot_return_a_value() {
    assert_error_contains(
        r#"
        fn f() { return 5; }
        #[reset]
        fn main() { f(); loop {} }
        "#,
        "return type mismatch",
    );
}

/// The same, where the returned value is a call rather than a literal — the
/// check has to look at the expression's type, not just spot a literal.
#[test]
fn a_void_function_cannot_return_a_call_result() {
    assert_error_contains(
        r#"
        fn g() -> u8 { return 1; }
        fn f() { return g(); }
        #[reset]
        fn main() { f(); loop {} }
        "#,
        "return type mismatch",
    );
}

/// Nested inside control flow, where a shallower walk would miss it.
#[test]
fn a_void_function_cannot_return_a_value_from_a_branch() {
    assert_error_contains(
        r#"
        fn f(n: u8) { if n == 0 { return 7; } }
        #[reset]
        fn main() { f(1); loop {} }
        "#,
        "return type mismatch",
    );
}

/// A bare `return;` is how a void function exits early, and must stay legal.
#[test]
fn a_void_function_may_return_without_a_value() {
    let _asm = compile_success(
        r#"
        const OUT: addr = 0x0400;
        fn f(n: u8) { if n == 0 { return; } OUT = n; }
        #[reset]
        fn main() { f(0); f(3); loop {} }
        "#,
    );
}

/// The mirror: a value-returning function cannot exit without one.
#[test]
fn a_value_returning_function_cannot_return_nothing() {
    assert_error_contains(
        r#"
        fn f(n: u8) -> u8 { if n == 0 { return; } return 1; }
        #[reset]
        fn main() { let x: u8 = f(0); loop {} }
        "#,
        "return type mismatch",
    );
}

#[test]
fn a_wrongly_typed_return_value_is_rejected() {
    assert_error_contains(
        r#"
        fn f() -> u8 { return "x"; }
        #[reset]
        fn main() { let a: u8 = f(); loop {} }
        "#,
        "return type mismatch",
    );
}

/// Narrowing is not implicit anywhere else, and return position is no
/// exception — `u16` into a `u8` slot needs an explicit cast.
#[test]
fn a_narrowing_return_needs_an_explicit_cast() {
    assert_error_contains(
        r#"
        fn f() -> u8 { let x: u16 = 300; return x; }
        #[reset]
        fn main() { let a: u8 = f(); loop {} }
        "#,
        "return type mismatch",
    );
}

/// Lossless widening *is* implicit (spec: "Only lossless widening is implicit
/// (`u8` → `u16`, `i8` → `i16`, `bool` → `u8`)"), so these must keep compiling.
/// Pinned here so the stricter checks above are never mistaken for a licence to
/// tighten the conversion rules too.
#[test]
fn a_widening_return_stays_implicit() {
    let _u16_from_u8 = compile_success(
        r#"
        fn f() -> u16 { let x: u8 = 3; return x; }
        #[reset]
        fn main() { let a: u16 = f(); loop {} }
        "#,
    );
    let _u8_from_bool = compile_success(
        r#"
        fn f() -> u8 { return true; }
        #[reset]
        fn main() { let a: u8 = f(); loop {} }
        "#,
    );
}

// ---------------------------------------------------------------------------
// Falling off the end
// ---------------------------------------------------------------------------

#[test]
fn a_value_returning_function_must_return_on_every_path() {
    assert_error_contains(
        r#"
        fn f(n: u8) -> u8 { if n == 0 { return 1; } }
        #[reset]
        fn main() { let a: u8 = f(2); loop {} }
        "#,
        "missing return",
    );
}

#[test]
fn a_body_with_no_return_at_all_is_rejected() {
    assert_error_contains(
        r#"
        fn f() -> u8 { let x: u8 = 1; }
        #[reset]
        fn main() { let a: u8 = f(); loop {} }
        "#,
        "missing return",
    );
}

/// A conditional loop may run zero times, so a `return` inside one guarantees
/// nothing about the path that skips it.
#[test]
fn a_return_only_inside_a_conditional_loop_is_not_enough() {
    assert_error_contains(
        r#"
        fn f(n: u8) -> u8 { while n > 0 { return 1; } }
        #[reset]
        fn main() { let a: u8 = f(2); loop {} }
        "#,
        "missing return",
    );
    assert_error_contains(
        r#"
        fn f() -> u8 { for i in 0..4 { return i; } }
        #[reset]
        fn main() { let a: u8 = f(); loop {} }
        "#,
        "missing return",
    );
}

/// Both arms of an `if`/`else` returning is enough; only one arm is not.
#[test]
fn an_if_else_that_returns_on_both_arms_is_complete() {
    let _asm = compile_success(
        r#"
        fn f(n: u8) -> u8 { if n == 0 { return 1; } else { return 2; } }
        #[reset]
        fn main() { let a: u8 = f(2); loop {} }
        "#,
    );
}

/// `loop {}` never completes, so control never reaches past it — that is what
/// makes it a valid way to end a value-returning function.
#[test]
fn an_infinite_loop_ends_a_function_without_a_return() {
    let _asm = compile_success(
        r#"
        fn f() -> u8 { loop {} }
        #[reset]
        fn main() { let a: u8 = f(); loop {} }
        "#,
    );
}

/// ...but a `loop` with a `break` does complete, so it guarantees nothing.
#[test]
fn a_breakable_loop_does_not_end_a_function() {
    assert_error_contains(
        r#"
        fn f(n: u8) -> u8 { loop { if n == 0 { break; } return 1; } }
        #[reset]
        fn main() { let a: u8 = f(0); loop {} }
        "#,
        "missing return",
    );
}

/// A `break` belonging to a *nested* loop does not give the outer loop an exit,
/// so the outer `loop` still never completes.
#[test]
fn a_break_in_a_nested_loop_does_not_make_the_outer_loop_exit() {
    let _asm = compile_success(
        r#"
        fn f(n: u8) -> u8 { loop { while n > 0 { break; } } }
        #[reset]
        fn main() { let a: u8 = f(1); loop {} }
        "#,
    );
}

/// A match that names every enum variant and returns in each arm is complete,
/// with no wildcard needed.
#[test]
fn an_exhaustive_match_returning_in_every_arm_is_complete() {
    let _asm = compile_success(
        r#"
        enum Dir { North, South, East, West }
        fn code(d: Dir) -> u8 {
            match d {
                Dir::North => { return 1; }
                Dir::South => { return 2; }
                Dir::East  => { return 3; }
                Dir::West  => { return 4; }
            }
        }
        #[reset]
        fn main() { let a: u8 = code(Dir::East); loop {} }
        "#,
    );
}

/// Drop one variant and the match no longer covers every value, so the function
/// can fall through it.
#[test]
fn a_non_exhaustive_match_does_not_complete_a_function() {
    assert_error_contains(
        r#"
        enum Dir { North, South, East, West }
        fn code(d: Dir) -> u8 {
            match d {
                Dir::North => { return 1; }
                Dir::South => { return 2; }
            }
        }
        #[reset]
        fn main() { let a: u8 = code(Dir::North); loop {} }
        "#,
        "missing return",
    );
}

/// A wildcard arm makes any match exhaustive, including over an integer.
#[test]
fn a_wildcard_arm_completes_a_match_over_an_integer() {
    let _asm = compile_success(
        r#"
        fn code(n: u8) -> u8 {
            match n {
                0 => { return 10; }
                1 => { return 20; }
                _ => { return 30; }
            }
        }
        #[reset]
        fn main() { let a: u8 = code(7); loop {} }
        "#,
    );
}

/// Exhaustive, but one arm falls through: still incomplete.
#[test]
fn a_match_arm_that_does_not_return_leaves_the_function_incomplete() {
    assert_error_contains(
        r#"
        const OUT: addr = 0x0400;
        fn code(n: u8) -> u8 {
            match n {
                0 => { return 10; }
                _ => { OUT = 1; }
            }
        }
        #[reset]
        fn main() { let a: u8 = code(7); loop {} }
        "#,
        "missing return",
    );
}

/// The stdlib is written as whole-function `asm` bodies that leave the result
/// in the accumulator, which is the calling convention's return path. The check
/// must not reject them.
#[test]
fn an_asm_body_satisfies_the_return_requirement() {
    let _asm = compile_success(
        r#"
        fn pick(a: u8, b: u8) -> u8 {
            asm {
                "LDA {a}",
                "CMP {b}",
                "BCC PICKED",
                "LDA {b}",
                "PICKED:",
            }
        }
        #[reset]
        fn main() { let x: u8 = pick(3, 9); loop {} }
        "#,
    );
}

/// A void function is under no obligation, so none of this applies to it.
#[test]
fn a_void_function_need_not_return() {
    let _asm = compile_success(
        r#"
        const OUT: addr = 0x0400;
        fn f(n: u8) { OUT = n; }
        #[reset]
        fn main() { f(1); loop {} }
        "#,
    );
}
