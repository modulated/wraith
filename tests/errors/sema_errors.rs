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
            let x: u8 = 10;
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
            let x: u8 = 10;
            let y: u16 = 20;
            let z: u8 = x + y;
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
            let x: u8 = y;
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
        const INVALID: addr = 0x10000;
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
// Array Bounds Checking
// ============================================================================

#[test]
fn array_index_in_bounds_zero() {
    // Valid: accessing first element
    let _asm = compile_success(
        r#"
        fn main() {
            let arr: [u8; 5] = [1, 2, 3, 4, 5];
            let x: u8 = arr[0];
        }
        "#,
    );
}

#[test]
fn array_index_in_bounds_last() {
    // Valid: accessing last element (size - 1)
    let _asm = compile_success(
        r#"
        fn main() {
            let arr: [u8; 5] = [1, 2, 3, 4, 5];
            let x: u8 = arr[4];
        }
        "#,
    );
}

#[test]
fn array_index_equals_size() {
    // Error: index 5 on array[5] (valid indices are 0-4)
    assert_error_contains(
        r#"
        fn main() {
            let arr: [u8; 5] = [0; 5];
            let x: u8 = arr[5];
        }
        "#,
        "out of bounds",
    );
}

#[test]
fn array_index_greater_than_size() {
    // Error: index 10 on array[5]
    assert_error_contains(
        r#"
        fn main() {
            let arr: [u8; 5] = [0; 5];
            let x: u8 = arr[10];
        }
        "#,
        "out of bounds",
    );
}

#[test]
fn array_index_const_expression() {
    // Error: const IDX = 10, array size 5
    assert_error_contains(
        r#"
        const IDX: u8 = 10;
        fn main() {
            let arr: [u8; 5] = [0; 5];
            let x: u8 = arr[IDX];
        }
        "#,
        "out of bounds",
    );
}

#[test]
fn array_index_arithmetic_valid() {
    // Valid: 2 + 2 = 4, which is < 5
    let _asm = compile_success(
        r#"
        fn main() {
            let arr: [u8; 5] = [0; 5];
            let x: u8 = arr[2 + 2];
        }
        "#,
    );
}

#[test]
fn array_index_arithmetic_invalid() {
    // Error: 3 * 4 = 12, which is >= 5
    assert_error_contains(
        r#"
        fn main() {
            let arr: [u8; 5] = [0; 5];
            let x: u8 = arr[3 * 4];
        }
        "#,
        "out of bounds",
    );
}

#[test]
fn array_index_variable_not_checked() {
    // Valid: variable index is not checked at compile time
    let _asm = compile_success(
        r#"
        fn main() {
            let arr: [u8; 5] = [0; 5];
            let i: u8 = 10;
            let x: u8 = arr[i];
        }
        "#,
    );
}

#[test]
fn array_index_zero_length_array() {
    // Error: cannot index into zero-length array
    assert_error_contains(
        r#"
        fn main() {
            let empty: [u8; 0] = [];
            let x: u8 = empty[0];
        }
        "#,
        "out of bounds",
    );
}

#[test]
fn array_index_multidimensional() {
    // Error: inner array bounds violation
    assert_error_contains(
        r#"
        fn main() {
            let matrix: [[u8; 5]; 10] = [[0; 5]; 10];
            let x: u8 = matrix[2][7];
        }
        "#,
        "out of bounds",
    );
}

#[test]
fn array_index_multidimensional_outer() {
    // Error: outer array bounds violation
    assert_error_contains(
        r#"
        fn main() {
            let matrix: [[u8; 5]; 10] = [[0; 5]; 10];
            let x: u8 = matrix[12][2];
        }
        "#,
        "out of bounds",
    );
}

#[test]
fn array_assignment_index_bounds() {
    // Error: bounds checking should also apply to assignment
    assert_error_contains(
        r#"
        fn main() {
            let arr: [u8; 5] = [0; 5];
            arr[10] = 42;
        }
        "#,
        "out of bounds",
    );
}

// ============================================================================
// Instruction Conflicts
// ============================================================================

#[test]
fn addr_instruction_conflict() {
    assert_error_contains(
        r#"
        const ORA: addr = 0x6500;
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
    // Should catch lowercase 'ora' as well (even though constants should be uppercase)
    assert_error_contains(
        r#"
        const ora: addr = 0x6500;
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
        #[inline]
        fn nop() {
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
            let counter: u8 = 0;
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
            let x: u8 = y;
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
