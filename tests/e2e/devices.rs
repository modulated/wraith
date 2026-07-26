//! Driver-level tests against modelled hardware (TL16C550 UART, 6522 VIA).
//!
//! These exercise real device semantics that plain RAM cannot express: reading
//! the UART receive register consumes a byte and clears the data-ready flag,
//! transmitted bytes are captured, and the VIA's timer counts down and raises
//! an interrupt. That makes it possible to test drivers, not just inspect the
//! assembly they compile to.

use crate::common::devices::{Devices, VIA_IRQ_T1, via_reg};
use crate::common::exec::run_with_devices;

const UART_BASE: u16 = 0x7F00;
const VIA_BASE: u16 = 0x7F10;

// ---------------------------------------------------------------------------
// UART: polled transmit and receive.
// ---------------------------------------------------------------------------

#[test]
fn uart_polled_transmit() {
    // Wait for the transmitter to be ready, then send each byte — the standard
    // polled TX loop.
    let mut e = run_with_devices(
        r#"
        const UART_THR: write addr = 0x7F00;
        const UART_LSR: read  addr = 0x7F05;
        const THR_EMPTY: u8 = 0x20;

        fn putc(c: u8) {
            while (UART_LSR & THR_EMPTY) == 0 {}
            UART_THR = c;
        }

        #[reset]
        fn main() {
            putc(0x48);   // 'H'
            putc(0x49);   // 'I'
            loop {}
        }
    "#,
        Devices::default().with_uart(UART_BASE),
    );
    assert_eq!(e.uart_output(), "HI", "driver transmitted both bytes");
}

#[test]
fn uart_polled_receive_consumes_fifo() {
    // Reading RBR must consume a byte and clear data-ready once drained; if the
    // model were plain RAM this loop would never terminate or would re-read the
    // same byte forever.
    let mut devices = Devices::default().with_uart(UART_BASE);
    devices.uart.as_mut().unwrap().feed(b"AB");

    let mut e = run_with_devices(
        r#"
        const UART_RBR: read  addr = 0x7F00;
        const UART_THR: write addr = 0x7F00;
        const UART_LSR: read  addr = 0x7F05;
        const DATA_READY: u8 = 0x01;
        static COUNT: u8 = 0;

        #[reset]
        fn main() {
            // Echo every byte that arrives, until the FIFO drains.
            while (UART_LSR & DATA_READY) != 0 {
                let c: u8 = UART_RBR;
                COUNT = COUNT + 1;
                UART_THR = c;
            }
            loop {}
        }
    "#,
        devices,
    );
    assert_eq!(e.uart_output(), "AB", "both received bytes echoed back");
    assert_eq!(e.mem(0x0400), 2, "loop ran exactly twice (FIFO drained)");
}

#[test]
fn uart_baud_divisor_written_through_dlab() {
    // Setting DLAB switches the first two registers to the divisor latch; the
    // model records it, so a driver's baud setup can be asserted.
    let mut e = run_with_devices(
        r#"
        const UART_DLL: addr = 0x7F00;
        const UART_DLM: addr = 0x7F01;
        const UART_LCR: addr = 0x7F03;
        const LCR_DLAB: u8 = 0x80;
        const LCR_8N1:  u8 = 0x03;

        #[reset]
        fn main() {
            UART_LCR = LCR_DLAB;      // divisor latch visible
            UART_DLL = 12;            // 9600 baud @ 1.8432 MHz
            UART_DLM = 0;
            UART_LCR = LCR_8N1;       // back to data registers, 8N1
            loop {}
        }
    "#,
        Devices::default().with_uart(UART_BASE),
    );
    let uart = e.devices().uart.as_ref().unwrap();
    assert_eq!(uart.divisor, 12, "driver programmed the baud divisor");
}

// ---------------------------------------------------------------------------
// UART: interrupt-driven receive into a ring buffer.
// ---------------------------------------------------------------------------

#[test]
fn uart_irq_driven_receive_into_ring_buffer() {
    // The device asserts IRQ while data is pending and RX interrupts are
    // enabled. The handler drains it into a static ring buffer that main reads —
    // the shape of a real serial driver.
    let mut devices = Devices::default().with_uart(UART_BASE);
    devices.uart.as_mut().unwrap().feed(b"Hi!");

    let mut e = run_with_devices(
        r#"
        const UART_RBR: read  addr = 0x7F00;
        const UART_IER: addr = 0x7F01;
        const UART_LSR: read  addr = 0x7F05;
        const DATA_READY: u8 = 0x01;
        const IER_RX: u8 = 0x01;

        static RX_BUF: [u8; 8] = [0; 8];
        static RX_HEAD: u8 = 0;

        #[irq]
        fn on_irq() {
            while (UART_LSR & DATA_READY) != 0 {
                let c: u8 = UART_RBR;
                RX_BUF[RX_HEAD] = c;
                RX_HEAD = RX_HEAD + 1;
            }
        }

        #[reset]
        fn main() {
            UART_IER = IER_RX;    // enable receive interrupts
            asm { "CLI" }
            loop {}
        }
    "#,
        devices,
    );

    // RX_BUF is the first static, so it sits at the base of BSS ($0400);
    // RX_HEAD follows it.
    assert_eq!(e.mem(0x0408), 3, "handler received three bytes");
    assert_eq!(e.mem(0x0400), b'H');
    assert_eq!(e.mem(0x0401), b'i');
    assert_eq!(e.mem(0x0402), b'!');
}

