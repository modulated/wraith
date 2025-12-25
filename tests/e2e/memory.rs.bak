//! End-to-end tests for memory and addresses

use crate::common::*;

#[test]
fn address_declaration() {
    let asm = compile_success(r#"
        addr SCREEN = 0x0400;
        fn main() {
            SCREEN = 42;
        }
    "#);

    assert_asm_contains(&asm, "SCREEN = $0400");
    assert_asm_contains(&asm, "STA SCREEN");
}

#[test]
fn constant_address_expression() {
    let asm = compile_success(r#"
        addr BASE = 0x0400;
        addr OFFSET = BASE + 0x0010;
        fn main() {
            OFFSET = 42;
        }
    "#);

    assert_asm_contains(&asm, "OFFSET = $0410");
    assert_asm_contains(&asm, "STA OFFSET");
}
