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
    let asm = compile_success(
        r#"
    let asm = compile_success(
        r#"
        const SCREEN: addr = 0x0400;
        fn main() {
            SCREEN = 42;
        }
    "#,
    );
    "#,
    );

    assert_asm_contains(&asm, "SCREEN = $0400");
    assert_asm_contains(&asm, "LDA #$2A");
    assert_asm_contains(&asm, "STA SCREEN");
    assert_asm_order(&asm, "LDA #$2A", "STA SCREEN");
}

#[test]
fn constant_folding() {
    let asm = compile_success(
        r#"
    let asm = compile_success(
        r#"
        const RESULT: addr = 0x0400;
        fn main() {
            RESULT = 10 + 20;
        }
    "#,
    );
    "#,
    );

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
    let asm = compile_success(
        r#"
    let asm = compile_success(
        r#"
        fn main() {
            let a: u8 = 10;
            let b: u8 = 5;
            let x: u8 = a + b;
        }
    "#,
    );
    "#,
    );

    assert_asm_contains(&asm, "LDA");
    assert_asm_contains(&asm, "ADC");
}

#[test]
fn multiplication() {
    let asm = compile_success(
        r#"
    let asm = compile_success(
        r#"
        const RESULT: addr = 0x0400;
        fn main() {
            RESULT = 3 * 4;
        }
    "#,
    );
    "#,
    );

    // Should fold to 12 (0x0C)
    assert_asm_contains(&asm, "LDA #$0C");
}

#[test]
fn division() {
    let asm = compile_success(
        r#"
    let asm = compile_success(
        r#"
        const RESULT: addr = 0x0400;
        fn main() {
            RESULT = 12 / 3;
        }
    "#,
    );
    "#,
    );

    // Should fold to 4
    assert_asm_contains(&asm, "LDA #$04");
}

#[test]
fn shift_operations() {
    let asm = compile_success(
        r#"
    let asm = compile_success(
        r#"
        const RESULT: addr = 0x0400;
        fn main() {
            RESULT = 8 << 1;
        }
    "#,
    );
    "#,
    );

    // Should fold to 16 (0x10)
    assert_asm_contains(&asm, "LDA #$10");
}

#[test]
fn unary_operations() {
    let asm = compile_success(
        r#"
    let asm = compile_success(
        r#"
        const RESULT: addr = 0x0400;
        fn main() {
            RESULT = -10;
        }
    "#,
    );
    "#,
    );

    // Negative 10 in u8 is 246 (0xF6)
    assert_asm_contains(&asm, "LDA");
}

// ============================================================================
// Control Flow
// ============================================================================

#[test]
fn control_flow_if() {
    let asm = compile_success(
        r#"
    let asm = compile_success(
        r#"
        fn main() {
            if true {
                let x: u8 = 10;
            }
        }
    "#,
    );
    "#,
    );

    assert_asm_contains(&asm, "BEQ");
}

#[test]
fn comparison_eq() {
    let asm = compile_success(
        r#"
    let asm = compile_success(
        r#"
        const RESULT: addr = 0x0400;
        const X: addr = 0x0401;
        fn main() {
            X = 5;
            RESULT = X == 5;
        }
    "#,
    );
    "#,
    );

    assert_asm_contains(&asm, "CMP");
    assert_asm_contains(&asm, "BEQ");
}

#[test]
fn logical_and_short_circuit() {
    let asm = compile_success(
        r#"
    let asm = compile_success(
        r#"
        fn main() {
            let x: u8 = 10;
            let y: u8 = 20;
            if x == 10 && y == 20 {
                let z: u8 = 30;
            }
        }
    "#,
    );
    "#,
    );

    // Should have conditional branch for short-circuit
    assert_asm_contains(&asm, "BEQ");
}

#[test]
fn for_loop() {
    let asm = compile_success(
        r#"
    let asm = compile_success(
        r#"
        fn main() {
            for i: u8 in 0..10 {
                let x: u8 = i;
            }
        }
    "#,
    );
    "#,
    );

    // Should use X register for loop counter
    assert_asm_contains(&asm, "INX");
    assert_asm_contains(&asm, "CPX");
}

