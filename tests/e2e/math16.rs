//! End-to-end tests for 16-bit math functions

use crate::common::*;

#[test]
fn mul16_small_numbers() {
    // Test: 10 * 20 = 200 (0x00C8)
    let asm = compile_success(
        r#"
        import { mul16 } from "std/math.wr";

        const RESULT_LO: addr = 0x6000;
        const RESULT_HI: addr = 0x6001;

        fn main() {
            let result: u16 = mul16(10, 20);
            RESULT_LO = result.low;
            RESULT_HI = result.high;
        }
    "#,
    );

    // Check that mul16 function is called
    assert_asm_contains(&asm, "JSR mul16");

    // Result should be 200 = 0x00C8
    // Low byte and high byte should be stored
    assert_asm_contains(&asm, "STA RESULT_LO");
    assert_asm_contains(&asm, "STA RESULT_HI");
}

#[test]
fn mul16_large_numbers() {
    // Test: 100 * 200 = 20000 (0x4E20)
    let asm = compile_success(
        r#"
        import { mul16 } from "std/math.wr";

        fn main() {
            let result: u16 = mul16(100, 200);
        }
    "#,
    );

    assert_asm_contains(&asm, "JSR mul16");
}

#[test]
fn mul16_max_values() {
    // Test: 255 * 255 = 65025 (0xFE01)
    let asm = compile_success(
        r#"
        import { mul16 } from "std/math.wr";

        fn main() {
            let result: u16 = mul16(255, 255);
        }
    "#,
    );

    assert_asm_contains(&asm, "JSR mul16");
}

#[test]
fn mul16_with_zero() {
    // Test: 1000 * 0 = 0
    let asm = compile_success(
        r#"
        import { mul16 } from "std/math.wr";

        fn main() {
            let result: u16 = mul16(1000, 0);
        }
    "#,
    );

    assert_asm_contains(&asm, "JSR mul16");
}

#[test]
fn div16_even_division() {
    // Test: 200 / 10 = 20
    let asm = compile_success(
        r#"
        import { div16 } from "std/math.wr";

        const RESULT_LO: addr = 0x6000;
        const RESULT_HI: addr = 0x6001;

        fn main() {
            let result: u16 = div16(200, 10);
            RESULT_LO = result.low;
            RESULT_HI = result.high;
        }
    "#,
    );

    assert_asm_contains(&asm, "JSR div16");
    assert_asm_contains(&asm, "STA RESULT_LO");
}

#[test]
fn div16_large_numbers() {
    // Test: 20000 / 100 = 200
    let asm = compile_success(
        r#"
        import { div16 } from "std/math.wr";

        fn main() {
            let result: u16 = div16(20000, 100);
        }
    "#,
    );

    assert_asm_contains(&asm, "JSR div16");
}

#[test]
fn div16_with_remainder() {
    // Test: 17 / 3 = 5 (remainder 2, but we only return quotient)
    let asm = compile_success(
        r#"
        import { div16 } from "std/math.wr";

        fn main() {
            let result: u16 = div16(17, 3);
        }
    "#,
    );

    assert_asm_contains(&asm, "JSR div16");
}

#[test]
fn div16_by_zero() {
    // Test: 1000 / 0 = 0xFFFF (error value)
    let asm = compile_success(
        r#"
        import { div16 } from "std/math.wr";

        fn main() {
            let result: u16 = div16(1000, 0);
        }
    "#,
    );

    assert_asm_contains(&asm, "JSR div16");
}

#[test]
fn div16_by_one() {
    // Test: 12345 / 1 = 12345
    let asm = compile_success(
        r#"
        import { div16 } from "std/math.wr";

        fn main() {
            let result: u16 = div16(12345, 1);
        }
    "#,
    );

    assert_asm_contains(&asm, "JSR div16");
}

#[test]
fn mul16_and_div16_together() {
    // Test both operations in same program
    let asm = compile_success(
        r#"
        import { mul16, div16 } from "std/math.wr";

        fn main() {
            let product: u16 = mul16(50, 4);    // 200
            let quotient: u16 = div16(200, 4);  // 50
        }
    "#,
    );

    assert_asm_contains(&asm, "JSR mul16");
    assert_asm_contains(&asm, "JSR div16");
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

/// Extract the zero-page address a caller stores its Nth argument's low byte
/// to immediately before `JSR <callee>` (0-indexed). Looks at the last `STA $XX`
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

/// Regression test: `mul16` (imported from std/math.wr) must read its own
/// parameters from their real zero-page frame slots, not a hardcoded address.
/// Before this was fixed, the hand-written asm in mul16/div16 hardcoded
/// `$80`-`$83` for parameters `a`/`b` - correct only under the old fixed
/// `$80`-`$BF` parameter region. Under frame allocation, a callee's parameters
/// can land anywhere in the frame region, so `mul16` silently read stale/wrong
/// data. See std/math.wr `{a}`/`{a}+1` substitution.
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
    let mul16_body = extract_function_body(&asm, "mul16");

    assert!(
        mul16_body.contains(&format!("LDA {}", a_low_addr)),
        "mul16 body should load parameter 'a' from its actual frame address {} \
         (where the caller stored it), but didn't.\nmul16 body:\n{}\nFull asm:\n{}",
        a_low_addr,
        mul16_body,
        asm
    );

    // The historical bug hardcoded these exact addresses; assert they're gone.
    assert!(
        !mul16_body.contains("LDA $80") && !mul16_body.contains("LDA $82"),
        "mul16 body should not read parameters from the stale fixed $80/$82 \
         addresses.\nmul16 body:\n{}",
        mul16_body
    );
}

/// Same regression coverage as `mul16_reads_actual_parameter_frame_address`,
/// for `div16`.
#[test]
fn div16_reads_actual_parameter_frame_address() {
    let asm = compile_success(
        r#"
        import { div16 } from "std/math.wr";

        const OUT: addr = 0x6000;

        fn main() {
            let unrelated: u8 = 1;
            let result: u16 = div16(200, 4);
            OUT = result.low;
        }
    "#,
    );

    let a_low_addr = arg_store_addr_before_call(&asm, "div16", 0);
    let div16_body = extract_function_body(&asm, "div16");

    assert!(
        div16_body.contains(&format!("LDA {}", a_low_addr)),
        "div16 body should load parameter 'a' from its actual frame address {} \
         (where the caller stored it), but didn't.\ndiv16 body:\n{}\nFull asm:\n{}",
        a_low_addr,
        div16_body,
        asm
    );

    assert!(
        !div16_body.contains("LDA $80") && !div16_body.contains("LDA $82"),
        "div16 body should not read parameters from the stale fixed $80/$82 \
         addresses.\ndiv16 body:\n{}",
        div16_body
    );
}
