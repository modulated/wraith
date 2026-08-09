//! Multi-error reporting: sema collects independent errors instead of stopping
//! at the first.
//!
//! The two properties that matter pull against each other. Reporting *more*
//! errors is the point — a user fixing a file should not recompile once per
//! mistake. But reporting errors that are only consequences of an earlier one is
//! worse than reporting a single error, so the cascade tests below are as
//! load-bearing as the counting ones.

use crate::common::harness::{CompileResult, compile};

/// The rendered diagnostics from a compile expected to fail.
fn errors_of(src: &str) -> String {
    match compile(src) {
        CompileResult::SemaError(e) => e,
        other => panic!("expected a semantic error, got {other:?}"),
    }
}

/// How many distinct diagnostics were reported.
fn error_count(src: &str) -> usize {
    errors_of(src).matches("error:").count()
}

// ---------------------------------------------------------------------------
// Independent errors are all reported
// ---------------------------------------------------------------------------

#[test]
fn independent_errors_in_one_body_all_report() {
    let n = error_count(
        r#"
        #[reset]
        fn main() {
            let a: u8 = undefined_one;
            let b: u8 = undefined_two;
            let c: u8 = undefined_three;
            loop {}
        }
    "#,
    );
    assert_eq!(n, 3, "each undefined name is its own mistake");
}

#[test]
fn errors_in_sibling_functions_all_report() {
    // A failure in one body must not hide the next function entirely.
    let n = error_count(
        r#"
        fn f() { let a: u8 = nope_one; }
        fn g() { let b: u8 = nope_two; }
        #[reset]
        fn main() { f(); g(); loop {} }
    "#,
    );
    assert_eq!(n, 2, "one error per broken function");
}

#[test]
fn every_reported_error_names_its_own_position() {
    let e = errors_of(
        r#"
        #[reset]
        fn main() {
            let a: u8 = undefined_one;
            let b: u8 = undefined_two;
            loop {}
        }
    "#,
    );
    assert!(e.contains("--> 4:"), "first error at its line:\n{e}");
    assert!(e.contains("--> 5:"), "second error at its line:\n{e}");
}

// ---------------------------------------------------------------------------
// Cascades are suppressed (the anti-goal)
// ---------------------------------------------------------------------------

#[test]
fn a_failed_initializer_does_not_undeclare_the_variable() {
    // Only `missing` is wrong. `x` and `y` are declared with known types, so
    // their later uses must resolve — otherwise one real mistake reports three
    // times, which is worse than the old fail-fast behavior.
    let e = errors_of(
        r#"
        #[reset]
        fn main() {
            let x: u8 = missing;
            let y: u8 = x + 1;
            let z: u8 = y + x;
            loop {}
        }
    "#,
    );
    assert_eq!(
        e.matches("error:").count(),
        1,
        "one mistake, one diagnostic:\n{e}"
    );
    assert!(e.contains("missing"), "and it names the real cause:\n{e}");
}

#[test]
fn a_broken_function_does_not_cascade_into_its_callers() {
    let e = errors_of(
        r#"
        fn helper() -> u8 { return oops; }
        #[reset]
        fn main() { let v: u8 = helper(); loop {} }
    "#,
    );
    assert_eq!(
        e.matches("error:").count(),
        1,
        "the call site is fine; only the body is broken:\n{e}"
    );
}

// ---------------------------------------------------------------------------
// Bounds and output
// ---------------------------------------------------------------------------

#[test]
fn the_error_count_is_capped() {
    // A pathological file must not produce unbounded output.
    let mut src = String::from("#[reset]\nfn main() {\n");
    for i in 0..200 {
        src.push_str(&format!("    let v{i}: u8 = missing{i};\n"));
    }
    src.push_str("    loop {}\n}\n");
    let n = error_count(&src);
    assert!(n <= 50, "capped at 50, got {n}");
    assert!(n > 1, "but still reports many, got {n}");
}

#[test]
fn a_file_with_errors_produces_no_assembly() {
    // Multi-error reporting is a diagnostics change: nothing partial is emitted.
    match compile(
        r#"
        #[reset]
        fn main() { let a: u8 = one_bad; let b: u8 = two_bad; loop {} }
    "#,
    ) {
        CompileResult::SemaError(_) => {}
        other => panic!("expected failure with no output, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// A single error is unchanged
// ---------------------------------------------------------------------------

#[test]
fn a_lone_error_is_reported_exactly_as_before() {
    // `take_errors` returns a single error as itself rather than wrapping it, so
    // the golden diagnostics in error_diagnostics.rs keep their exact shape.
    let e = errors_of("#[reset]\nfn main() { let x: u8 = y; loop {} }\n");
    assert!(e.starts_with("error: cannot find `y` in this scope"), "{e}");
    assert!(!e.contains("semantic errors:"), "not wrapped:\n{e}");
}
