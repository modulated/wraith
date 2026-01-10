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
    let asm = compile_success(
        r#"
        const OUT: addr = 0x400;
        fn main() {
            let x: u8 = 42;
            OUT = x;
        }
        "#,
    );
    assert_asm_contains(&asm, "LDA #$2A"); // Load 42
}

#[test]
fn test_variable_declaration_u16() {
    let asm = compile_success(
        r#"
        const OUT: addr = 0x400;
        fn main() {
            let x: u16 = 0x1234;
            OUT = x as u8;
        }
        "#,
    );
    assert_asm_contains(&asm, "LDA #$34"); // Low byte of 0x1234
}

#[test]
fn test_hex_literals() {
    let asm = compile_success(
        r#"
        const OUT: addr = 0xC000;
        fn main() {
            let a: u8 = 0xFF;
            let b: u16 = 0xFA00;
            OUT = a;
        }
        "#,
    );
    assert_asm_contains(&asm, "LDA #$FF");
}

#[test]
fn test_binary_literals() {
    let asm = compile_success(
        r#"
        const OUT: addr = 0x400;
        fn main() {
            let flags: u8 = 0b11010010;
            OUT = flags;
        }
        "#,
    );
    assert_asm_contains(&asm, "LDA #$D2"); // 0b11010010 = 0xD2
}

