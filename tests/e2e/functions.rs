//! End-to-end tests for functions

use crate::common::*;

#[test]
fn function_call_no_args() {
    let asm = compile_success(r#"
        fn foo() {
        }
        fn main() {
            foo();
        }
    "#);

    assert_asm_contains(&asm, "JSR foo");
    assert_asm_contains(&asm, "foo:");
}

#[test]
fn function_call_with_args() {
    let asm = compile_success(r#"
        fn add(a: u8, b: u8) -> u8 {
            return a + b;
        }
        fn main() {
            result: u8 = add(5, 10);
        }
    "#);

    assert_asm_contains(&asm, "JSR add");
    assert_asm_contains(&asm, "add:");
}

#[test]
fn function_return_value() {
    let asm = compile_success(r#"
        fn get_value() -> u8 {
            return 42;
        }
        fn main() {
            x: u8 = get_value();
        }
    "#);

    assert_asm_contains(&asm, "JSR get_value");
    assert_asm_contains(&asm, "RTS");
}
