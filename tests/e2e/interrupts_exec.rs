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
    assert_eq!(
        e.mem(0x0401),
        88,
        "handler computed 1000 * 7 = 7000 (low byte)"
    );
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
    assert!(
        e.irq_stays_masked(),
        "IRQ must not fire while the I-flag is set"
    );
    assert_eq!(e.mem(0x0400), 0, "handler never ran");
}

// ---------------------------------------------------------------------------
// An interrupt arriving while `main` is still working.
// ---------------------------------------------------------------------------

/// A handler must leave `main`'s *live* state alone, not just its stored state.
///
/// Frame colouring cannot separate a handler from `main`: a handler is not
/// `main`'s callee, so their zero-page frames may share addresses. Correctness
/// rests entirely on the handler saving that span on entry and restoring it on
/// exit — and until now nothing tested it. Every interrupt test above fires
/// through `pulse_irq`, which waits for the idle loop first, where `main` has
/// stored everything it computed and there is nothing live left to corrupt.
///
/// `run_interrupted` asserts the line every *n* instructions while `main` is
/// mid-computation, so the handler is entered from wherever it happens to be:
/// between the halves of a 16-bit add, inside a multiply, across a `JSR`, with
/// the argument pool staged. Several frequencies, because a fixed one lands in
/// the same few places every time.
///
/// The result is a negative one — the saves hold — which is worth pinning
/// precisely because so much rests on them.
#[test]
fn a_handler_does_not_disturb_a_computation_it_interrupts() {
    // Sum over i in 0..20 of 3 * (i + 1000) = 3 * (20_000 + 190) = 60_570.
    const WANT: u16 = 60_570;

    // Both halves varied: whether `main` crosses a call while interrupted, and
    // whether the handler itself does 16-bit work (which uses the shared math
    // temps) or only touches a static.
    let bodies = [
        "let t: u16 = (i as u16) + 1000; acc = acc + scale(t);",
        "let t: u16 = (i as u16) + 1000; acc = acc + t + t + t;",
    ];
    let handlers = [
        "let x: u16 = 700; let y: u16 = 9; let p: u16 = x * y; COUNT = COUNT + p.low;",
        "COUNT = COUNT + 1;",
    ];

    for body in bodies {
        for handler in handlers {
            // Frequencies chosen to interleave differently without starving
            // the program: at every 3 instructions the handler runs more often
            // than `main` progresses and the run never reaches its idle loop.
            for every in [5usize, 7, 11, 17, 29] {
                let mut e = crate::common::exec::run_interrupted(
                    &format!(
                        "const OUT0: addr = 0x0900;\n\
                         const OUT1: addr = 0x0901;\n\
                         static COUNT: u8 = 0;\n\
                         fn scale(v: u16) -> u16 {{ return v * 3; }}\n\
                         #[reset]\n\
                         fn main() {{\n\
                         \x20   asm {{ \"CLI\" }}\n\
                         \x20   let acc: u16 = 0;\n\
                         \x20   for i in 0..20 {{ {body} }}\n\
                         \x20   OUT0 = acc.low; OUT1 = acc.high;\n\
                         \x20   loop {{}}\n\
                         }}\n\
                         #[irq]\n\
                         fn on_irq() {{ {handler} }}\n"
                    ),
                    every,
                );
                let got = ((e.mem(0x0901) as u16) << 8) | e.mem(0x0900) as u16;
                assert_eq!(
                    got, WANT,
                    "interrupted every {every} instructions, with body `{body}` \
                     and handler `{handler}`"
                );
            }
        }
    }
}
