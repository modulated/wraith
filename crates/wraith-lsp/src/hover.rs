//! Hover: the type of the expression under the cursor.
//!
//! `resolved_types` maps every typed sub-expression's span to its `Type`. The
//! narrowest span that contains the cursor is the most specific thing there, so
//! that is what we report.

use lsp_types::{Hover, HoverContents, MarkupContent, MarkupKind, Range};
use wraith::ast::ROOT_FILE;

use crate::document::Doc;

pub fn hover(doc: &Doc, offset: usize) -> Option<Hover> {
    let good = doc.good.as_ref()?;

    let (span, ty) = good
        .info
        .resolved_types
        .iter()
        // Only spans in the file being edited; imported modules carry spans that
        // index other files and could overlap this offset by coincidence.
        .filter(|(s, _)| s.file == ROOT_FILE && s.start <= offset && offset < s.end)
        .min_by_key(|(s, _)| s.end - s.start)?;

    let contents = HoverContents::Markup(MarkupContent {
        kind: MarkupKind::Markdown,
        value: format!("```wraith\n{}\n```", ty.display_name()),
    });
    let range = Range {
        start: doc.index.position(&doc.text, span.start),
        end: doc.index.position(&doc.text, span.end),
    };
    Some(Hover {
        contents,
        range: Some(range),
    })
}
