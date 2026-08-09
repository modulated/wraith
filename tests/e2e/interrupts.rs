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
    // Default target is the 65C02: X and Y are pushed/pulled directly.
    assert_asm_contains(&asm, "PHA");
    assert_asm_contains(&asm, "PHX");
    assert_asm_contains(&asm, "PHY");
    assert_asm_contains(&asm, "PLY");
    assert_asm_contains(&asm, "PLX");
    // Should have vector table
    assert_asm_contains(&asm, ".ORG $FFFA");
    assert_asm_contains(&asm, ".WORD nmi_handler");
}

#[test]
fn nmi_handler_prologue_on_nmos_routes_x_and_y_through_a() {
    // The NMOS 6502 has no PHX/PHY, so X and Y are saved via A (which is why A
    // is pushed first) and restored in reverse.
    let asm = compile_success_with_target(
        r#"
        const OUT: addr = 0x400;
        #[nmi]
        fn nmi_handler() {
            OUT = 0xFF;
        }
        fn main() {}
    "#,
        wraith::codegen::TargetCpu::Nmos6502,
    );
    for op in ["PHA", "TXA", "TYA", "TAY", "TAX", "RTI"] {
        assert_asm_contains(&asm, op);
    }
    // ...and none of the 65C02 stack ops.
    assert!(!asm.contains("PHX"), "NMOS output must not use PHX:\n{asm}");
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

// ---------------------------------------------------------------------------
// Hardware-stack (page 1) depth
//
// A handler entry costs 3 bytes for the CPU sequence, 3 for the A/X/Y pushes,
// the zero-page save (which can reach ~220 bytes), and 2 per nested JSR.
// Overflow wraps silently on a 6502, so it is caught here or not at all.
// ---------------------------------------------------------------------------

fn warnings_of(src: &str) -> String {
    match compile(src) {
        CompileResult::Success(warnings, _) => warnings,
        other => panic!("expected success, got {other:?}"),
    }
}

#[test]
fn a_trivial_handler_does_not_warn_about_the_hardware_stack() {
    let w = warnings_of(
        r#"
        const R: addr = 0x0900;
        #[irq]
        fn on_irq() { R = 1; }
        #[reset]
        fn main() { loop {} }
    "#,
    );
    assert!(!w.contains("hardware-stack"), "{w}");
}

#[test]
fn nesting_handlers_that_exceed_page_one_warns() {
    // Two heavy handlers: an NMI can preempt an IRQ, so their entry costs sum.
    // Each saves a wide zero-page span (deep chain of large u16 frames) plus the
    // math scratch, which alone is ~135 bytes; two of them overflow page 1.
    let w = warnings_of(
        r#"
        const R: addr = 0x0900;
        const R2: addr = 0x0901;
        fn e(a: u16) -> u16 { let p:u16=a; let q:u16=a; let r:u16=a; let s:u16=a; let t:u16=a; let u:u16=a; let v:u16=a; return p+q+r+s+t+u+v; }
        fn d(a: u16) -> u16 { let p:u16=a; let q:u16=a; let r:u16=a; let s:u16=a; let t:u16=a; let u:u16=a; let v:u16=a; return e(a)+p+q+r+s+t+u+v; }
        fn c(a: u16) -> u16 { let p:u16=a; let q:u16=a; let r:u16=a; let s:u16=a; let t:u16=a; let u:u16=a; let v:u16=a; return d(a)+p+q+r+s+t+u+v; }
        fn b(a: u16) -> u16 { let p:u16=a; let q:u16=a; let r:u16=a; let s:u16=a; let t:u16=a; let u:u16=a; let v:u16=a; return c(a)+p+q+r+s+t+u+v; }
        #[irq]
        fn on_irq() { let v: u16 = b(7 as u16); R = v.low; }
        #[nmi]
        fn on_nmi() { let v: u16 = b(9 as u16); R2 = v.low; }
        #[reset]
        fn main() { loop {} }
    "#,
    );
    assert!(
        w.contains("hardware stack"),
        "expected a depth warning:\n{w}"
    );
    assert!(
        w.contains("nested call"),
        "the message should break down the cost:\n{w}"
    );
}

#[test]
fn an_indirect_call_in_a_handler_suppresses_the_depth_warning() {
    // Depth through a function pointer is unknowable (no call edge is recorded),
    // so the check declines rather than reporting a wrong number.
    let w = warnings_of(
        r#"
        const R: addr = 0x0900;
        const R2: addr = 0x0901;
        fn e(a: u16) -> u16 { let p:u16=a; let q:u16=a; let r:u16=a; let s:u16=a; let t:u16=a; let u:u16=a; let v:u16=a; return p+q+r+s+t+u+v; }
        fn d(a: u16) -> u16 { let p:u16=a; let q:u16=a; let r:u16=a; let s:u16=a; let t:u16=a; let u:u16=a; let v:u16=a; return e(a)+p+q+r+s+t+u+v; }
        fn c(a: u16) -> u16 { let p:u16=a; let q:u16=a; let r:u16=a; let s:u16=a; let t:u16=a; let u:u16=a; let v:u16=a; return d(a)+p+q+r+s+t+u+v; }
        fn b(a: u16) -> u16 { let p:u16=a; let q:u16=a; let r:u16=a; let s:u16=a; let t:u16=a; let u:u16=a; let v:u16=a; return c(a)+p+q+r+s+t+u+v; }
        fn leaf(x: u8) -> u8 { return x; }
        static TBL: [fn(u8) -> u8; 1] = [leaf];
        #[irq]
        fn on_irq() { let i: u8 = 0; R = TBL[i](1); }
        #[nmi]
        fn on_nmi() { let v: u16 = b(9 as u16); R2 = v.low; }
        #[reset]
        fn main() { loop {} }
    "#,
    );
    assert!(!w.contains("hardware stack"), "should be silent:\n{w}");
}
