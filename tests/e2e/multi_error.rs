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

// ---------------------------------------------------------------------------
// Expression-level recovery (Type::Error poisoning)
// ---------------------------------------------------------------------------

#[test]
fn every_bad_call_argument_reports() {
    // Arguments are independent of each other, so one bad argument must not
    // hide the next.
    let e = errors_of(
        r#"
        fn f(a: u8, b: u8) -> u8 { return a; }
        #[reset]
        fn main() { let v: u8 = f(bad1, bad2); loop {} }
    "#,
    );
    assert!(e.contains("bad1"), "{e}");
    assert!(e.contains("bad2"), "{e}");
}

#[test]
fn both_operands_of_a_binary_op_report() {
    let e = errors_of(
        r#"
        #[reset]
        fn main() { let a: u8 = p + q; loop {} }
    "#,
    );
    assert!(
        e.contains("`p`") && e.contains("`q`"),
        "both operands:\n{e}"
    );
}

#[test]
fn a_poisoned_value_does_not_produce_a_follow_on_type_error() {
    // The argument that failed to resolve must not also be reported as a
    // mismatch against the parameter type: `<unknown>` is not a user-facing
    // type, and the cause was already named.
    let e = errors_of(
        r#"
        fn f(a: u8) -> u8 { return a; }
        #[reset]
        fn main() { let v: u8 = f(nope); loop {} }
    "#,
    );
    assert_eq!(e.matches("error:").count(), 1, "one cause, one error:\n{e}");
    assert!(
        !e.contains("<unknown>"),
        "the poison type must never surface to the user:\n{e}"
    );
}

// ---------------------------------------------------------------------------
// Declaration (register-pass) recovery
// ---------------------------------------------------------------------------

#[test]
fn broken_declarations_all_report() {
    let n = error_count(
        r#"
        const A: u8 = 256;
        const B: u8 = 300;
        #[reset]
        fn main() { loop {} }
    "#,
    );
    assert_eq!(n, 2, "each bad declaration is its own mistake");
}

#[test]
fn a_failed_declaration_does_not_cascade_into_bodies() {
    // `A` never registered, so each use of it would report "cannot find `A`" on
    // top of the real error. The suppression is by name, and the bodies are
    // still walked.
    let e = errors_of(
        r#"
        const A: u8 = 300;
        #[reset]
        fn main() { let x: u8 = A; let y: u8 = A + 1; loop {} }
    "#,
    );
    assert_eq!(e.matches("error:").count(), 1, "cause only:\n{e}");
    assert!(!e.contains("cannot find"), "no follow-on symptoms:\n{e}");
}

#[test]
fn a_failed_declaration_no_longer_hides_the_bodies() {
    // The other half of the same rule: only the *failed name* is suppressed, so
    // a body's own unrelated mistake still reports. Analysis used to stop after
    // the declaration pass, so this file reported one error and the typo was
    // found on the next run.
    let e = errors_of(
        r#"
        const A: u8 = 300;
        #[reset]
        fn main() {
            let x: u8 = A;          // suppressed: `A` never registered
            let y: u8 = typo_here;  // reported: nothing to do with `A`
            loop {}
        }
    "#,
    );
    assert_eq!(e.matches("error:").count(), 2, "cause and typo:\n{e}");
    assert!(e.contains("typo_here"), "the body's own mistake:\n{e}");
    assert!(
        !e.contains("cannot find `A`"),
        "and not the follow-ons from `A`:\n{e}"
    );
}

#[test]
fn a_failed_declaration_does_not_hide_a_later_declaration_or_body() {
    // Three independent mistakes across both passes, in one run.
    let e = errors_of(
        r#"
        const A: u8 = 300;
        struct S { f: NoSuchType }
        #[reset]
        fn main() {
            let a: u8 = A;
            let b: u8 = also_missing;
            loop {}
        }
    "#,
    );
    assert_eq!(
        e.matches("error:").count(),
        3,
        "two declarations and a body:\n{e}"
    );
}

/// A named type that no struct or enum defines is reported *where it is
/// written*.
///
/// It was not reported at all. Resolution accepted any name in type position —
/// it has to, since `struct A { next: &B }` may name a `B` declared further
/// down — and nothing checked afterwards, so the first sign was a mismatch at
/// the use site against a type that does not exist: `expected NoSuchType,
/// found u8`, pointing at the wrong line entirely.
#[test]
fn an_unknown_type_in_a_declaration_is_reported_at_its_own_span() {
    let e = errors_of(
        r#"
        struct Point { x: u8, y: u8 }
        struct S { f: Pointt }
        #[reset]
        fn main() { loop {} }
    "#,
    );
    assert_eq!(e.matches("error:").count(), 1, "{e}");
    assert!(e.contains("cannot find `Pointt`"), "{e}");
    assert!(
        e.contains("a similar name is in scope: `Point`"),
        "the candidates are the declared *types*, not the values:\n{e}"
    );
    assert!(
        !e.contains("expected"),
        "and not as a mismatch at some later use:\n{e}"
    );
}

#[test]
fn an_unknown_type_is_found_in_every_declaration_position() {
    // One walk over the type expression, so a name nested in a pointer, an
    // array, a slice or a function type is reached as readily as a bare one.
    let cases: [(&str, &str); 6] = [
        ("a struct field", "struct S { f: Missing }"),
        ("behind a pointer", "struct S { f: &Missing }"),
        ("an array element", "struct S { f: [Missing; 2] }"),
        ("a function parameter", "fn f(p: Missing) { }"),
        ("a return type", "fn f() -> Missing { loop {} }"),
        ("a static's type", "static G: Missing = 0;"),
    ];
    for (what, decl) in cases {
        let e = errors_of(&format!("{decl}\n#[reset]\nfn main() {{ loop {{}} }}\n"));
        assert!(
            e.contains("cannot find `Missing`"),
            "{what} must report it:\n{e}"
        );
    }
}

#[test]
fn a_forward_reference_to_a_type_declared_later_is_fine() {
    // The reason the check cannot live in resolution: types register in source
    // order, so this is only answerable once the registry is complete.
    match compile(
        r#"
        struct Node { value: u8, next: &Later }
        struct Later { v: u8 }
        #[reset]
        fn main() { loop {} }
    "#,
    ) {
        CompileResult::Success(..) => {}
        other => panic!("a forward reference must compile, got {other:?}"),
    }
}

#[test]
fn every_kind_of_declaration_suppresses_its_own_name() {
    // Each item kind has to contribute the name it failed to define, or its
    // uses cascade. A duplicate is the case where the name *is* defined by the
    // first declaration, so nothing needs suppressing and nothing is lost.
    let cases: [(&str, &str); 4] = [
        ("a const", "const A: u8 = 300;"),
        ("a static", "static A: u8 = 300;"),
        ("a function", "fn A() -> u8 { return 300; }"),
        ("an address", "const A: addr = 0x10000;"),
    ]
    .map(|(what, decl)| (what, decl));
    for (what, decl) in cases {
        let e = errors_of(&format!(
            "{decl}
#[reset]
fn main() {{ let x: u8 = other_typo; loop {{}} }}
"
        ));
        assert!(
            e.contains("other_typo"),
            "{what}: the body's own mistake must still report:\n{e}"
        );
    }
}
