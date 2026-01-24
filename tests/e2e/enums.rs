//! End-to-end tests for enum types and pattern matching
//!
//! Tests cover:
//! - Simple enum creation and matching
//! - Tuple variant pattern matching with data extraction
//! - Multi-field tuple variants
//! - u16/multi-byte fields in tuple variants
//! - Mixed variant types
//! - Edge cases

use crate::common::*;

// ============================================================
// Simple Enum Tests (Unit Variants)
// ============================================================

#[test]
fn simple_enum_creation() {
    let asm = compile_success(
        r#"
    let asm = compile_success(
        r#"
        enum Direction {
            North,
            South,
            East,
            West,
        }

        fn main() {
            let dir: Direction = Direction::North;
        }
    "#,
    );
    "#,
    );

    assert_asm_contains(&asm, "Direction::North");
    assert_asm_contains(&asm, ".BYTE $00");
}

#[test]
fn simple_enum_match() {
    let asm = compile_success(
        r#"
    let asm = compile_success(
        r#"
        enum Status {
            Off,
            On,
            Error,
        }

        fn main() {
            let s: Status = Status::On;
            match s {
                Status::Off => {
                    let x: u8 = 0;
                }
                Status::On => {
                    let x: u8 = 1;
                }
                Status::Error => {
                    let x: u8 = 255;
                }
            }
        }
    "#,
    );
    "#,
    );

    // With 3 variants, uses jump table dispatch
    assert_asm_contains(&asm, "; Match statement (jump table)");
    assert_asm_contains(&asm, "ASL"); // Double tag for address indexing
}

// ============================================================
// Tuple Variant Creation Tests
// ============================================================

#[test]
fn tuple_variant_single_field_u8() {
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

    // Should generate enum data with tag and field
    assert_asm_contains(&asm, "; Enum variant: Option::Some");
    assert_asm_contains(&asm, "en_");
    assert_asm_contains(&asm, ".BYTE $01"); // Tag for Some (second variant)
    assert_asm_contains(&asm, ".BYTE $2A"); // 42 in hex
    assert_asm_contains(&asm, ".BYTE $01"); // Tag for Some (second variant)
    assert_asm_contains(&asm, ".BYTE $2A"); // 42 in hex
}

#[test]
fn tuple_variant_multi_field_u8() {
    let asm = compile_success(
        r#"
    let asm = compile_success(
        r#"
        enum Color {
            RGB(u8, u8, u8),
        }

        fn main() {
            let red: Color = Color::RGB(255, 0, 0);
        }
    "#,
    );
    "#,
    );

    // Should generate enum data with tag and three fields
    assert_asm_contains(&asm, "; Enum variant: Color::RGB");
    assert_asm_contains(&asm, ".BYTE $00"); // Tag
    assert_asm_contains(&asm, ".BYTE $FF"); // Red = 255
    assert_asm_contains(&asm, ".BYTE $00"); // Green = 0
    assert_asm_contains(&asm, ".BYTE $00"); // Tag
    assert_asm_contains(&asm, ".BYTE $FF"); // Red = 255
    assert_asm_contains(&asm, ".BYTE $00"); // Green = 0
    // Third .BYTE $00 for blue
}

#[test]
fn tuple_variant_u16_field() {
    let asm = compile_success(
        r#"
    let asm = compile_success(
        r#"
        enum Result {
            Ok(u16),
            Err,
        }

        fn main() {
            let res: Result = Result::Ok(1000);
        }
    "#,
    );
    "#,
    );

    // Should generate enum with tag + 2 bytes for u16
    assert_asm_contains(&asm, "; Enum variant: Result::Ok");
    assert_asm_contains(&asm, ".BYTE $00"); // Tag
    assert_asm_contains(&asm, ".BYTE $00"); // Tag
    // 1000 = 0x03E8, little-endian: E8 03
    assert_asm_contains(&asm, ".BYTE $E8"); // Low byte
    assert_asm_contains(&asm, ".BYTE $03"); // High byte
    assert_asm_contains(&asm, ".BYTE $E8"); // Low byte
    assert_asm_contains(&asm, ".BYTE $03"); // High byte
}

#[test]
fn tuple_variant_mixed_types() {
    let asm = compile_success(
        r#"
    let asm = compile_success(
        r#"
        enum Message {
            SetValue(u8),
            SetAddress(u16),
            SetColor(u8, u8, u8),
        }

        fn main() {
            let msg: Message = Message::SetColor(128, 64, 32);
        }
    "#,
    );
    "#,
    );

    // Should generate enum with tag + three u8 fields
    assert_asm_contains(&asm, "; Enum variant: Message::SetColor");
    assert_asm_contains(&asm, ".BYTE $02"); // Tag for third variant
    assert_asm_contains(&asm, ".BYTE $80"); // 128
    assert_asm_contains(&asm, ".BYTE $40"); // 64
    assert_asm_contains(&asm, ".BYTE $20"); // 32
    assert_asm_contains(&asm, ".BYTE $02"); // Tag for third variant
    assert_asm_contains(&asm, ".BYTE $80"); // 128
    assert_asm_contains(&asm, ".BYTE $40"); // 64
    assert_asm_contains(&asm, ".BYTE $20"); // 32
}

