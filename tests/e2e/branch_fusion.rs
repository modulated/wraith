//! Comparisons that drive an `if` or `while` should branch on their own flags,
//! not build a 0/1 in A and test that.
//!
//! `collapse_boolean_compares` in the peephole does exactly that, and the
//! comment calls it "the hottest pattern in the language". But it matched a
//! *one-branch* tail, and only four of the six comparisons end in one. Unsigned
//! `<=` and `>` have no single 6502 branch — `A > m` is `!Z && C` — so they end
//! in two, missed the window, and materialised the boolean:
//!
//! ```text
//!     CMP #$05
//!     BEQ gf         LDA #$00 / JMP / LDA #$01 / CMP #$00 / BNE then
//!     BCS gt         — eleven instructions to decide one branch
//! ```
//!
//! The two shapes differ, which is why this needed two matchers rather than
//! one. `>` sends its branches to *different* labels (equal → false, carry →
//! true), so the false paths converge on a kept label; `<=` is `Z || !C`, so
//! both branches go to the *same* true label and the false path is the
//! fall-through.
//!
//! These are behavioural tests. The peephole rewrites a *test* into a branch,
//! and the previous bug in this pass — a boolean read after it had been
//! optimised away — was invisible in the assembly and obvious in the answer.

use crate::common::exec::run;

/// Run `cond` as an `if` over `x`, reporting 1 when the branch was taken.
fn taken(ty: &str, x: &str, cond: &str) -> u8 {
    run(&format!(
        "const OUT: addr = 0x0900;\n\
         static N: u8 = 0;\n\
         #[reset]\nfn main() {{\n\
         \x20   let x: {ty} = {x};\n\
         \x20   if {cond} {{ N = 1; }}\n\
         \x20   OUT = N;\n    loop {{}}\n}}\n"
    ))
    .mem(0x0900)
}

#[test]
fn unsigned_greater_than_decides_at_every_boundary() {
    // The value either side of the constant, and the constant itself: a
    // collapse that drops or inverts one branch shows up at exactly one of
    // these three.
    assert_eq!(taken("u8", "4", "x > 5"), 0, "4 > 5");
    assert_eq!(taken("u8", "5", "x > 5"), 0, "5 > 5");
    assert_eq!(taken("u8", "6", "x > 5"), 1, "6 > 5");
}

#[test]
fn unsigned_less_or_equal_decides_at_every_boundary() {
    assert_eq!(taken("u8", "4", "x <= 5"), 1, "4 <= 5");
    assert_eq!(taken("u8", "5", "x <= 5"), 1, "5 <= 5");
    assert_eq!(taken("u8", "6", "x <= 5"), 0, "6 <= 5");
}

#[test]
fn sixteen_bit_comparisons_decide_at_every_boundary() {
    for (x, gt, le) in [(299u16, 0, 1), (300, 0, 1), (301, 1, 0)] {
        assert_eq!(taken("u16", &x.to_string(), "x > 300"), gt, "{x} > 300");
        assert_eq!(taken("u16", &x.to_string(), "x <= 300"), le, "{x} <= 300");
    }
}

#[test]
fn signed_comparisons_are_unaffected() {
    // Signed comparisons already ended in a single branch and took the older
    // path; they must keep answering the same.
    assert_eq!(taken("i8", "(-1)", "x > 0"), 0);
    assert_eq!(taken("i8", "1", "x > 0"), 1);
    assert_eq!(taken("i8", "(-1)", "x <= 0"), 1);
    assert_eq!(taken("i16", "(-300)", "x > 0"), 0);
    assert_eq!(taken("i16", "300", "x > 0"), 1);
}

#[test]
fn a_while_loop_on_a_two_branch_comparison_terminates_correctly() {
    // A loop is where getting the sense backwards is fatal rather than merely
    // wrong: it runs zero times or forever. The count pins which.
    let mut e = run(r#"
        const OUT: addr = 0x0900;
        static N: u8 = 0;
        #[reset]
        fn main() {
            let x: u8 = 5;
            while x > 2 { N = N + 1; x = x - 1; }
            OUT = N;
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0900), 3, "5, 4, 3 pass the test; 2 does not");
}

