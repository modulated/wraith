//! Golden tests for the top diagnostics, in the exact-position style of
//! `import_diagnostics.rs`.
//!
//! Each case pins the rendered diagnostic: its `--> line:col` position, a
//! keyword from the message, and — via `render` — the shared invariant that
//! every error carries a caret excerpt and never leaks a `Debug`-printed struct
//! or raw `Span`. A regression that moves the caret, drops the excerpt, or
//! coarsens a message trips the matching test with a readable diff.
//!
//! The snippets are written so the interesting token sits at a stable column;
//! `compile` renders with no filename, so the position line is a bare
//! `--> line:col`.
//!
//! A diagnostic that is a property of the whole program rather than of one
//! token carries no span, and so goes through `render_spanless` instead. That
//! is a deliberately narrow exemption: everything that *can* point at a token
//! must, and using the spanless helper for a diagnostic that should have had a
//! caret is the mistake this split exists to make visible.

use crate::common::harness::{CompileResult, compile};

/// Compile `src`, expect it to fail, and return the rendered diagnostic — after
/// asserting the invariants every rendered error must satisfy.
fn render(src: &str) -> String {
    let err = match compile(src) {
        CompileResult::SemaError(e)
        | CompileResult::ParseError(e)
        | CompileResult::LexError(e)
        | CompileResult::CodegenError(e) => e,
        CompileResult::Success(..) => panic!("expected a compile error, but it compiled"),
    };
    // A caret excerpt, and no internals leaking through Debug formatting.
    assert!(err.contains('^'), "diagnostic has no caret excerpt:\n{err}");
    for leak in [
        "Span {",
        "Error {",
        "ParseError {",
        "SemaError::",
        "TokenKind::",
    ] {
        assert!(!err.contains(leak), "diagnostic leaks `{leak}`:\n{err}");
    }
    err
}

/// Compile `src` and return the rendered diagnostic without requiring a caret
/// excerpt. A few diagnostics are properties of the whole program rather than
/// of one token (the frame budget is the standing example) and so carry no
/// span; they still must not leak internals.
fn render_spanless(src: &str) -> String {
    let err = match compile(src) {
        CompileResult::SemaError(e)
        | CompileResult::ParseError(e)
        | CompileResult::LexError(e)
        | CompileResult::CodegenError(e) => e,
        CompileResult::Success(..) => panic!("expected a compile error, but it compiled"),
    };
    for leak in ["Span {", "Error {", "SemaError::", "TokenKind::"] {
        assert!(!err.contains(leak), "diagnostic leaks `{leak}`:\n{err}");
    }
    err
}

/// Assert the rendered error names an exact position and carries the keyword.
fn assert_at(err: &str, pos: &str, keyword: &str) {
    assert!(err.contains(pos), "expected position `{pos}`:\n{err}");
    assert!(
        err.contains(keyword),
        "expected message to contain `{keyword}`:\n{err}"
    );
}

#[test]
fn undefined_variable() {
    let e = render("#[reset]\nfn main() { let x: u8 = y; loop {} }\n");
    assert_at(&e, "--> 2:25", "cannot find `y` in this scope");
}

#[test]
fn undefined_function() {
    let e = render("#[reset]\nfn main() { let x: u8 = nope(); loop {} }\n");
    assert_at(&e, "--> 2:25", "cannot find `nope` in this scope");
}

#[test]
fn undefined_variable_suggests_a_similar_name() {
    // rustc-style "did you mean": a near typo points at the real name.
    let e = render("#[reset]\nfn main() { let counter: u8 = 0; let x: u8 = countr; loop {} }\n");
    assert!(
        e.contains("a similar name is in scope: `counter`"),
        "expected a did-you-mean hint:\n{e}"
    );
}

#[test]
fn wrong_argument_count_reads_naturally() {
    // The top line is the rustc-style summary; the caret keeps the exact counts.
    let e = render(
        "fn add(a: u8, b: u8) -> u8 { return a; }\n#[reset]\nfn main() { let x: u8 = add(1); loop {} }\n",
    );
    assert!(
        e.contains("this function takes 2 arguments but 1 was supplied"),
        "{e}"
    );
}

