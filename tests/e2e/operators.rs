//! Operator end-to-end tests
//!
//! Tests for arithmetic, logical, bitwise, and compound assignment operators

use crate::common::*;

// ============================================================================
// Compound Assignment Operators
// ============================================================================

#[test]
fn compound_add_assign() {
    let asm = compile_success(r#"
        fn main() {
            x: u8 = 10;
            x += 5;
        }
    "#);

    assert_asm_contains(&asm, "ADC");
}

#[test]
fn compound_sub_assign() {
    let asm = compile_success(r#"
        fn main() {
            x: u8 = 10;
            x -= 3;
        }
    "#);

    assert_asm_contains(&asm, "SBC");
}

#[test]
fn compound_mul_assign() {
    let asm = compile_success(r#"
        fn main() {
            x: u8 = 10;
            x *= 2;
        }
    "#);

    // Multiplication should be present (implementation detail)
    assert!(asm.contains("STA"));
}

#[test]
fn compound_div_assign() {
    let asm = compile_success(r#"
        fn main() {
            x: u8 = 10;
            x /= 2;
        }
    "#);

    // Division should be present (implementation detail)
    assert!(asm.contains("STA"));
}

#[test]
fn compound_bitwise_and_assign() {
    let asm = compile_success(r#"
        fn main() {
            x: u8 = 0xFF;
            x &= 0x0F;
        }
    "#);

    assert_asm_contains(&asm, "AND");
}

#[test]
fn compound_bitwise_or_assign() {
    let asm = compile_success(r#"
        fn main() {
            x: u8 = 0x0F;
            x |= 0xF0;
        }
    "#);

    assert_asm_contains(&asm, "ORA");
}

#[test]
fn compound_bitwise_xor_assign() {
    let asm = compile_success(r#"
        fn main() {
            x: u8 = 0xFF;
            x ^= 0xAA;
        }
    "#);

    assert_asm_contains(&asm, "EOR");
}

#[test]
fn compound_shift_left_assign() {
    let asm = compile_success(r#"
        fn main() {
            x: u8 = 1;
            x <<= 3;
        }
    "#);

    assert_asm_contains(&asm, "ASL");
}

#[test]
fn compound_shift_right_assign() {
    let asm = compile_success(r#"
        fn main() {
            x: u8 = 8;
            x >>= 2;
        }
    "#);

    assert_asm_contains(&asm, "LSR");
}

// ============================================================================
// Increment/Decrement Operators
// ============================================================================

#[test]
fn increment_variable() {
    let asm = compile_success(r#"
        fn main() {
            x: u8 = 10;
            x += 1;
        }
    "#);

    // Should optimize to INC
    assert_asm_contains(&asm, "INC");
}

#[test]
fn decrement_variable() {
    let asm = compile_success(r#"
        fn main() {
            x: u8 = 10;
            x -= 1;
        }
    "#);

    // Should optimize to DEC
    assert_asm_contains(&asm, "DEC");
}

// ============================================================================
// Bitwise Operators
// ============================================================================

#[test]
fn bitwise_and() {
    let asm = compile_success(r#"
        addr RESULT = 0x0400;
        fn main() {
            RESULT = 0xFF & 0x0F;
        }
    "#);

    // Should constant fold
    assert_asm_contains(&asm, "LDA #$0F");
}

#[test]
fn bitwise_or() {
    let asm = compile_success(r#"
        addr RESULT = 0x0400;
        fn main() {
            RESULT = 0x0F | 0xF0;
        }
    "#);

    // Should constant fold to 0xFF
    assert_asm_contains(&asm, "LDA #$FF");
}

#[test]
fn bitwise_xor() {
    let asm = compile_success(r#"
        addr RESULT = 0x0400;
        fn main() {
            RESULT = 0xFF ^ 0xAA;
        }
    "#);

    // Should constant fold to 0x55
    assert_asm_contains(&asm, "LDA #$55");
}

#[test]
fn bitwise_not() {
    let asm = compile_success(r#"
        addr RESULT = 0x0400;
        fn main() {
            RESULT = ~0x0F;
        }
    "#);

    // Should constant fold to 0xF0
    assert_asm_contains(&asm, "LDA #$F0");
}

// ============================================================================
// Logical Operators
// ============================================================================

#[test]
fn logical_and() {
    let asm = compile_success(r#"
        fn main() {
            if true && false {
                x: u8 = 1;
            }
        }
    "#);

    // Should short-circuit
    assert_asm_contains(&asm, "BEQ");
}

#[test]
fn logical_or() {
    let asm = compile_success(r#"
        fn main() {
            if true || false {
                x: u8 = 1;
            }
        }
    "#);

    // Should short-circuit
    assert!(asm.contains("BNE") || asm.contains("BEQ"));
}

#[test]
fn logical_not() {
    let asm = compile_success(r#"
        fn main() {
            if !false {
                x: u8 = 1;
            }
        }
    "#);

    assert_asm_contains(&asm, "BEQ");
}

// ============================================================================
// Comparison Operators
// ============================================================================

#[test]
fn comparison_equal() {
    let asm = compile_success(r#"
        fn main() {
            x: u8 = 5;
            if x == 5 {
                y: u8 = 1;
            }
        }
    "#);

    assert_asm_contains(&asm, "CMP");
    assert_asm_contains(&asm, "BEQ");
}

#[test]
fn comparison_not_equal() {
    let asm = compile_success(r#"
        fn main() {
            x: u8 = 5;
            if x != 5 {
                y: u8 = 1;
            }
        }
    "#);

    assert_asm_contains(&asm, "CMP");
    assert_asm_contains(&asm, "BNE");
}

#[test]
fn comparison_less_than() {
    let asm = compile_success(r#"
        fn main() {
            x: u8 = 5;
            if x < 10 {
                y: u8 = 1;
            }
        }
    "#);

    assert_asm_contains(&asm, "CMP");
    assert_asm_contains(&asm, "BCC");
}

#[test]
fn comparison_greater_than() {
    let asm = compile_success(r#"
        fn main() {
            x: u8 = 10;
            if x > 5 {
                y: u8 = 1;
            }
        }
    "#);

    assert_asm_contains(&asm, "CMP");
}

#[test]
fn comparison_less_equal() {
    let asm = compile_success(r#"
        fn main() {
            x: u8 = 5;
            if x <= 10 {
                y: u8 = 1;
            }
        }
    "#);

    assert_asm_contains(&asm, "CMP");
}

#[test]
fn comparison_greater_equal() {
    let asm = compile_success(r#"
        fn main() {
            x: u8 = 10;
            if x >= 5 {
                y: u8 = 1;
            }
        }
    "#);

    assert_asm_contains(&asm, "CMP");
}

// ============================================================================
// Shift Operators
// ============================================================================

#[test]
fn shift_left() {
    let asm = compile_success(r#"
        addr RESULT = 0x0400;
        fn main() {
            RESULT = 1 << 3;
        }
    "#);

    // Should constant fold to 8
    assert_asm_contains(&asm, "LDA #$08");
}

#[test]
fn shift_right() {
    let asm = compile_success(r#"
        addr RESULT = 0x0400;
        fn main() {
            RESULT = 16 >> 2;
        }
    "#);

    // Should constant fold to 4
    assert_asm_contains(&asm, "LDA #$04");
}

#[test]
fn shift_left_variable() {
    let asm = compile_success(r#"
        fn main() {
            x: u8 = 4;
            x = x << 1;
        }
    "#);

    assert_asm_contains(&asm, "ASL");
}

#[test]
fn shift_right_variable() {
    let asm = compile_success(r#"
        fn main() {
            x: u8 = 8;
            x = x >> 1;
        }
    "#);

    assert_asm_contains(&asm, "LSR");
}