#[test]
fn test_mutable_variable() {
    let asm = compile_success(
        r#"
        const OUT: addr = 0x400;
        fn main() {
            let x: u8 = 10;
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
    let asm = compile_success(
        r#"
        const OUT: addr = 0x400;
        fn main() {
            let x: u8 = 10 + 20;
            OUT = x;
        }
        "#,
    );
    // Should constant fold to 30 (0x1E)
    assert_asm_contains(&asm, "LDA #$1E");
}

#[test]
fn test_arithmetic_multiplication() {
    let asm = compile_success(
        r#"
        const OUT: addr = 0x400;
        fn main() {
            let x: u8 = 5;
            let y: u8 = x * 3;
            OUT = y;
        }
        "#,
    );
    // Should have multiplication code
    assert!(asm.contains("LDA") && asm.contains("STA"));
}

#[test]
fn test_bitwise_and() {
    let asm = compile_success(
        r#"
        const OUT: addr = 0x400;
        fn main() {
            let x: u8 = 0xFF & 0x0F;
            OUT = x;
        }
        "#,
    );
    // Should constant fold to 0x0F
    assert_asm_contains(&asm, "LDA #$0F");
}

#[test]
fn test_bitwise_or() {
    let asm = compile_success(
        r#"
        const OUT: addr = 0x400;
        fn main() {
            let x: u8 = 0xF0 | 0x0F;
            OUT = x;
        }
        "#,
    );
    // Should constant fold to 0xFF
    assert_asm_contains(&asm, "LDA #$FF");
}

#[test]
fn test_shift_left() {
    let asm = compile_success(
        r#"
        const OUT: addr = 0x400;
        fn main() {
            let x: u8 = 1 << 4;
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
    let asm = compile_success(
        r#"
        const OUT: addr = 0x400;
        fn main() {
            let x: u8 = 10;
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
    let asm = compile_success(
        r#"
        const OUT: addr = 0x400;
        fn main() {
            let x: u8 = 10;
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
    let asm = compile_success(
        r#"
        const OUT: addr = 0x400;
        fn main() {
            let i: u8 = 0;
            while i < 10 {
                i = i + 1;
            }
            OUT = i;
        }
        "#,
    );
    assert_asm_contains(&asm, "wh_");
    assert_asm_contains(&asm, "JMP");
}

#[test]
fn test_for_range_loop() {
    let asm = compile_success(
        r#"
        const OUT: addr = 0x400;
        fn main() {
            let sum: u8 = 0;
            for i in 0..10 {
                sum = sum + i;
            }
            OUT = sum;
        }
        "#,
    );
    assert_asm_contains(&asm, "fl_");
}

// ============================================================================
// FUNCTIONS
// ============================================================================

#[test]
fn test_function_call_no_args() {
    let asm = compile_success(
        r#"
        const OUT: addr = 0x400;
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
    let asm = compile_success(
        r#"
        const OUT: addr = 0x400;
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
    let asm = compile_success(
        r#"
        const OUT: addr = 0x400;
        fn get_value() -> u8 {
            return 42;
        }
        fn main() {
            let x: u8 = get_value();
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
    let asm = compile_success(
        r#"
        fn main() {
            let data: [u8; 3] = [1, 2, 3];
        }
        "#,
    );
    assert_asm_contains(&asm, ".BYTE $01");
    assert_asm_contains(&asm, ".BYTE $02");
    assert_asm_contains(&asm, ".BYTE $03");
}

#[test]
fn test_array_fill() {
    let asm = compile_success(
        r#"
        fn main() {
            let buffer: [u8; 5] = [0; 5];
        }
        "#,
    );
    // Should have 5 zero bytes
    assert_eq!(count_pattern(&asm, ".BYTE $00"), 5);
}

// ============================================================================
// STRUCTS AND ENUMS
// ============================================================================

#[test]
fn test_struct_definition() {
    let asm = compile_success(
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
    let asm = compile_success(
        r#"
        struct Point {
            u8 x,
            u8 y
        }
        const OUT: addr = 0x400;
        fn main() {
            let p: Point = Point { x: 10, y: 20 };
        }
        "#,
    );
    assert!(asm.contains("main:"));
}

#[test]
fn test_enum_definition() {
    let asm = compile_success(
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
    let asm = compile_success(
        r#"
        const SCREEN: addr = 0xC000;
        fn main() {
            SCREEN = 42;
        }
        "#,
    );
    assert_asm_contains(&asm, "SCREEN = $C000");  // Address label
    assert_asm_contains(&asm, "STA SCREEN");      // Symbolic name
}

#[test]
fn test_constant_address_expression() {
    let asm = compile_success(
        r#"
        const BASE: addr = 0xC000;
        const SCREEN: addr = BASE + 0x100;
        fn main() {
            SCREEN = 42;
        }
        "#,
    );
    assert_asm_contains(&asm, "SCREEN = $C100");  // 0xC000 + 0x100
    assert_asm_contains(&asm, "STA SCREEN");      // Symbolic name
}

// ============================================================================
// INTERRUPT HANDLERS
// ============================================================================

#[test]
fn test_nmi_handler() {
    let asm = compile_success(
        r#"
        const OUT: addr = 0x400;
        #[nmi]
        fn nmi_handler() {
            OUT = 0xFF;
        }
        fn main() {}
        "#,
    );
    // Should have prologue
    assert_asm_contains(&asm, "PHA");
    assert_asm_contains(&asm, "TXA");
    assert_asm_contains(&asm, "TYA");
    // Should have epilogue
    assert_asm_contains(&asm, "TAY");
    assert_asm_contains(&asm, "TAX");
    assert_asm_contains(&asm, "RTI");
    // Should have vector table
    assert_asm_contains(&asm, ".ORG $FFFA");
    assert_asm_contains(&asm, ".WORD nmi_handler");
}

#[test]
fn test_irq_handler() {
    let asm = compile_success(
        r#"
        const OUT: addr = 0x400;
        #[irq]
        fn irq_handler() {
            OUT = 0x42;
        }
        fn main() {}
        "#,
    );
    // Should use RTI
    assert_asm_contains(&asm, "RTI");
    // Should have IRQ vector
    assert_asm_contains(&asm, ".WORD irq_handler");
}

#[test]
fn test_reset_handler() {
    let asm = compile_success(
        r#"
        #[reset]
        fn start() {
        }
        fn main() {}
        "#,
    );
    // Should have RESET vector
    assert_asm_contains(&asm, ".WORD start");
}

#[test]
fn test_all_interrupt_vectors() {
    let asm = compile_success(
        r#"
        const OUT: addr = 0x400;
        #[nmi]
        fn nmi_handler() {
            OUT = 1;
        }
        #[reset]
        fn reset_handler() {
            OUT = 2;
        }
        #[irq]
        fn irq_handler() {
            OUT = 3;
        }
        fn main() {}
        "#,
    );
    // Verify vector table exists at correct location
    assert_asm_contains(&asm, ".ORG $FFFA");
    // Verify all three vectors are present
    assert_asm_contains(&asm, ".WORD nmi_handler");
    assert_asm_contains(&asm, ".WORD reset_handler");
    assert_asm_contains(&asm, ".WORD irq_handler");
}
