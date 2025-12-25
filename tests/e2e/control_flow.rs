//! End-to-end tests for control flow constructs

use crate::common::*;

#[test]
fn if_statement() {
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
fn if_else() {
    let asm = compile_success(r#"
        fn main() {
            if false {
                x: u8 = 10;
            } else {
                x: u8 = 20;
            }
        }
    "#);

    assert_asm_contains(&asm, "BEQ");
    assert_asm_contains(&asm, "JMP");
}

#[test]
fn while_loop() {
    let asm = compile_success(r#"
        fn main() {
            x: u8 = 0;
            while x < 10 {
                x = x + 1;
            }
        }
    "#);

    assert_asm_contains(&asm, "CMP");
    assert_asm_contains(&asm, "JMP");
}

#[test]
fn for_range_loop() {
    let asm = compile_success(r#"
        fn main() {
            for i: u8 in 0..10 {
                x: u8 = i;
            }
        }
    "#);

    assert_asm_contains(&asm, "INX");
    assert_asm_contains(&asm, "CPX");
}