#[test]
fn an_actionable_help_line_accompanies_common_errors() {
    // A const write names the fix; an out-of-range literal names the range.
    let assign = render("const K: u8 = 5;\n#[reset]\nfn main() { K = 6; loop {} }\n");
    assert!(
        assign.contains("= help:") && assign.contains("`const` is fixed"),
        "{assign}"
    );
    let overflow = render("const K: u8 = 256;\n#[reset]\nfn main() { loop {} }\n");
    assert!(
        overflow.contains("= note: `u8` holds 0 to 255"),
        "{overflow}"
    );
}

#[test]
fn mixed_width_binary_operation() {
    let e = render(
        "#[reset]\nfn main() { let a: u8 = 1; let b: u16 = 2 as u16; let c: u8 = a + b; loop {} }\n",
    );
    assert_at(&e, "--> 2:63", "binary operation");
}

#[test]
fn constant_overflow() {
    let e = render("const K: u8 = 256;\n#[reset]\nfn main() { loop {} }\n");
    assert_at(&e, "--> 1:15", "constant value 256 does not fit in type u8");
}

#[test]
fn wrong_argument_count() {
    let e = render(
        "fn add(a: u8, b: u8) -> u8 { return a + b; }\n#[reset]\nfn main() { let x: u8 = add(1); loop {} }\n",
    );
    assert_at(&e, "--> 3:25", "expected 2 argument(s), found 1");
}

#[test]
fn array_index_out_of_bounds() {
    let e =
        render("static T: [u8; 4] = [0; 4];\n#[reset]\nfn main() { let x: u8 = T[9]; loop {} }\n");
    assert_at(&e, "--> 3:27", "out of bounds");
}

#[test]
fn assign_to_const() {
    let e = render("const K: u8 = 5;\n#[reset]\nfn main() { K = 6; loop {} }\n");
    assert_at(&e, "--> 3:13", "cannot assign to immutable variable 'K'");
}

#[test]
fn read_from_write_only_address() {
    let e = render(
        "const REG: write addr = 0x6000;\n#[reset]\nfn main() { let x: u8 = REG; loop {} }\n",
    );
    assert_at(&e, "--> 3:25", "cannot read from write-only address 'REG'");
}

#[test]
fn write_to_read_only_address() {
    let e = render("const REG: read addr = 0x6000;\n#[reset]\nfn main() { REG = 1; loop {} }\n");
    assert_at(&e, "--> 3:13", "cannot write to read-only address 'REG'");
}

#[test]
fn field_not_found() {
    let e = render(
        "struct P { x: u8 }\n#[reset]\nfn main() { let p: P = P { x: 1 }; let y: u8 = p.z; loop {} }\n",
    );
    assert_at(&e, "--> 3:50", "field 'z' not found in struct 'P'");
}

#[test]
fn duplicate_symbol() {
    let e = render("#[reset]\nfn main() { let a: u8 = 1; let a: u8 = 2; loop {} }\n");
    assert_at(&e, "--> 2:32", "duplicate symbol 'a'");
}

#[test]
fn bit_index_out_of_range() {
    let e = render("#[reset]\nfn main() { let f: u8 = 0; f.set_bit(8); loop {} }\n");
    assert_at(&e, "--> 2:38", "out of range");
}

#[test]
fn runtime_bit_index() {
    let e = render("#[reset]\nfn main() { let f: u8 = 0; let i: u8 = 3; f.set_bit(i); loop {} }\n");
    assert_at(&e, "--> 2:53", "compile-time constant");
}

#[test]
fn self_referential_struct() {
    let e = render("struct Node { next: Node }\n#[reset]\nfn main() { loop {} }\n");
    assert_at(&e, "--> 1:8", "contains itself by value");
}

#[test]
fn parse_error_points_at_the_token() {
    let e = render("#[reset]\nfn main() { let x: u8 = ; loop {} }\n");
    assert_at(&e, "--> 2:25", "expected expression, found ';'");
}

