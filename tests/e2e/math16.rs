//! 16-bit math (`std/math.wr` mul16/div16), verified by execution.
//!
//! These previously asserted only that a `JSR mul16` appeared, while their
//! comments claimed results like "10 * 20 = 200" that nothing checked. The
//! arithmetic is now actually run: 16-bit routines are exactly where a dropped
//! carry or a truncated high byte hides, and those produce plausible assembly.
//!
//! The two frame-address tests stay at the assembly level on purpose — they
//! guard against a specific regression (the routine reading its parameters from
//! stale fixed addresses) that is a property of the emitted code, not a value.

use crate::common::compile_success;
use crate::common::exec::run;

/// Call a 16-bit math routine and return its result.
fn call16(func: &str, a: u32, b: u32) -> u16 {
    let src = format!(
        r#"
        import {{ {func} }} from "std/math.wr";
        const LO: addr = 0x0900;
        const HI: addr = 0x0901;
        #[reset]
        fn main() {{
            let r: u16 = {func}({a} as u16, {b} as u16);
            LO = r.low;
            HI = r.high;
            loop {{}}
        }}
    "#
    );
    run(&src).mem16(0x0900)
}

// ============================================================================
// mul16
// ============================================================================

#[test]
fn mul16_small_numbers() {
    assert_eq!(call16("mul16", 10, 20), 200);
}

#[test]
fn mul16_large_numbers() {
    // 100 * 200 = 20000 ($4E20): a product that needs both result bytes.
    assert_eq!(call16("mul16", 100, 200), 20000);
}

#[test]
fn mul16_max_result() {
    // 255 * 255 = 65025 ($FE01), the largest product that still fits in u16.
    assert_eq!(call16("mul16", 255, 255), 65025);
}

#[test]
fn mul16_with_zero() {
    assert_eq!(call16("mul16", 0, 1234), 0);
    assert_eq!(call16("mul16", 1234, 0), 0);
}

#[test]
fn mul16_by_one_is_identity() {
    assert_eq!(call16("mul16", 1, 4321), 4321);
    assert_eq!(call16("mul16", 4321, 1), 4321);
}

#[test]
fn mul16_operands_above_255() {
    // Both operands need their high byte; a routine that only multiplied the
    // low bytes would give 0 here (300 & 0xFF == 44, 5 -> 220, not 1500).
    assert_eq!(call16("mul16", 300, 5), 1500);
    assert_eq!(call16("mul16", 256, 3), 768);
}

#[test]
fn mul16_wraps_past_65535() {
    // 1000 * 100 = 100000, which wraps to 100000 - 65536 = 34464.
    assert_eq!(call16("mul16", 1000, 100), 34464);
}

// ============================================================================
// div16
// ============================================================================

#[test]
fn div16_even_division() {
    assert_eq!(call16("div16", 100, 10), 10);
}

#[test]
fn div16_large_numbers() {
    assert_eq!(call16("div16", 20000, 200), 100);
}

#[test]
fn div16_with_remainder() {
    // Truncating division: 7 / 2 = 3, not 4.
    assert_eq!(call16("div16", 7, 2), 3);
    assert_eq!(call16("div16", 1000, 3), 333);
}

#[test]
fn div16_by_one_is_identity() {
    assert_eq!(call16("div16", 4321, 1), 4321);
}

#[test]
fn div16_by_zero_returns_sentinel() {
    // The routine's documented behaviour: $FFFF rather than a hang or a crash.
    assert_eq!(call16("div16", 100, 0), 0xFFFF);
}

#[test]
fn div16_divisor_larger_than_dividend() {
    assert_eq!(call16("div16", 5, 100), 0);
}

#[test]
fn div16_operands_above_255() {
    // Needs the high byte of both operands to participate.
    assert_eq!(call16("div16", 1000, 250), 4);
    assert_eq!(call16("div16", 512, 256), 2);
}

// ============================================================================
// Combined
// ============================================================================

#[test]
fn mul16_and_div16_round_trip() {
    // (a * b) / b == a for values that do not overflow.
    let src = r#"
        import { mul16, div16 } from "std/math.wr";
        const LO: addr = 0x0900;
        const HI: addr = 0x0901;
        #[reset]
        fn main() {
            let p: u16 = mul16(123 as u16, 45 as u16);
            let q: u16 = div16(p, 45 as u16);
            LO = q.low;
            HI = q.high;
            loop {}
        }
    "#;
    assert_eq!(run(src).mem16(0x0900), 123);
}

