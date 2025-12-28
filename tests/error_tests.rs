//! Error tests - verify compiler produces correct error messages
//!
//! Categories:
//! - Parse errors
//! - Type errors
//! - Semantic errors
//! - Code generation errors

mod test_harness;
use test_harness::*;

// ============================================================================
// PARSE ERRORS
// ============================================================================

#[test]
fn test_parse_error_missing_semicolon() {
    assert_fails_at(
        r#"
        fn main() {
            x: u8 = 10
        }
        "#,
        "parse",
    );
}

#[test]
fn test_parse_error_unclosed_brace() {
    assert_fails_at(
        r#"
        fn main() {
            x: u8 = 10;
        "#,
        "parse",
    );
}

#[test]
fn test_parse_error_missing_type() {
    // Test for missing type in variable declaration (colon without type)
    assert_fails_at(
        r#"
        fn main() {
            x: = 10;
        }
        "#,
        "parse",
    );
}

#[test]
fn test_parse_error_invalid_token() {
    assert_fails_at(
        r#"
        fn main() {
            @ invalid
        }
        "#,
        "lex",
    );
}

// ============================================================================
// TYPE ERRORS
// ============================================================================

#[test]
fn test_type_error_mismatch_assignment() {
    assert_error_contains(
        r#"
        fn main() {
            x: u8 = 10;
            x = 300;  // 300 doesn't fit in u8, will be inferred as u16
        }
        "#,
        "type mismatch",
    );
}

#[test]
fn test_type_error_invalid_operation() {
    assert_error_contains(
        r#"
        fn main() {
            x: u8 = 10;
            y: bool = true;
            z: u8 = x + y;  // Can't add u8 and bool
        }
        "#,
        "invalid binary operation",
    );
}

#[test]
fn test_type_error_function_arity() {
    assert_error_contains(
        r#"
        fn add(a: u8, b: u8) -> u8 {
            return a + b;
        }
        fn main() {
            x: u8 = add(10);  // Missing second argument
        }
        "#,
        "expected 2",
    );
}

#[test]
fn test_type_error_undefined_variable() {
    assert_error_contains(
        r#"
        fn main() {
            x: u8 = undefined_var;
        }
        "#,
        "undefined",
    );
}

#[test]
fn test_type_error_undefined_function() {
    assert_error_contains(
        r#"
        fn main() {
            undefined_func();
        }
        "#,
        "undefined",
    );
}

#[test]
fn test_type_error_return_type_mismatch() {
    assert_error_contains(
        r#"
        fn get_number() -> u16 {
            return 10;  // Returns u8 but function expects u16
        }
        fn main() {}
        "#,
        "type mismatch",
    );
}

// ============================================================================
// SEMANTIC ERRORS
// ============================================================================

#[test]
fn test_semantic_error_duplicate_function() {
    assert_error_contains(
        r#"
        fn foo() {}
        fn foo() {}
        fn main() {}
        "#,
        "duplicate",
    );
}

#[test]
fn test_semantic_error_break_outside_loop() {
    assert_error_contains(
        r#"
        fn main() {
            break;
        }
        "#,
        "outside",
    );
}

#[test]
fn test_semantic_error_continue_outside_loop() {
    assert_error_contains(
        r#"
        fn main() {
            continue;
        }
        "#,
        "outside",
    );
}

#[test]
fn test_semantic_error_integer_overflow() {
    assert_error_contains(
        r#"
        fn main() {
            x: u8 = 70000;  // Way too large for u16 even
        }
        "#,
        "too large",
    );
}

#[test]
fn test_semantic_error_invalid_address_range() {
    assert_error_contains(
        r#"
        addr INVALID = 0x10000;  // > 0xFFFF
        fn main() {}
        "#,
        "out of range",
    );
}

// ============================================================================
// ERROR MESSAGE QUALITY
// ============================================================================

#[test]
fn test_error_has_source_context() {
    let source = r#"
        fn main() {
            x: u8 = y;  // undefined variable
        }
    "#;

    match compile(source) {
        CompileResult::SemaError(msg) => {
            // Should have line/column information
            assert!(msg.contains("-->"), "Error should have source location");
            assert!(msg.contains("|"), "Error should have source context");
        }
        other => panic!("Expected SemaError but got: {:?}", other),
    }
}

#[test]
fn test_error_contains_helpful_message() {
    let source = r#"
        fn add(a: u8, b: u8) -> u8 {
            return a + b;
        }
        fn main() {
            add(10, 20, 30);  // Too many arguments
        }
    "#;

    match compile(source) {
        CompileResult::SemaError(msg) => {
            // Should mention the arity mismatch
            assert!(
                msg.contains("expected 2") || msg.contains("found 3"),
                "Error should be specific about argument count"
            );
        }
        other => panic!("Expected SemaError but got: {:?}", other),
    }
}
// ============================================================================
// CONST IMMUTABILITY TESTS
// ============================================================================

#[test]
fn test_const_cannot_be_reassigned() {
    assert_error_contains(
        r#"
        const MAX: u8 = 100;
        fn main() {
            MAX = 50;  // Should error: cannot assign to const
        }
        "#,
        "cannot assign",
    );
}

#[test]
fn test_const_cannot_be_modified() {
    assert_error_contains(
        r#"
        const VALUE: u8 = 10;
        fn main() {
            VALUE = VALUE + 1;  // Should error: cannot assign to const
        }
        "#,
        "cannot assign",
    );
}

#[test]
fn test_mut_variable_can_be_reassigned() {
    // Variables declared with 'mut' can be reassigned
    let _asm = assert_compiles(
        r#"
        fn main() {
            counter: u8 = 0;
            counter = counter + 1;  // Works because variable is declared with 'mut'
        }
        "#,
    );
}

#[test]
fn test_semantic_error_duplicate_struct() {
    assert_error_contains(
        r#"
        struct Point {
            u8 x,
            u8 y,
        }
        struct Point {
            u8 a,
            u8 b,
        }
        fn main() {}
        "#,
        "duplicate",
    );
}

#[test]
fn test_semantic_error_duplicate_enum() {
    assert_error_contains(
        r#"
        enum Color {
            Red,
            Blue,
        }
        enum Color {
            Green,
            Yellow,
        }
        fn main() {}
        "#,
        "duplicate",
    );
}

#[test]
fn test_semantic_error_duplicate_struct_field() {
    assert_error_contains(
        r#"
        struct Point {
            u8 x,
            u8 y,
            u8 x,
        }
        fn main() {}
        "#,
        "duplicate",
    );
}

#[test]
fn test_semantic_error_duplicate_enum_variant() {
    assert_error_contains(
        r#"
        enum Direction {
            North,
            South,
            North,
        }
        fn main() {}
        "#,
        "duplicate",
    );
}

#[test]
fn test_semantic_error_duplicate_function_parameter() {
    assert_error_contains(
        r#"
        fn foo(x: u8, y: u8, x: u8) {
        }
        fn main() {}
        "#,
        "duplicate",
    );
}
