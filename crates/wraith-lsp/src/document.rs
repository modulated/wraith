//! The set of open documents and their latest analysis.

use lsp_types::{Diagnostic, Uri};
use std::collections::HashMap;

use crate::analysis::{Analyzed, analyze_text};
use crate::line_index::LineIndex;

pub struct Doc {
    pub text: String,
    pub index: LineIndex,
    /// The most recent analysis that fully succeeded. Completion and hover read
    /// from this, so they keep working while the current buffer is mid-edit and
    /// does not parse.
    pub good: Option<Analyzed>,
}

#[derive(Default)]
pub struct Documents {
    docs: HashMap<String, Doc>,
}

impl Documents {
    /// Replace a document's text, re-analyze, and return the diagnostics to
    /// publish. A failed analysis leaves the previous good analysis in place.
    pub fn set(&mut self, uri: &Uri, text: String) -> Vec<Diagnostic> {
        let index = LineIndex::new(&text);
        let analysis = analyze_text(&text, &index);
        let good = analysis
            .good
            .or_else(|| self.docs.remove(&key(uri)).and_then(|d| d.good));
        self.docs.insert(key(uri), Doc { text, index, good });
        analysis.diagnostics
    }

    pub fn remove(&mut self, uri: &Uri) {
        self.docs.remove(&key(uri));
    }

    pub fn get(&self, uri: &Uri) -> Option<&Doc> {
        self.docs.get(&key(uri))
    }
}

fn key(uri: &Uri) -> String {
    uri.as_str().to_string()
}
