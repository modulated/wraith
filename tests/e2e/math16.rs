//! End-to-end tests for 16-bit math functions

use crate::common::*;

#[test]
fn mul16_small_numbers() {
    // Test: 10 * 20 = 200 (0x00C8)
    let asm = compile_success(r#"
        import { mul16 } from "std/math.wr";

        const RESULT_LO: addr = 0x6000;
        const RESULT_HI: addr = 0x6001;

        fn main() {
            let result: u16 = mul16(10, 20);
            RESULT_LO = result.low;
            RESULT_HI = result.high;
        }
    "#);

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
    let asm = compile_success(r#"
        import { mul16 } from "std/math.wr";

        fn main() {
            let result: u16 = mul16(100, 200);
        }
    "#);

    assert_asm_contains(&asm, "JSR mul16");
}

#[test]
fn mul16_max_values() {
    // Test: 255 * 255 = 65025 (0xFE01)
    let asm = compile_success(r#"
        import { mul16 } from "std/math.wr";

        fn main() {
            let result: u16 = mul16(255, 255);
        }
    "#);

    assert_asm_contains(&asm, "JSR mul16");
}

#[test]
fn mul16_with_zero() {
    // Test: 1000 * 0 = 0
    let asm = compile_success(r#"
        import { mul16 } from "std/math.wr";

        fn main() {
            let result: u16 = mul16(1000, 0);
        }
    "#);

    assert_asm_contains(&asm, "JSR mul16");
}

#[test]
fn div16_even_division() {
    // Test: 200 / 10 = 20
    let asm = compile_success(r#"
        import { div16 } from "std/math.wr";

        const RESULT_LO: addr = 0x6000;
        const RESULT_HI: addr = 0x6001;

        fn main() {
            let result: u16 = div16(200, 10);
            RESULT_LO = result.low;
            RESULT_HI = result.high;
        }
    "#);

    assert_asm_contains(&asm, "JSR div16");
    assert_asm_contains(&asm, "STA RESULT_LO");
}

#[test]
fn div16_large_numbers() {
    // Test: 20000 / 100 = 200
    let asm = compile_success(r#"
        import { div16 } from "std/math.wr";

        fn main() {
            let result: u16 = div16(20000, 100);
        }
    "#);

    assert_asm_contains(&asm, "JSR div16");
}

#[test]
fn div16_with_remainder() {
    // Test: 17 / 3 = 5 (remainder 2, but we only return quotient)
    let asm = compile_success(r#"
        import { div16 } from "std/math.wr";

        fn main() {
            let result: u16 = div16(17, 3);
        }
    "#);

    assert_asm_contains(&asm, "JSR div16");
}

#[test]
fn div16_by_zero() {
    // Test: 1000 / 0 = 0xFFFF (error value)
    let asm = compile_success(r#"
        import { div16 } from "std/math.wr";

        fn main() {
            let result: u16 = div16(1000, 0);
        }
    "#);

    assert_asm_contains(&asm, "JSR div16");
}

#[test]
fn div16_by_one() {
    // Test: 12345 / 1 = 12345
    let asm = compile_success(r#"
        import { div16 } from "std/math.wr";

        fn main() {
            let result: u16 = div16(12345, 1);
        }
    "#);

    assert_asm_contains(&asm, "JSR div16");
}

#[test]
fn mul16_and_div16_together() {
    // Test both operations in same program
    let asm = compile_success(r#"
        import { mul16, div16 } from "std/math.wr";

        fn main() {
            let product: u16 = mul16(50, 4);    // 200
            let quotient: u16 = div16(200, 4);  // 50
        }
    "#);

    assert_asm_contains(&asm, "JSR mul16");
    assert_asm_contains(&asm, "JSR div16");
}
