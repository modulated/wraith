//! End-to-end tests for variables and literals

use crate::common::*;

#[test]
fn variable_declaration_u8() {
    let asm = compile_success(r#"
        fn main() {
            x: u8 = 42;
        }
    "#);

    assert_asm_contains(&asm, "LDA");
    assert_asm_contains(&asm, "STA");
}

#[test]
fn variable_declaration_u16() {
    let asm = compile_success(r#"
        fn main() {
            x: u16 = 1000;
        }
    "#);

    assert_asm_contains(&asm, "LDA");
}

#[test]
fn hex_literals() {
    let asm = compile_success(r#"
        fn main() {
            x: u8 = 0xFF;
        }
    "#);

    assert_asm_contains(&asm, "$FF");
}

#[test]
fn binary_literals() {
    let asm = compile_success(r#"
        fn main() {
            x: u8 = 0b11110000;
        }
    "#);

    assert_asm_contains(&asm, "$F0");
}

#[test]
fn mutable_variable() {
    let asm = compile_success(r#"
        fn main() {
            x: u8 = 10;
            x = 20;
        }
    "#);

    assert_asm_contains(&asm, "LDA");
    assert_asm_contains(&asm, "STA");
}

#[test]
fn arithmetic_addition() {
    let asm = compile_success(r#"
        fn main() {
            x: u8 = 10 + 20;
        }
    "#);

    // Should fold to 30
    assert_asm_contains(&asm, "LDA");
}

#[test]
fn arithmetic_multiplication() {
    let asm = compile_success(r#"
        fn main() {
            x: u8 = 3 * 4;
        }
    "#);

    // Should fold to 12
    assert_asm_contains(&asm, "LDA");
}

#[test]
fn bitwise_and() {
    let asm = compile_success(r#"
        fn main() {
            result: u8 = 0xFF & 0x0F;
        }
    "#);

    // Should fold to 0x0F
    assert_asm_contains(&asm, "$0F");
}

#[test]
fn bitwise_or() {
    let asm = compile_success(r#"
        fn main() {
            result: u8 = 0xF0 | 0x0F;
        }
    "#);

    // Should fold to 0xFF
    assert_asm_contains(&asm, "$FF");
}

#[test]
fn shift_left() {
    let asm = compile_success(r#"
        fn main() {
            result: u8 = 1 << 4;
        }
    "#);

    // Should fold to 16 (0x10)
    assert_asm_contains(&asm, "$10");
}
