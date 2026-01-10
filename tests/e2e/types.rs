//! End-to-end tests for complex types

use crate::common::*;

#[test]
fn array_literal() {
    let asm = compile_success(r#"
        fn main() {
            let data: [u8; 3] = [1, 2, 3];
        }
    "#);

    assert_asm_contains(&asm, ".BYTE $01");
    assert_asm_contains(&asm, ".BYTE $02");
    assert_asm_contains(&asm, ".BYTE $03");
}

#[test]
fn array_fill() {
    let asm = compile_success(r#"
        fn main() {
            let buffer: [u8; 5] = [0; 5];
        }
    "#);

    assert_eq!(count_pattern(&asm, ".BYTE $00"), 5);
}

#[test]
fn struct_definition() {
    let asm = compile_success(r#"
        struct Point {
            u8 x,
            u8 y,
        }
        fn main() {}
    "#);

    // Struct definition doesn't generate code by itself
    assert_asm_contains(&asm, "main:");
}

#[test]
fn struct_initialization() {
    let asm = compile_success(r#"
        struct Point {
            u8 x,
            u8 y,
        }
        fn main() {
            let p: Point = Point { x: 10, y: 20 };
        }
    "#);

    assert_asm_contains(&asm, "LDA");
    assert_asm_contains(&asm, "STA");
}

#[test]
fn enum_definition() {
    let asm = compile_success(r#"
        enum Direction {
            North,
            South,
            East,
            West,
        }
        fn main() {
            let d: Direction = Direction::North;
        }
    "#);

    assert_asm_contains(&asm, "Direction::North");
    assert_asm_contains(&asm, ".BYTE $00");
}

// ============================================================
// Array Indexing Tests
// ============================================================

#[test]
fn array_index_constant() {
    let asm = compile_success(r#"
        fn main() {
            let data: [u8; 5] = [10, 20, 30, 40, 50];
            let x: u8 = data[2];
        }
    "#);

    // Should load from array with constant offset
    assert_asm_contains(&asm, "LDA");
    assert_asm_contains(&asm, "STA");
}

#[test]
fn array_index_variable() {
    let asm = compile_success(r#"
        fn main() {
            let data: [u8; 5] = [1, 2, 3, 4, 5];
            let idx: u8 = 2;
            let val: u8 = data[idx];
        }
    "#);

    // Variable index requires indexed addressing mode
    assert_asm_contains(&asm, "LDA");
    assert_asm_contains(&asm, "STA");
}

#[test]
fn array_index_write() {
    let asm = compile_success(r#"
        fn main() {
            let data: [u8; 5] = [0, 0, 0, 0, 0];
            data[2] = 42;
        }
    "#);

    // Should generate LDA followed by STA with offset
    assert_asm_contains(&asm, "LDA #$2A");  // Load 42
    assert_asm_contains(&asm, "STA");
}

#[test]
fn array_index_bounds() {
    let asm = compile_success(r#"
        fn main() {
            let data: [u8; 3] = [1, 2, 3];
            let first: u8 = data[0];
            let last: u8 = data[2];
        }
    "#);

    // Should compile and access boundary indices
    assert_asm_contains(&asm, "LDA");
    assert_asm_contains(&asm, "STA");
}

#[test]
fn array_index_u16_elements() {
    let asm = compile_success(r#"
        fn main() {
            let data: [u8; 3] = [100, 200, 50];
            let val: u8 = data[1];
        }
    "#);

    // Should load from array correctly
    assert_asm_contains(&asm, "LDA");
}

// ============================================================
// Struct Field Access Tests
// ============================================================

#[test]
fn struct_field_read() {
    let asm = compile_success(r#"
        struct Point {
            u8 x,
            u8 y,
        }
        fn main() {
            let p: Point = Point { x: 10, y: 20 };
            let val: u8 = p.x;
        }
    "#);

    // Should load from base address + field offset
    assert_asm_contains(&asm, "LDA");
    assert_asm_contains(&asm, "STA");
}

#[test]
#[ignore] // TODO: Struct field assignment not yet implemented
fn struct_field_write() {
    let asm = compile_success(r#"
        struct Point {
            u8 x,
            u8 y,
        }
        fn main() {
            let p: Point = Point { x: 0, y: 0 };
            p.y = 42;
        }
    "#);

    // Should store to base address + field offset
    assert_asm_contains(&asm, "LDA #$2A");  // Load 42
    assert_asm_contains(&asm, "STA");
}

#[test]
fn struct_multiple_field_access() {
    let asm = compile_success(r#"
        struct RGB {
            u8 r,
            u8 g,
            u8 b,
        }
        fn main() {
            let color: RGB = RGB { r: 255, g: 128, b: 64 };
            let red: u8 = color.r;
            let green: u8 = color.g;
            let blue: u8 = color.b;
        }
    "#);

    // Should access three different field offsets
    assert_asm_contains(&asm, "LDA");
    assert_asm_contains(&asm, "STA");
}

#[test]
#[ignore] // TODO: Nested struct initialization not yet supported
fn nested_struct_field_access() {
    let asm = compile_success(r#"
        struct Inner {
            u8 value,
        }
        struct Outer {
            Inner inner,
            u8 other,
        }
        fn main() {
            let obj: Outer = Outer {
                inner: Inner { value: 42 },
                other: 10
            };
            let x: u8 = obj.inner.value;
        }
    "#);

    // Should calculate nested offset correctly
    assert_asm_contains(&asm, "LDA");
    assert_asm_contains(&asm, "STA");
}

#[test]
fn struct_field_u16() {
    let asm = compile_success(r#"
        struct Data {
            u16 value,
            u8 flags,
        }
        fn main() {
            let d: Data = Data { value: 0x1234, flags: 0xFF };
            let a: u16 = d.value;
        }
    "#);

    // Should load 2 bytes for u16 field (little-endian on 6502)
    assert_asm_contains(&asm, "LDA");
    assert_asm_contains(&asm, "STA");
}

// ============================================================
// Signed Integer Tests
// ============================================================

#[test]
#[ignore] // TODO: i8 literals need explicit cast from u8
fn i8_basic_operations() {
    let asm = compile_success(r#"
        fn main() {
            let x: i8 = 10 as i8;
            let y: i8 = 20 as i8;
            let z: i8 = x + y;
        }
    "#);

    // Signed addition works same as unsigned on 6502
    assert_asm_contains(&asm, "ADC");
    assert_asm_contains(&asm, "LDA");
}

#[test]
#[ignore] // TODO: Negative literals not yet supported
fn i8_negative_values() {
    let asm = compile_success(r#"
        fn main() {
            let x: i8 = -5;
            let y: i8 = -10;
        }
    "#);

    // Negative numbers use two's complement
    // -5 = 0xFB, -10 = 0xF6
    assert_asm_contains(&asm, "LDA");
    assert_asm_contains(&asm, "STA");
}

#[test]
#[ignore] // TODO: i16 operations need more work
fn i16_basic_operations() {
    let asm = compile_success(r#"
        fn main() {
            let x: i16 = 100;
            let y: i16 = 200;
            let z: i16 = x + y;
        }
    "#);

    // 16-bit operations need carry handling
    assert_asm_contains(&asm, "ADC");
    assert_asm_contains(&asm, "CLC");
}

#[test]
#[ignore] // TODO: Negative literals and sign extension not yet supported
fn i8_to_i16_cast() {
    let asm = compile_success(r#"
        fn main() {
            let x: i8 = -5;
            let y: i16 = x as i16;
        }
    "#);

    // Cast should handle type conversion
    assert_asm_contains(&asm, "LDA");
    assert_asm_contains(&asm, "STA");
}

#[test]
fn signed_unsigned_cast() {
    let asm = compile_success(r#"
        fn main() {
            let x: u8 = 255;
            let y: i8 = x as i8;
            let z: u8 = 127;
            let w: i8 = z as i8;
        }
    "#);

    // Explicit casts between signed and unsigned (same bit pattern)
    assert_asm_contains(&asm, "LDA");
    assert_asm_contains(&asm, "STA");
}

// ============================================================
// Array Write Tests (Comprehensive)
// ============================================================

#[test]
fn array_write_constant_index() {
    let asm = compile_success(r#"
        fn main() {
            let data: [u8; 5] = [0, 0, 0, 0, 0];
            data[2] = 42;
        }
    "#);

    assert_asm_contains(&asm, "LDA #$2A");
    assert_asm_contains(&asm, "STA ($");  // Indirect indexed store
    assert_asm_contains(&asm, "TAY");     // Index to Y
}

#[test]
fn array_write_variable_index() {
    let asm = compile_success(r#"
        fn main() {
            let data: [u8; 5] = [1, 2, 3, 4, 5];
            let idx: u8 = 3;
            data[idx] = 99;
        }
    "#);

    assert_asm_contains(&asm, "TAY");
    assert_asm_contains(&asm, "STA ($");
}

#[test]
fn array_write_expression_index() {
    let asm = compile_success(r#"
        fn main() {
            let data: [u8; 10] = [0; 10];
            let i: u8 = 2;
            data[i + 1] = 42;
        }
    "#);

    // Should evaluate i + 1
    assert_asm_contains(&asm, "ADC");
    assert_asm_contains(&asm, "TAY");
    assert_asm_contains(&asm, "STA ($");
}

#[test]
fn array_write_in_loop() {
    let asm = compile_success(r#"
        fn main() {
            let data: [u8; 5] = [0; 5];
            for i: u8 in 0..5 {
                data[i] = i;
            }
        }
    "#);

    // Loop counter (X) should be copied for array indexing
    assert_asm_contains(&asm, "STA ($");
}

#[test]
fn array_write_complex_value() {
    let asm = compile_success(r#"
        fn main() {
            let data: [u8; 5] = [0; 5];
            let a: u8 = 10;
            let b: u8 = 20;
            data[1] = a + b;
        }
    "#);

    // Should evaluate a + b, save it, then store
    assert_asm_contains(&asm, "ADC");
    assert_asm_contains(&asm, "STA $20");  // Save to temp
    assert_asm_contains(&asm, "STA ($");   // Store to array
}

// ============================================================
// Shorthand Array Initializer Tests
// ============================================================

#[test]
fn shorthand_array_u8_basic() {
    let asm = compile_success(r#"
        fn main() {
            let buffer: [u8; 5] = [0xFF];
        }
    "#);

    // Should expand to 5 bytes of 0xFF
    assert_eq!(count_pattern(&asm, ".BYTE $FF"), 5);
    assert_asm_contains(&asm, "Expanding [value] to [5 elements]");
}

#[test]
fn shorthand_array_zero_fill() {
    let asm = compile_success(r#"
        fn main() {
            let buffer: [u8; 10] = [0];
        }
    "#);

    // Should expand to 10 bytes of 0x00
    assert_eq!(count_pattern(&asm, ".BYTE $00"), 10);
}

#[test]
fn shorthand_array_different_sizes() {
    let asm = compile_success(r#"
        fn main() {
            let small: [u8; 3] = [42];
            let medium: [u8; 8] = [99];
            let large: [u8; 16] = [0x55];
        }
    "#);

    // Should have appropriate counts for each
    assert_asm_contains(&asm, ".BYTE $2A");  // 42
    assert_asm_contains(&asm, ".BYTE $63");  // 99
    assert_asm_contains(&asm, ".BYTE $55");  // 0x55
}

#[test]
#[ignore] // TODO: Shorthand syntax requires constant expressions (ArrayFill limitation)
fn shorthand_array_expression_value() {
    let asm = compile_success(r#"
        fn main() {
            let x: u8 = 10;
            let y: u8 = 20;
            let buffer: [u8; 4] = [x + y];
        }
    "#);

    // Expression should be evaluated and repeated
    assert_asm_contains(&asm, "ADC");  // Addition
    assert_asm_contains(&asm, "Expanding [value] to [4 elements]");
}

#[test]
#[ignore] // TODO: ArrayFill doesn't handle multi-byte elements (u16, i16) yet
fn shorthand_array_u16_elements() {
    let asm = compile_success(r#"
        fn main() {
            let buffer: [u16; 3] = [0x1234];
        }
    "#);

    // Should expand to 3 u16 values (6 bytes total)
    // Each u16 is stored little-endian: low byte, high byte
    assert_asm_contains(&asm, ".BYTE $34");  // Low byte
    assert_asm_contains(&asm, ".BYTE $12");  // High byte
}

#[test]
fn shorthand_vs_explicit_array() {
    let asm1 = compile_success(r#"
        fn main() {
            let a: [u8; 4] = [7];
        }
    "#);

    let asm2 = compile_success(r#"
        fn main() {
            let a: [u8; 4] = [7, 7, 7, 7];
        }
    "#);

    // Both should generate the same number of .BYTE directives
    assert_eq!(count_pattern(&asm1, ".BYTE $07"), 4);
    assert_eq!(count_pattern(&asm2, ".BYTE $07"), 4);
}

#[test]
fn shorthand_array_with_bool() {
    let asm = compile_success(r#"
        fn main() {
            let flags: [u8; 5] = [true];
        }
    "#);

    // true converts to 1
    assert_eq!(count_pattern(&asm, ".BYTE $01"), 5);
}

#[test]
fn shorthand_array_size_one_no_expand() {
    let asm = compile_success(r#"
        fn main() {
            let single: [u8; 1] = [42];
        }
    "#);

    // Size 1 should not trigger expansion comment
    assert!(!asm.contains("Expanding [value]"));
    assert_asm_contains(&asm, ".BYTE $2A");
}
