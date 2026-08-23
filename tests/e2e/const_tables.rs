//! `[|i| => …]` — a table generated at compile time.
//!
//! The 6502's oldest trade is ROM for cycles: a multiply becomes a lookup, a
//! screen row becomes a pair of high/low tables. Before this form the table had
//! to be written out by hand or by a build script, which put the rule that
//! produced the numbers somewhere the compiler could not see — and left the
//! length stated twice, in the type and in the literal.
//!
//! The length is not written here: it comes from the array type the expression
//! is declared at. `i` is a `u8`, and the body follows the language's ordinary
//! arithmetic, so a generated table holds exactly what the equivalent run-time
//! loop would have computed — that equivalence is what
//! [`a_generated_table_matches_the_same_loop_at_run_time`] pins.

use crate::common::exec::run;
use crate::common::harness::{CompileResult, compile, compile_success};

fn expect_error(src: &str) -> String {
    match compile(src) {
        CompileResult::SemaError(e) | CompileResult::CodegenError(e) => e,
        CompileResult::Success(..) => panic!("expected a compile error, but it compiled"),
        other => panic!("expected an error, got {other:?}"),
    }
}

/// The `.BYTE` line the compiler emits under a named const array.
fn table_bytes(asm: &str, name: &str) -> Vec<u8> {
    let label = format!("{name}:");
    let line = asm
        .lines()
        .skip_while(|l| l.trim() != label)
        .nth(1)
        .unwrap_or_else(|| panic!("no data line under `{label}`:\n{asm}"));
    line.trim()
        .strip_prefix(".BYTE ")
        .unwrap_or_else(|| panic!("expected a `.BYTE` line under `{label}`, got `{line}`"))
        .split(", ")
        .map(|b| u8::from_str_radix(b.trim().trim_start_matches('$'), 16).unwrap())
        .collect()
}

// ============================================================================
// The bytes are in ROM
// ============================================================================

#[test]
fn a_const_table_is_folded_into_rom_bytes() {
    // The point of the feature: no code runs to build this, the squares are
    // in the image.
    let asm = compile_success(
        r#"
        const SQR: [u8; 16] = [|i| => i * i];
        const OUT: addr = 0x0900;
        #[reset]
        fn main() {
            OUT = SQR[3];
            loop {}
        }
    "#,
    );
    let expected: Vec<u8> = (0u8..16).map(|i| i.wrapping_mul(i)).collect();
    assert_eq!(table_bytes(&asm, "SQR"), expected);
}

#[test]
fn a_u16_table_is_emitted_low_byte_first() {
    // Screen row addresses: the classic reason to want this at all.
    let asm = compile_success(
        r#"
        const ROW: [u16; 4] = [|i| => 0x0400 + (i as u16) * 40];
        const OUT: addr = 0x0900;
        #[reset]
        fn main() {
            OUT = ROW[1].low;
            loop {}
        }
    "#,
    );
    let mut expected = Vec::new();
    for i in 0u16..4 {
        let v = 0x0400 + i * 40;
        expected.push((v & 0xFF) as u8);
        expected.push((v >> 8) as u8);
    }
    assert_eq!(table_bytes(&asm, "ROW"), expected);
}

#[test]
fn a_table_read_with_a_runtime_index_gives_the_folded_entry() {
    let mut e = run(r#"
        const SQR: [u8; 16] = [|i| => i * i];
        const OUT: addr = 0x0900;
        #[reset]
        fn main() {
            let k: u8 = 7;
            OUT = SQR[k];
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0900), 49);
}

// ============================================================================
// The three places a table may be declared
// ============================================================================

#[test]
fn a_static_table_is_written_at_startup() {
    // A `static` is mutable, so its entries are a startup image rather than
    // ROM data — a separate path through the initializer flattener, and the
    // one that would silently write zeroes if it did not know the form.
    let mut e = run(r#"
        static TBL: [u8; 4] = [|i| => i + 1];
        const OUT: addr = 0x0900;
        #[reset]
        fn main() {
            TBL[2] = TBL[2] + 10;
            OUT = TBL[0] + TBL[1] + TBL[2] + TBL[3];
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0900), 1 + 2 + (3 + 10) + 4);
}

#[test]
fn a_local_table_is_stored_into_the_frame() {
    // A local array is built by stores at run time; the values are still the
    // folded ones, so this is the same table with a different delivery.
    let mut e = run(r#"
        const OUT: addr = 0x0900;
        #[reset]
        fn main() {
            let buf: [u8; 4] = [|i| => i * 3];
            OUT = buf[0] + buf[1] + buf[2] + buf[3];
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0900), 0 + 3 + 6 + 9);
}

// ============================================================================
// What the body may say
// ============================================================================

#[test]
fn a_generated_table_matches_the_same_loop_at_run_time() {
    // The whole semantic claim: `i` is a `u8` and the body is the language's
    // ordinary arithmetic, so a table and a loop over the same expression
    // cannot disagree. `i * 100` runs past 255 at i = 3, which is where a
    // constant evaluator that quietly worked in wider integers would show up.
    let mut e = run(r#"
        const T: [u8; 8] = [|i| => i * 100];
        const OUT: addr = 0x0900;
        #[reset]
        fn main() {
            let i: u8 = 0;
            while i < 8 {
                let computed: u8 = i * 100;
                if computed != T[i] {
                    OUT = 0xFF;
                    loop {}
                }
                i = i + 1;
            }
            OUT = 1;
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0900), 1, "a generated table disagreed with the loop");
}

