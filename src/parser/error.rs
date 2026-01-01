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

    /// Format error with source code context showing the actual line and error marker
    pub fn format_with_source(&self, source: &str) -> String {
        self.format_with_source_and_file(source, None)
    }

    /// Format error with source code context and filename
    pub fn format_with_source_and_file(&self, source: &str, filename: Option<&str>) -> String {
        let (error_type, message) = match &self.kind {
            ParseErrorKind::UnexpectedToken { expected, found } => {
                let found_str = match found {
                    Some(tok) => format_token(tok),
                    None => "end of file".to_string(),
                };
                ("error", format!("expected {}, found {}", expected, found_str))
            }
            ParseErrorKind::UnexpectedEof { expected } => {
                ("error", format!("unexpected end of file, expected {}", expected))
            }
            ParseErrorKind::InvalidInteger(s) => {
                ("error", format!("invalid integer: {}", s))
            }
            ParseErrorKind::InvalidType(s) => {
                ("error", format!("invalid type: {}", s))
            }
            ParseErrorKind::Custom(msg) => {
                ("error", msg.clone())
            }
        };

        format!("{}: {}\n{}",
            error_type,
            message,
            self.span.format_error_context(source, filename, &message)
        )
    }
}

/// Format a token for display in error messages
fn format_token(token: &Token) -> String {
    match token {
        Token::Ident(name) => format!("identifier '{}'", name),
        Token::Integer(n) => format!("integer {}", n),
        Token::String(s) => format!("string \"{}\"", s),
        Token::Semi => "';'".to_string(),
        Token::Comma => "','".to_string(),
        Token::Colon => "':'".to_string(),
        Token::ColonColon => "'::'".to_string(),
        Token::Dot => "'.'".to_string(),
        Token::LParen => "'('".to_string(),
        Token::RParen => "')'".to_string(),
        Token::LBrace => "'{'".to_string(),
        Token::RBrace => "'}'".to_string(),
        Token::LBracket => "'['".to_string(),
        Token::RBracket => "']'".to_string(),
        Token::Plus => "'+'".to_string(),
        Token::Minus => "'-'".to_string(),
        Token::Star => "'*'".to_string(),
        Token::Slash => "'/'".to_string(),
        Token::Percent => "'%'".to_string(),
        Token::Amp => "'&'".to_string(),
        Token::Pipe => "'|'".to_string(),
        Token::Caret => "'^'".to_string(),
        Token::Tilde => "'~'".to_string(),
        Token::Bang => "'!'".to_string(),
        Token::Lt => "'<'".to_string(),
        Token::Gt => "'>'".to_string(),
        Token::Eq => "'='".to_string(),
        Token::EqEq => "'=='".to_string(),
        Token::Ne => "'!='".to_string(),
        Token::Le => "'<='".to_string(),
        Token::Ge => "'>='".to_string(),
        Token::Shl => "'<<'".to_string(),
        Token::Shr => "'>>'".to_string(),
        Token::AndAnd => "'&&'".to_string(),
        Token::OrOr => "'||'".to_string(),
        Token::Arrow => "'->'".to_string(),
        Token::FatArrow => "'=>'".to_string(),
        Token::DotDot => "'..'".to_string(),
        Token::DotDotEq => "'..='".to_string(),
        Token::PlusEq => "'+='".to_string(),
        Token::MinusEq => "'-='".to_string(),
        Token::StarEq => "'*='".to_string(),
        Token::SlashEq => "'/='".to_string(),
        Token::PercentEq => "'%='".to_string(),
        Token::AmpEq => "'&='".to_string(),
        Token::PipeEq => "'|='".to_string(),
        Token::CaretEq => "'^='".to_string(),
        Token::ShlEq => "'<<='".to_string(),
        Token::ShrEq => "'>>='".to_string(),
        Token::Hash => "'#'".to_string(),
        Token::Fn => "keyword 'fn'".to_string(),
        Token::If => "keyword 'if'".to_string(),
        Token::Else => "keyword 'else'".to_string(),
        Token::While => "keyword 'while'".to_string(),
        Token::Loop => "keyword 'loop'".to_string(),
        Token::For => "keyword 'for'".to_string(),
        Token::In => "keyword 'in'".to_string(),
        Token::Match => "keyword 'match'".to_string(),
        Token::Return => "keyword 'return'".to_string(),
        Token::Break => "keyword 'break'".to_string(),
        Token::Continue => "keyword 'continue'".to_string(),
        Token::Const => "keyword 'const'".to_string(),
        Token::True => "keyword 'true'".to_string(),
        Token::False => "keyword 'false'".to_string(),
        Token::Carry => "keyword 'carry'".to_string(),
        Token::Zero => "keyword 'zero'".to_string(),
        Token::Overflow => "keyword 'overflow'".to_string(),
        Token::Negative => "keyword 'negative'".to_string(),
        Token::As => "keyword 'as'".to_string(),
        Token::Struct => "keyword 'struct'".to_string(),
        Token::Enum => "keyword 'enum'".to_string(),
        Token::Import => "keyword 'import'".to_string(),
        Token::From => "keyword 'from'".to_string(),
        Token::Addr => "keyword 'addr'".to_string(),
        Token::Zp => "keyword 'zp'".to_string(),
        Token::Read => "keyword 'read'".to_string(),
        Token::Write => "keyword 'write'".to_string(),
        Token::Asm => "keyword 'asm'".to_string(),
        Token::U8 => "type 'u8'".to_string(),
        Token::I8 => "type 'i8'".to_string(),
        Token::U16 => "type 'u16'".to_string(),
        Token::I16 => "type 'i16'".to_string(),
        Token::Bool => "type 'bool'".to_string(),
        Token::Str => "type 'str'".to_string(),
        Token::B8 => "type 'b8'".to_string(),
        Token::B16 => "type 'b16'".to_string(),
        Token::Comment => "comment".to_string(),
    }
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.kind {
            ParseErrorKind::UnexpectedToken { expected, found } => {
                let found_str = match found {
                    Some(tok) => format_token(tok),
                    None => "end of file".to_string(),
                };
                write!(
                    f,
                    "expected {}, found {} at {}..{}",
                    expected, found_str, self.span.start, self.span.end
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