#[test]
fn the_boolean_is_still_a_value_where_one_is_needed() {
    // The rewrite is only safe when nothing reads the boolean afterwards. Bound
    // to a variable, returned, and combined — the value has to survive.
    let mut e = run(r#"
        const OUT0: addr = 0x0900;
        const OUT1: addr = 0x0901;
        const OUT2: addr = 0x0902;
        static A: u8 = 0;
        static B: u8 = 0;
        static C: u8 = 0;
        fn gt(a: u8, b: u8) -> bool { return a > b; }
        #[reset]
        fn main() {
            let x: u8 = 7;
            let c: bool = x > 5;
            A = c as u8;
            B = gt(3, 9) as u8;
            if x > 5 && x <= 9 { C = 1; }
            OUT0 = A; OUT1 = B; OUT2 = C;
            loop {}
        }
    "#);
    assert_eq!(
        (e.mem(0x0900), e.mem(0x0901), e.mem(0x0902)),
        (1, 0, 1),
        "a bound, a returned and a combined comparison all keep their value"
    );
}

#[test]
fn both_operands_of_a_conjunction_are_two_branch_comparisons() {
    // Two of these back to back is where a rewrite that leaves the wrong flags
    // behind shows up: the second test reads what the first left.
    for (x, want) in [(1u8, 0), (3, 1), (5, 1), (7, 0)] {
        let got = run(&format!(
            "const OUT: addr = 0x0900;\n\
             static N: u8 = 0;\n\
             #[reset]\nfn main() {{\n\
             \x20   let x: u8 = {x};\n\
             \x20   if x > 2 && x <= 5 {{ N = 1; }}\n\
             \x20   OUT = N;\n    loop {{}}\n}}\n"
        ))
        .mem(0x0900);
        assert_eq!(got, want, "x = {x}");
    }
}

/// The overflow flag is what let the guard relax, so it needs pinning.
///
/// Everything the collapse deletes — a `CMP #$00`, two `LDA`s and a `JMP` —
/// writes N, Z and C and never V. Requiring V dead as well refused 48 of the
/// 225 candidate sites in the example corpus for a property the rewrite cannot
/// affect. It is excluded from the guard now, so a program that reads `V`
/// across a fused comparison has to still see the right bit.
#[test]
fn overflow_survives_a_fused_comparison() {
    let mut e = run(r#"
        const OUT0: addr = 0x0900;
        const OUT1: addr = 0x0901;
        static A: u8 = 0;
        static B: u8 = 0;
        #[reset]
        fn main() {
            // 100 + 100 overflows a signed byte, so V comes out set.
            let x: i8 = 100;
            let y: i8 = x + 100;
            let v: bool = overflow;
            // A comparison the collapse rewrites, between setting V and
            // reading it. It writes N/Z/C and must leave V alone.
            let n: u8 = 7;
            if n > 5 { A = 1; }
            OUT0 = A;
            OUT1 = v as u8;
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0900), 1, "the comparison still decided correctly");
    assert_eq!(e.mem(0x0901), 1, "and the overflow flag survived it");
}

/// The collapse fires inside a *standalone* function, not only when the loop is
/// inlined into `main`. A void function ends in `RTS`, and the liveness that
/// guards the collapse used to treat `RTS` as reading the flags and the
/// accumulator — as if a caller took them as a return value. A value returns in
/// a register, never in a flag, and a *void* function returns nothing at all, so
/// the throwaway boolean of a condition looked live all the way to the return
/// and the collapse was refused. The same loop inlined (one call site) collapsed
/// while the standalone form (two call sites) did not — a difference of layout,
/// not of meaning.
#[test]
fn a_condition_collapses_inside_a_standalone_function() {
    let asm = crate::common::harness::compile_success(
        "const OUT: addr = 0x0900;\n\
         const COLS: u8 = 80;\n\
         fn helper(v: u8) -> u8 { OUT = v; return v; }\n\
         fn loop_in_fn(start: u8) {\n\
         \x20   let c: u8 = start;\n\
         \x20   while c < COLS { OUT = helper(c); c = c + 1; }\n\
         }\n\
         #[reset]\n\
         fn main() { loop_in_fn(0); loop_in_fn(3); loop {} }\n",
    );
    let body = asm
        .split("loop_in_fn:")
        .nth(1)
        .expect("loop_in_fn emitted standalone")
        .split("\nmain:")
        .next()
        .unwrap();
    // A collapsed condition branches on its own flags; a materialised one builds
    // a 0/1 in A and re-tests it.
    assert!(
        !body.contains("LDA #$01"),
        "the loop condition still materialises a boolean:\n{body}"
    );
    assert!(
        !body.contains("CMP #$00"),
        "the loop condition still re-tests a materialised boolean:\n{body}"
    );
}

/// The behavioural half: the loop that now collapses still runs the right number
/// of times. `count_below` returns how many steps `0, 1, 2, …` takes to reach
/// its bound, computed by a standalone function whose `while` is the collapsed
/// site.
#[test]
fn the_collapsed_standalone_loop_still_counts_correctly() {
    let mut e = run("const OUT: addr = 0x0900;\n\
         static N: u8 = 0;\n\
         fn count_below(bound: u8) {\n\
         \x20   let c: u8 = 0;\n\
         \x20   while c < bound { N = N + 1; c = c + 1; }\n\
         }\n\
         #[reset]\n\
         fn main() { count_below(9); count_below(4); OUT = N; loop {} }\n");
    assert_eq!(e.mem(0x0900), 13, "9 + 4 iterations");
}