#[test]
fn a_wide_intermediate_needs_a_written_cast() {
    // The other side of that rule: at `u8` the product wraps, so a table of
    // 16-bit results says so. Both are folded, and they differ — which is the
    // reader's call to make, not the compiler's.
    let asm = compile_success(
        r#"
        const NARROW: [u8; 4] = [|i| => i * 100];
        const WIDE: [u16; 4] = [|i| => (i as u16) * 100];
        const OUT: addr = 0x0900;
        #[reset]
        fn main() {
            OUT = NARROW[3] + WIDE[3].low;
            loop {}
        }
    "#,
    );
    assert_eq!(table_bytes(&asm, "NARROW"), vec![0, 100, 200, 44]);
    assert_eq!(
        table_bytes(&asm, "WIDE"),
        vec![0, 0, 100, 0, 200, 0, 44, 1]
    );
}

#[test]
fn the_body_may_name_other_constants() {
    let asm = compile_success(
        r#"
        const STRIDE: u8 = 5;
        const T: [u8; 4] = [|i| => i * STRIDE];
        const OUT: addr = 0x0900;
        #[reset]
        fn main() {
            OUT = T[3];
            loop {}
        }
    "#,
    );
    assert_eq!(table_bytes(&asm, "T"), vec![0, 5, 10, 15]);
}

#[test]
fn a_constant_named_only_by_a_table_body_is_not_reported_unused() {
    // The initializer walker did not descend into a generated table, so a
    // constant used only there was warned about — and, worse, was a candidate
    // for elimination.
    let CompileResult::Success(warnings, _) = compile(
        r#"
        const STRIDE: u8 = 5;
        const T: [u8; 4] = [|i| => i * STRIDE];
        const OUT: addr = 0x0900;
        #[reset]
        fn main() {
            OUT = T[3];
            loop {}
        }
    "#,
    ) else {
        panic!("expected the program to compile");
    };
    assert!(
        !warnings.contains("STRIDE"),
        "`STRIDE` is used by the table body:\n{warnings}"
    );
}

#[test]
fn a_signed_table_folds_with_signed_wrapping() {
    let asm = compile_success(
        r#"
        const T: [i8; 4] = [|i| => 0 - (i as i8)];
        const OUT: addr = 0x0900;
        #[reset]
        fn main() {
            OUT = T[3] as u8;
            loop {}
        }
    "#,
    );
    assert_eq!(table_bytes(&asm, "T"), vec![0x00, 0xFF, 0xFE, 0xFD]);
}

#[test]
fn the_index_parameter_does_not_leak_out_of_the_table() {
    // `i` is bound only while the body is folded. If the binding survived,
    // the next declaration would silently see the last index.
    let err = expect_error(
        r#"
        const T: [u8; 4] = [|i| => i];
        const AFTER: u8 = i;
        const OUT: addr = 0x0900;
        #[reset]
        fn main() {
            OUT = T[0] + AFTER;
            loop {}
        }
    "#,
    );
    assert!(
        err.contains("i"),
        "expected `i` to be undefined after the table:\n{err}"
    );
}

// ============================================================================
// What it refuses
// ============================================================================

#[test]
fn a_table_without_an_array_type_is_rejected() {
    // The length comes from the type, so there has to be one.
    let err = expect_error(
        r#"
        const T: u8 = [|i| => i];
        #[reset]
        fn main() { loop {} }
    "#,
    );
    assert!(
        err.contains("needs an array type"),
        "expected the missing-array-type error, got:\n{err}"
    );
}

#[test]
fn a_table_longer_than_256_entries_is_rejected() {
    // `i` is a `u8`; entry 256 has no index that reaches it.
    let err = expect_error(
        r#"
        const T: [u8; 300] = [|i| => i];
        #[reset]
        fn main() { loop {} }
    "#,
    );
    assert!(
        err.contains("at most 256 entries") && err.contains("300"),
        "expected the length error, got:\n{err}"
    );
}

#[test]
fn a_body_that_reads_run_time_data_is_rejected() {
    // A table is data before the program runs, so there is nothing to read.
    let err = expect_error(
        r#"
        static S: u8 = 3;
        const T: [u8; 4] = [|i| => i + S];
        #[reset]
        fn main() { loop {} }
    "#,
    );
    assert!(
        err.contains("must be a constant expression"),
        "expected the non-constant-body error, got:\n{err}"
    );
}

#[test]
fn a_table_of_a_non_integer_element_type_is_rejected() {
    let err = expect_error(
        r#"
        const T: [bool; 4] = [|i| => true];
        #[reset]
        fn main() { loop {} }
    "#,
    );
    assert!(
        err.contains("must be an integer type"),
        "expected the element-type error, got:\n{err}"
    );
}

#[test]
fn a_body_of_the_wrong_type_is_reported_once_against_the_body() {
    // The body is checked once, with `i` in scope — not 256 times, and not
    // against a synthesised copy the reader never wrote.
    let err = expect_error(
        r#"
        const S: str = "x";
        const T: [u8; 4] = [|i| => S];
        #[reset]
        fn main() { loop {} }
    "#,
    );
    assert!(
        err.contains("u8"),
        "expected a type mismatch against the body, got:\n{err}"
    );
}
