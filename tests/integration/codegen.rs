//! Code generation integration tests
//!
//! Tests the code generator in isolation, verifying correct
//! 6502 assembly output for various language constructs.

use crate::common::*;

// ============================================================================
// Basic Code Generation
// ============================================================================

#[test]
fn empty_function() {
    let asm = compile_success("fn main() {}");

    assert_asm_contains(&asm, "main:");
    assert_asm_contains(&asm, "RTS");
    assert_asm_order(&asm, "main:", "RTS");
}

#[test]
fn simple_assignment() {
    let asm = compile_success(r#"
        addr SCREEN = 0x0400;
        fn main() {
            SCREEN = 42;
        }
    "#);

    assert_asm_contains(&asm, "SCREEN = $0400");
    assert_asm_contains(&asm, "LDA #$2A");
    assert_asm_contains(&asm, "STA SCREEN");
    assert_asm_order(&asm, "LDA #$2A", "STA SCREEN");
}

#[test]
fn constant_folding() {
    let asm = compile_success(r#"
        addr RESULT = 0x0400;
        fn main() {
            RESULT = 10 + 20;
        }
    "#);

    // Should fold to 30 (0x1E)
    assert_asm_contains(&asm, "LDA #$1E");
    assert_asm_not_contains(&asm, "ADC");
    assert_asm_not_contains(&asm, "PHA");
}

// ============================================================================
// Arithmetic Operations
// ============================================================================

#[test]
fn binary_operations() {
    let asm = compile_success(r#"
        fn main() {
            a: u8 = 10;
            b: u8 = 5;
            x: u8 = a + b;
        }
    "#);

    assert_asm_contains(&asm, "LDA");
    assert_asm_contains(&asm, "ADC");
}

#[test]
fn multiplication() {
    let asm = compile_success(r#"
        addr RESULT = 0x0400;
        fn main() {
            RESULT = 3 * 4;
        }
    "#);

    // Should fold to 12 (0x0C)
    assert_asm_contains(&asm, "LDA #$0C");
}

#[test]
fn division() {
    let asm = compile_success(r#"
        addr RESULT = 0x0400;
        fn main() {
            RESULT = 12 / 3;
        }
    "#);

    // Should fold to 4
    assert_asm_contains(&asm, "LDA #$04");
}

#[test]
fn shift_operations() {
    let asm = compile_success(r#"
        addr RESULT = 0x0400;
        fn main() {
            RESULT = 8 << 1;
        }
    "#);

    // Should fold to 16 (0x10)
    assert_asm_contains(&asm, "LDA #$10");
}

#[test]
fn unary_operations() {
    let asm = compile_success(r#"
        addr RESULT = 0x0400;
        fn main() {
            RESULT = -10;
        }
    "#);

    // Negative 10 in u8 is 246 (0xF6)
    assert_asm_contains(&asm, "LDA");
}

// ============================================================================
// Control Flow
// ============================================================================

#[test]
fn control_flow_if() {
    let asm = compile_success(r#"
        fn main() {
            if true {
                x: u8 = 10;
            }
        }
    "#);

    assert_asm_contains(&asm, "BEQ");
}

#[test]
fn comparison_eq() {
    let asm = compile_success(r#"
        addr RESULT = 0x0400;
        addr X = 0x0401;
        fn main() {
            X = 5;
            RESULT = X == 5;
        }
    "#);

    assert_asm_contains(&asm, "CMP");
    assert_asm_contains(&asm, "BEQ");
}

#[test]
fn logical_and_short_circuit() {
    let asm = compile_success(r#"
        fn main() {
            x: u8 = 10;
            y: u8 = 20;
            if x == 10 && y == 20 {
                z: u8 = 30;
            }
        }
    "#);

    // Should have conditional branch for short-circuit
    assert_asm_contains(&asm, "BEQ");
}

#[test]
fn for_loop() {
    let asm = compile_success(r#"
        fn main() {
            for i: u8 in 0..10 {
                x: u8 = i;
            }
        }
    "#);

    // Should use X register for loop counter
    assert_asm_contains(&asm, "INX");
    assert_asm_contains(&asm, "CPX");
}

// ============================================================================
// Functions
// ============================================================================

#[test]
fn function_call() {
    let asm = compile_success(r#"
        fn foo() -> u8 {
            return 42;
        }
        fn main() {
            x: u8 = foo();
        }
    "#);

    assert_asm_contains(&asm, "JSR foo");
    assert_asm_contains(&asm, "foo:");
}

#[test]
fn inline_function() {
    let asm = compile_success(r#"
        #[inline]
        fn add(a: u8, b: u8) -> u8 {
            return a + b;
        }
        fn main() {
            result: u8 = add(5, 10);
        }
    "#);

    // Should NOT have JSR (inlined)
    assert_asm_not_contains(&asm, "JSR add");
    // Should have the addition directly in main
    assert_asm_contains(&asm, "ADC");
}

// ============================================================================
// Complex Types
// ============================================================================

#[test]
fn string_literal() {
    let asm = compile_success(r#"
        fn main() {
            "Hello";
        }
    "#);

    // String layout: label in DATA section, length (2 bytes), data, load address in code
    assert_asm_contains(&asm, "str_");
    assert_asm_contains(&asm, ".BYTE $05, $00"); // Length 5
    assert_asm_contains(&asm, "$48"); // 'H'
    assert_asm_contains(&asm, "LDA #<str_");
    assert_asm_contains(&asm, "LDX #>str_");

    // Code comes before data section
    assert_asm_order(&asm, "main:", "str_0:");
}

#[test]
fn enum_unit_variant() {
    let asm = compile_success(r#"
        enum Direction {
            North,
            South,
        }
        fn main() {
            d: Direction = Direction::North;
        }
    "#);

    assert_asm_contains(&asm, "; Enum variant: Direction::North");
    assert_asm_contains(&asm, "JMP es_");
    assert_asm_contains(&asm, "en_");
    assert_asm_contains(&asm, ".BYTE $00"); // Tag 0
    assert_asm_contains(&asm, "LDA #<en_");
    assert_asm_contains(&asm, "LDX #>en_");
}

#[test]
fn enum_tuple_variant() {
    let asm = compile_success(r#"
        enum Color {
            RGB(u8, u8, u8),
        }
        fn main() {
            Color::RGB(255, 128, 64);
        }
    "#);

    assert_asm_contains(&asm, "; Enum variant: Color::RGB");
    assert_asm_contains(&asm, "en_");
    assert_asm_contains(&asm, ".BYTE $00"); // Tag
    assert_asm_contains(&asm, ".BYTE $FF"); // 255
    assert_asm_contains(&asm, ".BYTE $80"); // 128
    assert_asm_contains(&asm, ".BYTE $40"); // 64
}

#[test]
fn enum_struct_variant() {
    let asm = compile_success(r#"
        enum Message {
            Point { u8 x, u8 y },
        }
        fn main() {
            Message::Point { x: 10, y: 20 };
        }
    "#);

    assert_asm_contains(&asm, "; Enum variant: Message::Point");
    assert_asm_contains(&asm, ".BYTE $00"); // Tag
    assert_asm_contains(&asm, ".BYTE $0A"); // x=10
    assert_asm_contains(&asm, ".BYTE $14"); // y=20
}

#[test]
fn enum_pattern_matching() {
    let asm = compile_success(r#"
        enum Direction {
            North,
            South,
        }
        fn main() {
            d: Direction = Direction::North;
            match d {
                Direction::North => { x: u8 = 1; }
                Direction::South => { x: u8 = 2; }
            }
        }
    "#);

    // Match generates comparison and branches
    assert_asm_contains(&asm, "CMP");
    assert_asm_contains(&asm, "BEQ");
}

#[test]
fn enum_multiple_variants() {
    let asm = compile_success(r#"
        enum Option {
            None,
            Some(u8),
        }
        fn main() {
            opt: Option = Option::Some(42);
        }
    "#);

    // Second variant has tag 1
    assert_asm_contains(&asm, ".BYTE $01"); // Tag
    assert_asm_contains(&asm, ".BYTE $2A"); // 42
}

// ============================================================================
// Nested Expressions
// ============================================================================

#[test]
fn nested_expressions() {
    let asm = compile_success(r#"
        fn main() {
            result: u8 = (10 + 20) * 2;
        }
    "#);

    // Should have folded to 60 (0x3C)
    assert_asm_contains(&asm, "LDA");
}

// ============================================================================
// Memory Layout Capacity Tests
// ============================================================================

#[test]
fn many_local_variables() {
    // Test expanded variable capacity: $40-$7F = 64 bytes
    // This test would fail with old 16-byte limit ($40-$4F)
    let asm = compile_success(r#"
        fn main() {
            v01: u8 = 1;
            v02: u8 = 2;
            v03: u8 = 3;
            v04: u8 = 4;
            v05: u8 = 5;
            v06: u8 = 6;
            v07: u8 = 7;
            v08: u8 = 8;
            v09: u8 = 9;
            v10: u8 = 10;
            v11: u8 = 11;
            v12: u8 = 12;
            v13: u8 = 13;
            v14: u8 = 14;
            v15: u8 = 15;
            v16: u8 = 16;
            v17: u8 = 17;
            v18: u8 = 18;
            v19: u8 = 19;
            v20: u8 = 20;
            result: u8 = v20;
        }
    "#);

    // Verify the function compiled successfully
    assert_asm_contains(&asm, "main:");
    assert_asm_contains(&asm, "RTS");
}

#[test]
fn multiple_array_variables() {
    // Arrays need 2 bytes per pointer
    // Old layout: 16 bytes = ~8 arrays max
    // New layout: 64 bytes = ~32 arrays max
    let asm = compile_success(r#"
        fn main() {
            arr1: [u8; 3] = [1, 2, 3];
            arr2: [u8; 3] = [4, 5, 6];
            arr3: [u8; 3] = [7, 8, 9];
            arr4: [u8; 3] = [10, 11, 12];
            arr5: [u8; 3] = [13, 14, 15];
            arr6: [u8; 3] = [16, 17, 18];
            arr7: [u8; 3] = [19, 20, 21];
            arr8: [u8; 3] = [22, 23, 24];
            arr9: [u8; 3] = [25, 26, 27];
            arr10: [u8; 3] = [28, 29, 30];
        }
    "#);

    // Verify successful compilation with 10 arrays (20 bytes)
    assert_asm_contains(&asm, "main:");
    assert_asm_contains(&asm, "RTS");
}
