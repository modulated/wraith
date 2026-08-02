//! 65C02 (`--cpu 65c02`, the default) base instruction selection.
//!
//! These are code-size wins over the NMOS lowering, gated on the target. Runtime
//! behavior is checked on the W65C02S emulator (`run` targets the 65C02); the
//! target-specific choice is checked by inspecting the emitted assembly and
//! confirming the NMOS build differs.

use crate::common::exec::run;
use crate::common::harness::{compile_success, compile_success_with_target};
use wraith::codegen::TargetCpu;

// ============================================================================
// STZ — storing zero without an accumulator load
// ============================================================================

const SCATTERED: &str = r#"
    static A: u8 = 5;
    static B: u8 = 0;
    static C: u8 = 5;
    const R0: addr = 0x0900;
    #[reset]
    fn main() { R0 = A + B + C; loop {} }
"#;

#[test]
fn a_zero_byte_in_a_static_uses_stz_on_cmos() {
    let asm = compile_success(SCATTERED);
    assert!(
        asm.contains("STZ"),
        "expected STZ for the zero byte:\n{asm}"
    );
}

#[test]
fn the_same_zero_byte_uses_lda_sta_on_nmos() {
    let asm = compile_success_with_target(SCATTERED, TargetCpu::Nmos6502);
    assert!(!asm.contains("STZ"), "NMOS has no STZ:\n{asm}");
    assert!(asm.contains("LDA #$00"), "{asm}");
}

#[test]
fn a_zero_filled_array_loop_stores_zero_directly_on_cmos() {
    // The >8-byte all-zero path is a fill loop; on the 65C02 its body is
    // `STZ addr,X` with no `LDA #$00` feeding it.
    let asm = compile_success(
        r#"
        static BUF: [u8; 32] = [0; 32];
        const R0: addr = 0x0900;
        #[reset]
        fn main() { R0 = BUF[0]; loop {} }
    "#,
    );
    assert!(asm.contains("STZ"), "expected STZ in the fill loop:\n{asm}");
}

#[test]
fn statics_still_zero_initialize_through_stz() {
    // The optimization must not change behavior: RAM is undefined at reset, and
    // the reset handler still has to leave these zero.
    let mut e = run(r#"
        static COUNT: u8 = 0;
        static BUF: [u8; 16] = [0; 16];
        const R0: addr = 0x0900;
        const R1: addr = 0x0901;
        #[reset]
        fn main() { R0 = COUNT; R1 = BUF[9]; loop {} }
    "#);
    assert_eq!((e.mem(0x0900), e.mem(0x0901)), (0, 0));
}

#[test]
fn a_scattered_zero_preserves_the_accumulator_cache() {
    // STZ leaves A untouched, so the run of equal non-zero bytes around a zero
    // still coalesces to one load. Behaviorally: all three read back correctly.
    let mut e = run(r#"
        static A: u8 = 5;
        static B: u8 = 0;
        static C: u8 = 5;
        const R0: addr = 0x0900;
        const R1: addr = 0x0901;
        const R2: addr = 0x0902;
        #[reset]
        fn main() { R0 = A; R1 = B; R2 = C; loop {} }
    "#);
    assert_eq!((e.mem(0x0900), e.mem(0x0901), e.mem(0x0902)), (5, 0, 5));
}

// ============================================================================
// INC A / DEC A — accumulator increment/decrement
// ============================================================================
//
// Note: `x = x + 1` on a u8 *variable* is a memory `INC $zp` (optimal already);
// the accumulator pattern this pass targets arises when the +1 result is used
// rather than stored back, e.g. `R0 = x + 1`.

const U8_INC: &str = r#"
    const R0: addr = 0x0900;
    #[reset]
    fn main() { let x: u8 = 40; R0 = x + 1; loop {} }
"#;

#[test]
fn an_accumulator_increment_uses_inc_a_on_cmos() {
    let asm = compile_success(U8_INC);
    assert!(asm.contains("INC A"), "expected INC A:\n{asm}");
    assert!(!asm.contains("ADC #$01"), "the pair should be gone:\n{asm}");
}

#[test]
fn the_same_increment_uses_clc_adc_on_nmos() {
    let asm = compile_success_with_target(U8_INC, TargetCpu::Nmos6502);
    assert!(!asm.contains("INC A"), "NMOS has no INC A:\n{asm}");
    assert!(asm.contains("ADC #$01"), "{asm}");
}

#[test]
fn an_accumulator_decrement_uses_dec_a_on_cmos() {
    let asm = compile_success(
        r#"
        const R0: addr = 0x0900;
        #[reset]
        fn main() { let x: u8 = 40; R0 = x - 1; loop {} }
    "#,
    );
    assert!(asm.contains("DEC A"), "expected DEC A:\n{asm}");
}

#[test]
fn folded_increment_and_decrement_still_compute_correctly() {
    // Exercises the accumulator path (R0 = x + 1) so the fold actually runs.
    let mut e = run(r#"
        const R0: addr = 0x0900;
        const R1: addr = 0x0901;
        #[reset]
        fn main() {
            let x: u8 = 40;
            let y: u8 = 10;
            R0 = x + 1;
            R1 = y - 1;
            loop {}
        }
    "#);
    assert_eq!((e.mem(0x0900), e.mem(0x0901)), (41, 9));
}

#[test]
fn a_u16_increment_across_a_byte_boundary_keeps_its_carry() {
    // The critical safety case. A 16-bit `+ 1` adds 1 to the low byte and the
    // carry to the high byte, so folding the low-byte `ADC #$01` into `INC A`
    // (which sets no carry) would lose the wrap: 0x00FF + 1 must be 0x0100, not
    // 0x0000. The liveness guard sees C live and declines the fold.
    let mut e = run(r#"
        const R0: addr = 0x0900;
        const R1: addr = 0x0901;
        #[reset]
        fn main() { let x: u16 = 0x00FF; x = x + 1; R0 = x.low; R1 = x.high; loop {} }
    "#);
    assert_eq!((e.mem(0x0900), e.mem(0x0901)), (0x00, 0x01));
}