// ===========================================================================
// Type errors
//
// `mismatched types` is one variant covering many situations, and the useful
// part is the `expected X, found Y` line: it is what tells a reader whether the
// compiler understood the expression at all. These pin the wording for each
// shape that reaches it, so a refactor that collapses them into a generic
// "type error" is caught.
// ===========================================================================

#[test]
fn mismatched_types_in_a_let_initializer() {
    let e = render("#[reset]\nfn main() { let x: u8 = 1; let p: bool = x; loop {} }\n");
    assert_at(&e, "--> 2:42", "expected `bool`, found `u8`");
}

#[test]
fn mismatched_argument_type_names_both_types() {
    let e = render(
        "fn f(a: u8) -> u8 { return a; }\n#[reset]\nfn main() { let s: str = \"x\"; let y: u8 = f(s); loop {} }\n",
    );
    assert_at(&e, "--> 3:45", "expected `u8`, found `string`");
}

#[test]
fn calling_a_non_function_says_what_was_expected() {
    // The caret sits on the callee, not on the argument list.
    let e = render("#[reset]\nfn main() { let v: u8 = 3; let x: u8 = v(); loop {} }\n");
    assert_at(&e, "--> 2:40", "expected `function`, found `u8`");
}

#[test]
fn indexing_a_non_array_lists_the_indexable_types() {
    // The "expected" side enumerates what *would* have worked, which is the
    // actionable half of the message.
    let e = render("#[reset]\nfn main() { let v: u8 = 3; let x: u8 = v[0]; loop {} }\n");
    assert_at(
        &e,
        "--> 2:40",
        "expected `array, slice, pointer, or string`",
    );
}

#[test]
fn field_access_on_a_non_struct() {
    let e = render("#[reset]\nfn main() { let v: u8 = 3; let x: u8 = v.foo; loop {} }\n");
    assert_at(&e, "--> 2:40", "expected `struct`, found `u8`");
}

#[test]
fn invalid_unary_operand_names_the_operator_and_the_type() {
    let e = render("#[reset]\nfn main() { let s: str = \"hi\"; let x: u8 = -s; loop {} }\n");
    assert_at(&e, "--> 2:44", "cannot apply '-' to type string");
}

// ===========================================================================
// Structs and enums
// ===========================================================================

#[test]
fn unknown_field_in_a_struct_initializer() {
    // Distinct from `field_not_found` above, which reads a field; this is the
    // initializer path, and the caret must land on the offending key.
    let e = render(
        "struct P { x: u8 }\n#[reset]\nfn main() { let p: P = P { x: 1, z: 2 }; loop {} }\n",
    );
    assert_at(&e, "--> 3:34", "field 'z' not found in struct 'P'");
}

#[test]
fn unknown_enum_variant_names_the_enum() {
    let e = render("enum E { A, B }\n#[reset]\nfn main() { let e: E = E::C; loop {} }\n");
    assert_at(&e, "--> 3:27", "variant 'C' not found in enum 'E'");
}

// ===========================================================================
// Statement and declaration placement
// ===========================================================================

#[test]
fn break_outside_a_loop() {
    let e = render("#[reset]\nfn main() { break; loop {} }\n");
    assert_at(&e, "--> 2:13", "break/continue outside loop");
}

#[test]
fn a_statement_at_top_level_is_a_parse_error() {
    // The parser is item-driven at top level, so a stray statement reports
    // "expected item" against the keyword that starts it.
    let e = render("#[reset]\nfn main() { loop {} }\nreturn 1;\n");
    assert_at(&e, "--> 3:1", "expected item, found keyword 'return'");
}

#[test]
fn addr_outside_a_const_declaration_says_where_it_is_allowed() {
    // The caret is on the type, not the variable, and the message names the one
    // context that does work.
    let e = render("#[reset]\nfn main() { let a: addr = 0x10; loop {} }\n");
    assert_at(
        &e,
        "--> 2:20",
        "addr type can only be used in const declarations",
    );
}

