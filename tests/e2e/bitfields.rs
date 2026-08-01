//! Bitfield operations: `x.bit(n)` / `set_bit` / `clear_bit` / `toggle_bit`.
//!
//! Runtime behavior is verified on the default 65C02 target (whose zero-page
//! set/clear lower to `SMB`/`RMB` and execute on the W65C02S emulator). The
//! target-specific lowering is checked by inspecting the emitted assembly.

use crate::common::exec::run;
use crate::common::harness::{CompileResult, compile_success_with_target};
use wraith::codegen::TargetCpu;

fn eval(body: &str) -> u8 {
    let src = format!(
        r#"
        const OUT: addr = 0x0900;
        #[reset]
        fn main() {{
            {body}
            loop {{}}
        }}
    "#
    );
    run(&src).mem(0x0900)
}

// ---------------------------------------------------------------------------
// Set / clear / toggle
// ---------------------------------------------------------------------------

#[test]
fn set_clear_toggle_on_a_u8_local() {
    // 0 -> set 7 (0x80) -> clear 0 (still 0x80) -> toggle 3 (0x88).
    assert_eq!(
        eval("let f: u8 = 0; f.set_bit(7); f.clear_bit(0); f.toggle_bit(3); OUT = f;"),
        0x88
    );
}

#[test]
fn clear_and_toggle_from_all_ones() {
    // 0xFF -> clear 4 (0xEF) -> toggle 0 (0xEE) -> toggle 7 (0x6E).
    assert_eq!(
        eval("let f: u8 = 0xFF; f.clear_bit(4); f.toggle_bit(0); f.toggle_bit(7); OUT = f;"),
        0x6E
    );
}

#[test]
fn each_bit_position_sets_the_right_mask() {
    for n in 0u8..8 {
        let got = eval(&format!("let f: u8 = 0; f.set_bit({n}); OUT = f;"));
        assert_eq!(got, 1 << n, "set_bit({n})");
    }
}

// ---------------------------------------------------------------------------
// Read: x.bit(n) -> bool
// ---------------------------------------------------------------------------

#[test]
fn bit_read_yields_a_bool() {
    assert_eq!(
        eval("let f: u8 = 0x80; if f.bit(7) { OUT = 1; }"),
        1,
        "bit 7 set"
    );
    assert_eq!(
        eval("let f: u8 = 0x80; OUT = 9; if f.bit(6) { OUT = 1; }"),
        9,
        "bit 6 clear leaves OUT untouched"
    );
    // Used directly as a value.
    assert_eq!(eval("let f: u8 = 0x20; OUT = f.bit(5) as u8;"), 1);
    assert_eq!(eval("let f: u8 = 0x20; OUT = f.bit(4) as u8;"), 0);
}

#[test]
fn bit_read_canonicalizes_to_one() {
    // The masked value is 0x40, but the bool must be exactly 1.
    assert_eq!(eval("let f: u8 = 0x40; OUT = f.bit(6) as u8;"), 1);
}

// ---------------------------------------------------------------------------
// u16 targets (bit >= 8 lands in the high byte)
// ---------------------------------------------------------------------------

#[test]
fn bit_ops_on_a_u16_high_byte() {
    let src = r#"
        const LO: addr = 0x0900;
        const HI: addr = 0x0901;
        #[reset]
        fn main() {
            let w: u16 = 0 as u16;
            w.set_bit(12);      // bit 4 of the high byte -> 0x1000
            w.set_bit(1);       // bit 1 of the low byte  -> 0x1002
            LO = w.low;
            HI = w.high;
            loop {}
        }
    "#;
    let mut e = run(src);
    assert_eq!(e.mem(0x0900), 0x02, "low byte");
    assert_eq!(e.mem(0x0901), 0x10, "high byte");
}

#[test]
fn bit_read_on_a_u16_high_byte() {
    assert_eq!(
        eval("let w: u16 = 0x1000 as u16; OUT = w.bit(12) as u8;"),
        1,
        "bit 12 of 0x1000 is set"
    );
    assert_eq!(
        eval("let w: u16 = 0x1000 as u16; OUT = w.bit(13) as u8;"),
        0
    );
}

// ---------------------------------------------------------------------------
// addr (MMIO) targets — read-modify-write on the register
// ---------------------------------------------------------------------------

#[test]
fn set_and_clear_bits_of_an_addr_register() {
    let src = r#"
        const REG: addr = 0x0600;
        const OUT: addr = 0x0900;
        #[reset]
        fn main() {
            REG = 0x0F;
            REG.set_bit(7);     // 0x8F
            REG.clear_bit(0);   // 0x8E
            OUT = REG;
            loop {}
        }
    "#;
    assert_eq!(run(src).mem(0x0900), 0x8E);
}

// ---------------------------------------------------------------------------
// Target-specific lowering (assembly inspection)
// ---------------------------------------------------------------------------

#[test]
fn a_65c02_local_set_clear_uses_smb_rmb() {
    let src = r#"
        #[reset]
        fn main() { let f: u8 = 0; f.set_bit(7); f.clear_bit(3); loop {} }
    "#;
    let asm = compile_success_with_target(src, TargetCpu::Cmos65C02);
    assert!(asm.contains("SMB7 "), "set_bit(7) -> SMB7:\n{asm}");
    assert!(asm.contains("RMB3 "), "clear_bit(3) -> RMB3");
}

#[test]
fn a_6502_local_set_clear_uses_ora_and_masks() {
    let src = r#"
        #[reset]
        fn main() { let f: u8 = 0; f.set_bit(7); f.clear_bit(3); loop {} }
    "#;
    let asm = compile_success_with_target(src, TargetCpu::Nmos6502);
    assert!(!asm.contains("SMB"), "NMOS must not use SMB:\n{asm}");
    assert!(!asm.contains("RMB"), "NMOS must not use RMB");
    assert!(asm.contains("ORA #$80"), "set_bit(7) -> ORA #$80");
    assert!(asm.contains("AND #$F7"), "clear_bit(3) -> AND #$F7");
}

// ---------------------------------------------------------------------------
// Rejections
// ---------------------------------------------------------------------------

fn sema_err(body: &str) -> String {
    let src = format!("#[reset]\nfn main() {{\n{body}\nloop {{}}\n}}");
    match crate::common::harness::compile(&src) {
        CompileResult::SemaError(e) => e,
        other => panic!("expected a sema error, got {other:?}"),
    }
}

#[test]
fn a_runtime_bit_index_is_rejected() {
    let e = sema_err("let f: u8 = 0; let i: u8 = 3; f.set_bit(i);");
    assert!(e.contains("compile-time constant"), "{e}");
}

#[test]
fn an_out_of_range_bit_index_is_rejected() {
    let e = sema_err("let f: u8 = 0; f.set_bit(8);");
    assert!(e.contains("out of range"), "{e}");
}

#[test]
fn setting_a_bit_of_a_const_is_rejected() {
    let src = "const K: u8 = 5;\n#[reset]\nfn main() { K.set_bit(0); loop {} }";
    match crate::common::harness::compile(src) {
        CompileResult::SemaError(e) => assert!(e.contains("const"), "{e}"),
        other => panic!("expected a sema error, got {other:?}"),
    }
}
