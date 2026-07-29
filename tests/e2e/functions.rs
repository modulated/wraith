//! End-to-end tests for functions

use crate::common::exec::run;
use crate::common::*;

#[test]
fn function_call_no_args() {
    let asm = compile_success(
        r#"
        fn foo() {
        }
        fn main() {
            foo();
        }
    "#,
    );

    // Tail call optimization may convert JSR+RTS to JMP
    assert!(
        asm.contains("JSR foo") || asm.contains("JMP foo"),
        "Expected JSR or JMP foo (tail-call optimized)"
    );
    assert_asm_contains(&asm, "foo:");
}

#[test]
fn function_call_with_args() {
    let asm = compile_success(
        r#"
        fn add(a: u8, b: u8) -> u8 {
            return a + b;
        }
        fn main() {
            let result: u8 = add(5, 10);
        }
    "#,
    );

    assert_asm_contains(&asm, "JSR add");
    assert_asm_contains(&asm, "add:");
}

#[test]
fn function_return_value() {
    let asm = compile_success(
        r#"
        fn get_value() -> u8 {
            return 42;
        }
        fn main() {
            let x: u8 = get_value();
        }
    "#,
    );

    assert_asm_contains(&asm, "JSR get_value");
    assert_asm_contains(&asm, "RTS");
}

#[test]
fn a_tail_recursive_function_can_be_address_taken() {
    // The tail-call loop label used to sit *before* the function-pointer
    // prologue, so each "iteration" re-copied the $E0 staging block over the
    // freshly updated parameter: the loop reloaded the original argument
    // forever. Called through a pointer, count(5) must actually reach 0.
    let mut e = run(r#"
        const OUT: addr = 0x0900;
        fn count(n: u8) -> u8 {
            if n == 0 { return 0; }
            return count(n - 1);
        }
        #[reset]
        fn main() {
            let f: fn(u8) -> u8 = count;
            OUT = f(5);
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0900), 0);
}
