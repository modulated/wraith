//! Feature tests - verify each language feature works correctly
//!
//! Tests are organized by feature:
//! - Variables and types
//! - Operators
//! - Control flow
//! - Functions
//! - Structs and enums
//! - Arrays
//! - Pointers
//! - Inline assembly

mod test_harness;
use test_harness::*;

// ============================================================================
// VARIABLES AND TYPES
// ============================================================================

#[test]
fn test_variable_declaration_u8() {
    let asm = assert_compiles(
        r#"
        addr OUT = 0x400;
        fn main() {
            x: u8 = 42;
            OUT = x;
        }
        "#,
    );
    assert_asm_contains(&asm, "LDA #$2A"); // Load 42
}

#[test]
fn test_variable_declaration_u16() {
    let asm = assert_compiles(
        r#"
        addr OUT = 0x400;
        fn main() {
            x: u16 = 0x1234;
            OUT = x as u8;
        }
        "#,
    );
    assert_asm_contains(&asm, "LDA #$34"); // Low byte of 0x1234
}

#[test]
fn test_hex_literals() {
    let asm = assert_compiles(
        r#"
        addr OUT = 0xC000;
        fn main() {
            a: u8 = 0xFF;
            b: u16 = 0xFA00;
            OUT = a;
        }
        "#,
    );
    assert_asm_contains(&asm, "LDA #$FF");
}

#[test]
fn test_binary_literals() {
    let asm = assert_compiles(
        r#"
        addr OUT = 0x400;
        fn main() {
            flags: u8 = 0b11010010;
            OUT = flags;
        }
        "#,
    );
    assert_asm_contains(&asm, "LDA #$D2"); // 0b11010010 = 0xD2
}

#[test]
fn test_mutable_variable() {
    let asm = assert_compiles(
        r#"
        addr OUT = 0x400;
        fn main() {
            x: u8 = 10;
            x = 20;
            OUT = x;
        }
        "#,
    );
    assert_asm_contains(&asm, "LDA #$0A"); // Initial value
    assert_asm_contains(&asm, "LDA #$14"); // New value 20
}

// ============================================================================
// OPERATORS
// ============================================================================

#[test]
fn test_arithmetic_addition() {
    let asm = assert_compiles(
        r#"
        addr OUT = 0x400;
        fn main() {
            x: u8 = 10 + 20;
            OUT = x;
        }
        "#,
    );
    // Should constant fold to 30 (0x1E)
    assert_asm_contains(&asm, "LDA #$1E");
}

#[test]
fn test_arithmetic_multiplication() {
    let asm = assert_compiles(
        r#"
        addr OUT = 0x400;
        fn main() {
            x: u8 = 5;
            y: u8 = x * 3;
            OUT = y;
        }
        "#,
    );
    // Should have multiplication code
    assert!(asm.contains("LDA") && asm.contains("STA"));
}

#[test]
fn test_bitwise_and() {
    let asm = assert_compiles(
        r#"
        addr OUT = 0x400;
        fn main() {
            x: u8 = 0xFF & 0x0F;
            OUT = x;
        }
        "#,
    );
    // Should constant fold to 0x0F
    assert_asm_contains(&asm, "LDA #$0F");
}

#[test]
fn test_bitwise_or() {
    let asm = assert_compiles(
        r#"
        addr OUT = 0x400;
        fn main() {
            x: u8 = 0xF0 | 0x0F;
            OUT = x;
        }
        "#,
    );
    // Should constant fold to 0xFF
    assert_asm_contains(&asm, "LDA #$FF");
}

#[test]
fn test_shift_left() {
    let asm = assert_compiles(
        r#"
        addr OUT = 0x400;
        fn main() {
            x: u8 = 1 << 4;
            OUT = x;
        }
        "#,
    );
    // Should constant fold to 16 (0x10)
    assert_asm_contains(&asm, "LDA #$10");
}

// ============================================================================
// CONTROL FLOW
// ============================================================================

#[test]
fn test_if_statement() {
    let asm = assert_compiles(
        r#"
        addr OUT = 0x400;
        fn main() {
            x: u8 = 10;
            if x == 10 {
                OUT = 0xFF;
            }
        }
        "#,
    );
    assert_asm_contains(&asm, "CMP");
    assert_asm_contains(&asm, "BEQ");
}

