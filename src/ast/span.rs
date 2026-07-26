//! Source span tracking for error messages

/// A span in the source code
///
/// The `file` field is what makes a span globally unique. Spans are used as
/// map keys throughout semantic analysis and code generation — resolved
/// symbols, folded constants, resolved types — and byte offsets alone are only
/// unique within one file. Two modules will happily both have a symbol at
/// offset 300, and when an imported module's tables are merged into the root's
/// one silently overwrites the other.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct Span {
    /// Start byte offset
    pub start: usize,
    /// End byte offset (exclusive)
    pub end: usize,
    /// Identifier of the file these offsets index into. 0 is the file being
    /// compiled; imported modules get a stable id derived from their path (see
    /// `file_id`), so a rebuild produces the same spans.
    pub file: u32,
}

/// The file id for the module being compiled. Its spans index into the source
/// text the driver already has, which is what diagnostics are rendered against.
pub const ROOT_FILE: u32 = 0;

/// A stable id for a source file, derived from its path.
///
/// Derived rather than handed out by a counter because imports are processed
/// depth-first across independent analyzers, which would each need to agree on
/// the next number; a function of the path needs no coordination and is
/// identical between runs. FNV-1a over the path bytes, forced away from
/// `ROOT_FILE` so an imported module can never claim the root's id.
pub fn file_id(path: &str) -> u32 {
    let mut hash: u32 = 0x811c_9dc5;
    for byte in path.as_bytes() {
        hash ^= *byte as u32;
        hash = hash.wrapping_mul(0x0100_0193);
    }
    if hash == ROOT_FILE { 1 } else { hash }
}

/// Line and column position in source code (1-indexed)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineCol {
    pub line: usize,
    pub col: usize,
}

impl Span {
    /// Create a new span in the file being compiled
    pub fn new(start: usize, end: usize) -> Self {
        Self {
            start,
            end,
            file: ROOT_FILE,
        }
    }

    /// Create a span in a specific file
    pub fn in_file(start: usize, end: usize, file: u32) -> Self {
        Self { start, end, file }
    }

    /// Create a dummy span for testing
    pub fn dummy() -> Self {
        Self::default()
    }

    /// Merge two spans into one that covers both.
    ///
    /// Merging across files is meaningless — the offsets index different texts
    /// — so the left span is kept unchanged in that case.
    pub fn merge(self, other: Span) -> Span {
        if self.file != other.file {
            return self;
        }
        Span {
            start: self.start.min(other.start),
            end: self.end.max(other.end),
            file: self.file,
        }
    }

    /// Convert byte offset to line and column (1-indexed)
    pub fn to_line_col(&self, source: &str) -> LineCol {
        offset_to_line_col(source, self.start)
    }

    /// Format span as "line:col"
    pub fn format_position(&self, source: &str) -> String {
        let pos = self.to_line_col(source);
        format!("{}:{}", pos.line, pos.col)
    }

    /// Format span as "line:col..line:col" if multi-line, or "line:col-col" if same line
    pub fn format_range(&self, source: &str) -> String {
        let start = offset_to_line_col(source, self.start);
        let end = offset_to_line_col(source, self.end.saturating_sub(1));

        if start.line == end.line {
            format!("{}:{}-{}", start.line, start.col, end.col)
        } else {
            format!("{}:{}..{}:{}", start.line, start.col, end.line, end.col)
        }
    }

