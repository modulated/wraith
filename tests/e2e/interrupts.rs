//! End-to-end tests for interrupt handlers

use crate::common::*;

#[test]
fn nmi_handler() {
    let asm = compile_success(r#"
        const OUT: addr = 0x400;
        #[nmi]
        fn nmi_handler() {
            OUT = 0xFF;
        }
        fn main() {}
    "#);

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
    let asm = compile_success(r#"
        const OUT: addr = 0x400;
        #[irq]
        fn irq_handler() {
            OUT = 0x42;
        }
        fn main() {}
    "#);

    // Should use RTI
    assert_asm_contains(&asm, "RTI");
    // Should have IRQ vector
    assert_asm_contains(&asm, ".WORD irq_handler");
}

#[test]
fn reset_handler() {
    let asm = compile_success(r#"
        #[reset]
        fn start() {
        }
        fn main() {}
    "#);

    // Should have RESET vector
    assert_asm_contains(&asm, ".WORD start");
}

#[test]
fn all_interrupt_vectors() {
    let asm = compile_success(r#"
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
    "#);

    // Verify vector table exists at correct location
    assert_asm_contains(&asm, ".ORG $FFFA");
    // Verify all three vectors are present
    assert_asm_contains(&asm, ".WORD nmi_handler");
    assert_asm_contains(&asm, ".WORD reset_handler");
    assert_asm_contains(&asm, ".WORD irq_handler");
}
