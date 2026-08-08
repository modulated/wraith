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
// BBR/BBS fusion: `if x.bit(n)` folds into a single bit-test-branch on the 65C02
// ---------------------------------------------------------------------------

#[test]
fn if_bit_set_folds_to_bbs_on_cmos() {
    let src = r#"
        #[reset]
        fn main() { let f: u8 = 0x80; if f.bit(7) { loop {} } loop {} }
    "#;
    let asm = compile_success_with_target(src, TargetCpu::Cmos65C02);
    assert!(asm.contains("BBS7 "), "if f.bit(7) -> BBS7:\n{asm}");
}

#[test]
fn if_not_bit_folds_to_bbr_on_cmos() {
    let src = r#"
        #[reset]
        fn main() { let f: u8 = 0; if !f.bit(3) { loop {} } loop {} }
    "#;
    let asm = compile_success_with_target(src, TargetCpu::Cmos65C02);
    assert!(asm.contains("BBR3 "), "if !f.bit(3) -> BBR3:\n{asm}");
}

#[test]
fn if_bit_does_not_fold_on_nmos() {
    let src = r#"
        #[reset]
        fn main() { let f: u8 = 0x80; if f.bit(7) { loop {} } loop {} }
    "#;
    let asm = compile_success_with_target(src, TargetCpu::Nmos6502);
    assert!(!asm.contains("BBS"), "NMOS must not use BBS:\n{asm}");
    assert!(!asm.contains("BBR"), "NMOS must not use BBR");
    // The mask-and-compare read is used instead.
    assert!(asm.contains("AND #$80"), "NMOS masks the bit:\n{asm}");
}

#[test]
fn fused_bit_test_runs_correctly() {
    // bit 5 of 0x20 is set -> then-branch writes 0xAA.
    assert_eq!(
        eval("let f: u8 = 0x20; OUT = 0; if f.bit(5) { OUT = 0xAA; }"),
        0xAA
    );
    // bit 6 of 0x20 is clear -> then-branch skipped, else path leaves OUT.
    assert_eq!(
        eval("let f: u8 = 0x20; OUT = 0x11; if f.bit(6) { OUT = 0xAA; }"),
        0x11
    );
}

#[test]
fn fused_negated_bit_test_runs_correctly() {
    // !bit(0) is true when bit 0 is clear -> then-branch taken.
    assert_eq!(
        eval("let f: u8 = 0b1111_1110; OUT = 0; if !f.bit(0) { OUT = 0xCC; }"),
        0xCC
    );
    // !bit(1) is false when bit 1 is set -> then-branch skipped.
    assert_eq!(
        eval("let f: u8 = 0b0000_0010; OUT = 0x22; if !f.bit(1) { OUT = 0xCC; }"),
        0x22
    );
}

#[test]
fn fused_bit_test_takes_the_else_branch() {
    // A full if/else through the fused branch: bit clear -> else.
    assert_eq!(
        eval("let f: u8 = 0; if f.bit(4) { OUT = 1; } else { OUT = 2; }"),
        2
    );
    assert_eq!(
        eval("let f: u8 = 0x10; if f.bit(4) { OUT = 1; } else { OUT = 2; }"),
        1
    );
}

#[test]
fn fused_bit_test_on_a_high_byte() {
    // bit 12 lives in the high byte of a u16; the fusion selects that byte.
    assert_eq!(
        eval("let w: u16 = 0x1000 as u16; OUT = 0; if w.bit(12) { OUT = 0x55; }"),
        0x55
    );
    assert_eq!(
        eval("let w: u16 = 0x1000 as u16; OUT = 0x66; if w.bit(13) { OUT = 0x55; }"),
        0x66
    );
}