#[test]
fn a_duplicate_definition_points_at_the_redefinition() {
    // Not at the original: the second one is the line to delete.
    let e = render(
        "fn f() -> u8 { return 1; }\nfn f() -> u8 { return 2; }\n#[reset]\nfn main() { let x: u8 = f(); loop {} }\n",
    );
    assert_at(&e, "--> 2:4", "duplicate symbol 'f'");
}

#[test]
fn a_name_colliding_with_a_mnemonic_is_rejected() {
    // Inline asm resolves bare identifiers, so a function named `LDA` would be
    // ambiguous inside an `asm` block.
    let e = render("fn LDA() { }\n#[reset]\nfn main() { LDA(); loop {} }\n");
    assert_at(&e, "--> 1:4", "conflicts with instruction mnemonic");
}

// ===========================================================================
// Whole-program diagnostics
// ===========================================================================

#[test]
fn an_escaping_pointer_explains_why_it_is_unsafe() {
    // The message has to carry the *reason*: "returns a pointer to a local" is
    // meaningless on a machine whose frames are statically colored unless it
    // also says the frame gets reused.
    let e = render(
        "fn f() -> &u8 { let local: u8 = 1; return &local; }\n#[reset]\nfn main() { let p: &u8 = f(); loop {} }\n",
    );
    assert_at(&e, "--> 1:43", "pointer escapes its frame");
    assert!(
        e.contains("reused by unrelated functions"),
        "expected the diagnostic to explain the reuse hazard:\n{e}"
    );
}

#[test]
fn frame_overflow_reports_the_budget_it_blew() {
    // Spanless by nature: no single token is at fault, so the diagnostic has to
    // carry the numbers instead — what was needed, what was available, and
    // which functions made up the deepest chain.
    let e = render_spanless(
        "struct Big { data: [u8; 200] }\n#[reset]\nfn main() { let b: Big = { data: [0; 200] }; b.data[0] = 1; loop {} }\n",
    );
    assert!(e.contains("zero-page frame region overflow"), "{e}");
    assert!(e.contains("200 bytes"), "expected the requirement:\n{e}");
    assert!(
        e.contains("$40-$CF, 144 bytes"),
        "expected the frame region and its size:\n{e}"
    );
    assert!(e.contains("main"), "expected the offending chain:\n{e}");
}

#[test]
fn missing_return_points_at_the_function_name() {
    // Not at the closing brace: the name is what the reader scans for, and the
    // fix (adding a `return`) is not necessarily at the end of the body.
    let e = render(
        "fn f(n: u8) -> u8 { if n == 0 { return 1; } }\n#[reset]\nfn main() { let x: u8 = f(2); loop {} }\n",
    );
    assert_at(&e, "--> 1:4", "must return a value of type `u8`");
    assert!(
        e.contains("missing return in function 'f'"),
        "expected the function named in the summary:\n{e}"
    );
    assert!(
        e.contains("= help:") && e.contains("every path"),
        "expected a help line naming the requirement:\n{e}"
    );
}

#[test]
fn returning_a_value_from_a_void_function() {
    let e = render("fn f() { return 5; }\n#[reset]\nfn main() { f(); loop {} }\n");
    assert_at(&e, "--> 1:17", "expected void, found u8");
}

#[test]
fn a_divisor_known_to_be_zero_points_at_the_divisor() {
    // `x / 0` has a defined answer — the all-ones sentinel — but no program
    // means it, so the constant case is refused. The span is the *divisor*,
    // not the whole expression: that is the sub-expression to change.
    let e = render(
        "#[reset]\nfn main() { let a: u8 = 5; let x: u8 = a / 0; OUT = x; loop {} }\nconst OUT: addr = 0x0900;\n",
    );
    assert_at(&e, "--> 2:44", "division by zero");
    assert!(
        e.contains("all-ones"),
        "expected the message to say what the value would have been:\n{e}"
    );
}

