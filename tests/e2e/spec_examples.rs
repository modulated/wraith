//! Spec examples that must compile.
//!
//! The language specification is full of code, and it drifts: a rename or a
//! semantics change silently invalidates examples nobody recompiles. This
//! harness pins that down for the examples that are meant to be complete,
//! self-contained programs — the ones fenced ` ```rust,compile ` (rustdoc's
//! opt-in style; plain ` ```rust ` blocks are illustrative fragments that may
//! reference peripherals or functions defined elsewhere and are not tested).
//!
//! Tag an example ` ```rust,compile ` when it should build on its own. If it
//! stops compiling, this test names the line in the spec.

use crate::common::harness::{CompileResult, compile};

/// A tagged spec block: its 1-based line in the spec and its source.
struct Tagged {
    line: usize,
    body: String,
}

/// Extract every ` ```rust,compile ` block from the spec, with its line number.
fn compile_blocks(md: &str) -> Vec<Tagged> {
    let mut out = Vec::new();
    let mut lines = md.lines().enumerate();
    while let Some((i, line)) = lines.next() {
        if line.trim() == "```rust,compile" {
            let mut body = String::new();
            for (_, l) in lines.by_ref() {
                if l.trim_start().starts_with("```") {
                    break;
                }
                body.push_str(l);
                body.push('\n');
            }
            out.push(Tagged { line: i + 1, body });
        }
    }
    out
}

#[test]
fn spec_compile_examples_build() {
    let md = include_str!("../../docs/specification.md");
    let blocks = compile_blocks(md);

    // A floor guards against the tags silently vanishing (a bulk spec edit that
    // drops them would otherwise make this test pass vacuously).
    assert!(
        blocks.len() >= 30,
        "expected the spec to keep its ```rust,compile examples, found {}",
        blocks.len()
    );

    let mut failures = Vec::new();
    for b in &blocks {
        match compile(&b.body) {
            CompileResult::Success(..) => {}
            other => failures.push(format!(
                "specification.md:{} — {}",
                b.line,
                match other {
                    CompileResult::LexError(e) => format!("lex error: {e}"),
                    CompileResult::ParseError(e) => format!("parse error: {e}"),
                    CompileResult::SemaError(e) => format!("sema error: {e}"),
                    CompileResult::CodegenError(e) => format!("codegen error: {e}"),
                    CompileResult::Success(..) => unreachable!(),
                }
            )),
        }
    }

    assert!(
        failures.is_empty(),
        "spec examples marked ```rust,compile no longer build:\n{}",
        failures.join("\n\n")
    );
}