#[test]
fn test_if_else() {
    let asm = assert_compiles(
        r#"
        addr OUT = 0x400;
        fn main() {
            x: u8 = 10;
            if x == 10 {
                OUT = 0xFF;
            } else {
                OUT = 0x00;
            }
        }
        "#,
    );
    assert_asm_contains(&asm, "BEQ");
    assert_asm_contains(&asm, "JMP");
}

#[test]
fn test_while_loop() {
    let asm = assert_compiles(
        r#"
        addr OUT = 0x400;
        fn main() {
            i: u8 = 0;
            while i < 10 {
                i = i + 1;
            }
            OUT = i;
        }
        "#,
    );
    assert_asm_contains(&asm, "while_");
    assert_asm_contains(&asm, "JMP");
}

#[test]
fn test_for_range_loop() {
    let asm = assert_compiles(
        r#"
        addr OUT = 0x400;
        fn main() {
            sum: u8 = 0;
            for i in 0..10 {
                sum = sum + i;
            }
            OUT = sum;
        }
        "#,
    );
    assert_asm_contains(&asm, "loop_");
}

// ============================================================================
// FUNCTIONS
// ============================================================================

#[test]
fn test_function_call_no_args() {
    let asm = assert_compiles(
        r#"
        addr OUT = 0x400;
        fn helper() {
            OUT = 42;
        }
        fn main() {
            helper();
        }
        "#,
    );
    assert_asm_contains(&asm, "JSR helper");
}

#[test]
fn test_function_call_with_args() {
    let asm = assert_compiles(
        r#"
        addr OUT = 0x400;
        fn add(a: u8, b: u8) {
            OUT = a + b;
        }
        fn main() {
            add(10, 20);
        }
        "#,
    );
    assert_asm_contains(&asm, "JSR add");
}

#[test]
fn test_function_return_value() {
    let asm = assert_compiles(
        r#"
        addr OUT = 0x400;
        fn get_value() -> u8 {
            return 42;
        }
        fn main() {
            x: u8 = get_value();
            OUT = x;
        }
        "#,
    );
    assert_asm_contains(&asm, "JSR get_value");
    assert_asm_contains(&asm, "RTS");
}

// ============================================================================
// ARRAYS
// ============================================================================

#[test]
fn test_array_literal() {
    let asm = assert_compiles(
        r#"
        fn main() {
            data: [u8; 3] = [1, 2, 3];
        }
        "#,
    );
    assert_asm_contains(&asm, ".byte $01");
    assert_asm_contains(&asm, ".byte $02");
    assert_asm_contains(&asm, ".byte $03");
}

#[test]
fn test_array_fill() {
    let asm = assert_compiles(
        r#"
        fn main() {
            buffer: [u8; 5] = [0; 5];
        }
        "#,
    );
    // Should have 5 zero bytes
    assert_eq!(count_pattern(&asm, ".byte $00"), 5);
}

// ============================================================================
// STRUCTS AND ENUMS
// ============================================================================

#[test]
fn test_struct_definition() {
    let asm = assert_compiles(
        r#"
        struct Point {
            u8 x,
            u8 y
        }
        fn main() {}
        "#,
    );
    assert!(asm.contains("main:"));
}

#[test]
fn test_struct_initialization() {
    let asm = assert_compiles(
        r#"
        struct Point {
            u8 x,
            u8 y
        }
        addr OUT = 0x400;
        fn main() {
            p: Point = Point { x: 10, y: 20 };
        }
        "#,
    );
    assert!(asm.contains("main:"));
}

#[test]
fn test_enum_definition() {
    let asm = assert_compiles(
        r#"
        enum Status {
            Idle,
            Active,
            Done
        }
        fn main() {}
        "#,
    );
    assert!(asm.contains("main:"));
}

// ============================================================================
// MEMORY OPERATIONS
// ============================================================================

#[test]
fn test_address_declaration() {
    let asm = assert_compiles(
        r#"
        addr SCREEN = 0xC000;
        fn main() {
            SCREEN = 42;
        }
        "#,
    );
    assert_asm_contains(&asm, "STA $C000");
}

#[test]
fn test_constant_address_expression() {
    let asm = assert_compiles(
        r#"
        addr BASE = 0xC000;
        addr SCREEN = BASE + 0x100;
        fn main() {
            SCREEN = 42;
        }
        "#,
    );
    assert_asm_contains(&asm, "STA $C100"); // 0xC000 + 0x100
}
