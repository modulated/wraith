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
    /// Collected parse errors for multi-error reporting
    errors: Vec<ParseError>,
    /// File these tokens came from, stamped into every span produced. Byte
    /// offsets alone repeat across modules, and spans are used as map keys.
    file: u32,
}

impl<'a> Parser<'a> {
    /// Create a new parser for the file being compiled
    pub fn new(tokens: &'a [SpannedToken]) -> Self {
        Self::new_in_file(tokens, crate::ast::ROOT_FILE)
    }

    /// Create a parser whose spans are stamped with `file`
    pub fn new_in_file(tokens: &'a [SpannedToken], file: u32) -> Self {
        Self {
            tokens,
            pos: 0,
            errors: Vec::with_capacity(tokens.len() / 20),
            file,
        }
    }

    /// Parse a complete source file
    pub fn parse(tokens: &'a [SpannedToken]) -> ParseResult<SourceFile> {
        let mut parser = Parser::new(tokens);
        parser.parse_source_file()
    }

    /// Parse a module whose spans belong to `file` rather than to the file
    /// being compiled.
    pub fn parse_in_file(tokens: &'a [SpannedToken], file: u32) -> ParseResult<SourceFile> {
        let mut parser = Parser::new_in_file(tokens, file);
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
            // `read` and `write` are *contextual* keywords: they only act as
            // access modifiers in `const NAME: read addr = ...`, which is checked
            // before this point. Everywhere else they are ordinary identifiers,
            // so natural names like `struct Device { read: fn(u8) -> u8 }` and
            // `dev.write(v)` are legal.
            Some(Token::Read) => {
                self.advance();
                Ok(Spanned::new("read".to_string(), span))
            }
            Some(Token::Write) => {
                self.advance();
                Ok(Spanned::new("write".to_string(), span))
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
            .map(|t| Span::in_file(t.span.start, t.span.end, self.file))
            .unwrap_or_else(|| {
                // EOF span - use end of last token or 0
                self.tokens
                    .last()
                    .map(|t| Span::in_file(t.span.end, t.span.end, self.file))
                    .unwrap_or_default()
            })
    }

    /// Get the span of the previous token
    fn previous_span(&self) -> Span {
        if self.pos > 0 {
            let t = &self.tokens[self.pos - 1];
            Span::in_file(t.span.start, t.span.end, self.file)
        } else {
            Span::default()
        }
    }

    // === Error Recovery ===

    /// Get current parser position
    fn position(&self) -> usize {
        self.pos
    }

    /// The most errors a single parse collects before it stops trying. One
    /// malformed statement used to cascade into thousands of bogus follow-up
    /// errors; statement-level recovery makes that rare, but a pathological
    /// file still gets a ceiling.
    const MAX_ERRORS: usize = 50;

    /// Record a parse error and continue parsing
    fn record_error(&mut self, error: ParseError) {
        if self.errors.len() < Self::MAX_ERRORS {
            self.errors.push(error);
        }
    }

    /// Whether the error ceiling has been hit and parsing should wind down.
    fn error_limit_reached(&self) -> bool {
        self.errors.len() >= Self::MAX_ERRORS
    }

    /// Synchronize parser state after an error by skipping to a recovery point
    /// Recovery points are: semicolons, closing braces, function keywords
    fn synchronize(&mut self) {
        while let Some(tok) = self.peek() {
            match tok {
                // Statement terminators
                Token::Semi => {
                    self.advance();
                    return;
                }
                // Block/scope boundaries
                Token::RBrace => {
                    return; // Don't consume the closing brace
                }
                // Top-level item starts (including Let to prevent cascading errors)
                Token::Fn
                | Token::Struct
                | Token::Enum
                | Token::Const
                | Token::Addr
                | Token::Let => {
                    return; // Don't consume, let item parser handle it
                }
                _ => {
                    self.advance();
                }
            }
        }
    }

    /// Check if we have collected any errors
    fn has_errors(&self) -> bool {
        !self.errors.is_empty()
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
                x: u8,
                y: u8,
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
                let x: u8 = 42;
                let fast: u8 = 0;
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
                for i in 0..10 {
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
                let x: u8 = 1 + 2 * 3;
            }
        "#;
        let file = parse(source).expect("parse error");
        assert_eq!(file.items.len(), 1);
    }

    #[test]
    fn test_parse_static() {
        let source = r#"
            const SCREEN: addr = 0x0400;
        "#;
        let file = parse(source).expect("parse error");
        assert_eq!(file.items.len(), 1);
    }

    #[test]
    fn statement_errors_recover_within_the_block() {
        // Each bad statement is reported once and parsing resumes at the
        // next one: the count is proportional to the mistakes, not to the
        // number of following tokens (which used to cascade into thousands).
        let source = r#"
            fn main() {
                let x: u8 = ;
                let y: u8 = 2;
                let z: u8 = y + ;
                let w: u8 = 4;
            }
        "#;
        let tokens = lex(source).expect("lexer error");
        let err = Parser::parse(&tokens).expect_err("should fail to parse");
        match err.kind {
            ParseErrorKind::Multiple(errors) => {
                assert_eq!(errors.len(), 2, "one error per bad statement");
            }
            other => panic!("expected multiple errors, got {other:?}"),
        }
    }

    #[test]
    fn a_pathological_file_hits_the_error_cap() {
        // 200 malformed statements must not produce 200+ errors.
        let mut source = String::from("fn main() {\n");
        for i in 0..200 {
            source.push_str(&format!("let v{}: u8 = ;\n", i));
        }
        source.push_str("}\n");
        let tokens = lex(&source).expect("lexer error");
        let err = Parser::parse(&tokens).expect_err("should fail to parse");
        match err.kind {
            ParseErrorKind::Multiple(errors) => {
                assert!(
                    errors.len() <= Parser::MAX_ERRORS,
                    "capped at {} errors, got {}",
                    Parser::MAX_ERRORS,
                    errors.len()
                );
            }
            other => panic!("expected multiple errors, got {other:?}"),
        }
    }
}
