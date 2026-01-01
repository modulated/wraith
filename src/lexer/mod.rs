//! Lexer for the Wraith programming language
//!
//! Uses logos for efficient tokenization.

use logos::Logos;

/// Tokens for the Wraith language
#[derive(Logos, Debug, Clone, PartialEq)]
#[logos(skip r"[ \t\r\n\f]+")]
pub enum Token {
    // === Keywords ===
    #[token("fn")]
    Fn,
    #[token("struct")]
    Struct,
    #[token("enum")]
    Enum,
    #[token("if")]
    If,
    #[token("else")]
    Else,
    #[token("while")]
    While,
    #[token("loop")]
    Loop,
    #[token("for")]
    For,
    #[token("match")]
    Match,
    #[token("return")]
    Return,
    #[token("break")]
    Break,
    #[token("continue")]
    Continue,
    #[token("const")]
    Const,
    #[token("zp")]
    Zp,
    #[token("as")]
    As,
    #[token("in")]
    In,
    #[token("import")]
    Import,
    #[token("from")]
    From,
    #[token("asm")]
    Asm,
    #[token("addr")]
    Addr,
    #[token("read")]
    Read,
    #[token("write")]
    Write,

    // === Boolean literals ===
    #[token("true")]
    True,
    #[token("false")]
    False,

    // === CPU Status Flags (read-only) ===
    #[token("carry")]
    Carry,
    #[token("zero")]
    Zero,
    #[token("overflow")]
    Overflow,
    #[token("negative")]
    Negative,

    // === Type keywords ===
    #[token("u8")]
    U8,
    #[token("i8")]
    I8,
    #[token("u16")]
    U16,
    #[token("i16")]
    I16,
    #[token("bool")]
    Bool,
    #[token("str")]
    Str,
    #[token("b8")]
    B8,
    #[token("b16")]
    B16,

    // === Arithmetic operators ===
    #[token("+")]
    Plus,
    #[token("-")]
    Minus,
    #[token("*")]
    Star,
    #[token("/")]
    Slash,
    #[token("%")]
    Percent,

    // === Bitwise operators ===
    #[token("&")]
    Amp,
    #[token("|")]
    Pipe,
    #[token("^")]
    Caret,
    #[token("~")]
    Tilde,
    #[token("<<")]
    Shl,
    #[token(">>")]
    Shr,

    // === Comparison operators ===
    #[token("==")]
    EqEq,
    #[token("!=")]
    Ne,
    #[token("<")]
    Lt,
    #[token(">")]
    Gt,
    #[token("<=")]
    Le,
    #[token(">=")]
    Ge,

    // === Logical operators ===
    #[token("&&")]
    AndAnd,
    #[token("||")]
    OrOr,
    #[token("!")]
    Bang,

    // === Assignment operators ===
    #[token("=")]
    Eq,
    #[token("+=")]
    PlusEq,
    #[token("-=")]
    MinusEq,
    #[token("*=")]
    StarEq,
    #[token("/=")]
    SlashEq,
    #[token("%=")]
    PercentEq,
    #[token("&=")]
    AmpEq,
    #[token("|=")]
    PipeEq,
    #[token("^=")]
    CaretEq,
    #[token("<<=")]
    ShlEq,
    #[token(">>=")]
    ShrEq,

    // === Delimiters ===
    #[token("{")]
    LBrace,
    #[token("}")]
    RBrace,
    #[token("(")]
    LParen,
    #[token(")")]
    RParen,
    #[token("[")]
    LBracket,
    #[token("]")]
    RBracket,
    #[token(";")]
    Semi,
    #[token(",")]
    Comma,
    #[token(".")]
    Dot,
    #[token("::")]
    ColonColon,
    #[token(":")]
    Colon,
    #[token("->")]
    Arrow,
    #[token("=>")]
    FatArrow,
    #[token("..=")]
    DotDotEq,
    #[token("..")]
    DotDot,
    #[token("#")]
    Hash,

    // === Literals ===
    #[regex(r"0x[0-9a-fA-F]+", |lex| parse_hex(lex.slice()))]
    #[regex(r"0b[01]+", |lex| parse_binary(lex.slice()))]
    #[regex(r"[0-9]+", |lex| lex.slice().parse::<i64>().ok())]
    Integer(i64),

    #[regex(r#""([^"\\]|\\.)*""#, |lex| {
        let s = lex.slice();
        let content = &s[1..s.len()-1];
        Some(unescape_string(content))
    })]
    String(String),

    // === Identifier ===
    #[regex(r"[a-zA-Z_][a-zA-Z0-9_]*", |lex| lex.slice().to_string())]
    Ident(String),

    // === Comments (skipped) ===
    #[regex(r"//[^\n]*?", logos::skip)]
    #[regex(r"/\*([^*]|\*[^/])*\*/", logos::skip)]
    Comment,
}

fn parse_hex(s: &str) -> Option<i64> {
    i64::from_str_radix(&s[2..], 16).ok()
}

fn parse_binary(s: &str) -> Option<i64> {
    i64::from_str_radix(&s[2..], 2).ok()
}

/// Process escape sequences in a string literal
fn unescape_string(s: &str) -> String {
    let mut result = String::new();
    let mut chars = s.chars();

    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.next() {
                Some('n') => result.push('\n'),
                Some('r') => result.push('\r'),
                Some('t') => result.push('\t'),
                Some('\\') => result.push('\\'),
                Some('"') => result.push('"'),
                Some('0') => result.push('\0'),
                Some(c) => {
                    // Unknown escape sequence - preserve it as-is
                    result.push('\\');
                    result.push(c);
                }
                None => result.push('\\'),
            }
        } else {
            result.push(ch);
        }
    }
    result
}

