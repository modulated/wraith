//! Running the Wraith front-end over a document and shaping the result for LSP.
//!
//! The pipeline is fail-fast — lex, then parse, then analyze, each returning a
//! `Result` — so a broken buffer yields at most one diagnostic (the first
//! error), not the full set an error-recovering parser would. That is the known
//! ceiling of reusing a batch compiler; it is honest and cheap, and it still
//! surfaces the mistake you just made, which is what matters while typing.
//!
//! On success we keep the whole `ProgramInfo` (types and symbols, keyed by
//! span) and the parsed tree. Completion and hover read from *that* — the last
//! analysis that succeeded — because the buffer under the cursor often does not
//! parse (you have just typed `foo.` and there is no expression yet).

use lsp_types::{Diagnostic, DiagnosticSeverity, Range};
use wraith::ast::SourceFile;
use wraith::sema::ProgramInfo;

use crate::line_index::LineIndex;

/// A successful analysis: the parsed tree plus the semantic model.
pub struct Analyzed {
    pub ast: SourceFile,
    pub info: ProgramInfo,
}

/// The outcome of analyzing one document's current text.
pub struct Analysis {
    pub diagnostics: Vec<Diagnostic>,
    /// The analysis, if the whole pipeline succeeded. `None` on any error.
    pub good: Option<Analyzed>,
}

pub fn analyze_text(text: &str, index: &LineIndex) -> Analysis {
    // Lex.
    let tokens = match wraith::lexer::lex(text) {
        Ok(t) => t,
        Err(e) => {
            let range = span_range(text, index, e.span.start, e.span.end);
            return Analysis {
                diagnostics: vec![error(range, e.message.to_string())],
                good: None,
            };
        }
    };

    // Parse.
    let ast = match wraith::parser::Parser::parse(&tokens) {
        Ok(a) => a,
        Err(e) => {
            let range = span_range(text, index, e.span.start, e.span.end);
            let msg = strip_offsets(e.to_string(), e.span.start, e.span.end);
            return Analysis {
                diagnostics: vec![error(range, msg)],
                good: None,
            };
        }
    };

    // Analyze.
    match wraith::sema::analyze(&ast) {
        Ok(info) => {
            // A clean parse+analyze: the only diagnostics are warnings.
            let diagnostics = info
                .warnings
                .iter()
                .map(|w| {
                    let (message, span) = w.parts();
                    let range = span_range(text, index, span.start, span.end);
                    warning(range, message)
                })
                .collect();
            Analysis {
                diagnostics,
                good: Some(Analyzed { ast, info }),
            }
        }
        Err(e) => {
            // Sema reports several independent errors per run, so flatten a
            // `Multiple` into one diagnostic each — squashing them into a single
            // marker would put every message on whichever line came first.
            let flattened: Vec<&wraith::sema::SemaError> = match &e {
                wraith::sema::SemaError::Multiple(errors) => errors.iter().collect(),
                single => vec![single],
            };
            let diagnostics = flattened
                .into_iter()
                .map(|e| {
                    let (start, end) = e.span().map(|s| (s.start, s.end)).unwrap_or((0, 0));
                    let range = span_range(text, index, start, end);
                    error(range, strip_offsets(e.to_string(), start, end))
                })
                .collect();
            Analysis {
                diagnostics,
                good: None,
            }
        }
    }
}

/// `SemaError`/`ParseError` `Display` embeds the byte offsets as ` at N..M`,
/// which is redundant once the diagnostic carries a range. Cut it out.
fn strip_offsets(message: String, start: usize, end: usize) -> String {
    message.replace(&format!(" at {}..{}", start, end), "")
}

fn span_range(text: &str, index: &LineIndex, start: usize, end: usize) -> Range {
    Range {
        start: index.position(text, start),
        end: index.position(text, end),
    }
}

fn error(range: Range, message: String) -> Diagnostic {
    diag(range, message, DiagnosticSeverity::ERROR)
}

fn warning(range: Range, message: String) -> Diagnostic {
    diag(range, message, DiagnosticSeverity::WARNING)
}

fn diag(range: Range, message: String, severity: DiagnosticSeverity) -> Diagnostic {
    Diagnostic {
        range,
        severity: Some(severity),
        source: Some("wraith".to_string()),
        message,
        ..Default::default()
    }
}
