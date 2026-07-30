//! End-to-end tests for interrupt handlers

use crate::common::*;

#[test]
fn nmi_handler() {
    let asm = compile_success(
        r#"
        const OUT: addr = 0x400;
        #[nmi]
        fn nmi_handler() {
            OUT = 0xFF;
        }
        fn main() {}
    "#,
    );

    // Should have RTI for interrupt handler
    assert_asm_contains(&asm, "RTI");
    // Should have prologue (save registers)
    assert_asm_contains(&asm, "PHA");
    assert_asm_contains(&asm, "TXA");
    assert_asm_contains(&asm, "TYA");
    // Should have epilogue (restore registers)
    assert_asm_contains(&asm, "TAY");
    assert_asm_contains(&asm, "TAX");
    // Should have vector table
    assert_asm_contains(&asm, ".ORG $FFFA");
    assert_asm_contains(&asm, ".WORD nmi_handler");
}

#[test]
fn irq_handler() {
    let asm = compile_success(
        r#"
        const OUT: addr = 0x400;
        #[irq]
        fn irq_handler() {
            OUT = 0x42;
        }
        fn main() {}
    "#,
    );

    // Should use RTI
    assert_asm_contains(&asm, "RTI");
    // Should have IRQ vector
    assert_asm_contains(&asm, ".WORD irq_handler");
}

#[test]
fn reset_handler() {
    let asm = compile_success(
        r#"
        #[reset]
        fn start() {
        }
        fn main() {}
    "#,
    );

    // Should have RESET vector
    assert_asm_contains(&asm, ".WORD start");
}

#[test]
fn all_interrupt_vectors() {
    let asm = compile_success(
        r#"
        const OUT: addr = 0x400;
        #[nmi]
        fn nmi_handler() {
            OUT = 1;
        }
        #[reset]
        fn reset_handler() {
            OUT = 2;
        }
        #[irq]
        fn irq_handler() {
            OUT = 3;
        }
        fn main() {}
    "#,
    );

    // Verify vector table exists at correct location
    assert_asm_contains(&asm, ".ORG $FFFA");
    // Verify all three vectors are present
    assert_asm_contains(&asm, ".WORD nmi_handler");
    assert_asm_contains(&asm, ".WORD reset_handler");
    assert_asm_contains(&asm, ".WORD irq_handler");
}

#[test]
fn interrupt_handler_preserves_the_indirect_arg_staging_block() {
    // An NMI landing between indirect-arg staging ($E0-$EF) and the callee's
    // prologue copy used to destroy the in-flight args when the handler
    // itself called indirectly: the save list skipped the block.
    let asm = compile_success(
        r#"
        const OUT: addr = 0x400;
        fn run_op(f: fn(u8) -> u8, v: u8) -> u8 { return f(v); }
        fn id(x: u8) -> u8 { return x; }
        #[nmi]
        fn nmi_handler() {
            OUT = run_op(id, 1);
        }
        fn main() {
            OUT = run_op(id, 2);
        }
    "#,
    );
    assert_asm_contains(&asm, "LDA $E0");
    assert_asm_contains(&asm, "LDA $EF");
}

#[test]
fn the_generic_interrupt_attribute_is_rejected() {
    // #[interrupt] got a full handler prologue and RTI but was never
    // installed in any vector — a handler that silently never runs.
    let err = match crate::common::harness::compile(
        r#"
        #[interrupt]
        fn handler() {}
        fn main() {}
    "#,
    ) {
        crate::common::harness::CompileResult::SemaError(e) => e,
        other => panic!("expected a sema error, got {other:?}"),
    };
    assert!(
        err.contains("#[irq]"),
        "should point at the real attributes: {err}"
    );
}

#[test]
fn a_trivial_handler_saves_no_zero_page() {
    // The prologue used to save 60 zero-page bytes unconditionally (~780
    // cycles per interrupt). A handler that touches no scratch saves none.
    let asm = compile_success(
        r#"
        const OUT: addr = 0x400;
        #[irq]
        fn on_irq() {
            OUT = 1;
        }
        #[reset]
        fn main() { loop {} }
    "#,
    );
    let handler = asm.split("on_irq:").nth(1).expect("handler emitted");
    let body = handler.split("RTI").next().unwrap();
    assert!(
        !body.contains("Save zero-page"),
        "trivial handler must not save zero-page:\n{body}"
    );
}

#[test]
fn a_math_handler_saves_the_math_scratch() {
    // 16-bit multiply runs through mul16's $D0-$DC working storage, which
    // main code may be using when the interrupt lands.
    let asm = compile_success(
        r#"
        const OUT: addr = 0x400;
        #[irq]
        fn on_irq() {
            let x: u16 = 300;
            let y: u16 = x * 3;
            OUT = y.low;
        }
        #[reset]
        fn main() { loop {} }
    "#,
    );
    let handler = asm.split("on_irq:").nth(1).expect("handler emitted");
    let body = handler.split("RTI").next().unwrap();
    assert!(body.contains("LDA $D0"), "math scratch saved:\n{body}");
    assert!(body.contains("LDA $20"), "binary-op scratch saved:\n{body}");
}
