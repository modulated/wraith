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
