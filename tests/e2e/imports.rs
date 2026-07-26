//! Module import tests: glob imports, and dead-import elimination.
//!
//! Importing a module makes its *whole file* available — an imported function
//! may call siblings the importing program never named — so codegen receives
//! every item in the file and decides what to keep by liveness. Two things are
//! worth testing separately: that the reachable code still runs correctly
//! (execution), and that the unreachable code is genuinely absent from the
//! output (text). A program can be perfectly correct and still carry a kilobyte
//! of standard library it never calls.
//!
//! The fixture module is `tests/fixtures/toolkit.wr`. Import paths here are
//! relative to the crate root, which is the working directory under `cargo test`.

use crate::common::exec::run;
use crate::common::harness::{CompileResult, compile, compile_success};

const TOOLKIT: &str = "tests/fixtures/toolkit.wr";

/// Build a program with the given import line and body, then run it and read
/// back the u8 left at `$0900`.
fn out8(import: &str, body: &str) -> u8 {
    run(&source(import, body)).mem(0x0900)
}

fn source(import: &str, body: &str) -> String {
    format!(
        r#"
        import {import} from "{TOOLKIT}";
        const OUT: addr = 0x0900;
        #[reset]
        fn main() {{
            {body}
            loop {{}}
        }}
    "#
    )
}

// ============================================================================
// Glob imports
// ============================================================================

#[test]
fn glob_import_brings_in_every_pub_item() {
    // Neither name is listed anywhere; `*` is the only thing making them visible.
    assert_eq!(out8("{ * }", "OUT = double(add_three(10));"), 26);
}

#[test]
fn glob_import_works_without_braces() {
    assert_eq!(out8("*", "OUT = double(21);"), 42);
}

#[test]
fn glob_and_named_imports_can_be_mixed() {
    // Naming something the glob already covers is redundant, not an error.
    assert_eq!(out8("{ double, * }", "OUT = double(add_three(1));"), 8);
}

#[test]
fn glob_import_does_not_expose_private_items() {
    // `private_helper` has no `pub`, so the wildcard must not reach it. Were the
    // glob to import everything regardless of visibility, this would compile.
    let result = compile(&source("{ * }", "OUT = private_helper(1);"));
    match result {
        CompileResult::SemaError(e) => {
            let msg = &e;
            assert!(
                msg.contains("private_helper"),
                "error should name the private symbol, got: {msg}"
            );
        }
        other => panic!("expected a semantic error, got {other:?}"),
    }
}

#[test]
fn a_named_import_of_a_private_item_is_still_rejected() {
    // The glob path and the named path must agree about visibility.
    let result = compile(&source("{ private_helper }", "OUT = private_helper(1);"));
    assert!(
        matches!(result, CompileResult::SemaError(_)),
        "importing a private symbol by name must fail"
    );
}

#[test]
fn glob_import_emits_the_same_code_as_naming_the_symbols() {
    // A wildcard is an ergonomic shorthand, not a different compilation mode:
    // once the dead items are dropped, the two must be indistinguishable.
    let body = "OUT = double(add_three(10));";
    let globbed = compile_success(&source("{ * }", body));
    let named = compile_success(&source("{ double, add_three }", body));
    assert_eq!(globbed, named);
}

// ============================================================================
// Dead import elimination
// ============================================================================

/// Labels of the functions defined in the output, i.e. `name:` at column 0.
fn defined_functions(asm: &str) -> Vec<String> {
    asm.lines()
        .filter_map(|l| l.strip_suffix(':'))
        .filter(|l| !l.starts_with(char::is_whitespace) && !l.is_empty())
        .map(str::to_string)
        .collect()
}

#[test]
fn unused_imported_functions_are_not_emitted() {
    let asm = compile_success(&source("{ * }", "OUT = double(4);"));
    let defined = defined_functions(&asm);

    assert!(
        defined.iter().any(|f| f == "double"),
        "the called function must be emitted, got {defined:?}"
    );
    for dead in ["unused_helper", "unused_caller", "unused_leaf", "asm_only"] {
        assert!(
            !defined.iter().any(|f| f == dead),
            "`{dead}` is never reached and must not be emitted, got {defined:?}"
        );
    }
}

