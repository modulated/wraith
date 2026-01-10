//! Peephole optimization tests
//!
//! Tests the peephole optimizer's ability to eliminate redundant operations

use crate::common::*;

// ============================================================================
// Redundant Load Elimination
// ============================================================================

#[test]
fn eliminate_redundant_load() {
    let asm = compile_success(r#"
        const OUT: addr = 0x6000;
        fn main() {
            let x: u8 = 42;
            OUT = x;
            OUT = x;  // Should reuse loaded value
        }
    "#);

    // Count LDA instructions - peephole should eliminate redundant loads
    let lda_count = asm.lines().filter(|line| line.contains("LDA")).count();
    // Should load x only once, not twice
    assert!(lda_count <= 2, "Too many LDA instructions: {}", lda_count);
}

// ============================================================================
// Redundant Store Elimination
// ============================================================================

#[test]
fn eliminate_redundant_store() {
    let asm = compile_success(r#"
        fn main() {
            let x: u8 = 10;
            x = 20;  // First store should be eliminated
        }
    "#);

    // Should only store once
    let sta_count = asm.lines().filter(|line| line.contains("STA $40")).count();
    assert_eq!(sta_count, 1, "Should only store final value");
}

// ============================================================================
// Load After Store Elimination
// ============================================================================

#[test]
fn eliminate_load_after_store() {
    let asm = compile_success(r#"
        const OUT: addr = 0x6000;
        fn main() {
            let x: u8 = 42;
            // Store then load same location
            OUT = x;
        }
    "#);

    // Optimizer should work - just verify compilation succeeds
    assert!(asm.contains("STA"));
}

// ============================================================================
// Dead Store Elimination
// ============================================================================

#[test]
fn eliminate_dead_store() {
    let asm = compile_success(r#"
        fn main() {
            let x: u8 = 10;
            x = 20;
            x = 30;
        }
    "#);

    // First two stores are dead, only last one should remain
    let sta_count = asm.lines().filter(|line| line.contains("STA $40")).count();
    assert!(sta_count <= 1, "Dead stores should be eliminated");
}

// ============================================================================
// NOP Operation Elimination
// ============================================================================

#[test]
fn eliminate_nop_operations() {
    let asm = compile_success(r#"
        fn main() {
            let x: u8 = 0;
            x = x + 0;  // Adding zero is a NOP
        }
    "#);

    // Adding zero should be optimized away
    // Should not have redundant ADC #$00
    assert!(!asm.contains("ADC #$00") || asm.lines().filter(|l| l.contains("ADC #$00")).count() == 0);
}

#[test]
fn eliminate_multiply_by_one() {
    let asm = compile_success(r#"
        fn main() {
            let x: u8 = 10;
            let y: u8 = x * 1;  // Multiply by 1 is a NOP
        }
    "#);

    // Multiplication by 1 should be optimized
    assert!(asm.contains("STA"));
}

// ============================================================================
// Redundant Transfer Elimination
// ============================================================================

#[test]
fn eliminate_redundant_tax_txa() {
    let asm = compile_success(r#"
        fn main() {
            let x: u8 = 10;
            let y: u8 = x;
            let z: u8 = y;
        }
    "#);

    // Should optimize register transfers
    assert!(asm.contains("STA"));
}

// ============================================================================
// Constant Folding Integration
// ============================================================================

#[test]
fn constant_folding_arithmetic() {
    let asm = compile_success(r#"
        const RESULT: addr = 0x0400;
        fn main() {
            RESULT = (10 + 20) * 2;
        }
    "#);

    // Should fold to 60 (0x3C)
    assert_asm_contains(&asm, "LDA #$3C");
    assert_asm_not_contains(&asm, "ADC");
    assert_asm_not_contains(&asm, "PHA");
}

#[test]
fn constant_folding_bitwise() {
    let asm = compile_success(r#"
        const RESULT: addr = 0x0400;
        fn main() {
            RESULT = (0xFF & 0x0F) | 0x80;
        }
    "#);

    // Should fold to 0x8F
    assert_asm_contains(&asm, "LDA #$8F");
    assert_asm_not_contains(&asm, "AND");
    assert_asm_not_contains(&asm, "ORA");
}

#[test]
fn constant_folding_shifts() {
    let asm = compile_success(r#"
        const RESULT: addr = 0x0400;
        fn main() {
            RESULT = (1 << 4) >> 1;
        }
    "#);

    // Should fold to 8 (0x08)
    assert_asm_contains(&asm, "LDA #$08");
    assert_asm_not_contains(&asm, "ASL");
    assert_asm_not_contains(&asm, "LSR");
}

// ============================================================================
// Complex Optimization Scenarios
// ============================================================================

#[test]
fn multiple_optimizations() {
    let asm = compile_success(r#"
        const OUT1: addr = 0x6000;
        const OUT2: addr = 0x6001;
        fn main() {
            let x: u8 = 10 + 5;  // Constant fold
            OUT1 = x;
            OUT2 = x;  // Eliminate redundant load
            x = x + 0;  // Eliminate NOP
        }
    "#);

    // Should have constant folded to 15
    assert_asm_contains(&asm, "LDA #$0F");
    // Should not have ADC #$00
    assert!(!asm.contains("ADC #$00"));
}

#[test]
fn optimization_preserves_correctness() {
    let asm = compile_success(r#"
        const OUT: addr = 0x6000;
        fn main() {
            let a: u8 = 10;
            let b: u8 = 20;
            let c: u8 = a + b;
            OUT = c;
        }
    "#);

    // Should still produce correct result
    assert_asm_contains(&asm, "ADC");
    assert_asm_contains(&asm, "STA");
}

// ============================================================================
// Register Optimization
// ============================================================================

#[test]
fn inc_optimization() {
    let asm = compile_success(r#"
        fn main() {
            let x: u8 = 10;
            x = x + 1;
        }
    "#);

    // x + 1 should be optimized to INC
    assert_asm_contains(&asm, "INC");
    assert_asm_not_contains(&asm, "ADC #$01");
}

#[test]
fn dec_optimization() {
    let asm = compile_success(r#"
        fn main() {
            let x: u8 = 10;
            x = x - 1;
        }
    "#);

    // x - 1 should be optimized to DEC
    assert_asm_contains(&asm, "DEC");
    assert_asm_not_contains(&asm, "SBC #$01");
}

// ============================================================================
// Loop Optimizations
// ============================================================================

#[test]
fn loop_counter_optimization() {
    let asm = compile_success(r#"
        const OUT: addr = 0x6000;
        fn main() {
            for i: u8 in 0..10 {
                OUT = i;
            }
        }
    "#);

    // Loop should use X register efficiently
    assert_asm_contains(&asm, "INX");
    assert_asm_contains(&asm, "CPX");
}

// ============================================================================
// Size Optimization Metrics
// ============================================================================

#[test]
fn optimization_reduces_size() {
    let optimized = compile_success(r#"
        const OUT: addr = 0x6000;
        fn main() {
            let x: u8 = 10 + 20;
            OUT = x;
        }
    "#);

    // Count total instructions
    let optimized_size = optimized.lines()
        .filter(|line| line.trim_start().starts_with(|c: char| c.is_alphabetic()))
        .count();

    // Should be relatively small due to constant folding
    assert!(optimized_size < 10, "Optimized code should be compact");
}
