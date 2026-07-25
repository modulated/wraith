//! Runtime (emulator) tests for interrupt handlers. The rest of the suite only
//! checks the generated assembly (vectors present, RTI emitted); these actually
//! pulse the IRQ/NMI lines and verify the handler runs, its results land, the
//! I-flag masks IRQs, and the prologue/epilogue preserve A/X/Y across a call.
//!
//! The program runs to its idle `loop {}`; the harness then asserts an interrupt
//! line, runs the handler, and returns control to the idle loop.

use crate::common::exec::run;

// ---------------------------------------------------------------------------
// IRQ handler runs and its side effect lands.
// ---------------------------------------------------------------------------

#[test]
fn irq_handler_increments_counter() {
    let mut e = run(r#"
        const CTR: addr = 0x0400;
        #[reset]
        fn main() {
            CTR = 0;
            asm { "CLI" }
            loop {}
        }
        #[irq]
        fn on_irq() {
            CTR = CTR + 1;
        }
    "#);
    assert_eq!(e.mem(0x0400), 0, "counter starts at 0 before any interrupt");
    e.pulse_irq();
    assert_eq!(e.mem(0x0400), 1, "one IRQ -> counter 1");
    e.pulse_irq();
    e.pulse_irq();
    assert_eq!(e.mem(0x0400), 3, "three IRQs -> counter 3");
}

// ---------------------------------------------------------------------------
// The IRQ prologue/epilogue must preserve A/X/Y across the handler, even though
// the handler clobbers them.
// ---------------------------------------------------------------------------

#[test]
fn irq_preserves_registers() {
    let mut e = run(r#"
        const CTR: addr = 0x0400;
        #[reset]
        fn main() {
            CTR = 0;
            asm {
                "LDA #$AA",
                "LDX #$BB",
                "LDY #$CC",
                "CLI",
            }
            loop {}
        }
        #[irq]
        fn on_irq() {
            // Clobber A (and generally the working registers) inside the handler.
            let s: u8 = 0;
            for i in 0..4 {
                s = s + i;
            }
            CTR = s;
        }
    "#);
    // Registers seeded by main just before the idle loop.
    let (a0, x0, y0) = (e.a(), e.x(), e.y());
    assert_eq!(a0, 0xAA);
    assert_eq!(x0, 0xBB);
    assert_eq!(y0, 0xCC);

    e.pulse_irq();

    assert_eq!(e.mem(0x0400), 6, "handler ran (0+1+2+3 = 6)");
    assert_eq!(e.a(), 0xAA, "A restored by the epilogue");
    assert_eq!(e.x(), 0xBB, "X restored by the epilogue");
    assert_eq!(e.y(), 0xCC, "Y restored by the epilogue");
}

// ---------------------------------------------------------------------------
// A handler must not corrupt main's zero-page state.
// ---------------------------------------------------------------------------

#[test]
fn irq_does_not_corrupt_main_state() {
    let mut e = run(r#"
        const MAIN_VAL: addr = 0x0400;
        const IRQ_VAL: addr = 0x0401;
        #[reset]
        fn main() {
            // A local computation whose result is observable; the handler does
            // its own arithmetic (using the shared zero-page temps) in between.
            let a: u16 = 300;
            let b: u16 = 45;
            let sum: u16 = a + b;
            MAIN_VAL = sum.low;
            asm { "CLI" }
            loop {}
        }
        #[irq]
        fn on_irq() {
            let x: u16 = 1000;
            let y: u16 = 7;
            let p: u16 = x * y;
            IRQ_VAL = p.low;
        }
    "#);
    // 345 = 0x0159 (low 0x59 = 89); 7000 = 0x1B58 (low 0x58 = 88): distinct.
    assert_eq!(e.mem(0x0400), 89, "main's result before the interrupt");
    e.pulse_irq();
    assert_eq!(e.mem(0x0401), 88, "handler computed 1000 * 7 = 7000 (low byte)");
    assert_eq!(e.mem(0x0400), 89, "main's result untouched by the handler");
}

// ---------------------------------------------------------------------------
// NMI is non-maskable and edge-triggered.
// ---------------------------------------------------------------------------

#[test]
fn nmi_handler_fires_per_edge() {
    let mut e = run(r#"
        const CTR: addr = 0x0400;
        #[reset]
        fn main() {
            CTR = 0;
            loop {}
        }
        #[nmi]
        fn on_nmi() {
            CTR = CTR + 1;
        }
    "#);
    // No CLI needed: NMI ignores the I-flag.
    e.pulse_nmi();
    assert_eq!(e.mem(0x0400), 1, "one NMI edge -> counter 1");
    e.pulse_nmi();
    assert_eq!(e.mem(0x0400), 2, "each edge services once");
}

// ---------------------------------------------------------------------------
// The I-flag masks IRQs (reset leaves interrupts disabled).
// ---------------------------------------------------------------------------

#[test]
fn irq_masked_when_disabled() {
    let mut e = run(r#"
        const CTR: addr = 0x0400;
        #[reset]
        fn main() {
            CTR = 0;
            // No CLI: interrupts stay disabled from reset.
            loop {}
        }
        #[irq]
        fn on_irq() {
            CTR = CTR + 1;
        }
    "#);
    assert!(e.irq_stays_masked(), "IRQ must not fire while the I-flag is set");
    assert_eq!(e.mem(0x0400), 0, "handler never ran");
}
