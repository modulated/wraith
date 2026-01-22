//! Tests for BCD (Binary-Coded Decimal) literal validation

use crate::common::*;

// ============================================================
// Valid BCD Literal Tests
// ============================================================

#[test]
fn valid_bcd_decimal_literals() {
    // These should all work - converting decimal to BCD format
    let _asm = compile_success(
        r#"
        fn main() {
            let a: b8 = 0 as b8;      // 0 decimal -> 0x00 BCD
            let b: b8 = 99 as b8;     // 99 decimal -> 0x99 BCD
            let c: b16 = 1234 as b16; // 1234 decimal -> 0x1234 BCD
        }
    "#,
    );
}

// ============================================================
// Invalid BCD Range Tests
// ============================================================

#[test]
fn invalid_bcd_decimal_out_of_range() {
    // b8 can only hold 0-99
    assert_error_contains(
        r#"
        fn main() {
            let a: b8 = 100 as b8;  // 100 is out of range for b8
        }
        "#,
        "range",
    );
}

#[test]
fn invalid_bcd_b16_out_of_range() {
    // b16 can only hold 0-9999
    assert_error_contains(
        r#"
        fn main() {
            let a: b16 = 10000 as b16;  // 10000 is out of range for b16
        }
        "#,
        "range",
    );
}

#[test]
fn bcd_negative_value_compiles_but_wraps() {
    // BCD doesn't support negative numbers, but -1 as a unary expression
    // cannot be fully evaluated at compile time in all cases
    // This is a known limitation - the value will wrap at runtime
    let _asm = compile_success(
        r#"
        fn main() {
            let a: b8 = -1 as b8;  // Will wrap to 255 at runtime
        }
    "#,
    );
    // TODO: Consider adding a warning for this case
}

// ============================================================
// Hex Literal Tests
// ============================================================

#[test]
fn bcd_from_valid_hex_literal() {
    // Hex literals are just integers - they get converted to decimal first
    // 0x42 = 66 decimal, which converts to 0x66 BCD
    let _asm = compile_success(
        r#"
        fn main() {
            let a: b8 = 0x42 as b8;  // 0x42 hex = 66 decimal -> 0x66 BCD
        }
    "#,
    );
}

#[test]
fn bcd_from_invalid_hex_literal_out_of_range() {
    // 0x99 hex = 153 decimal, which is out of range for b8 (max 99)
    assert_error_contains(
        r#"
        fn main() {
            let a: b8 = 0x99 as b8;  // 0x99 hex = 153 decimal -> ERROR
        }
        "#,
        "range",
    );
}

// ============================================================
// Runtime Cast Tests (Current Limitation)
// ============================================================

#[test]
fn runtime_u8_to_bcd_cast_compiles() {
    // This is the problematic case - runtime cast from u8 to b8
    // The u8 might contain invalid BCD digits (e.g., 0xAB)
    // Currently this compiles but ideally should warn
    let _asm = compile_success(
        r#"
        fn main() {
            let x: u8 = 171;  // Would be 0xAB in hex
            let y: b8 = x as b8;  // Runtime cast - no validation possible
        }
    "#,
    );
    // TODO: Add warning for runtime casts to BCD types
}
