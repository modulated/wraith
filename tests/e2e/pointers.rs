//! End-to-end tests for pointer operations
//!
//! Tests pointer features:
//! - Address-of operator (&)
//! - Dereference operator (*)
//! - Pointer arithmetic
//! - Pointers to arrays
//! - Pointers with different types

use crate::common::*;

// ============================================================================
// BASIC POINTER OPERATIONS
// ============================================================================

#[test]
fn pointer_address_of_u8() {
    let asm = compile_success(r#"
        fn main() {
            let x: u8 = 42;
            let ptr: *u8 = &x;
        }
    "#);

    // Should load address of x into A/Y
    assert_asm_contains(&asm, "LDA");
    assert_asm_contains(&asm, "LDY");
}

#[test]
fn pointer_dereference_u8() {
    let asm = compile_success(r#"
        const OUT: addr = 0x400;
        fn main() {
            let x: u8 = 42;
            let ptr: *u8 = &x;
            let value: u8 = *ptr;
            OUT = value;
        }
    "#);

    // Should store address in zero page and use indirect addressing
    assert_asm_contains(&asm, "STA $30");
    assert_asm_contains(&asm, "LDA ($30),Y");
}

#[test]
fn pointer_read_write() {
    let asm = compile_success(r#"
        fn main() {
            let x: u8 = 10;
            let ptr: *u8 = &x;
            let value: u8 = *ptr;
            x = value + 5;
        }
    "#);

    assert_asm_contains(&asm, "LDA");
    assert_asm_contains(&asm, "STA");
}

#[test]
fn pointer_to_u16() {
    let asm = compile_success(r#"
        fn main() {
            let x: u16 = 0x1234;
            let ptr: *u16 = &x;
        }
    "#);

    // Should load address
    assert_asm_contains(&asm, "LDA");
}

// ============================================================================
// POINTER ARITHMETIC
// ============================================================================

#[test]
fn pointer_arithmetic_add_constant() {
    let asm = compile_success(r#"
        fn main() {
            let arr: [u8; 5] = [1, 2, 3, 4, 5];
            let ptr: *u8 = &arr[0];

            // Calculate ptr + 2
            let offset_ptr: u16 = (ptr as u16) + 2;
        }
    "#);

    // Should perform addition
    assert_asm_contains(&asm, "CLC");
    assert_asm_contains(&asm, "ADC");
}

#[test]
fn pointer_arithmetic_iterate_array() {
    let asm = compile_success(r#"
        const OUT: addr = 0x400;
        fn main() {
            let arr: [u8; 3] = [10, 20, 30];
            let base: u16 = &arr[0] as u16;

            // Access arr[1] via pointer arithmetic
            let ptr1: u16 = base + 1;

            // Would need mem_read to actually read
            OUT = arr[1];
        }
    "#);

    assert_asm_contains(&asm, "LDA");
}

#[test]
fn pointer_arithmetic_increment() {
    let asm = compile_success(r#"
        fn main() {
            let x: u8 = 100;
            let ptr: u16 = &x as u16;

            // Increment pointer
            ptr = ptr + 1;
        }
    "#);

    // Should use increment/add
    assert_asm_contains(&asm, "CLC");
    assert_asm_contains(&asm, "ADC");
}

// ============================================================================
// POINTERS WITH ARRAYS
// ============================================================================

#[test]
fn pointer_to_array_element() {
    let asm = compile_success(r#"
        fn main() {
            let arr: [u8; 5] = [1, 2, 3, 4, 5];
            let ptr: *u8 = &arr[2];
        }
    "#);

    // Should calculate address of arr[2]
    assert_asm_contains(&asm, "LDA");
}

#[test]
fn pointer_array_iteration() {
    let asm = compile_success(r#"
        const OUT: addr = 0x400;
        fn main() {
            let arr: [u8; 4] = [11, 22, 33, 44];

            // Get base address
            let base: u16 = &arr[0] as u16;

            // Access elements via pointer arithmetic
            let addr0: u16 = base;
            let addr1: u16 = base + 1;
            let addr2: u16 = base + 2;

            OUT = arr[0];
        }
    "#);

    assert_asm_contains(&asm, "LDA");
}

#[test]
fn pointer_to_multidim_array() {
    let asm = compile_success(r#"
        fn main() {
            let matrix: [[u8; 2]; 2] = [[1, 2], [3, 4]];
            let ptr: *u8 = &matrix[0][0];
        }
    "#);

    assert_asm_contains(&asm, "LDA");
}

// ============================================================================
// COMPLEX POINTER OPERATIONS
// ============================================================================

#[test]
fn pointer_through_function() {
    let asm = compile_success(r#"
        fn get_addr(x: u8) -> u16 {
            return &x as u16;
        }

        fn main() {
            let value: u8 = 42;
            let addr: u16 = get_addr(value);
        }
    "#);

    assert_asm_contains(&asm, "JSR get_addr");
}

#[test]
fn pointer_comparison() {
    let asm = compile_success(r#"
        fn main() {
            let x: u8 = 10;
            let y: u8 = 20;

            let ptr1: u16 = &x as u16;
            let ptr2: u16 = &y as u16;

            let same: bool = ptr1 == ptr2;
        }
    "#);

    // Should perform 16-bit comparison
    assert_asm_contains(&asm, "CMP");
}

#[test]
fn pointer_difference() {
    let asm = compile_success(r#"
        fn main() {
            let arr: [u8; 10] = [0; 10];
            let start: u16 = &arr[0] as u16;
            let end: u16 = &arr[9] as u16;

            // Calculate distance
            let distance: u16 = end - start;
        }
    "#);

    // Should perform subtraction
    assert_asm_contains(&asm, "SBC");
}

// ============================================================================
// POINTER TYPE CHECKING
// ============================================================================

#[test]
fn pointer_type_mismatch() {
    assert_error_contains(r#"
        fn main() {
            let x: u8 = 42;
            let ptr: *u16 = &x;  // Error: type mismatch
        }
    "#, "type");
}

#[test]
fn dereference_non_pointer() {
    assert_error_contains(r#"
        fn main() {
            let x: u8 = 42;
            let value: u8 = *x;  // Error: cannot dereference non-pointer
        }
    "#, "cannot dereference");
}

#[test]
fn address_of_literal() {
    assert_sema_error(r#"
        fn main() {
            let ptr: *u8 = &42;  // Error: cannot take address of literal
        }
    "#);
}
