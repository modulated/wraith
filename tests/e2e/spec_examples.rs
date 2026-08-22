//! Spec examples that must compile.
//!
//! The language specification is full of code, and it drifts: a rename or a
//! semantics change silently invalidates examples nobody recompiles. This
//! harness pins that down for every example that is meant to be real code.
//!
//! Two opt-in tags, both in rustdoc's style (a plain ` ```rust ` block is an
//! illustrative fragment that may reference peripherals or functions defined
//! elsewhere in the prose, and is not tested):
//!
//! - ` ```rust,compile ` — a complete translation unit, compiled as written.
//!   Use it for anything with its own `fn`, `struct`, `enum`, `static`,
//!   `const` or `import` at the top level.
//! - ` ```rust,compile,fragment ` — a run of statements, compiled inside a
//!   generated `#[reset] fn main() { … loop {} }`. Use it for the many
//!   examples that illustrate one expression or declaration and would only be
//!   made noisier by spelling out a wrapper the reader does not care about.
//!
//! Fragments exist so that widening the net does not mean rewriting the prose:
//! most examples in the spec are statement-level, and wrapping each one in a
//! visible `fn main()` would bury the line actually being taught.
//!
//! If a tagged example stops compiling, this test names the line in the spec.

use crate::common::harness::{CompileResult, compile};

/// How a tagged block becomes a translation unit.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Kind {
    /// Compiled as written.
    Program,
    /// Wrapped in a generated `main` before compiling.
    Fragment,
}

/// A tagged spec block: its 1-based line in the spec and its source.
struct Tagged {
    line: usize,
    kind: Kind,
    body: String,
}

impl Tagged {
    /// The source to hand the compiler.
    fn translation_unit(&self) -> String {
        match self.kind {
            Kind::Program => self.body.clone(),
            // `#[reset]` and the terminating `loop {}` are what make it a
            // whole program; the fragment supplies the statements.
            Kind::Fragment => {
                format!("#[reset]\nfn main() {{\n{}\nloop {{}}\n}}\n", self.body)
            }
        }
    }
}

/// Classify a fence line, or `None` if it is not a tagged block.
fn tag_of(line: &str) -> Option<Kind> {
    match line.trim() {
        "```rust,compile" => Some(Kind::Program),
        "```rust,compile,fragment" => Some(Kind::Fragment),
        _ => None,
    }
}

/// Extract every tagged block from the spec, with its line number.
fn compile_blocks(md: &str) -> Vec<Tagged> {
    let mut out = Vec::new();
    let mut lines = md.lines().enumerate();
    while let Some((i, line)) = lines.next() {
        let Some(kind) = tag_of(line) else { continue };
        let mut body = String::new();
        for (_, l) in lines.by_ref() {
            if l.trim_start().starts_with("```") {
                break;
            }
            body.push_str(l);
            body.push('\n');
        }
        out.push(Tagged {
            line: i + 1,
            kind,
            body,
        });
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
        blocks.len() >= 130,
        "expected the spec to keep its ```rust,compile examples, found {}",
        blocks.len()
    );

    let mut failures = Vec::new();
    for b in &blocks {
        match compile(&b.translation_unit()) {
            CompileResult::Success(..) => {}
            other => failures.push(format!(
                "specification.md:{} ({:?}) — {}",
                b.line,
                b.kind,
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

/// Both tags must actually be in use. Without this, a bad find-and-replace that
/// turned every `fragment` tag back into a plain block would still satisfy the
/// count floor above by accident.
#[test]
fn both_block_kinds_are_represented() {
    let md = include_str!("../../docs/specification.md");
    let blocks = compile_blocks(md);
    for kind in [Kind::Program, Kind::Fragment] {
        let n = blocks.iter().filter(|b| b.kind == kind).count();
        assert!(n >= 20, "expected at least 20 {kind:?} blocks, found {n}");
    }
}

/// A fragment must be statements, not a translation unit. Tagging a block that
/// declares its own `fn` as a fragment nests the declaration inside the
/// generated `main`, where it would either fail confusingly or — worse — quietly
/// test something other than what the spec shows.
#[test]
fn fragments_do_not_declare_top_level_items() {
    let md = include_str!("../../docs/specification.md");
    for b in compile_blocks(md)
        .iter()
        .filter(|b| b.kind == Kind::Fragment)
    {
        for (n, line) in b.body.lines().enumerate() {
            let starts_item = ["fn ", "struct ", "enum ", "static ", "import ", "#["]
                .iter()
                .any(|kw| line.starts_with(kw));
            assert!(
                !starts_item,
                "specification.md:{} declares a top-level item on its line {} \
                 (`{}`) but is tagged `fragment`; tag it ```rust,compile instead",
                b.line,
                n + 1,
                line.trim()
            );
        }
    }
}

/// How many fenced Rust blocks carry no compile tag.
fn untagged_count(md: &str) -> usize {
    let mut n = 0;
    let mut lines = md.lines();
    while let Some(line) = lines.next() {
        if !line.trim().starts_with("```rust") {
            continue;
        }
        if tag_of(line).is_none() {
            n += 1;
        }
        for l in lines.by_ref() {
            if l.trim_start().starts_with("```") {
                break;
            }
        }
    }
    n
}

/// The untagged blocks are a shrinking, documented set — not a default.
///
/// A ceiling rather than an exact number, so tagging one more block does not
/// need this line edited; but it cannot grow, so a new example arrives tagged
/// or the author has to say why here.
///
/// What is left, and why each resists tagging:
///
/// * **~29 truncated signatures** — `fn read_line(buf: &u8, max: u8) -> u8 { ... }`
///   and the stdlib reference's bodyless forms. The `...` is the point; giving
///   them bodies would make the reference longer and no clearer.
/// * **~20 deliberate error examples** — a constant that overflows, a duplicate
///   name, `addr` where it is not allowed, a pointer that escapes. They are in
///   the spec *because* they fail; `error_diagnostics.rs` pins the messages.
/// * **9 import examples** naming modules that are illustrations rather than
///   files in this tree.
/// * The rest are prose fragments (bare `..` range syntax) and features the
///   spec marks as not implemented, which cannot compile by definition.
#[test]
fn the_untagged_blocks_stay_a_short_list() {
    let md = include_str!("../../docs/specification.md");
    let n = untagged_count(md);
    assert!(
        n <= 92,
        "{n} spec blocks carry no compile tag, up from 92. Tag the new example, \
         or add its category to the list above this test."
    );
}