// ============================================================
// Tuple Variant Pattern Matching Tests
// ============================================================

#[test]
fn match_tuple_variant_single_field_extract() {
    let asm = compile_success(
        r#"
    let asm = compile_success(
        r#"
        enum Option {
            None,
            Some(u8),
        }

        const RESULT: addr = 0x6000;

        fn main() {
            let opt: Option = Option::Some(42);
            match opt {
                Option::Some(value) => {
                    RESULT = value;
                }
                Option::None => {
                    RESULT = 0;
                }
            }
        }
    "#,
    );
    "#,
    );

    // Should have match structure with tag comparison
    assert_asm_contains(&asm, "; Match statement");
    assert_asm_contains(&asm, "STA $30"); // Store pointer low
    assert_asm_contains(&asm, "STX $31"); // Store pointer high
    assert_asm_contains(&asm, "LDA ($30),Y"); // Load tag byte

    // Should compare tags
    assert_asm_contains(&asm, "CMP #$01"); // Compare with Some tag
    assert_asm_contains(&asm, "CMP #$01"); // Compare with Some tag
    // Note: CMP #$00 is optimized away since LDA sets the Z flag
    // and BEQ can be used directly to check for zero (None)

    // Should have field extraction code
    // The binding extraction happens after tag match
}

#[test]
fn match_tuple_variant_multi_field_extract() {
    let asm = compile_success(
        r#"
    let asm = compile_success(
        r#"
        enum Color {
            RGB(u8, u8, u8),
            Grayscale(u8),
        }

        const RED_OUT: addr = 0x6000;
        const GREEN_OUT: addr = 0x6001;
        const BLUE_OUT: addr = 0x6002;

        fn main() {
            let color: Color = Color::RGB(255, 128, 64);
            match color {
                Color::RGB(r, g, b) => {
                    RED_OUT = r;
                    GREEN_OUT = g;
                    BLUE_OUT = b;
                }
                Color::Grayscale(gray) => {
                    RED_OUT = gray;
                    GREEN_OUT = gray;
                    BLUE_OUT = gray;
                }
            }
        }
    "#,
    );
    "#,
    );

    // Should have match structure
    assert_asm_contains(&asm, "; Match statement");

    // Should compare with both variant tags
    assert_asm_contains(&asm, "CMP #$00"); // RGB tag
    assert_asm_contains(&asm, "CMP #$01"); // Grayscale tag
    assert_asm_contains(&asm, "CMP #$00"); // RGB tag
    assert_asm_contains(&asm, "CMP #$01"); // Grayscale tag

    // Should extract multiple fields using indirect indexed addressing
    assert_asm_contains(&asm, "($30),Y");
}

#[test]
fn match_tuple_variant_u16_extract() {
    let asm = compile_success(
        r#"
    let asm = compile_success(
        r#"
        enum Result {
            Ok(u16),
            Err(u8),
        }

        const VALUE_LOW: addr = 0x6000;
        const VALUE_HIGH: addr = 0x6001;

        fn main() {
            let res: Result = Result::Ok(1000);
            match res {
                Result::Ok(value) => {
                    // value is u16, should be extracted properly
                    VALUE_LOW = value as u8;
                    VALUE_HIGH = (value >> 8) as u8;
                }
                Result::Err(code) => {
                    VALUE_LOW = code;
                    VALUE_HIGH = 0;
                }
            }
        }
    "#,
    );
    "#,
    );

    // Should have match structure
    assert_asm_contains(&asm, "; Match statement");

    // Should extract u16 value (2 bytes)
    assert_asm_contains(&asm, "($30),Y");
}

#[test]
fn match_tuple_variant_with_wildcard() {
    let asm = compile_success(
        r#"
    let asm = compile_success(
        r#"
        enum Option {
            None,
            Some(u8),
        }

        const OUTPUT: addr = 0x6000;

        fn main() {
            let opt: Option = Option::Some(99);
            match opt {
                Option::Some(val) => {
                    OUTPUT = val;
                }
                _ => {
                    OUTPUT = 0;
                }
            }
        }
    "#,
    );
    "#,
    );

    // Should have match with wildcard pattern
    assert_asm_contains(&asm, "; Match statement");
    assert_asm_contains(&asm, "CMP #$01"); // Check for Some
    assert_asm_contains(&asm, "JMP"); // Wildcard jumps
    assert_asm_contains(&asm, "CMP #$01"); // Check for Some
    assert_asm_contains(&asm, "JMP"); // Wildcard jumps
}

