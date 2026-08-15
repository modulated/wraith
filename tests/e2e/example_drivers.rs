//! `examples/device_drivers.wr`, run against the modelled UART and VIA.
//!
//! The example is an OS-shaped driver system: a vtable of function pointers
//! that the kernel calls through, a registration step that swaps which driver
//! is behind it, and a table indexed by device number. Compiling it proves
//! nothing about whether the dispatch reaches the right driver — an example
//! that silently talks to the wrong device is worse than no example — so it is
//! executed here on emulated hardware and its output checked byte for byte.
//!
//! Keeping it in `examples/` rather than inline in this file means the same
//! text is what a reader sees and what the test runs, and `tests/code_size.rs`
//! measures it alongside every other example.

use crate::common::devices::Devices;
use crate::common::exec::run_with_devices;

const UART_BASE: u16 = 0x7F00;
const VIA_BASE: u16 = 0x7F10;

/// What the VIA's input port is holding when the example samples it.
const PORT_A: u8 = 0x5A;

fn source() -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/device_drivers.wr");
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{} unreadable: {e}", path.display()))
}

/// Run the example with `typed` already waiting in the UART's receive FIFO.
fn run_example(typed: &[u8]) -> crate::common::exec::Exec {
    let mut devices = Devices::default().with_uart(UART_BASE).with_via(VIA_BASE);
    devices.uart.as_mut().unwrap().feed(typed);
    devices.via.as_mut().unwrap().port_a_in = PORT_A;
    run_with_devices(&source(), devices)
}

#[test]
fn the_driver_example_talks_to_both_devices() {
    let mut e = run_example(b"hi");

    // Everything the kernel printed went out through the vtable: the banner
    // before anything was read, the two echoed bytes, then the sensor report
    // and a newline sent by device number instead of through the console.
    assert_eq!(
        e.uart_output(),
        "wraith> hi\nport a = 5A\n",
        "the console vtable reached the UART for every write"
    );

    assert_eq!(e.mem(0x0900), 2, "echo_pending drained both queued bytes");
    assert_eq!(
        e.mem(0x0901),
        PORT_A,
        "get() read the VIA's input port, not the UART, once the driver was swapped"
    );
    assert_eq!(e.mem(0x0902), 1, "the console ended on the UART driver");
}

#[test]
fn the_uart_driver_programmed_the_baud_rate() {
    // `uart_init` runs because `register_console` calls it through the vtable
    // it has just installed — the one call in the example that dispatches to a
    // driver entry point with no arguments and no result.
    let mut e = run_example(b"");
    let uart = e.devices().uart.as_ref().unwrap();
    assert_eq!(uart.divisor, 12, "9600 baud at 1.8432 MHz");
}

#[test]
fn the_via_driver_set_its_data_direction() {
    // Port B drives, port A listens. If `via_init` had not run, both would
    // still read back as zero and the sample above would be meaningless.
    let mut e = run_example(b"");
    let via = e.devices().via.as_ref().unwrap();
    assert_eq!((via.ddrb, via.ddra), (0xFF, 0x00));
}

#[test]
fn nothing_is_echoed_when_nothing_was_typed() {
    // The read path has to cope with an idle device: `echo_pending` returns
    // without blocking, and the banner is still the only thing transmitted
    // before the sensor report.
    let mut e = run_example(b"");
    assert_eq!(e.mem(0x0900), 0, "no bytes waiting, none echoed");
    assert_eq!(e.uart_output(), "wraith> \nport a = 5A\n");
}
