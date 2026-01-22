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

    // Tail call optimization may convert JSR+RTS to JMP
    assert!(
        asm.contains("JSR foo") || asm.contains("JMP foo"),
        "Expected JSR or JMP foo (tail-call optimized)"
    );
    assert_asm_contains(&asm, "foo:");
}

#[test]
fn function_call_with_args() {
    let asm = compile_success(r#"
        fn add(a: u8, b: u8) -> u8 {
            return a + b;
        }
        fn main() {
            let result: u8 = add(5, 10);
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
            let x: u8 = get_value();
        }
    "#);

    assert_asm_contains(&asm, "JSR get_value");
    assert_asm_contains(&asm, "RTS");
}