#[test]
fn match_tuple_variant_nested_enums() {
    let asm = compile_success(
        r#"
    let asm = compile_success(
        r#"
        enum Inner {
            Value(u8),
        }

        enum Outer {
            Contains(u8),
            Empty,
        }

        const OUTPUT: addr = 0x6000;

        fn main() {
            let inner: Inner = Inner::Value(42);
            let outer: Outer = Outer::Contains(10);

            match outer {
                Outer::Contains(x) => {
                    OUTPUT = x;
                }
                Outer::Empty => {
                    OUTPUT = 0;
                }
            }
        }
    "#,
    );
    "#,
    );

    // Both enums should compile and generate proper structures
    assert_asm_contains(&asm, "; Enum variant: Inner::Value");
    assert_asm_contains(&asm, "; Enum variant: Outer::Contains");
    assert_asm_contains(&asm, "; Match statement");
}

// ============================================================
// Edge Cases and Error Conditions
// ============================================================

#[test]
fn match_tuple_variant_no_bindings() {
    let asm = compile_success(
        r#"
    let asm = compile_success(
        r#"
        enum Option {
            None,
            Some(u8),
        }

        const MATCHED: addr = 0x6000;

        fn main() {
            let opt: Option = Option::Some(42);
            match opt {
                Option::Some(_) => {
                    // Don't extract the value, just match
                    MATCHED = 1;
                }
                Option::None => {
                    MATCHED = 0;
                }
            }
        }
    "#,
    );
    "#,
    );

    // Should match but not extract
    assert_asm_contains(&asm, "; Match statement");
    assert_asm_contains(&asm, "CMP");
}

#[test]
fn match_multiple_tuple_variants() {
    let asm = compile_success(
        r#"
    let asm = compile_success(
        r#"
        enum Input {
            Key(u8),
            Mouse(u8, u8),
            Joystick(u8),
            None,
        }

        const OUTPUT: addr = 0x6000;

        fn main() {
            let input: Input = Input::Mouse(100, 50);
            match input {
                Input::Key(code) => {
                    OUTPUT = code;
                }
                Input::Mouse(x, y) => {
                    OUTPUT = x;
                }
                Input::Joystick(dir) => {
                    OUTPUT = dir;
                }
                Input::None => {
                    OUTPUT = 255;
                }
            }
        }
    "#,
    );
    "#,
    );

    // With 4 variants, uses jump table dispatch
    assert_asm_contains(&asm, "; Match statement (jump table)");
    assert_asm_contains(&asm, "ASL"); // Double tag for address indexing
    assert_asm_contains(&asm, "($30),Y"); // Should extract fields using indirect indexed
}

#[test]
fn tuple_variant_return_from_function() {
    let asm = compile_success(
        r#"
    let asm = compile_success(
        r#"
        enum Option {
            None,
            Some(u8),
        }

        fn unwrap_or(opt: Option, default: u8) -> u8 {
            match opt {
                Option::Some(value) => {
                    return value;
                }
                Option::None => {
                    return default;
                }
            }
        }

        fn main() {
            let opt: Option = Option::Some(42);
            let result: u8 = unwrap_or(opt, 0);
        }
    "#,
    );
    "#,
    );

    // Should compile function that takes enum and extracts value
    assert_asm_contains(&asm, "; Match statement");
    assert_asm_contains(&asm, "RTS");
}

#[test]
fn tuple_variant_in_loop() {
    let asm = compile_success(
        r#"
    let asm = compile_success(
        r#"
        enum Option {
            None,
            Some(u8),
        }

        const OUTPUT: addr = 0x6000;

        fn main() {
            let i: u8 = 0;
            loop {
                // Note: Enum variant creation requires constants, so we use 42 instead of i
                let opt: Option = Option::Some(42);
                match opt {
                    Option::Some(val) => {
                        OUTPUT = val;
                    }
                    Option::None => {
                        break;
                    }
                }

                i = i + 1;
                if i >= 10 {
                    break;
                }
            }
        }
    "#,
    );
    "#,
    );

    // Should handle pattern matching in loop context
    assert_asm_contains(&asm, "; Match statement");
    assert_asm_contains(&asm, "($30),Y"); // Field extraction
    assert_asm_contains(&asm, "JMP"); // Loop structure
}

#[test]
fn complex_tuple_variant_pattern() {
    let asm = compile_success(
        r#"
    let asm = compile_success(
        r#"
        enum Message {
            Quit,
            Move(u8, u8),
            Write(u8),
            ChangeColor(u8, u8, u8),
        }

        const X_POS: addr = 0x6000;
        const Y_POS: addr = 0x6001;

        fn main() {
            let msg: Message = Message::Move(10, 20);

            match msg {
                Message::Move(x, y) => {
                    X_POS = x;
                    Y_POS = y;
                }
                Message::Write(ch) => {
                    X_POS = ch;
                }
                Message::ChangeColor(r, g, b) => {
                    X_POS = r;
                }
                Message::Quit => {
                    X_POS = 0;
                }
            }
        }
    "#,
    );
    "#,
    );

    // Complex enum with multiple variant types - uses jump table for 4+ variants
    assert_asm_contains(&asm, "; Enum variant: Message::Move");
    assert_asm_contains(&asm, "; Match statement (jump table)");
    assert_asm_contains(&asm, "ASL"); // Double tag for address indexing
    assert_asm_contains(&asm, ".WORD match_"); // Jump table entries
}
