//! Constant folding must agree with the code it replaces.
//!
//! Folding ran in `i64` and truncated once at the end, while generated code
//! wraps at the type's width after *every* operation. Whenever an intermediate
//! left the type's range the two disagreed, so a constant expression and the
//! identical expression written with a variable produced different answers —
//! silently, since both compiled.
//!
//! Found by `tests/fuzz_exec.rs` on its first run.

use crate::common::exec::run;

/// Compute `expr` twice — once folded, once through a variable that forces the
/// runtime path — and require them to match. `v` is bound to `seed`.
fn both_ways(ty: &str, seed: &str, folded: &str, runtime: &str) -> (u32, u32) {
    let (store, read16) = if ty == "u16" {
        (
            "let a: u16 = FOLDED; let b: u16 = RUNTIME; \
             OUT = a.low; OUT1 = a.high; OUT2 = b.low; OUT3 = b.high;",
            true,
        )
    } else {
        ("OUT = FOLDED; OUT2 = RUNTIME;", false)
    };
    let body = store.replace("FOLDED", folded).replace("RUNTIME", runtime);
    let mut e = run(&format!(
        "const OUT: addr = 0x0900;\nconst OUT1: addr = 0x0901;\n\
         const OUT2: addr = 0x0902;\nconst OUT3: addr = 0x0903;\n\
         #[reset]\nfn main() {{ let v: {ty} = {seed}; {body} loop {{}} }}\n"
    ));
    if read16 {
        (e.mem16(0x0900) as u32, e.mem16(0x0902) as u32)
    } else {
        (e.mem(0x0900) as u32, e.mem(0x0902) as u32)
    }
}

/// The case the fuzzer found: a shift whose intermediate leaves `u8`.
#[test]
fn a_shift_wraps_before_the_next_operation() {
    let (folded, runtime) = both_ways("u8", "94", "(94 << 6) >> 3", "(v << 6) >> 3");
    assert_eq!(folded, runtime, "folded and runtime forms must agree");
    assert_eq!(folded, 16, "94 << 6 wraps to 128, then >> 3 is 16");
}

/// Multiplication has the same shape — the intermediate exceeds the width.
#[test]
fn a_product_wraps_before_the_next_operation() {
    let (folded, runtime) = both_ways("u8", "200", "(200 * 2) / 4", "(v * 2) / 4");
    assert_eq!(folded, runtime);
    assert_eq!(folded, 36, "200 * 2 wraps to 144, then / 4 is 36");
}

#[test]
fn a_sum_wraps_before_the_next_operation() {
    let (folded, runtime) = both_ways("u8", "200", "(200 + 100) / 2", "(v + 100) / 2");
    assert_eq!(folded, runtime);
    assert_eq!(folded, 22, "200 + 100 wraps to 44, then / 2 is 22");
}

/// u16 has the same rule at its own width.
#[test]
fn u16_intermediates_wrap_at_sixteen_bits() {
    let (folded, runtime) = both_ways("u16", "60000", "(60000 * 2) / 4", "(v * 2) / 4");
    assert_eq!(folded, runtime);
    // 60000 * 2 = 120000, which wraps to 54464; / 4 is 13616.
    assert_eq!(folded, 13616);
}

/// The spec's own wrapping examples, which already worked, pinned so the
/// narrowing cannot regress them.
#[test]
fn the_documented_wrapping_examples_still_hold() {
    let (folded, runtime) = both_ways("u8", "255", "255 + 1", "v + 1");
    assert_eq!((folded, runtime), (0, 0), "255 + 1 wraps to 0");

    let (folded, runtime) = both_ways("u8", "200", "200 * 2", "v * 2");
    assert_eq!((folded, runtime), (144, 144), "200 * 2 is 400 % 256");
}

/// A shift at or past the width clears the value, folded or not.
#[test]
fn a_shift_past_the_width_is_zero_either_way() {
    let (folded, runtime) = both_ways("u8", "1", "1 << 8", "v << 8");
    assert_eq!((folded, runtime), (0, 0));
}

/// Deeper chains, where several intermediates leave the range in turn.
#[test]
fn a_chain_of_wrapping_intermediates_agrees() {
    let (folded, runtime) = both_ways(
        "u8",
        "20",
        "(((20 << 7) / 61) - 172) << 3",
        "(((v << 7) / 61) - 172) << 3",
    );
    assert_eq!(folded, runtime);
    assert_eq!(folded, 160);
}
