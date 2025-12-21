//! Source span tracking for error messages

/// A span in the source code
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct Span {
    /// Start byte offset
    pub start: usize,
    /// End byte offset (exclusive)
    pub end: usize,
}

/// Line and column position in source code (1-indexed)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineCol {
    pub line: usize,
    pub col: usize,
}

impl Span {
    /// Create a new span
    pub fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }

    /// Create a dummy span for testing
    pub fn dummy() -> Self {
        Self::default()
    }

    /// Merge two spans into one that covers both
    pub fn merge(self, other: Span) -> Span {
        Span {
            start: self.start.min(other.start),
            end: self.end.max(other.end),
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
    pub fn format_error_context(&self, source: &str, filename: Option<&str>, message: &str) -> String {
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
        let source_line = format!("{:>width$} | {}", pos.line, line_text, width = line_num_width);

        // Create the error marker (^^^ underline)
        let marker = if pos.line == end_pos.line {
            // Single line error - draw underline
            let start_col = pos.col;
            let end_col = end_pos.col;
            let underline_len = (end_col - start_col + 1).max(1);

            // Calculate padding (accounting for line number and separator)
            let padding = start_col - 1;
            let marker_line = format!("{} {}{} {}",
                gutter,
                " ".repeat(padding),
                "^".repeat(underline_len),
                message
            );
            marker_line
        } else {
            // Multi-line error - just mark the start
            let padding = pos.col - 1;
            let marker_line = format!("{} {}^ {}",
                gutter,
                " ".repeat(padding),
                message
            );
            marker_line
        };

        format!("{}\n{}\n{}\n{}", header, gutter, source_line, marker)
    }
}

/// Get a specific line from source code (1-indexed)
fn get_line(source: &str, line_num: usize) -> &str {
    source
        .lines()
        .nth(line_num.saturating_sub(1))
        .unwrap_or("")
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
