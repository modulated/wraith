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
