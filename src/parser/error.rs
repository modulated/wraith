//! Parse error types for the Wraith parser

use crate::ast::Span;
use crate::lexer::Token;

/// A parse error
#[derive(Debug, Clone)]
pub struct ParseError {
    pub span: Span,
    pub kind: ParseErrorKind,
}

/// The kind of parse error
#[derive(Debug, Clone)]
pub enum ParseErrorKind {
    /// Unexpected token
    UnexpectedToken {
        expected: String,
        found: Option<Token>,
    },
    /// Unexpected end of input
    UnexpectedEof { expected: String },
    /// Invalid integer literal
    InvalidInteger(String),
    /// Invalid type
    InvalidType(String),
    /// Custom error message
    Custom(String),
}

impl ParseError {
    pub fn unexpected_token(span: Span, expected: impl Into<String>, found: Option<Token>) -> Self {
        Self {
            span,
            kind: ParseErrorKind::UnexpectedToken {
                expected: expected.into(),
                found,
            },
        }
    }

    pub fn unexpected_eof(span: Span, expected: impl Into<String>) -> Self {
        Self {
            span,
            kind: ParseErrorKind::UnexpectedEof {
                expected: expected.into(),
            },
        }
    }

    pub fn custom(span: Span, message: impl Into<String>) -> Self {
        Self {
            span,
            kind: ParseErrorKind::Custom(message.into()),
        }
    }
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.kind {
            ParseErrorKind::UnexpectedToken { expected, found } => {
                write!(
                    f,
                    "expected {}, found {:?} at {}..{}",
                    expected, found, self.span.start, self.span.end
                )
            }
            ParseErrorKind::UnexpectedEof { expected } => {
                write!(f, "unexpected end of file, expected {}", expected)
            }
            ParseErrorKind::InvalidInteger(s) => {
                write!(f, "invalid integer: {}", s)
            }
            ParseErrorKind::InvalidType(s) => {
                write!(f, "invalid type: {}", s)
            }
            ParseErrorKind::Custom(msg) => {
                write!(f, "{} at {}..{}", msg, self.span.start, self.span.end)
            }
        }
    }
}

impl std::error::Error for ParseError {}

pub type ParseResult<T> = Result<T, ParseError>;
