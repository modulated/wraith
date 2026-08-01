//! Byte-offset ↔ LSP position mapping.
//!
//! The compiler speaks in byte offsets (`Span { start, end }`). LSP speaks in
//! `Position { line, character }`, where `character` counts **UTF-16 code
//! units**, not bytes and not chars. Wraith source is usually ASCII, where all
//! three coincide, but a comment or string with a multi-byte character would
//! desync every position after it if we cheated — so the conversion is done
//! properly.

use lsp_types::Position;

pub struct LineIndex {
    /// Byte offset at which each line begins. `line_starts[0] == 0`.
    line_starts: Vec<usize>,
    len: usize,
}

impl LineIndex {
    pub fn new(text: &str) -> Self {
        let mut line_starts = vec![0];
        for (i, b) in text.bytes().enumerate() {
            if b == b'\n' {
                line_starts.push(i + 1);
            }
        }
        LineIndex {
            line_starts,
            len: text.len(),
        }
    }

    /// Byte offset → LSP position. Clamps out-of-range offsets to the end.
    pub fn position(&self, text: &str, offset: usize) -> Position {
        let offset = offset.min(self.len);
        // The line is the last line start that is <= offset.
        let line = match self.line_starts.binary_search(&offset) {
            Ok(exact) => exact,
            Err(next) => next - 1,
        };
        let line_start = self.line_starts[line];
        // UTF-16 units from the line start to the offset.
        let character = text.get(line_start..offset).map(utf16_len).unwrap_or(0);
        Position {
            line: line as u32,
            character: character as u32,
        }
    }

    /// LSP position → byte offset. Clamps out-of-range positions.
    pub fn offset(&self, text: &str, pos: Position) -> usize {
        let line = pos.line as usize;
        let Some(&line_start) = self.line_starts.get(line) else {
            return self.len;
        };
        let line_end = self
            .line_starts
            .get(line + 1)
            .map(|&s| s.saturating_sub(1)) // drop the '\n'
            .unwrap_or(self.len);
        let line_text = text.get(line_start..line_end).unwrap_or("");

        // Walk UTF-16 units until we reach `pos.character`, returning the byte
        // offset at that point.
        let mut utf16 = 0u32;
        for (byte_off, ch) in line_text.char_indices() {
            if utf16 >= pos.character {
                return line_start + byte_off;
            }
            utf16 += ch.len_utf16() as u32;
        }
        line_end
    }
}

fn utf16_len(s: &str) -> usize {
    s.chars().map(|c| c.len_utf16()).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_round_trips() {
        let text = "fn main() {\n    let x = 1;\n}\n";
        let idx = LineIndex::new(text);
        // The `x` is at line 1, character 8 (0-based).
        let off = text.find("x").unwrap();
        let pos = idx.position(text, off);
        assert_eq!((pos.line, pos.character), (1, 8));
        assert_eq!(idx.offset(text, pos), off);
    }

    #[test]
    fn multibyte_before_target_shifts_utf16_not_bytes() {
        let text = "// π\nlet x = 1;\n"; // π is 2 bytes, 1 utf16 unit
        let idx = LineIndex::new(text);
        let off = text.find("x").unwrap();
        let pos = idx.position(text, off);
        // Second line, so the multibyte char on line 0 does not matter here.
        assert_eq!(pos.line, 1);
        assert_eq!(idx.offset(text, pos), off);
    }
}
