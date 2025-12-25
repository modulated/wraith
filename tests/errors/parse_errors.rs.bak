//! Parser error tests
//!
//! Tests for syntax errors and malformed programs

use crate::common::*;

#[test]
fn missing_semicolon() {
    assert_parse_error(
        r#"
        fn main() {
            x: u8 = 10
        }
        "#,
    );
}

#[test]
fn unclosed_brace() {
    assert_parse_error(
        r#"
        fn main() {
        "#,
    );
}

#[test]
fn missing_type() {
    // Missing type annotation results in undefined variable error during sema
    assert_sema_error(
        r#"
        fn main() {
            x = 10;
        }
        "#,
    );
}

#[test]
fn invalid_token() {
    // Invalid tokens are caught by lexer, not parser
    assert_lex_error(
        r#"
        fn main() {
            @@@
        }
        "#,
    );
}

#[test]
fn error_contains_helpful_message() {
    assert_error_contains("fn main() {", "expected");
}
