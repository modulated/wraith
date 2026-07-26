//! End-to-end tests for BCD (Binary Coded Decimal) support

use crate::common::*;

// ============================================================
// Type Checking Tests
// ============================================================

#[test]
fn bcd_type_mismatch_error() {
    assert_error_contains(
        r#"
        fn test() {
            let x: b8 = 42;
            let y: u8 = 10;
            let z: b8 = x + y;  // ERROR: cannot mix b8 and u8
        }
        "#,
        "type",
    );
}

#[test]
fn bcd_explicit_cast_allowed() {
    let asm = compile_success(
        r#"
        fn test() {
            let x: b8 = 42 as b8;
            let y: u8 = 10;
            let z: b8 = x + (y as b8);  // OK
        }
    "#,
    );
    assert_asm_contains(&asm, "SED");
    assert_asm_contains(&asm, "CLD");
}

#[test]
fn bcd_mul_not_allowed() {
    assert_error_contains(
        r#"
        fn test() {
            let x: b8 = 10 as b8;
            let y: b8 = x * (2 as b8);  // ERROR
        }
        "#,
        "BCD",
    );
}

#[test]
fn bcd_bitwise_not_allowed() {
    assert_error_contains(
        r#"
        fn test() {
            let x: b8 = 10 as b8;
            let y: b8 = x & (0xFF as b8);  // ERROR
        }
        "#,
        "BCD",
    );
}

// ============================================================
// Codegen Tests
// ============================================================

#[test]
fn b8_addition_codegen() {
    let asm = compile_success(
        r#"
        fn add(a: b8, b: b8) -> b8 {
            return a + b;
        }
    "#,
    );

    // Verify SED before ADC, CLD after
    assert_asm_contains(&asm, "SED");
    assert_asm_contains(&asm, "ADC");
    assert_asm_contains(&asm, "CLD");

    let sed_pos = asm.find("SED").unwrap();
    let adc_pos = asm.find("ADC").unwrap();
    let cld_pos = asm.find("CLD").unwrap();
    assert!(sed_pos < adc_pos && adc_pos < cld_pos);
}

#[test]
fn b8_subtraction_codegen() {
    let asm = compile_success(
        r#"
        fn sub(a: b8, b: b8) -> b8 {
            return a - b;
        }
    "#,
    );

    assert_asm_contains(&asm, "SED");
    assert_asm_contains(&asm, "SBC");
    assert_asm_contains(&asm, "CLD");
}

#[test]
fn b16_addition_codegen() {
    let asm = compile_success(
        r#"
        fn add(a: b16, b: b16) -> b16 {
            return a + b;
        }
    "#,
    );

    // Multi-byte BCD
    assert_asm_contains(&asm, "SED");
    assert_eq!(count_pattern(&asm, "ADC"), 2); // Two ADC for low/high bytes
    assert_asm_contains(&asm, "CLD");
}

#[test]
fn bcd_comparison_no_sed() {
    let asm = compile_success(
        r#"
        fn compare(a: b8, b: b8) -> bool {
            return a > b;
        }
    "#,
    );

    // Comparisons don't need decimal mode
    assert!(!asm.contains("SED"));
    assert_asm_contains(&asm, "CMP");
}

#[test]
fn bcd_variable_declaration() {
    let asm = compile_success(
        r#"
        fn test() {
            let score: b8 = 99 as b8;
            let high_score: b16 = 9999 as b16;
        }
    "#,
    );

    // Literals are inlined, not stored as data
    // 99 decimal → 0x99 BCD (each nibble is a decimal digit)
    assert_asm_contains(&asm, "LDA #$99"); // 99 in BCD format
}

// ============================================================
// Cast Tests
// ============================================================

#[test]
fn bcd_cast_to_binary() {
    let asm = compile_success(
        r#"
        fn test() {
            let x: b8 = 42 as b8;
            let y: u8 = x as u8;
        }
    "#,
    );

    // Cast should compile (bit pattern unchanged)
    assert_asm_contains(&asm, "LDA");
}

