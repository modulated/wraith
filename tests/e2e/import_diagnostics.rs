//! Diagnostics for failures inside imported modules.
//!
//! An error raised while analyzing an imported module carries that module's
//! spans, and the driver only ever reads the file it was asked to compile. So
//! these errors used to come out unusable: a parse failure was printed as the
//! `Debug` formatting of the error struct, and a type error resolved its span
//! against the wrong source and gave up, leaving a bare note with no file, line
//! or excerpt.
//!
//! They are now rendered where they are caught — by the level that is holding
//! the failing module's text — and carried up already formatted, with each
//! import in the chain adding its own hop.

use crate::common::harness::{CompileResult, compile};

fn expect_error(src: &str) -> String {
    match compile(src) {
        CompileResult::SemaError(e) => e,
        CompileResult::Success(..) => panic!("expected a compile error, but it compiled"),
        other => panic!("expected a semantic error, got {other:?}"),
    }
}

// ============================================================================
// The failing module is named, with a real line and excerpt
// ============================================================================

#[test]
fn a_parse_error_in_an_imported_module_points_at_that_module() {
    let err = expect_error(
        r#"
        import * from "tests/fixtures/broken_syntax.wr";
        const OUT: addr = 0x0900;
        #[reset]
        fn main() { OUT = ok(); loop {} }
    "#,
    );

    assert!(
        err.contains("tests/fixtures/broken_syntax.wr:10:"),
        "the file and line of the stray `;` should be named: {err}"
    );
    assert!(
        err.contains("};"),
        "the offending line should be quoted: {err}"
    );
    assert!(
        !err.contains("ParseError {"),
        "the error struct must not be Debug-printed: {err}"
    );
    assert!(
        !err.contains("Span {"),
        "raw span internals must not leak: {err}"
    );
}

#[test]
fn a_type_error_in_an_imported_module_points_at_that_module() {
    let err = expect_error(
        r#"
        import * from "tests/fixtures/broken_types.wr";
        const OUT: addr = 0x0900;
        #[reset]
        fn main() { set_direction(Direction::OUTPUT); OUT = 1; loop {} }
    "#,
    );

    assert!(
        err.contains("tests/fixtures/broken_types.wr:12:"),
        "the assignment's own line should be named: {err}"
    );
    assert!(
        err.contains("DDRA = d;"),
        "the line should be quoted: {err}"
    );
    assert!(
        err.contains("expected u8, found Direction"),
        "the diagnosis itself must survive: {err}"
    );
    assert!(
        !err.contains("another module"),
        "it should render the module, not decline to: {err}"
    );
}

// ============================================================================
// The trail back to the file being compiled
// ============================================================================

#[test]
fn the_import_that_pulled_the_module_in_is_shown() {
    let err = expect_error(
        r#"
        import * from "tests/fixtures/broken_types.wr";
        const OUT: addr = 0x0900;
        #[reset]
        fn main() { set_direction(Direction::OUTPUT); OUT = 1; loop {} }
    "#,
    );
    assert!(
        err.contains("imported here"),
        "the import statement should be pointed at too: {err}"
    );
    assert!(
        err.contains("import * from"),
        "and the import line quoted: {err}"
    );
}

#[test]
fn a_failure_two_modules_deep_shows_every_hop() {
    // The root imports `imports_broken.wr`, which imports `broken_types.wr`,
    // which is where the error is. Each level can only render the source it
    // holds, so the chain has to be assembled as it unwinds.
    let err = expect_error(
        r#"
        import * from "tests/fixtures/imports_broken.wr";
        const OUT: addr = 0x0900;
        #[reset]
        fn main() { wrap(); OUT = 1; loop {} }
    "#,
    );

    assert!(
        err.contains("broken_types.wr:12:"),
        "the innermost failure keeps its own location: {err}"
    );
    assert!(
        err.contains("imports_broken.wr"),
        "the module in the middle should appear: {err}"
    );
    assert!(
        !err.contains("another module"),
        "no hop should be left unrendered: {err}"
    );
}

// ============================================================================
// A missing module
// ============================================================================

#[test]
fn a_missing_module_is_reported_once_against_the_importing_file() {
    let err = expect_error(
        r#"
        import * from "tests/fixtures/does_not_exist.wr";
        const OUT: addr = 0x0900;
        #[reset]
        fn main() { OUT = 1; loop {} }
    "#,
    );
    assert!(err.contains("does_not_exist.wr"), "{err}");
    assert_eq!(
        err.matches("failed to import").count(),
        1,
        "the reason should not be prefixed twice: {err}"
    );
}