#[test]
fn a_modulo_by_a_constant_zero_is_refused_too() {
    // Reached through a constant *expression*, not just a literal, since that
    // is as much as the compiler can decide here.
    let e = render(
        "#[reset]\nfn main() { let a: u8 = 5; let x: u8 = a % (3 - 3); OUT = x; loop {} }\nconst OUT: addr = 0x0900;\n",
    );
    assert!(
        e.contains("modulo by zero"),
        "expected a modulo-specific message:\n{e}"
    );
}

// ============================================================================
// Returning the wrong type
// ============================================================================

#[test]
fn returning_the_wrong_type_names_both_and_points_at_the_expression() {
    // The span is the returned *expression*, not the `return` keyword and not
    // the function: that is the thing to change.
    let e = render(
        "fn f() -> u8 { let s: str = \"x\"; return s; }\n#[reset]\nfn main() { let y: u8 = f(); loop {} }\n",
    );
    assert_at(&e, "--> 1:41", "expected u8, found str");
}

#[test]
fn returning_a_wider_type_than_declared_is_refused() {
    // Widening is implicit in one direction only. `u16` into a `-> u8` loses
    // the high byte, so it needs a written cast rather than a silent truncation.
    let e = render(
        "fn f() -> u8 { let w: u16 = 300; return w; }\n#[reset]\nfn main() { let y: u8 = f(); loop {} }\n",
    );
    assert_at(&e, "--> 1:41", "expected u8, found u16");
}

// ============================================================================
// Imports
// ============================================================================

#[test]
fn a_module_that_cannot_be_read_names_the_path_and_the_reason() {
    let e =
        render("import { a } from \"tests/fixtures/nope.wr\";\n#[reset]\nfn main() { loop {} }\n");
    assert_at(&e, "--> 1:19", "failed to import 'tests/fixtures/nope.wr'");
    assert!(
        e.contains("No such file") || e.contains("cannot find"),
        "the reason the file could not be read should survive:\n{e}"
    );
}

#[test]
fn a_failure_inside_a_module_is_rendered_against_that_modules_source() {
    // The one diagnostic that cannot be rendered by the driver: its spans index
    // a file the driver never read. It arrives already formatted, with the
    // import that pulled the module in shown beneath it.
    let e = render(
        "import * from \"tests/fixtures/broken_types.wr\";\n#[reset]\nfn main() { loop {} }\n",
    );
    assert!(
        e.contains("tests/fixtures/broken_types.wr:12:"),
        "the position inside the module:\n{e}"
    );
    assert_at(&e, "--> 1:15", "imported here");
}

#[test]
fn a_second_path_to_a_broken_module_points_at_the_report() {
    let e = render(
        "import { ONE } from \"tests/fixtures/via_one.wr\";\n\
         import { TWO } from \"tests/fixtures/via_two.wr\";\n\
         #[reset]\nfn main() { let x: u8 = ONE + TWO; loop {} }\n",
    );
    assert_at(&e, "--> 2:21", "has errors, reported above");
}

#[test]
fn a_circular_import_names_the_cycle() {
    let e = render_spanless(
        "import { a_value } from \"tests/fixtures/cycle_a.wr\";\n\
         #[reset]\nfn main() { loop {} }\n",
    );
    assert!(e.contains("circular"), "{e}");
    assert!(
        e.contains("cycle_a.wr") && e.contains("cycle_b.wr"),
        "both modules in the cycle should be named:\n{e}"
    );
}

// ============================================================================
// Every diagnostic is pinned, or says why not
// ============================================================================

/// Diagnostics that no test above can pin, with the reason.
///
/// Keep this list short and keep the reasons true: a variant parked here is one
/// whose rendering nothing checks, and the point of the check below is that
/// adding one has to be a decision rather than an oversight.
const UNPINNED: &[(&str, &str)] = &[(
    "Multiple",
    "not a diagnostic of its own — it renders its children, which \
         tests/e2e/multi_error.rs covers by counting them",
)];