// ============================================================================
// Functions
// ============================================================================

#[test]
fn function_call() {
    let asm = compile_success(
        r#"
    let asm = compile_success(
        r#"
        fn foo() -> u8 {
            return 42;
        }
        fn main() {
            let x: u8 = foo();
        }
    "#,
    );
    "#,
    );

    assert_asm_contains(&asm, "JSR foo");
    assert_asm_contains(&asm, "foo:");
}

#[test]
fn inline_function() {
    let asm = compile_success(
        r#"
    let asm = compile_success(
        r#"
        #[inline]
        fn add(a: u8, b: u8) -> u8 {
            return a + b;
        }
        fn main() {
            let result: u8 = add(5, 10);
        }
    "#,
    );
    "#,
    );

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
    let asm = compile_success(
        r#"
    let asm = compile_success(
        r#"
        fn main() {
            "Hello";
        }
    "#,
    );
    "#,
    );

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
    let asm = compile_success(
        r#"
    let asm = compile_success(
        r#"
        enum Direction {
            North,
            South,
        }
        fn main() {
            let d: Direction = Direction::North;
        }
    "#,
    );
    "#,
    );

    assert_asm_contains(&asm, "; Enum variant: Direction::North");
    assert_asm_contains(&asm, "JMP es_");
    assert_asm_contains(&asm, "en_");
    assert_asm_contains(&asm, ".BYTE $00"); // Tag 0
    assert_asm_contains(&asm, "LDA #<en_");
    assert_asm_contains(&asm, "LDX #>en_");
}

#[test]
fn enum_tuple_variant() {
    let asm = compile_success(
        r#"
    let asm = compile_success(
        r#"
        enum Color {
            RGB(u8, u8, u8),
        }
        fn main() {
            Color::RGB(255, 128, 64);
        }
    "#,
    );
    "#,
    );

    assert_asm_contains(&asm, "; Enum variant: Color::RGB");
    assert_asm_contains(&asm, "en_");
    assert_asm_contains(&asm, ".BYTE $00"); // Tag
    assert_asm_contains(&asm, ".BYTE $FF"); // 255
    assert_asm_contains(&asm, ".BYTE $80"); // 128
    assert_asm_contains(&asm, ".BYTE $40"); // 64
}

#[test]
fn enum_struct_variant() {
    let asm = compile_success(
        r#"
    let asm = compile_success(
        r#"
        enum Message {
            Point { x: u8, y: u8 },
        }
        fn main() {
            Message::Point { x: 10, y: 20 };
        }
    "#,
    );
    "#,
    );

    assert_asm_contains(&asm, "; Enum variant: Message::Point");
    assert_asm_contains(&asm, ".BYTE $00"); // Tag
    assert_asm_contains(&asm, ".BYTE $0A"); // x=10
    assert_asm_contains(&asm, ".BYTE $14"); // y=20
}

#[test]
fn enum_pattern_matching() {
    let asm = compile_success(
        r#"
    let asm = compile_success(
        r#"
        enum Direction {
            North,
            South,
        }
        fn main() {
            let d: Direction = Direction::North;
            match d {
                Direction::North => { let x: u8 = 1; }
                Direction::South => { let x: u8 = 2; }
            }
        }
    "#,
    );
    "#,
    );

    // Match generates comparison and branches
    assert_asm_contains(&asm, "CMP");
    assert_asm_contains(&asm, "BEQ");
}

#[test]
fn enum_multiple_variants() {
    let asm = compile_success(
        r#"
    let asm = compile_success(
        r#"
        enum Option {
            None,
            Some(u8),
        }
        fn main() {
            let opt: Option = Option::Some(42);
        }
    "#,
    );
    "#,
    );

    // Second variant has tag 1
    assert_asm_contains(&asm, ".BYTE $01"); // Tag
    assert_asm_contains(&asm, ".BYTE $2A"); // 42
}