/// A token with its span in the source
#[derive(Debug, Clone, PartialEq)]
pub struct SpannedToken {
    pub token: Token,
    pub span: std::ops::Range<usize>,
}

/// Lex source code into tokens
pub fn lex(source: &str) -> Result<Vec<SpannedToken>, LexError> {
    let mut lexer = Token::lexer(source);
    let mut tokens = Vec::new();

    while let Some(result) = lexer.next() {
        match result {
            Ok(token) => {
                tokens.push(SpannedToken {
                    token,
                    span: lexer.span(),
                });
            }
            Err(()) => {
                return Err(LexError {
                    span: lexer.span(),
                    message: format!("unexpected character: {:?}", &source[lexer.span()]),
                });
            }
        }
    }

    Ok(tokens)
}

/// An error that occurred during lexing
#[derive(Debug, Clone)]
pub struct LexError {
    pub span: std::ops::Range<usize>,
    pub message: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_keywords() {
        let tokens = lex("fn struct enum if else while loop for match return").unwrap();
        assert_eq!(tokens[0].token, Token::Fn);
        assert_eq!(tokens[1].token, Token::Struct);
        assert_eq!(tokens[2].token, Token::Enum);
        assert_eq!(tokens[3].token, Token::If);
        assert_eq!(tokens[4].token, Token::Else);
    }

    #[test]
    fn test_types() {
        let tokens = lex("u8 i8 u16 i16 bool").unwrap();
        assert_eq!(tokens[0].token, Token::U8);
        assert_eq!(tokens[1].token, Token::I8);
        assert_eq!(tokens[2].token, Token::U16);
        assert_eq!(tokens[3].token, Token::I16);
        assert_eq!(tokens[4].token, Token::Bool);
    }

    #[test]
    fn test_integers() {
        let tokens = lex("42 0xFF 0b1010").unwrap();
        assert_eq!(tokens[0].token, Token::Integer(42));
        assert_eq!(tokens[1].token, Token::Integer(255));
        assert_eq!(tokens[2].token, Token::Integer(10));
    }

    #[test]
    fn test_operators() {
        let tokens = lex("+ - * / == != && ||").unwrap();
        assert_eq!(tokens[0].token, Token::Plus);
        assert_eq!(tokens[1].token, Token::Minus);
        assert_eq!(tokens[2].token, Token::Star);
        assert_eq!(tokens[3].token, Token::Slash);
        assert_eq!(tokens[4].token, Token::EqEq);
        assert_eq!(tokens[5].token, Token::Ne);
        assert_eq!(tokens[6].token, Token::AndAnd);
        assert_eq!(tokens[7].token, Token::OrOr);
    }

    #[test]
    fn test_delimiters() {
        let tokens = lex("{ } ( ) [ ] ; , . :: -> =>").unwrap();
        assert_eq!(tokens[0].token, Token::LBrace);
        assert_eq!(tokens[1].token, Token::RBrace);
        assert_eq!(tokens[2].token, Token::LParen);
        assert_eq!(tokens[3].token, Token::RParen);
        assert_eq!(tokens[4].token, Token::LBracket);
        assert_eq!(tokens[5].token, Token::RBracket);
        assert_eq!(tokens[6].token, Token::Semi);
        assert_eq!(tokens[7].token, Token::Comma);
        assert_eq!(tokens[8].token, Token::Dot);
        assert_eq!(tokens[9].token, Token::ColonColon);
        assert_eq!(tokens[10].token, Token::Arrow);
        assert_eq!(tokens[11].token, Token::FatArrow);
    }

    #[test]
    fn test_identifier() {
        let tokens = lex("foo bar_baz _underscore CamelCase").unwrap();
        assert_eq!(tokens[0].token, Token::Ident("foo".to_string()));
        assert_eq!(tokens[1].token, Token::Ident("bar_baz".to_string()));
        assert_eq!(tokens[2].token, Token::Ident("_underscore".to_string()));
        assert_eq!(tokens[3].token, Token::Ident("CamelCase".to_string()));
    }

    #[test]
    fn test_comments_skipped() {
        let tokens = lex("foo // comment\nbar /* block */ baz").unwrap();
        assert_eq!(tokens.len(), 3);
        assert_eq!(tokens[0].token, Token::Ident("foo".to_string()));
        assert_eq!(tokens[1].token, Token::Ident("bar".to_string()));
        assert_eq!(tokens[2].token, Token::Ident("baz".to_string()));
    }

    #[test]
    fn test_function_signature() {
        let tokens = lex("fn add(a: u8, b: u8) -> u8").unwrap();
        assert_eq!(tokens[0].token, Token::Fn);
        assert_eq!(tokens[1].token, Token::Ident("add".to_string()));
        assert_eq!(tokens[2].token, Token::LParen);
        assert_eq!(tokens[3].token, Token::Ident("a".to_string()));
        assert_eq!(tokens[4].token, Token::Colon);
        assert_eq!(tokens[5].token, Token::U8);
        assert_eq!(tokens[6].token, Token::Comma);
        assert_eq!(tokens[7].token, Token::Ident("b".to_string()));
        assert_eq!(tokens[8].token, Token::Colon);
        assert_eq!(tokens[9].token, Token::U8);
        assert_eq!(tokens[10].token, Token::RParen);
        assert_eq!(tokens[11].token, Token::Arrow);
        assert_eq!(tokens[12].token, Token::U8);
    }
}
