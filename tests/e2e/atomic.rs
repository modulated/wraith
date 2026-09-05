//! `atomic static` — interrupt-safe access to a value shared with a handler.
//!
//! A two-byte load or store is two instructions, so an interrupt landing
//! between them lets a handler see (or cause) a torn value — half old, half new.
//! `atomic static` masks interrupts (`PHP; SEI; … ; PLP`) around each
//! whole-variable read and around each assignment, so the access is indivisible.
//! `PHP`/`PLP` rather than `SEI`/`CLI` so a handler's own access does not
//! re-enable interrupts partway through it.

use crate::common::exec::run;
use crate::common::harness::{CompileResult, compile};

fn error(src: &str) -> String {
    match compile(src) {
        CompileResult::SemaError(e)
        | CompileResult::ParseError(e)
        | CompileResult::CodegenError(e)
        | CompileResult::LexError(e) => e,
        CompileResult::Success(..) => panic!("expected a compile error, but it compiled:\n{src}"),
    }
}

/// A read and an assignment of a two-byte `atomic static` are wrapped in
/// `PHP;SEI;…;PLP`; a plain `static` beside it is not.
#[test]
fn a_two_byte_atomic_access_is_masked() {
    let asm = crate::common::harness::compile_success(
        "const OUT: addr = 0x0500;\n\
         atomic static TICKS: u16 = 0;\n\
         static PLAIN: u16 = 0;\n\
         #[reset]\n\
         fn main() {\n\
             TICKS = TICKS + 1;\n\
             let t: u16 = TICKS;\n\
             OUT = t.low;\n\
             PLAIN = PLAIN + 1;\n\
             loop {}\n\
         }\n",
    );
    assert!(
        asm.contains("SEI"),
        "an atomic access should mask interrupts:\n{asm}"
    );
    assert!(
        asm.contains("PHP") && asm.contains("PLP"),
        "save/restore the flag:\n{asm}"
    );
    // The plain static's increment must not be guarded — count the guards.
    let seis = asm.matches("SEI").count();
    assert_eq!(
        seis, 2,
        "one guard for the RMW, one for the read; PLAIN unguarded:\n{asm}"
    );
}

/// The guarded read-modify-write computes correctly: 300 increments of an
/// `atomic` counter, no interrupts, reads back 300 (crosses the byte boundary).
#[test]
fn an_atomic_counter_increments_correctly() {
    let mut e = run(r#"
        const OUT_LO: addr = 0x0500;
        const OUT_HI: addr = 0x0501;
        atomic static TICKS: u16 = 0;
        #[reset]
        fn main() {
            let c: u16 = 0;
            while c < 300 {
                TICKS = TICKS + 1;
                c = c + 1;
            }
            OUT_LO = TICKS.low;
            OUT_HI = TICKS.high;
            loop {}
        }
    "#);
    assert_eq!(
        e.mem16(0x0500),
        300,
        "300 guarded increments read back as 300"
    );
}

/// An interrupt handler increments and reads the same `atomic static`. The
/// handler runs with interrupts already masked, so the guard's save/restore must
/// leave them masked (not re-enable mid-handler); the increment still lands.
#[test]
fn a_handler_increments_a_shared_atomic() {
    let mut e = run(r#"
        const OUT_LO: addr = 0x0500;
        const OUT_HI: addr = 0x0501;
        atomic static TICKS: u16 = 0;
        #[reset]
        fn main() {
            asm { "CLI" }
            loop {}
        }
        #[irq]
        fn on_irq() {
            TICKS = TICKS + 1;
            OUT_LO = TICKS.low;
            OUT_HI = TICKS.high;
        }
    "#);
    assert_eq!(e.mem16(0x0500), 0, "no interrupt yet");
    e.pulse_irq();
    e.pulse_irq();
    e.pulse_irq();
    assert_eq!(
        e.mem16(0x0500),
        3,
        "three IRQs, each an atomic increment, mirror 3"
    );
}

/// A one-byte `atomic` static is a no-op: it warns and emits no guard, behaving
/// as a plain `static`.
#[test]
fn a_one_byte_atomic_warns_and_emits_no_guard() {
    let src = "const OUT: addr = 0x0500;\n\
               atomic static FLAG: u8 = 0;\n\
               #[reset]\n\
               fn main() { FLAG = FLAG + 1; OUT = FLAG; loop {} }\n";
    match compile(src) {
        CompileResult::Success(warnings, asm) => {
            assert!(
                warnings.contains("atomic") && warnings.contains("no effect"),
                "expected a no-op warning, got: {warnings}"
            );
            assert!(
                !asm.contains("SEI"),
                "a one-byte atomic must not emit a guard:\n{asm}"
            );
        }
        other => panic!("expected success with a warning, got {other:?}"),
    }
}

#[test]
fn atomic_is_refused_on_a_const() {
    // `atomic` may only precede `static`.
    let e = error("atomic const X: u16 = 5;\n#[reset]\nfn main() { loop {} }\n");
    assert!(e.contains("atomic") && e.contains("static"), "{e}");
}

#[test]
fn atomic_is_refused_on_an_aggregate() {
    let e = error("atomic static BUF: [u16; 4] = [0, 0, 0, 0];\n#[reset]\nfn main() { loop {} }\n");
    assert!(
        e.contains("atomic") && (e.contains("scalar") || e.contains("array")),
        "{e}"
    );
}

#[test]
fn atomic_before_a_function_is_refused() {
    let e = error("atomic fn helper() {}\n#[reset]\nfn main() { loop {} }\n");
    assert!(e.contains("atomic") && e.contains("static"), "{e}");
}
