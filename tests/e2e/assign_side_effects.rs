//! An assignment must evaluate its right-hand side exactly once.
//!
//! `generate_assignment` evaluated the value up front, into A, and then
//! dispatched on the target. That works for `x = v`, where the store is one
//! instruction. Every other target — an array element, a struct field, a
//! `.low`/`.high`/`.len` accessor, a slice — needs its destination address
//! staged *around* the evaluation, so those handlers take the value expression
//! and generate it themselves. Both ran: the value was emitted twice.
//!
//! For arithmetic that is only wasted bytes, which is why it survived so long
//! — removing it shrank `examples/slice_test.wr` by 611 bytes. For anything
//! with a side effect it is a second execution:
//!
//!   * `arr[i] = f()` called `f` twice.
//!   * `RX_BUF[head] = UART_RBR` read the receive register twice, and reading
//!     it consumes a byte. An interrupt-driven serial driver dropped every
//!     other character — found writing `examples/device_drivers.wr`.
//!
//! The device case is the one that matters: it is silent, it only appears
//! against real hardware, and no amount of staring at the source shows it.

use crate::common::exec::run;

/// A counter bumped by a call, so a second evaluation is visible in the count
/// as well as in the stored value.
const COUNTER: &str = "\
static CALLS: u8 = 0;
fn bump() -> u8 { CALLS = CALLS + 1; return CALLS; }
";

#[test]
fn an_indexed_assignment_evaluates_its_value_once() {
    let mut e = run(&format!(
        "{COUNTER}const OUT0: addr = 0x0900;\n\
         const OUT1: addr = 0x0901;\n\
         static ARR: [u8; 4] = [0; 4];\n\
         #[reset]\nfn main() {{\n\
         \x20   let i: u8 = 1;\n\
         \x20   ARR[i] = bump();\n\
         \x20   OUT0 = CALLS;\n    OUT1 = ARR[1];\n    loop {{}}\n}}\n"
    ));
    assert_eq!((e.mem(0x0900), e.mem(0x0901)), (1, 1));
}

#[test]
fn a_constant_indexed_assignment_evaluates_its_value_once() {
    let mut e = run(&format!(
        "{COUNTER}const OUT0: addr = 0x0900;\n\
         static ARR: [u8; 4] = [0; 4];\n\
         #[reset]\nfn main() {{ ARR[2] = bump(); OUT0 = CALLS; loop {{}} }}\n"
    ));
    assert_eq!(e.mem(0x0900), 1);
}

#[test]
fn a_field_assignment_evaluates_its_value_once() {
    let mut e = run(&format!(
        "{COUNTER}const OUT0: addr = 0x0900;\n\
         struct P {{ x: u8, y: u8 }}\n\
         static S: P = P {{ x: 0, y: 0 }};\n\
         #[reset]\nfn main() {{ S.x = bump(); OUT0 = CALLS; loop {{}} }}\n"
    ));
    assert_eq!(e.mem(0x0900), 1);
}

#[test]
fn a_local_struct_field_assignment_evaluates_its_value_once() {
    let mut e = run(&format!(
        "{COUNTER}const OUT0: addr = 0x0900;\n\
         struct P {{ x: u8, y: u8 }}\n\
         #[reset]\nfn main() {{\n\
         \x20   let p: P = P {{ x: 0, y: 0 }};\n\
         \x20   p.y = bump();\n\
         \x20   OUT0 = CALLS;\n    loop {{}}\n}}\n"
    ));
    assert_eq!(e.mem(0x0900), 1);
}

#[test]
fn an_accessor_named_field_assignment_evaluates_its_value_once() {
    // A struct field actually *named* `high` parses as the `.high` accessor
    // and is re-resolved by sema as a field access, so it reached the target
    // arm through `accessor_fields` — one of the arms that double-evaluated.
    // (`.low`/`.high` on a plain `u16` is not an assignable target at all,
    // before this change or after it.)
    let mut e = run(&format!(
        "{COUNTER}const OUT0: addr = 0x0900;\n\
         const OUT1: addr = 0x0901;\n\
         struct Halves {{ low: u8, high: u8 }}\n\
         #[reset]\nfn main() {{\n\
         \x20   let h: Halves = Halves {{ low: 0, high: 0 }};\n\
         \x20   h.high = bump();\n\
         \x20   OUT0 = CALLS;\n    OUT1 = h.high;\n    loop {{}}\n}}\n"
    ));
    assert_eq!((e.mem(0x0900), e.mem(0x0901)), (1, 1));
}

#[test]
fn an_array_of_struct_field_assignment_evaluates_its_value_once() {
    let mut e = run(&format!(
        "{COUNTER}const OUT0: addr = 0x0900;\n\
         struct P {{ x: u8, y: u8 }}\n\
         static PS: [P; 2] = [P {{ x: 0, y: 0 }}, P {{ x: 0, y: 0 }}];\n\
         #[reset]\nfn main() {{\n\
         \x20   let i: u8 = 1;\n\
         \x20   PS[i].x = bump();\n\
         \x20   OUT0 = CALLS;\n    loop {{}}\n}}\n"
    ));
    assert_eq!(e.mem(0x0900), 1);
}

#[test]
fn a_destructive_register_read_happens_once() {
    // The shape that found it. The UART's receive register hands over a byte
    // and drops it from the FIFO, so a second read is a lost character rather
    // than a repeated value. Two bytes are fed and both must land in the
    // buffer, in order.
    use crate::common::devices::Devices;
    use crate::common::exec::run_with_devices;

    let mut devices = Devices::default().with_uart(0x7F00);
    devices.uart.as_mut().unwrap().feed(b"hi");

    let mut e = run_with_devices(
        r#"
        const UART_RBR: read addr = 0x7F00;
        const UART_LSR: read addr = 0x7F05;
        const DATA_READY: u8 = 0x01;
        const OUT0: addr = 0x0900;
        const OUT1: addr = 0x0901;
        const OUT2: addr = 0x0902;
        static BUF: [u8; 4] = [0; 4];
        static HEAD: u8 = 0;

        #[reset]
        fn main() {
            while (UART_LSR & DATA_READY) != 0 {
                BUF[HEAD] = UART_RBR;
                HEAD = HEAD + 1;
            }
            OUT0 = HEAD;
            OUT1 = BUF[0];
            OUT2 = BUF[1];
            loop {}
        }
    "#,
        devices,
    );
    assert_eq!(
        (e.mem(0x0900), e.mem(0x0901), e.mem(0x0902)),
        (2, b'h', b'i'),
        "both bytes received; before the fix the loop ran once and stored 'i'"
    );
}