/// Every `SemaError` variant is either pinned by a test in this file or listed
/// in `UNPINNED` with a reason.
///
/// The variant list is read from the source at test time, in the same style as
/// the fuzzer's AST coverage: a diagnostic added to the compiler shows up here
/// as unpinned rather than going unmentioned. `OutOfZeroPage` was the case that
/// prompted this — a variant with two renderings and no construction site
/// anywhere, so the golden test it was down for could not be written. It is
/// deleted now; this is what would have said so.
#[test]
fn every_sema_error_variant_is_pinned_or_excused() {
    let src = include_str!("../../src/sema/mod.rs");
    let body = src
        .split_once("pub enum SemaError {")
        .expect("the error enum")
        .1
        .split_once("\n}\n")
        .expect("its closing brace")
        .0;

    // A variant is a line indented exactly one level, starting with a capital.
    let variants: Vec<&str> = body
        .lines()
        .filter_map(|l| {
            let rest = l.strip_prefix("    ")?;
            if rest.starts_with(' ') || !rest.starts_with(char::is_uppercase) {
                return None;
            }
            let name = rest.split(['{', '(', ',', ' ']).next()?;
            name.chars().all(char::is_alphanumeric).then_some(name)
        })
        .collect();
    assert!(
        variants.len() > 20,
        "the variant scrape found only {variants:?} — the enum's shape must have changed"
    );

    let tests = include_str!("error_diagnostics.rs");
    let excused: Vec<&str> = UNPINNED.iter().map(|(n, _)| *n).collect();

    // A variant is "pinned" when this file names it in a comment or a test —
    // which is what the `// pins: Name` markers below are for, since a
    // diagnostic's rendering rarely contains its variant's name.
    let unpinned: Vec<&str> = variants
        .iter()
        .filter(|v| !excused.contains(v) && !tests.contains(&format!("pins: {v}")))
        .copied()
        .collect();
    assert!(
        unpinned.is_empty(),
        "these diagnostics have no golden test and no reason: {unpinned:?}\n\
         Add a test above with a `// pins: <Variant>` marker, or a line in UNPINNED."
    );
}

// The markers the check above reads. Kept in one block so the list is legible,
// rather than scattered through the file where a rename would miss one.
//
// pins: UndefinedSymbol
// pins: TypeMismatch
// pins: InvalidBinaryOp
// pins: InvalidUnaryOp
// pins: ArityMismatch
// pins: ImmutableAssignment
// pins: CircularImport
// pins: ReturnTypeMismatch
// pins: MissingReturn
// pins: BreakOutsideLoop
// pins: DuplicateSymbol
// pins: FieldNotFound
// pins: EscapingPointer
// pins: ImportError
// pins: InModule
// pins: ImportFailedElsewhere
// pins: FrameRegionOverflow
// pins: InstructionConflict
// pins: Custom
// pins: ConstantOverflow
// pins: InvalidAddrUsage
// pins: ArrayIndexOutOfBounds
// pins: WriteOnlyRead
// pins: ReadOnlyWrite

#[test]
fn let_mut_names_the_absent_keyword() {
    // The language has no `mut` (locals are mutable by default). A Rust habit
    // writes `let mut x`, which used to parse `mut` as the name and fail at the
    // next token with a baffling "expected `:`". The message now names the cause.
    let err = render("#[reset]\nfn main() { let mut x: u8 = 5; loop {} }");
    assert!(
        err.contains("`mut` is not a keyword"),
        "expected the mut diagnostic, got:\n{err}"
    );
    assert!(
        err.contains("mutable by default"),
        "the diagnostic should explain why, got:\n{err}"
    );
}

#[test]
fn a_variable_named_mut_is_still_allowed() {
    // The guard fires only on `mut <ident>`; a variable literally named `mut`
    // (followed by `:`) is a legal, if odd, name.
    match compile("#[reset]\nfn main() { let mut: u8 = 5; loop {} }") {
        CompileResult::Success(..) => {}
        other => panic!("`let mut: u8` should compile, got {other:?}"),
    }
}