// ============================================================================
// Nested Expressions
// ============================================================================

#[test]
fn nested_expressions() {
    let asm = compile_success(
        r#"
    let asm = compile_success(
        r#"
        fn main() {
            let result: u8 = (10 + 20) * 2;
        }
    "#,
    );
    "#,
    );

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
    let asm = compile_success(
        r#"
    let asm = compile_success(
        r#"
        fn main() {
            let v01: u8 = 1;
            let v02: u8 = 2;
            let v03: u8 = 3;
            let v04: u8 = 4;
            let v05: u8 = 5;
            let v06: u8 = 6;
            let v07: u8 = 7;
            let v08: u8 = 8;
            let v09: u8 = 9;
            let v10: u8 = 10;
            let v11: u8 = 11;
            let v12: u8 = 12;
            let v13: u8 = 13;
            let v14: u8 = 14;
            let v15: u8 = 15;
            let v16: u8 = 16;
            let v17: u8 = 17;
            let v18: u8 = 18;
            let v19: u8 = 19;
            let v20: u8 = 20;
            let result: u8 = v20;
        }
    "#,
    );
    "#,
    );

    // Verify the function compiled successfully
    assert_asm_contains(&asm, "main:");
    assert_asm_contains(&asm, "RTS");
}

#[test]
fn multiple_array_variables() {
    // Arrays need 2 bytes per pointer
    // Old layout: 16 bytes = ~8 arrays max
    // New layout: 64 bytes = ~32 arrays max
    let asm = compile_success(
        r#"
    let asm = compile_success(
        r#"
        fn main() {
            let arr1: [u8; 3] = [1, 2, 3];
            let arr2: [u8; 3] = [4, 5, 6];
            let arr3: [u8; 3] = [7, 8, 9];
            let arr4: [u8; 3] = [10, 11, 12];
            let arr5: [u8; 3] = [13, 14, 15];
            let arr6: [u8; 3] = [16, 17, 18];
            let arr7: [u8; 3] = [19, 20, 21];
            let arr8: [u8; 3] = [22, 23, 24];
            let arr9: [u8; 3] = [25, 26, 27];
            let arr10: [u8; 3] = [28, 29, 30];
        }
    "#,
    );
    "#,
    );

    // Verify successful compilation with 10 arrays (20 bytes)
    assert_asm_contains(&asm, "main:");
    assert_asm_contains(&asm, "RTS");
}

#[test]
fn loop_unrolling_small_constant_loop() {
    let asm = compile_success(
        r#"
    let asm = compile_success(
        r#"
        const DATA: addr = 0x6000;
        fn main() {
            for i: u8 in 0..3 {
                DATA = i;
            }
        }
    "#,
    );
    "#,
    );

    // Verify loop was unrolled (should have "Loop unrolled" comment)
    assert_asm_contains(&asm, "Loop unrolled");

    // Should NOT have loop labels (fl, fx) or branch instructions for this simple loop
    assert!(
        !asm.contains("fl_"),
        "Expected loop to be unrolled, not use loop labels"
    );
    assert!(
        !asm.contains("BCS"),
        "Expected loop to be unrolled, not use conditional branches"
    );
    assert!(
        !asm.contains("fl_"),
        "Expected loop to be unrolled, not use loop labels"
    );
    assert!(
        !asm.contains("BCS"),
        "Expected loop to be unrolled, not use conditional branches"
    );

    // Should have inline assignments for i = 0, i = 1, i = 2
    assert_asm_contains(&asm, "i = 0");
    assert_asm_contains(&asm, "i = 1");
    assert_asm_contains(&asm, "i = 2");
}

#[test]
fn loop_unrolling_single_iteration() {
    let asm = compile_success(
        r#"
    let asm = compile_success(
        r#"
        const OUT: addr = 0x6000;
        fn main() {
            for i: u8 in 5..6 {
                OUT = i;
            }
        }
    "#,
    );
    "#,
    );

    // Single iteration loop should be unrolled
    assert_asm_contains(&asm, "Loop unrolled: 1 iteration");
    assert_asm_contains(&asm, "i = 5");
}