#[test]
fn binary_cast_to_bcd() {
    let asm = compile_success(
        r#"
        fn test() {
            let x: u8 = 99;
            let y: b8 = x as b8;
        }
    "#,
    );

    // Cast should compile (user responsible for valid BCD)
    assert_asm_contains(&asm, "LDA");
}

#[test]
fn b8_to_b16_cast() {
    let asm = compile_success(
        r#"
        fn test() {
            let x: b8 = 42 as b8;
            let y: b16 = x as b16;
        }
    "#,
    );

    // Zero-extend
    assert_asm_contains(&asm, "LDY #$00");
}

// ============================================================
// Complex Expression Tests
// ============================================================

#[test]
fn bcd_multiple_operations() {
    let asm = compile_success(
        r#"
        fn calc(a: b8, b: b8, c: b8) -> b8 {
            let temp: b8 = a + b;
            let result: b8 = temp - c;
            return result;
        }
    "#,
    );

    // Should have two SED/CLD pairs
    assert_eq!(count_pattern(&asm, "SED"), 2);
    assert_eq!(count_pattern(&asm, "CLD"), 2);
}

#[test]
fn bcd_equality_test() {
    let asm = compile_success(
        r#"
        fn equal(a: b8, b: b8) -> bool {
            return a == b;
        }
    "#,
    );

    // Equality doesn't need BCD mode
    assert!(!asm.contains("SED"));
    assert_asm_contains(&asm, "CMP");
}

// ============================================================
// Executed BCD arithmetic
//
// The tests above check that decimal mode is entered (SED/CLD) and that the
// type rules hold. Neither shows the arithmetic is *decimal*: 0x19 + 0x01 is
// 0x1A in binary but must be 0x20 in BCD. These run it.
// ============================================================

use crate::common::exec::run;

/// Evaluate a b8 expression, returning the raw BCD byte.
fn eval_b8(body: &str) -> u8 {
    let src = format!(
        r#"
        const OUT: addr = 0x0900;
        #[reset]
        fn main() {{
            {body}
            loop {{}}
        }}
    "#
    );
    run(&src).mem(0x0900)
}

#[test]
fn b8_addition_carries_decimally() {
    // 19 + 1 = 20 in BCD ($19 + $01 = $20), not $1A.
    assert_eq!(
        eval_b8("let a: b8 = 19 as b8; let b: b8 = 1 as b8; OUT = (a + b) as u8;"),
        0x20
    );
}

#[test]
fn b8_addition_within_a_digit() {
    // No carry between digits: 12 + 3 = 15 ($12 + $03 = $15).
    assert_eq!(
        eval_b8("let a: b8 = 12 as b8; let b: b8 = 3 as b8; OUT = (a + b) as u8;"),
        0x15
    );
}

#[test]
fn b8_addition_carries_into_upper_digit() {
    // 45 + 27 = 72 ($45 + $27 = $72); binary addition would give $6C.
    assert_eq!(
        eval_b8("let a: b8 = 45 as b8; let b: b8 = 27 as b8; OUT = (a + b) as u8;"),
        0x72
    );
}

#[test]
fn b8_subtraction_borrows_decimally() {
    // 20 - 1 = 19 ($20 - $01 = $19); binary subtraction would give $1F.
    assert_eq!(
        eval_b8("let a: b8 = 20 as b8; let b: b8 = 1 as b8; OUT = (a - b) as u8;"),
        0x19
    );
}

#[test]
fn b8_subtraction_across_digits() {
    // 50 - 23 = 27 ($50 - $23 = $27).
    assert_eq!(
        eval_b8("let a: b8 = 50 as b8; let b: b8 = 23 as b8; OUT = (a - b) as u8;"),
        0x27
    );
}

#[test]
fn decimal_mode_is_cleared_afterwards() {
    // A stray SED would corrupt every later binary add. Do BCD arithmetic, then
    // ordinary binary arithmetic, and check the binary result is not decimal.
    assert_eq!(
        eval_b8(
            "let a: b8 = 19 as b8; let b: b8 = 1 as b8; let c: b8 = a + b;
             let x: u8 = 0x19; let y: u8 = 0x01; OUT = x + y;"
        ),
        0x1A,
        "binary addition after BCD must stay binary ($19 + $01 = $1A)"
    );
}
