//! Semantic analysis error tests
//!
//! Tests for type errors, undefined symbols, duplicates, and semantic violations

use crate::common::*;

// ============================================================================
// Type Errors
// ============================================================================

#[test]
fn type_mismatch_assignment() {
    assert_error_contains(
        r#"
        fn main() {
            x: u8 = 10;
            x = 300;
        }
        "#,
        "type mismatch",
    );
}

#[test]
fn invalid_operation() {
    assert_error_contains(
        r#"
        fn main() {
            x: u8 = 10;
            y: u16 = 20;
            z: u8 = x + y;
        }
        "#,
        "type",
    );
}

#[test]
fn function_arity_mismatch() {
    assert_error_contains(
        r#"
        fn foo(x: u8) {}
        fn main() {
            foo();
        }
        "#,
        "argument",
    );
}

#[test]
fn return_type_mismatch() {
    assert_error_contains(
        r#"
        fn foo() -> u8 {
            return 300;
        }
        fn main() {}
        "#,
        "type mismatch",
    );
}

// ============================================================================
// Undefined Symbols
// ============================================================================

#[test]
fn undefined_variable() {
    assert_error_contains(
        r#"
        fn main() {
            x: u8 = y;
        }
        "#,
        "undefined",
    );
}

#[test]
fn undefined_function() {
    assert_error_contains(
        r#"
        fn main() {
            foo();
        }
        "#,
        "undefined",
    );
}

// ============================================================================
// Duplicate Symbols
// ============================================================================

#[test]
fn duplicate_function() {
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
fn duplicate_struct() {
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
fn duplicate_enum() {
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
fn duplicate_struct_field() {
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
fn duplicate_enum_variant() {
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
fn duplicate_function_parameter() {
    assert_error_contains(
        r#"
        fn foo(x: u8, y: u8, x: u8) {
        }
        fn main() {}
        "#,
        "duplicate",
    );
}

// ============================================================================
// Control Flow Errors
// ============================================================================

#[test]
fn break_outside_loop() {
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
fn continue_outside_loop() {
    assert_error_contains(
        r#"
        fn main() {
            continue;
        }
        "#,
        "outside",
    );
}

// ============================================================================
// Constant Errors
// ============================================================================

#[test]
fn integer_overflow_in_const() {
    assert_error_contains(
        r#"
        const VALUE: u8 = 256;
        fn main() {}
        "#,
        "overflow",
    );
}

#[test]
fn invalid_address_range() {
    assert_error_contains(
        r#"
        addr INVALID = 0x10000;
        fn main() {}
        "#,
        "address",
    );
}

#[test]
fn const_cannot_be_reassigned() {
    assert_error_contains(
        r#"
        const VALUE: u8 = 10;
        fn main() {
            VALUE = 20;
        }
        "#,
        "cannot assign",
    );
}

#[test]
fn const_cannot_be_modified() {
    assert_error_contains(
        r#"
        const VALUE: u8 = 10;
        fn main() {
            VALUE = VALUE + 1;
        }
        "#,
        "cannot assign",
    );
}

// ============================================================================
// Instruction Conflicts
// ============================================================================

#[test]
fn addr_instruction_conflict() {
    assert_error_contains(
        r#"
        addr ORA = 0x6500;
        fn main() {}
        "#,
        "conflicts with instruction mnemonic",
    );
}

#[test]
fn function_instruction_conflict() {
    assert_error_contains(
        r#"
        fn LDA() {}
        fn main() {}
        "#,
        "conflicts with instruction mnemonic",
    );
}

#[test]
fn const_instruction_conflict() {
    assert_error_contains(
        r#"
        const STA: u8 = 10;
        fn main() {}
        "#,
        "conflicts with instruction mnemonic",
    );
}

#[test]
fn struct_instruction_conflict() {
    assert_error_contains(
        r#"
        struct AND {
            u8 x,
        }
        fn main() {}
        "#,
        "conflicts with instruction mnemonic",
    );
}

#[test]
fn enum_instruction_conflict() {
    assert_error_contains(
        r#"
        enum BIT {
            Zero,
            One,
        }
        fn main() {}
        "#,
        "conflicts with instruction mnemonic",
    );
}

#[test]
fn case_insensitive_instruction_conflict() {
    // Should catch lowercase 'ora' as well
    assert_error_contains(
        r#"
        addr ora = 0x6500;
        fn main() {}
        "#,
        "conflicts with instruction mnemonic",
    );
}

#[test]
fn inline_function_can_use_instruction_name() {
    // Inline functions (intrinsics) are allowed to use instruction names
    // because they're meant to be direct wrappers for CPU instructions
    let _asm = compile_success(
        r#"
        inline fn nop() {
            asm {
                "NOP"
            }
        }
        fn main() {
            nop();
        }
        "#,
    );
}

// ============================================================================
// Positive Tests (should compile)
// ============================================================================

#[test]
fn regular_variable_can_be_reassigned() {
    // Regular variables (without const) should be mutable
    let _asm = compile_success(
        r#"
        fn main() {
            counter: u8 = 0;
            counter = counter + 1;
        }
        "#,
    );
}

#[test]
fn error_has_source_context() {
    // Test that errors include source context
    let result = compile(
        r#"
        fn main() {
            x: u8 = y;
        }
        "#,
    );

    match result {
        CompileResult::SemaError(msg) => {
            assert!(msg.contains("-->"), "Error should contain source location");
            assert!(msg.contains("|"), "Error should contain source line");
        }
        _ => panic!("Expected semantic error"),
    }
}