/// Extract the body of a labeled assembly function: everything from `label:`
/// up to (not including) the next line that looks like another top-level label.
fn extract_function_body<'a>(asm: &'a str, label: &str) -> &'a str {
    let start = asm
        .find(&format!("{}:", label))
        .unwrap_or_else(|| panic!("Could not find label '{}:' in assembly", label));
    let after_label = &asm[start..];
    let body_start = after_label.find('\n').map(|i| i + 1).unwrap_or(0);
    let rest = &after_label[body_start..];

    // A top-level label is a line with no leading whitespace ending in ':'
    // that isn't a local branch target (branch targets used inside mul16/div16
    // like "mul16_loop:" retain the function's own leading whitespace/indent
    // in this codegen, but to be safe we stop at the next function-style label
    // or RTS, whichever comes first).
    let end = rest
        .lines()
        .scan(0usize, |pos, line| {
            let start_of_line = *pos;
            *pos += line.len() + 1;
            Some((start_of_line, line))
        })
        .find(|(_, line)| {
            let trimmed = line.trim_start();
            trimmed.starts_with("RTS")
        })
        .map(|(pos, line)| pos + line.len())
        .unwrap_or(rest.len());

    &rest[..end.min(rest.len())]
}

/// before the JSR, walking backwards N*2 STA instructions for 16-bit args
/// (low then high byte per argument, in order).
fn arg_store_addr_before_call(asm: &str, callee: &str, arg_index: usize) -> String {
    let jsr = format!("JSR {}", callee);
    let jsr_pos = asm
        .find(&jsr)
        .unwrap_or_else(|| panic!("Could not find 'JSR {}' in assembly", callee));
    let before = &asm[..jsr_pos];

    let sta_addrs: Vec<&str> = before
        .lines()
        .rev()
        .filter_map(|line| {
            let trimmed = line.trim();
            trimmed.strip_prefix("STA ").map(|addr| addr.trim())
        })
        .collect();

    // sta_addrs is in reverse order (closest to JSR first); each 16-bit arg is
    // stored as [low, high] in program order, so reversed order is [..., high, low]
    // for the last argument first. We want arg_index's low-byte address.
    let low_byte_reverse_index = arg_index * 2 + 1;
    sta_addrs
        .get(low_byte_reverse_index)
        .unwrap_or_else(|| {
            panic!(
                "Could not find STA for argument {} before 'JSR {}'",
                arg_index, callee
            )
        })
        .to_string()
}

// ============================================================================
// Regression: routines must read parameters from their real frame slots
// ============================================================================

#[test]
fn mul16_reads_actual_parameter_frame_address() {
    let asm = compile_success(
        r#"
        import { mul16 } from "std/math.wr";
        const OUT: addr = 0x6000;
        fn main() {
            let unrelated: u8 = 1;
            let result: u16 = mul16(300, 7);
            OUT = result.low;
        }
    "#,
    );
    let a_low_addr = arg_store_addr_before_call(&asm, "mul16", 0);
    let body = extract_function_body(&asm, "mul16");
    assert!(
        body.contains(&format!("LDA {}", a_low_addr)),
        "mul16 should load parameter 'a' from the frame address the caller \
         wrote ({}).\nbody:\n{}",
        a_low_addr,
        body
    );
    // The historical bug hardcoded these addresses; assert they are gone.
    assert!(
        !body.contains("LDA $80") && !body.contains("LDA $82"),
        "mul16 must not read parameters from the stale fixed $80/$82 addresses"
    );
}

#[test]
fn div16_reads_actual_parameter_frame_address() {
    let asm = compile_success(
        r#"
        import { div16 } from "std/math.wr";
        const OUT: addr = 0x6000;
        fn main() {
            let unrelated: u8 = 1;
            let result: u16 = div16(300, 7);
            OUT = result.low;
        }
    "#,
    );
    let a_low_addr = arg_store_addr_before_call(&asm, "div16", 0);
    let body = extract_function_body(&asm, "div16");
    assert!(
        body.contains(&format!("LDA {}", a_low_addr)),
        "div16 should load parameter 'a' from the frame address the caller \
         wrote ({}).\nbody:\n{}",
        a_low_addr,
        body
    );
    assert!(
        !body.contains("LDA $80") && !body.contains("LDA $82"),
        "div16 must not read parameters from the stale fixed $80/$82 addresses"
    );
}