// ---------------------------------------------------------------------------
// VIA: ports and timer interrupt.
// ---------------------------------------------------------------------------

#[test]
fn via_port_output_and_input() {
    // Program drives port B and reads a value the "hardware" presents on port A
    // (a keyboard scancode, say).
    let mut devices = Devices::default().with_via(VIA_BASE);
    devices.via.as_mut().unwrap().port_a_in = 0x5A;

    let mut e = run_with_devices(
        r#"
        const VIA_PORTB: addr = 0x7F10;
        const VIA_PORTA: addr = 0x7F11;
        const VIA_DDRB:  addr = 0x7F12;
        const VIA_DDRA:  addr = 0x7F13;
        const OUT: addr = 0x0900;

        #[reset]
        fn main() {
            VIA_DDRB = 0xFF;      // port B all outputs
            VIA_DDRA = 0x00;      // port A all inputs
            VIA_PORTB = 0xC3;
            OUT = VIA_PORTA;      // read the scancode
            loop {}
        }
    "#,
        devices,
    );
    assert_eq!(e.mem(0x0900), 0x5A, "read the value presented on port A");
    let via = e.devices().via.as_ref().unwrap();
    assert_eq!(via.port_b_out, 0xC3, "driver drove port B");
}

#[test]
fn via_timer1_raises_interrupt_flag() {
    // Timer 1 counts down with CPU cycles; at underflow it sets its interrupt
    // flag. The test advances it directly, then checks the flag is visible.
    let mut e = run_with_devices(
        r#"
        const VIA_T1CL: addr = 0x7F14;
        const VIA_T1CH: addr = 0x7F15;
        const VIA_IER:  addr = 0x7F1E;
        const READY: addr = 0x0900;

        #[reset]
        fn main() {
            VIA_T1CL = 0x00;
            VIA_T1CH = 0x01;      // load and start (0x0100 cycles)
            VIA_IER = 0xC0;       // enable Timer 1 interrupts
            READY = 1;
            loop {}
        }
    "#,
        Devices::default().with_via(VIA_BASE),
    );
    assert_eq!(e.mem(0x0900), 1, "program reached its idle loop");

    let via = e.devices().via.as_mut().unwrap();
    assert!(!via.irq_asserted(), "timer has not underflowed yet");
    via.tick(0x0200); // run past the reload value
    assert!(via.irq_asserted(), "Timer 1 underflow asserts IRQ");

    // Reading the low counter clears the flag, as the real device does.
    let via = e.devices().via.as_mut().unwrap();
    via.tick(0);
    assert_ne!(e.mem(VIA_BASE + via_reg::T1CL), 0xFF, "counter readable");
    let via = e.devices().via.as_ref().unwrap();
    assert!(
        !via.irq_asserted(),
        "reading T1CL acknowledges the timer interrupt"
    );
}

#[test]
fn via_timer_interrupt_reaches_handler() {
    // End to end: the VIA asserts IRQ, the CPU vectors to the handler, and the
    // handler acknowledges it and bumps a shared counter.
    let mut e = run_with_devices(
        r#"
        const VIA_T1CL: addr = 0x7F14;
        const VIA_T1CH: addr = 0x7F15;
        const VIA_IER:  addr = 0x7F1E;
        const READY: addr = 0x0900;
        static TICKS: u8 = 0;

        #[irq]
        fn on_irq() {
            let _ack: u8 = VIA_T1CL;   // reading T1CL clears the timer flag
            TICKS = TICKS + 1;
        }

        #[reset]
        fn main() {
            VIA_T1CL = 0x00;
            VIA_T1CH = 0x01;
            VIA_IER = 0xC0;
            READY = 1;
            asm { "CLI" }
            loop {}
        }
    "#,
        Devices::default().with_via(VIA_BASE),
    );
    assert_eq!(e.mem(0x0900), 1, "reached the idle loop");
    assert_eq!(e.mem(0x0400), 0, "no timer interrupt yet");

    // Let the timer underflow, then run: the CPU should take the interrupt.
    e.devices().via.as_mut().unwrap().tick(0x0200);
    assert_eq!(
        e.devices().via.as_ref().unwrap().ifr & VIA_IRQ_T1,
        VIA_IRQ_T1,
        "timer flag set"
    );
    e.resume(2000);
    assert_eq!(e.mem(0x0400), 1, "handler ran and counted the tick");
}
