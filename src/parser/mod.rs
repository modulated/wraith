//! Parser for the Wraith programming language
//!
//! A recursive descent parser that builds an AST from tokens.

mod error;
mod expr;
mod item;
mod stmt;

pub use error::{ParseError, ParseErrorKind, ParseResult};

use crate::ast::{SourceFile, Span, Spanned};
use crate::lexer::{SpannedToken, Token};

/// The Wraith parser
pub struct Parser<'a> {
    tokens: &'a [SpannedToken],
    pos: usize,
}

impl<'a> Parser<'a> {
    /// Create a new parser
    pub fn new(tokens: &'a [SpannedToken]) -> Self {
        Self { tokens, pos: 0 }
    }

    /// Parse a complete source file
    pub fn parse(tokens: &'a [SpannedToken]) -> ParseResult<SourceFile> {
        let mut parser = Parser::new(tokens);
        parser.parse_source_file()
    }

    // === Token navigation ===

    /// Peek at the current token
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos).map(|t| &t.token)
    }

    /// Peek ahead at a future token (n=1 is next token, n=2 is token after that, etc.)
    fn peek_ahead(&self, n: usize) -> Option<&Token> {
        self.tokens.get(self.pos + n).map(|t| &t.token)
    }

    /// Advance to the next token
    fn advance(&mut self) {
        if self.pos < self.tokens.len() {
            self.pos += 1;
        }
    }

    /// Check if the current token matches
    fn check(&self, expected: &Token) -> bool {
        self.peek() == Some(expected)
    }

    /// Expect a specific token or return an error
    fn expect(&mut self, expected: &Token) -> ParseResult<()> {
        if self.check(expected) {
            self.advance();
            Ok(())
        } else {
            Err(ParseError::unexpected_token(
                self.current_span(),
                format!("{:?}", expected),
                self.peek().cloned(),
            ))
        }
    }

    /// Expect an identifier and return it
    fn expect_ident(&mut self) -> ParseResult<Spanned<String>> {
        let span = self.current_span();
        match self.peek().cloned() {
            Some(Token::Ident(name)) => {
                self.advance();
                Ok(Spanned::new(name, span))
            }
            tok => Err(ParseError::unexpected_token(span, "identifier", tok)),
        }
    }

    fn expect_string(&mut self) -> ParseResult<Spanned<String>> {
        let span = self.current_span();
        match self.peek().cloned() {
            Some(Token::String(s)) => {
                self.advance();
                Ok(Spanned::new(s, span))
            }
            tok => Err(ParseError::unexpected_token(span, "string literal", tok)),
        }
    }

    /// Get the span of the current token
    fn current_span(&self) -> Span {
        self.tokens
            .get(self.pos)
            .map(|t| Span::new(t.span.start, t.span.end))
            .unwrap_or_else(|| {
                // EOF span - use end of last token or 0
                self.tokens
                    .last()
                    .map(|t| Span::new(t.span.end, t.span.end))
                    .unwrap_or_default()
            })
    }

    /// Get the span of the previous token
    fn previous_span(&self) -> Span {
        if self.pos > 0 {
            let t = &self.tokens[self.pos - 1];
            Span::new(t.span.start, t.span.end)
        } else {
            Span::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::lex;

    fn parse(source: &str) -> ParseResult<SourceFile> {
        let tokens = lex(source).expect("lexer error");
        Parser::parse(&tokens)
    }

    #[test]
    fn test_parse_simple_function() {
        let source = r#"
            fn add(a: u8, b: u8) -> u8 {
                return a;
            }
        "#;
        let file = parse(source).expect("parse error");
        assert_eq!(file.items.len(), 1);
    }

    #[test]
    fn test_parse_struct() {
        let source = r#"
            struct Point {
                u8 x,
                u8 y,
            }
        "#;
        let file = parse(source).expect("parse error");
        assert_eq!(file.items.len(), 1);
    }

    #[test]
    fn test_parse_enum() {
        let source = r#"
            enum Direction {
                North = 0,
                South = 1,
                East = 2,
                West = 3,
            }
        "#;
        let file = parse(source).expect("parse error");
        assert_eq!(file.items.len(), 1);
    }

    #[test]
    fn test_parse_variable_decl() {
        let source = r#"
            fn main() {
                x: u8 = 42;
                zp fast: u8 = 0;
            }
        "#;
        let file = parse(source).expect("parse error");
        assert_eq!(file.items.len(), 1);
    }

    #[test]
    fn test_parse_if_statement() {
        let source = r#"
            fn test() {
                if x > 10 {
                    return;
                } else {
                    x = 5;
                }
            }
        "#;
        let file = parse(source).expect("parse error");
        assert_eq!(file.items.len(), 1);
    }

    #[test]
    fn test_parse_for_loop() {
        let source = r#"
            fn test() {
                for u8 i in 0..10 {
                    x = i;
                }
            }
        "#;
        let file = parse(source).expect("parse error");
        assert_eq!(file.items.len(), 1);
    }

    #[test]
    fn test_parse_binary_expr() {
        let source = r#"
            fn test() {
                x: u8 = 1 + 2 * 3;
            }
        "#;
        let file = parse(source).expect("parse error");
        assert_eq!(file.items.len(), 1);
    }

    #[test]
    fn test_parse_static() {
        let source = r#"
            addr SCREEN = 0x0400;
        "#;
        let file = parse(source).expect("parse error");
        assert_eq!(file.items.len(), 1);
    }
}
