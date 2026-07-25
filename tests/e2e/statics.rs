//! Runtime (emulator) tests for mutable globals (`static`), the foundation for
//! OS state: ring buffers, device tables and screen memory. Statics live in the
//! BSS (RAM) section and are initialized by the reset handler.

use crate::common::exec::run;

#[test]
fn static_scalar_read_write() {
    let mut e = run(r#"
        static COUNT: u8 = 7;
        const OUT: addr = 0x0900;
        #[reset]
        fn main() {
            COUNT = COUNT + 1;
            OUT = COUNT;
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0900), 8, "static initialized to 7 then incremented");
}

#[test]
fn static_initial_value_written_at_reset() {
    // The initializer must actually land in RAM (which is undefined at power-on).
    let mut e = run(r#"
        static MAGIC: u8 = 0xA5;
        const OUT: addr = 0x0900;
        #[reset]
        fn main() {
            OUT = MAGIC;
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0900), 0xA5, "declared initial value present at startup");
}

#[test]
fn static_persists_across_function_calls() {
    // Unlike a local (frame-allocated and call-graph colored), a static keeps its
    // value across calls and is visible to every function.
    let mut e = run(r#"
        static ACC: u8 = 0;
        const OUT: addr = 0x0900;
        fn bump() { ACC = ACC + 5; }
        #[reset]
        fn main() {
            bump();
            bump();
            bump();
            OUT = ACC;
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0900), 15, "static accumulated across three calls");
}

#[test]
fn two_statics_do_not_overlap() {
    let mut e = run(r#"
        static A: u8 = 1;
        static B: u8 = 2;
        const OUT_A: addr = 0x0900;
        const OUT_B: addr = 0x0901;
        #[reset]
        fn main() {
            A = 0x11;
            B = 0x22;
            OUT_A = A;
            OUT_B = B;
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0900), 0x11, "A unaffected by writing B");
    assert_eq!(e.mem(0x0901), 0x22, "B holds its own value");
}

#[test]
fn static_u16_holds_both_bytes() {
    let mut e = run(r#"
        static TICKS: u16 = 0x1234;
        const LO: addr = 0x0900;
        const HI: addr = 0x0901;
        #[reset]
        fn main() {
            LO = TICKS.low;
            HI = TICKS.high;
            loop {}
        }
    "#);
    assert_eq!(e.mem16(0x0900), 0x1234, "u16 static keeps both bytes");
}

#[test]
fn irq_handler_and_main_share_a_static() {
    // The core OS pattern: an interrupt handler updates shared state that the
    // main loop observes. Locals cannot do this (frames are colored per
    // function), so this is exactly what statics exist for.
    // The program settles into its idle loop; the harness then pulses IRQ. The
    // handler updates the static, which lives at a fixed RAM address the test
    // reads back directly (BSS starts at $0400).
    let mut e = run(r#"
        static IRQ_COUNT: u8 = 0;
        const READY: addr = 0x0900;
        #[irq]
        fn on_irq() {
            IRQ_COUNT = IRQ_COUNT + 1;
        }
        #[reset]
        fn main() {
            READY = 1;
            asm { "CLI" }
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0900), 1, "reached the idle loop");
    assert_eq!(e.mem(0x0400), 0, "shared counter starts at its initializer");

    e.pulse_irq();
    assert_eq!(e.mem(0x0400), 1, "handler incremented the shared static");

    e.pulse_irq();
    e.pulse_irq();
    assert_eq!(e.mem(0x0400), 3, "each interrupt updates shared state");
}

#[test]
fn static_array_indexed_read_write() {
    // A RAM-backed array is what a UART ring buffer / screen buffer needs.
    let mut e = run(r#"
        static BUF: [u8; 8] = [0; 8];
        const E0: addr = 0x0900;
        const E1: addr = 0x0901;
        #[reset]
        fn main() {
            let i: u8 = 3;
            BUF[i] = 0x5A;
            BUF[0 as u8] = 0x11;
            E0 = BUF[0 as u8];
            E1 = BUF[i];
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0900), 0x11, "BUF[0] written and read back");
    assert_eq!(e.mem(0x0901), 0x5A, "BUF[i] with a runtime index");
}