#[test]
fn loop_unrolling_inclusive_range() {
    let asm = compile_success(
        r#"
    let asm = compile_success(
        r#"
        const OUT: addr = 0x6000;
        fn main() {
            for i: u8 in 0..=2 {
                OUT = i;
            }
        }
    "#,
    );
    "#,
    );

    // Inclusive range 0..=2 should unroll to 3 iterations
    assert_asm_contains(&asm, "Loop unrolled: 3 iterations");
}

#[test]
fn no_loop_unrolling_large_count() {
    let asm = compile_success(
        r#"
    let asm = compile_success(
        r#"
        const OUT: addr = 0x6000;
        fn main() {
            for i: u8 in 0..20 {
                OUT = i;
            }
        }
    "#,
    );
    "#,
    );

    // Large loop (20 iterations) should NOT be unrolled
    assert!(
        !asm.contains("Loop unrolled"),
        "Large loops should not be unrolled"
    );
    assert!(
        !asm.contains("Loop unrolled"),
        "Large loops should not be unrolled"
    );

    // Should use normal loop with labels and branches
    assert_asm_contains(&asm, "fl_");
}

#[test]
fn no_loop_unrolling_non_constant() {
    let asm = compile_success(
        r#"
    let asm = compile_success(
        r#"
        const OUT: addr = 0x6000;
        fn main() {
            let n: u8 = 5;
            for i: u8 in 0..n {
                OUT = i;
            }
        }
    "#,
    );
    "#,
    );

    // Variable range end should NOT be unrolled
    assert!(
        !asm.contains("Loop unrolled"),
        "Non-constant ranges should not be unrolled"
    );
    assert!(
        !asm.contains("Loop unrolled"),
        "Non-constant ranges should not be unrolled"
    );
}

// ============================================================
// Dead Code Elimination Tests
// ============================================================

#[test]
fn dead_code_after_return() {
    let asm = compile_success(
        r#"
    let asm = compile_success(
        r#"
        fn main() {
            let x: u8 = 1;
            return;
            let y: u8 = 2;  // Unreachable
        }
    "#,
    );
    "#,
    );

    // Should contain comment about eliminated code
    assert_asm_contains(&asm, "Unreachable code eliminated");
    // Should NOT generate the dead assignment
    assert!(
        !asm.contains("LDA #$02"),
        "Dead code should not generate assembly"
    );
    assert!(
        !asm.contains("LDA #$02"),
        "Dead code should not generate assembly"
    );
}

#[test]
fn dead_code_after_break() {
    let asm = compile_success(
        r#"
    let asm = compile_success(
        r#"
        fn main() {
            for i: u8 in 0..20 {
                if i == 5 {
                    break;
                    let x: u8 = 10;  // Unreachable
                }
            }
        }
    "#,
    );
    "#,
    );

    // Should eliminate unreachable code after break
    assert_asm_contains(&asm, "Unreachable code eliminated");
}

#[test]
fn dead_code_after_continue() {
    let asm = compile_success(
        r#"
    let asm = compile_success(
        r#"
        fn main() {
            for i: u8 in 0..20 {
                if i == 5 {
                    continue;
                    let x: u8 = 10;  // Unreachable
                }
            }
        }
    "#,
    );
    "#,
    );

    // Should eliminate unreachable code after continue
    assert_asm_contains(&asm, "Unreachable code eliminated");
}

#[test]
fn multiple_dead_statements() {
    let asm = compile_success(
        r#"
    let asm = compile_success(
        r#"
        fn main() {
            return;
            let x: u8 = 1;   // Unreachable
            let y: u8 = 2;   // Unreachable
            let z: u8 = 3;   // Unreachable
        }
    "#,
    );
    "#,
    );

    // Should have multiple elimination comments
    let count = asm.matches("Unreachable code eliminated").count();
    assert!(
        count >= 3,
        "Expected at least 3 unreachable code eliminations, got {}",
        count
    );
    assert!(
        count >= 3,
        "Expected at least 3 unreachable code eliminations, got {}",
        count
    );
}