#[test]
fn dead_imports_are_dropped_transitively() {
    // `unused_caller` is dead, so `unused_leaf` — reachable only through it —
    // must go too. A one-level check would keep the leaf alive on the grounds
    // that "something calls it".
    let asm = compile_success(&source("{ * }", "OUT = double(4);"));
    assert!(!asm.contains("unused_leaf"));
}

#[test]
fn a_live_import_keeps_the_private_helpers_it_calls() {
    // `bump` is private to the module and named by nothing in the importing
    // file; only the call inside `add_three` keeps it alive. This is the case
    // that makes liveness-by-import-list wrong.
    let asm = compile_success(&source("{ * }", "OUT = add_three(0);"));
    let defined = defined_functions(&asm);
    assert!(
        defined.iter().any(|f| f == "bump"),
        "a private sibling of a live function must survive, got {defined:?}"
    );

    // And it has to actually work, not merely be present.
    assert_eq!(out8("{ * }", "OUT = add_three(0);"), 3);
}

#[test]
fn a_function_reached_only_from_inline_asm_survives() {
    // Nothing calls `asm_only` from Wraith — the only reference is the JSR in
    // the assembly below. Dropping it would assemble to a jump into whatever
    // happens to follow.
    let src = format!(
        r#"
        import {{ * }} from "{TOOLKIT}";
        const OUT: addr = 0x0900;
        fn via_asm() -> u8 {{
            asm {{
                "JSR asm_only",
            }}
        }}
        #[reset]
        fn main() {{
            OUT = via_asm();
            loop {{}}
        }}
    "#
    );

    let asm = compile_success(&src);
    assert!(
        defined_functions(&asm).iter().any(|f| f == "asm_only"),
        "a function called only from inline assembly must be kept"
    );
    assert_eq!(run(&src).mem(0x0900), 77);
}

#[test]
fn importing_a_module_and_using_nothing_from_it_emits_none_of_it() {
    let asm = compile_success(&source("{ * }", "OUT = 5;"));
    for f in [
        "double",
        "add_three",
        "bump",
        "unused_helper",
        "unused_caller",
        "unused_leaf",
        "asm_only",
        "private_helper",
    ] {
        assert!(
            !defined_functions(&asm).iter().any(|d| d == f),
            "nothing from the module is used, so `{f}` should not be emitted"
        );
    }
    assert_eq!(out8("{ * }", "OUT = 5;"), 5);
}

#[test]
fn taking_a_functions_address_keeps_it_alive() {
    // A function pointer is a reference no call edge records. If address-taken
    // functions were not treated as roots, this would drop `unused_helper` and
    // then call through a pointer into whatever followed it.
    let body = "let f: fn(u8) -> u8 = unused_helper; OUT = f(1);";
    let asm = compile_success(&source("{ * }", body));
    assert!(
        defined_functions(&asm).iter().any(|f| f == "unused_helper"),
        "an address-taken function must survive"
    );
    assert_eq!(out8("{ * }", body), 101);
}

#[test]
fn dropping_dead_imports_shrinks_the_output() {
    // The point of the exercise: pulling in a library must not cost code size
    // for the parts you don't call. Both programs run the same call, so the
    // only difference is that the second forces the rest of the module live by
    // taking its functions' addresses.
    let lean = compile_success(&source("{ * }", "OUT = double(4);"));
    let forced = compile_success(&source(
        "{ * }",
        "let a: fn(u8) -> u8 = unused_helper; \
         let b: fn(u8) -> u8 = unused_caller; \
         OUT = double(4);",
    ));
    assert!(
        lean.len() < forced.len(),
        "the unreferenced functions should be absent ({} vs {} bytes)",
        lean.len(),
        forced.len()
    );
}

// ============================================================================
// The importing module's own code
// ============================================================================

#[test]
fn an_unused_function_in_the_importing_file_is_dropped_too() {
    // Elimination is not special-cased to imported modules: an unreachable
    // function is dead wherever it was written. The difference is that one
    // written here also warns.
    let src = format!(
        r#"
        import {{ * }} from "{TOOLKIT}";
        const OUT: addr = 0x0900;
        fn never_called(x: u8) -> u8 {{ return x + 1; }}
        #[reset]
        fn main() {{
            OUT = double(4);
            loop {{}}
        }}
    "#
    );
    assert!(
        !defined_functions(&compile_success(&src))
            .iter()
            .any(|f| f == "never_called"),
        "an unreachable function in the root module is dropped"
    );
}