    /// Format error with source context showing the actual line and error marker
    ///
    /// Example output:
    /// ```text
    ///   --> test.wr:10:5
    ///    |
    /// 10 |     LED = "hello";
    ///    |           ^^^^^^^ expected u8, found string
    /// ```
    pub fn format_error_context(
        &self,
        source: &str,
        filename: Option<&str>,
        message: &str,
    ) -> String {
        // `source` is the file being compiled. A span from an imported module
        // indexes a different text, so resolving it here would print a real
        // line of the wrong file with a caret in an arbitrary place — worse
        // than printing no excerpt at all. This is reachable: an imported
        // function taking part in an address conflict is reported by span.
        if self.file != ROOT_FILE {
            return format!("  = note: {} (in an imported module)", message);
        }

        let pos = self.to_line_col(source);
        let end_pos = offset_to_line_col(source, self.end.saturating_sub(1).max(self.start));

        // Get the line containing the error
        let line_text = get_line(source, pos.line);

        // Calculate width for line numbers (for alignment)
        let line_num_width = pos.line.to_string().len().max(3);

        // Create the header (filename:line:col)
        let header = if let Some(fname) = filename {
            format!("  --> {}:{}:{}", fname, pos.line, pos.col)
        } else {
            format!("  --> {}:{}", pos.line, pos.col)
        };

        // Create the gutter separator
        let gutter = format!("{:>width$} |", "", width = line_num_width);

        // Create the line with line number
        let source_line = format!(
            "{:>width$} | {}",
            pos.line,
            line_text,
            width = line_num_width
        );

        // Create the error marker (^^^ underline)
        let marker = if pos.line == end_pos.line {
            // Single line error - draw underline
            let start_col = pos.col;
            let end_col = end_pos.col;
            let underline_len = (end_col - start_col + 1).max(1);

            // Calculate padding (accounting for line number and separator)
            let padding = start_col - 1;
            let marker_line = format!(
                "{} {}{} {}",
                gutter,
                " ".repeat(padding),
                "^".repeat(underline_len),
                message
            );
            marker_line
        } else {
            // Multi-line error - just mark the start
            let padding = pos.col - 1;
            let marker_line = format!("{} {}^ {}", gutter, " ".repeat(padding), message);
            marker_line
        };

        format!("{}\n{}\n{}\n{}", header, gutter, source_line, marker)
    }
}

/// Get a specific line from source code (1-indexed)
fn get_line(source: &str, line_num: usize) -> &str {
    source.lines().nth(line_num.saturating_sub(1)).unwrap_or("")
}

/// Convert byte offset to line and column (1-indexed)
fn offset_to_line_col(source: &str, offset: usize) -> LineCol {
    let mut line = 1;
    let mut col = 1;

    for (i, ch) in source.chars().enumerate() {
        if i >= offset {
            break;
        }

        if ch == '\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }

    LineCol { line, col }
}

/// A node with an associated span
#[derive(Debug, Clone, PartialEq)]
pub struct Spanned<T> {
    pub node: T,
    pub span: Span,
}

impl<T> Spanned<T> {
    pub fn new(node: T, span: Span) -> Self {
        Self { node, span }
    }

    pub fn dummy(node: T) -> Self {
        Self {
            node,
            span: Span::dummy(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn identical_offsets_in_different_files_are_different_spans() {
        // The whole point: spans key the symbol, type and constant tables, and
        // an imported module's entries are merged into the root's. Byte offsets
        // repeat across files, so without the file id one silently replaced the
        // other.
        let root = Span::new(300, 305);
        let imported = Span::in_file(300, 305, file_id("std/math.wr"));
        assert_ne!(root, imported);

        let mut table: HashMap<Span, &str> = HashMap::new();
        table.insert(root, "root symbol");
        table.insert(imported, "imported symbol");
        assert_eq!(table.len(), 2, "both entries must survive the merge");
        assert_eq!(table[&root], "root symbol");
    }

    #[test]
    fn file_ids_are_stable_and_distinct_per_path() {
        // Stable across runs, so a rebuild produces identical output.
        assert_eq!(file_id("std/math.wr"), file_id("std/math.wr"));
        assert_ne!(file_id("std/math.wr"), file_id("std/mem.wr"));
        // Nothing may claim the root's id.
        for path in ["a.wr", "std/math.wr", "", "a/b/c/deep.wr"] {
            assert_ne!(file_id(path), ROOT_FILE, "{path} collided with the root");
        }
    }

    #[test]
    fn merging_across_files_keeps_the_left_span() {
        // Offsets from two files describe different texts; a merged range over
        // them would be meaningless.
        let a = Span::new(10, 20);
        let b = Span::in_file(0, 5, 7);
        assert_eq!(a.merge(b), a);
        // Within one file it still merges normally.
        assert_eq!(a.merge(Span::new(30, 40)), Span::new(10, 40));
    }

    #[test]
    fn an_imported_span_does_not_render_an_excerpt_of_the_wrong_file() {
        let source = "fn main() {\n    let x: u8 = 1;\n}\n";
        let imported = Span::in_file(5, 9, 42);
        let rendered = imported.format_error_context(source, Some("main.wr"), "some problem");
        assert!(!rendered.contains("let x"), "must not quote the wrong file");
        assert!(rendered.contains("imported module"));
    }
}