// ============================================================
// Constant Array Optimization Tests
// ============================================================

#[test]
fn zero_array_small_no_optimization() {
    let asm = compile_success(
        r#"
    let asm = compile_success(
        r#"
        fn main() {
            let buf: [u8; 8] = [0; 8];
        }
    "#,
    );
    "#,
    );

    // Small arrays (< 16 bytes) should NOT be optimized
    assert!(
        !asm.contains(".RES"),
        "Small zero arrays should not use .RES"
    );
    assert!(
        !asm.contains("Zero-filled array optimized"),
        "Small arrays should not be optimized"
    );
    assert!(
        !asm.contains(".RES"),
        "Small zero arrays should not use .RES"
    );
    assert!(
        !asm.contains("Zero-filled array optimized"),
        "Small arrays should not be optimized"
    );
}

#[test]
fn zero_array_large_optimized() {
    let asm = compile_success(
        r#"
    let asm = compile_success(
        r#"
        fn main() {
            let buf: [u8; 32] = [0; 32];
        }
    "#,
    );
    "#,
    );

    // Large zero arrays emit .BYTE $00 for each element (portable format)
    assert_asm_contains(&asm, ".BYTE $00");
    assert_asm_contains(&asm, "Array data: 32 bytes");
}

#[test]
fn zero_array_threshold_optimized() {
    let asm = compile_success(
        r#"
    let asm = compile_success(
        r#"
        fn main() {
            let buf: [u8; 16] = [0; 16];
        }
    "#,
    );
    "#,
    );

    // Zero arrays emit .BYTE $00 for each element (portable format)
    assert_asm_contains(&asm, ".BYTE $00");
    assert_asm_contains(&asm, "Array data: 16 bytes");
}

#[test]
fn non_zero_array_not_optimized() {
    let asm = compile_success(
        r#"
    let asm = compile_success(
        r#"
        fn main() {
            let buf: [u8; 32] = [5; 32];
        }
    "#,
    );
    "#,
    );

    // Non-zero arrays should NOT be optimized (no .RES)
    assert!(!asm.contains(".RES"), "Non-zero arrays should not use .RES");
    // Should emit individual bytes
    assert_asm_contains(&asm, ".BYTE");
}

#[test]
fn very_large_zero_array() {
    let asm = compile_success(
        r#"
    let asm = compile_success(
        r#"
        fn main() {
            let buf: [u8; 256] = [0; 256];
        }
    "#,
    );
    "#,
    );

    // Very large zero arrays emit .BYTE $00 for each element (portable format)
    assert_asm_contains(&asm, ".BYTE $00");
    assert_asm_contains(&asm, "Array data: 256 bytes");
    // Should have 256 .BYTE $00 directives
    let byte_count = asm.matches(".BYTE $00").count();
    assert_eq!(
        byte_count, 256,
        "Zero array should have 256 .BYTE $00 directives, found {}",
        byte_count
    );
}

// ============================================================================
// Const Arrays (ROM Data)
// ============================================================================

#[test]
fn const_array_lookup_table() {
    let asm = compile_success(
        r#"
    let asm = compile_success(
        r#"
        const LUT: [u8; 16] = [0, 1, 4, 9, 16, 25, 36, 49, 64, 81, 100, 121, 144, 169, 196, 225];

        fn main() {
            let x: u8 = LUT[5];
        }
    "#,
    );
    "#,
    );

    // Should emit data section header
    assert_asm_contains(&asm, "Data Section (Const Arrays)");

    // Should emit .ORG for DATA section (default $C000)
    assert_asm_contains(&asm, ".ORG $C000");

    // Should emit const array label and data
    assert_asm_contains(&asm, "LUT:");
    assert_asm_contains(
        &asm,
        ".BYTE $00, $01, $04, $09, $10, $19, $24, $31, $40, $51, $64, $79, $90, $A9, $C4, $E1",
    );
    assert_asm_contains(
        &asm,
        ".BYTE $00, $01, $04, $09, $10, $19, $24, $31, $40, $51, $64, $79, $90, $A9, $C4, $E1",
    );

    // Data should come before code
    assert_asm_order(&asm, "Data Section (Const Arrays)", "Code from main module");
}