#[test]
fn fused_bit_test_on_a_struct_field() {
    // A local struct field lives in the zero-page frame, so the test fuses.
    let mut e = run(r#"
        const R0: addr = 0x0900;
        struct Ctrl { flags: u8 }
        #[reset]
        fn main() {
            let c: Ctrl = Ctrl { flags: 0b0000_1000 };
            R0 = 0;
            if c.flags.bit(3) { R0 = 0x99; }
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0900), 0x99);
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

// ---------------------------------------------------------------------------
// Mutations on field and array-element targets
// ---------------------------------------------------------------------------

fn expect_error(src: &str) -> String {
    use crate::common::harness::compile;
    match compile(src) {
        CompileResult::SemaError(e) | CompileResult::CodegenError(e) => e,
        other => panic!("expected a compile error, got {other:?}"),
    }
}

#[test]
fn set_and_clear_a_bit_of_a_local_struct_field() {
    let mut e = run(r#"
        const R0: addr = 0x0900;
        struct Ctrl { mode: u8, flags: u8 }
        #[reset]
        fn main() {
            let c: Ctrl = Ctrl { mode: 0, flags: 0b0000_0001 };
            c.flags.set_bit(3);
            c.flags.clear_bit(0);
            R0 = c.flags;
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0900), 0b0000_1000);
}

#[test]
fn a_local_struct_field_bit_set_lowers_to_smb_on_cmos() {
    // A local struct lives in a zero-page frame, so the field byte is a single
    // SMB on the 65C02.
    let asm = compile_success_with_target(
        r#"
        const R0: addr = 0x0900;
        struct Ctrl { flags: u8 }
        #[reset]
        fn main() { let c: Ctrl = Ctrl { flags: 0 }; c.flags.set_bit(3); R0 = c.flags; loop {} }
    "#,
        TargetCpu::Cmos65C02,
    );
    assert!(asm.contains("SMB3"), "expected SMB3:\n{asm}");
}

#[test]
fn set_a_bit_of_a_static_struct_field() {
    // A static lives in BSS, so this is an absolute read-modify-write.
    let mut e = run(r#"
        const R0: addr = 0x0900;
        struct Ctrl { flags: u8 }
        static DEV: Ctrl = Ctrl { flags: 0 };
        #[reset]
        fn main() { DEV.flags.set_bit(5); R0 = DEV.flags; loop {} }
    "#);
    assert_eq!(e.mem(0x0900), 0b0010_0000);
}

#[test]
fn set_a_bit_of_a_constant_index_array_element_field() {
    let mut e = run(r#"
        const R0: addr = 0x0900;
        const R1: addr = 0x0901;
        struct Ctrl { flags: u8 }
        #[reset]
        fn main() {
            let t: [Ctrl; 3] = [ Ctrl { flags: 0 }, Ctrl { flags: 0 }, Ctrl { flags: 0 } ];
            t[1].flags.set_bit(2);
            R0 = t[1].flags;
            R1 = t[0].flags;
            loop {}
        }
    "#);
    assert_eq!(
        (e.mem(0x0900), e.mem(0x0901)),
        (0b0000_0100, 0),
        "only element 1 changed"
    );
}

#[test]
fn set_the_high_bit_of_a_u16_field() {
    // The bit index selects the byte: bit 15 is the high byte.
    let mut e = run(r#"
        const R0: addr = 0x0900;
        const R1: addr = 0x0901;
        struct Reg { value: u16 }
        #[reset]
        fn main() {
            let r: Reg = Reg { value: 0x0001 };
            r.value.set_bit(15);
            R0 = r.value.low; R1 = r.value.high;
            loop {}
        }
    "#);
    assert_eq!((e.mem(0x0900), e.mem(0x0901)), (0x01, 0x80));
}

#[test]
fn a_bit_mutation_of_a_const_array_element_is_rejected() {
    // A const lives in ROM, so a bit write to one of its elements would be a
    // silent no-op on real hardware. (A struct can't be `const` at all — only a
    // const array reaches this path.)
    let err = expect_error(
        r#"
        const R0: addr = 0x0900;
        const LUT: [u8; 4] = [1, 2, 3, 4];
        #[reset]
        fn main() { LUT[0].set_bit(3); R0 = 0; loop {} }
    "#,
    );
    assert!(err.contains("ROM"), "{err}");
}

// ---------------------------------------------------------------------------
// Indirect bit mutation: through a pointer, and through a runtime index
// ---------------------------------------------------------------------------

#[test]
fn set_a_bit_through_a_pointer() {
    // A driver handed `&Ctrl` sets a control bit on the caller's struct.
    let mut e = run(r#"
        const R0: addr = 0x0900;
        struct Ctrl { flags: u8 }
        fn enable(c: &Ctrl) { c.flags.set_bit(3); }
        #[reset]
        fn main() {
            let dev: Ctrl = Ctrl { flags: 0b0000_0001 };
            enable(&dev);
            R0 = dev.flags;
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0900), 0b0000_1001, "bit 3 set through the pointer");
}

#[test]
fn clear_and_toggle_a_bit_through_a_pointer() {
    let mut e = run(r#"
        const R0: addr = 0x0900;
        struct Ctrl { flags: u8 }
        fn tweak(c: &Ctrl) { c.flags.clear_bit(0); c.flags.toggle_bit(7); }
        #[reset]
        fn main() {
            let dev: Ctrl = Ctrl { flags: 0b0000_0001 };
            tweak(&dev);
            R0 = dev.flags;
            loop {}
        }
    "#);
    assert_eq!(
        e.mem(0x0900),
        0b1000_0000,
        "bit 0 cleared, bit 7 toggled on"
    );
}

#[test]
fn set_a_bit_of_a_runtime_indexed_element() {
    let mut e = run(r#"
        const R0: addr = 0x0900;
        const R1: addr = 0x0901;
        struct Ctrl { flags: u8 }
        #[reset]
        fn main() {
            let t: [Ctrl; 4] = [ Ctrl{flags:0}, Ctrl{flags:0}, Ctrl{flags:0}, Ctrl{flags:0} ];
            let i: u8 = 2;
            t[i].flags.set_bit(1);
            R0 = t[2].flags;   // the one changed
            R1 = t[0].flags;   // untouched
            loop {}
        }
    "#);
    assert_eq!(
        (e.mem(0x0900), e.mem(0x0901)),
        (0b0000_0010, 0),
        "only element 2 changed"
    );
}

#[test]
fn set_a_high_bit_of_a_u16_field_through_a_pointer() {
    // The u16 target routes through the u16 assignment, landing bit 15 in the
    // high byte.
    let mut e = run(r#"
        const R0: addr = 0x0900;
        const R1: addr = 0x0901;
        struct Reg { value: u16 }
        fn hoist(r: &Reg) { r.value.set_bit(15); }
        #[reset]
        fn main() {
            let reg: Reg = Reg { value: 0x0001 };
            hoist(&reg);
            R0 = reg.value.low; R1 = reg.value.high;
            loop {}
        }
    "#);
    assert_eq!(
        (e.mem(0x0900), e.mem(0x0901)),
        (0x01, 0x80),
        "bit 15 set through the pointer"
    );
}
