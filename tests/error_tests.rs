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
    // Exact message and position: the error must point at the `}` that
    // followed the missing `;`, not merely fail somewhere in parsing.
    let src = r#"
        fn main() {
            let x: u8 = 10
        }
        "#;
    assert_error_contains(src, "expected Semi");
    assert_error_contains(src, "--> 4:9");
}

#[test]
fn test_parse_error_unclosed_brace() {
    let src = r#"
        fn main() {
            let x: u8 = 10;
        "#;
    assert_error_contains(src, "expected RBrace, found end of file");
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
    // Both types must be named, and the caret must land on the value.
    let src = r#"
        fn main() {
            let x: u8 = 10;
            x = 300;  // 300 doesn't fit in u8, will be inferred as u16
        }
        "#;
    assert_error_contains(src, "expected u8, found u16");
    assert_error_contains(src, "--> 4:17");
}

#[test]
fn test_type_error_invalid_operation() {
    assert_error_contains(
        r#"
        fn main() {
            let x: u8 = 10;
            let y: bool = true;
            let z: u8 = x + y;  // Can't add u8 and bool
        }
        "#,
        "invalid binary operation",
    );
}

#[test]
fn test_negating_a_bool_is_an_error() {
    // `-true` used to type-check as bool because bool is a primitive. Negation
    // is arithmetic, so it is rejected now.
    assert_error_contains(
        r#"
        fn main() {
            let b: bool = true;
            let x: i8 = -b;
        }
        "#,
        "cannot apply '-' to type bool",
    );
}

#[test]
fn test_negating_a_char_is_an_error() {
    assert_error_contains(
        r#"
        fn main() {
            let c: char = 'a';
            let x: i8 = -c;
        }
        "#,
        "cannot apply '-' to type char",
    );
}

#[test]
fn test_type_error_function_arity() {
    let src = r#"
        fn add(a: u8, b: u8) -> u8 {
            return a + b;
        }
        fn main() {
            let x: u8 = add(10);  // Missing second argument
        }
        "#;
    assert_error_contains(src, "expected 2 argument(s), found 1");
    assert_error_contains(src, "--> 6:25");
}

#[test]
fn test_type_error_undefined_variable() {
    // The message must name the symbol, and the caret the use site.
    let src = r#"
        fn main() {
            let x: u8 = undefined_var;
        }
        "#;
    assert_error_contains(src, "undefined symbol 'undefined_var'");
    assert_error_contains(src, "--> 3:25");
}

#[test]
fn test_type_error_undefined_function() {
    let src = r#"
        fn main() {
            undefined_func();
        }
        "#;
    assert_error_contains(src, "undefined symbol 'undefined_func'");
    assert_error_contains(src, "--> 3:13");
}

#[test]
fn test_type_error_return_type_mismatch() {
    assert_error_contains(
        r#"
        fn get_number() -> u8 {
            let x: u16 = 1000;
            return x;  // Returns u16 but function expects u8 (narrowing not allowed)
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
            let x: u8 = 70000;  // Way too large for u16 even
        }
        "#,
        "too large",
    );
}

#[test]
fn test_semantic_error_invalid_address_range() {
    assert_error_contains(
        r#"
        const INVALID: addr = 0x10000;  // > 0xFFFF
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
            let x: u8 = y;  // undefined variable
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
    let _asm = compile_success(
        r#"
        fn main() {
            let counter: u8 = 0;
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
            x: u8,
            y: u8,
        }
        struct Point {
            a: u8,
            b: u8,
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
            x: u8,
            y: u8,
            x: u8,
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