#[test]
fn const_array_zero_filled() {
    let asm = compile_success(
        r#"
    let asm = compile_success(
        r#"
        const BUFFER: [u8; 256] = [0; 256];

        fn main() {
            let x: u8 = BUFFER[0];
        }
    "#,
    );
    "#,
    );

    // Zero-filled const arrays emit .BYTE $00 for each element (portable format)
    assert_asm_contains(&asm, "BUFFER:");
    assert_asm_contains(&asm, ".BYTE $00");

    // Should be in DATA section
    assert_asm_contains(&asm, "Data Section (Const Arrays)");
}

#[test]
fn const_array_sprite_data() {
    let asm = compile_success(
        r#"
    let asm = compile_success(
        r#"
        const SPRITE: [u8; 8] = [0xFF, 0x00, 0xFF, 0x00, 0xFF, 0x00, 0xFF, 0x00];

        fn main() {
            let x: u8 = SPRITE[0];
        }
    "#,
    );
    "#,
    );

    // Should emit sprite data as bytes
    assert_asm_contains(&asm, "SPRITE:");
    assert_asm_contains(&asm, ".BYTE $FF, $00, $FF, $00, $FF, $00, $FF, $00");
}

#[test]
fn const_array_small_zero_fill() {
    let asm = compile_success(
        r#"
    let asm = compile_success(
        r#"
        const SMALL: [u8; 8] = [0; 8];

        fn main() {
            let x: u8 = SMALL[0];
        }
    "#,
    );
    "#,
    );

    // Small zero-filled arrays (< 16 bytes) should use .BYTE
    assert_asm_contains(&asm, "SMALL:");
    assert_asm_contains(&asm, ".BYTE");

    // Should NOT use .RES for small arrays
    assert!(
        !asm.contains(".RES 8"),
        "Small arrays should not use .RES optimization"
    );
    assert!(
        !asm.contains(".RES 8"),
        "Small arrays should not use .RES optimization"
    );
}

#[test]
fn multiple_const_arrays() {
    let asm = compile_success(
        r#"
    let asm = compile_success(
        r#"
        const FIRST: [u8; 4] = [1, 2, 3, 4];
        const SECOND: [u8; 4] = [5, 6, 7, 8];

        fn main() {
            let a: u8 = FIRST[0];
            let b: u8 = SECOND[0];
        }
    "#,
    );
    "#,
    );

    // Should emit both arrays
    assert_asm_contains(&asm, "FIRST:");
    assert_asm_contains(&asm, ".BYTE $01, $02, $03, $04");
    assert_asm_contains(&asm, "SECOND:");
    assert_asm_contains(&asm, ".BYTE $05, $06, $07, $08");

    // Both should be in DATA section
    assert_asm_order(&asm, "FIRST:", "SECOND:");
}

#[test]
fn const_array_separated_from_code() {
    let asm = compile_success(
        r#"
    let asm = compile_success(
        r#"
        const DATA: [u8; 4] = [1, 2, 3, 4];

        fn helper() {
            let x: u8 = 10;
        }

        fn main() {
            helper();
        }
    "#,
    );
    "#,
    );

    // Data section should come before code section
    assert_asm_order(&asm, "Data Section (Const Arrays)", "Code from main module");

    // DATA array should be in data section, not mixed with code
    let data_pos = asm.find("DATA:").expect("DATA label not found");
    let helper_pos = asm.find("helper:").expect("helper label not found");
    let main_pos = asm.find("main:").expect("main label not found");

    assert!(
        data_pos < helper_pos,
        "DATA should come before helper function"
    );
    assert!(
        data_pos < helper_pos,
        "DATA should come before helper function"
    );
    assert!(data_pos < main_pos, "DATA should come before main function");
}
