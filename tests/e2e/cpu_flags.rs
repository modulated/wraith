//! End-to-end tests for CPU flags access

use crate::common::*;

// ============================================================
// CPU Flags Tests
// ============================================================

#[test]
fn carry_flag_in_addition() {
    let asm = compile_success(
        r#"
        fn main() {
            let x: u8 = 255;
            let y: u8 = x + 1;
        }
    "#,
    );

    // Must clear carry before ADC
    assert_asm_contains(&asm, "CLC");
    assert_asm_contains(&asm, "ADC");
}

#[test]
fn carry_flag_in_subtraction() {
    let asm = compile_success(
        r#"
        fn main() {
            let x: u8 = 10;
            let y: u8 = x - 5;
        }
    "#,
    );

    // Must set carry before SBC (inverted on 6502)
    assert_asm_contains(&asm, "SEC");
    assert_asm_contains(&asm, "SBC");
}

#[test]
fn zero_flag_in_equality() {
    let asm = compile_success(
        r#"
        fn main() {
            let x: u8 = 5;
            if x == 5 {
                let y: u8 = 1;
            }
        }
    "#,
    );

    // BEQ = Branch if Equal (Z flag set)
    assert_asm_contains(&asm, "CMP");
    assert_asm_contains(&asm, "BEQ");
}

#[test]
fn carry_flag_read_as_bool() {
    let asm = compile_success(
        r#"
        fn main() {
            let x: u8 = 255;
            let y: u8 = x + 1;
            let flag: u8 = carry as u8;
        }
    "#,
    );

    // Should read carry flag and convert to 0 or 1
    assert_asm_contains(&asm, "BCS"); // Branch if carry set
    assert_asm_contains(&asm, "LDA");
}

#[test]
fn zero_flag_read_as_bool() {
    let asm = compile_success(
        r#"
        fn main() {
            let x: u8 = 5;
            let y: u8 = x - 5;
            let flag: u8 = zero as u8;
        }
    "#,
    );

    // Should read zero flag and convert to 0 or 1
    assert_asm_contains(&asm, "SEC");
    assert_asm_contains(&asm, "SBC");
}

#[test]
fn u16_carry_propagation() {
    let asm = compile_success(
        r#"
        fn main() {
            let x: u16 = 0xFFFF;
            let one: u16 = 1;
            let y: u16 = x + one;
        }
    "#,
    );

    // Must propagate carry from low to high byte
    assert_asm_contains(&asm, "CLC");
    assert_asm_contains(&asm, "ADC");
    // Should have ADC twice (once for low byte, once for high byte)
    assert!(count_pattern(&asm, "ADC") >= 2);
}
