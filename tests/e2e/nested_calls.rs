//! A call inside another call's argument list.
//!
//! Arguments are staged before the JSR — into the callee's parameter slots for
//! an inlined call, into a fixed zero-page pool for an ordinary one — and both
//! kinds of storage were reused by a nested callee, which destroyed the
//! arguments already sitting there. Every case below returned a *different
//! argument* than the one asked for, and compiled without complaint.
//!
//! Found by `tests/fuzz_exec.rs` once it started generating calls. Three
//! distinct causes, all with the same symptom:
//!
//! 1. Frame colouring gave two functions the same parameter addresses because
//!    neither calls the other — true of the call graph, false of their
//!    lifetimes when one is invoked while the other's parameters are being
//!    filled in.
//! 2. The argument-staging pool is at a fixed address and its allocator resets
//!    per function, so a callee stages over the caller's staged arguments.
//! 3. An inlined call stores straight into parameter slots, and when the nested
//!    call is to the *same* function those are literally the same bytes — no
//!    colouring can separate them.

use crate::common::exec::run;

/// Read the first `n` output bytes of a program.
fn outs(src: &str, n: u16) -> Vec<u8> {
    let head: String = (0..n)
        .map(|i| format!("const OUT{i}: addr = 0x{:04X};\n", 0x0900 + i))
        .collect();
    let mut e = run(&format!("{head}{src}"));
    (0..n).map(|i| e.mem(0x0900 + i)).collect()
}

/// The callee returns the argument at the named position, so a wrong answer
/// names the argument that overwrote it.
const PICKERS: &str = "\
fn pick3a(a: u8, b: u8, c: u8) -> u8 { return a; }
fn pick3b(a: u8, b: u8, c: u8) -> u8 { return b; }
fn pick3c(a: u8, b: u8, c: u8) -> u8 { return c; }
fn pick2a(a: u8, b: u8) -> u8 { return a; }
fn pick2b(a: u8, b: u8) -> u8 { return b; }
";

#[test]
fn a_nested_call_leaves_the_outer_arguments_alone() {
    let got = outs(
        &format!(
            "{PICKERS}#[reset]\nfn main() {{\n\
             \x20   let v: u8 = 7;\n\
             \x20   OUT0 = pick3a(v, pick2a(200, 201), v);\n\
             \x20   OUT1 = pick3b(v, v, pick2a(200, 201));\n\
             \x20   OUT2 = pick3c(pick2a(200, 201), v, v);\n\
             \x20   OUT3 = pick2a(v, pick3a(200, 201, 202));\n\
             \x20   loop {{}}\n}}\n"
        ),
        4,
    );
    assert_eq!(got, vec![7, 7, 7, 7], "each outer argument must survive");
}

/// The nested call in the *last* argument is the dangerous position: every
/// earlier argument is already staged when it runs.
#[test]
fn every_earlier_argument_survives_a_call_in_the_last_one() {
    let got = outs(
        &format!(
            "{PICKERS}#[reset]\nfn main() {{\n\
             \x20   OUT0 = pick3a(11, 22, pick2a(200, 201));\n\
             \x20   OUT1 = pick3b(11, 22, pick2a(200, 201));\n\
             \x20   OUT2 = pick3c(11, 22, pick2b(200, 201));\n\
             \x20   loop {{}}\n}}\n"
        ),
        3,
    );
    assert_eq!(got, vec![11, 22, 201]);
}

/// Two nested calls in one argument list: the shelter has to nest, not just
/// happen once.
#[test]
fn two_nested_calls_in_one_argument_list() {
    let got = outs(
        &format!(
            "{PICKERS}#[reset]\nfn main() {{\n\
             \x20   OUT0 = pick3a(11, pick2a(200, 201), pick2a(100, 101));\n\
             \x20   OUT1 = pick3b(11, pick2a(200, 201), pick2a(100, 101));\n\
             \x20   OUT2 = pick3c(11, pick2a(200, 201), pick2a(100, 101));\n\
             \x20   loop {{}}\n}}\n"
        ),
        3,
    );
    assert_eq!(got, vec![11, 200, 100]);
}

/// A function nested in its *own* argument list. The inlined form stores into
/// the one set of parameter slots, so this is the case no frame colouring can
/// fix — the storage is the same by construction.
#[test]
fn a_call_nested_in_its_own_argument_list() {
    let got = outs(
        &format!(
            "{PICKERS}#[reset]\nfn main() {{\n\
             \x20   let v: u8 = 3;\n\
             \x20   OUT0 = pick3a(0, v, pick3a(12, v, v));\n\
             \x20   OUT1 = pick3b(0, v, pick3a(12, v, v));\n\
             \x20   OUT2 = pick3c(0, v, pick3a(12, v, v));\n\
             \x20   OUT3 = pick2b(7, pick2b(8, 9));\n\
             \x20   loop {{}}\n}}\n"
        ),
        4,
    );
    assert_eq!(got, vec![0, 3, 12, 9]);
}

/// 16-bit arguments take two staging bytes each, so they exercise the same
/// paths at a different width.
#[test]
fn wide_arguments_survive_a_nested_call() {
    let mut e = run("const OUT0: addr = 0x0900;\nconst OUT1: addr = 0x0901;\n\
         fn w3a(a: u16, b: u16, c: u16) -> u16 { return a; }\n\
         fn w2a(a: u16, b: u16) -> u16 { return a; }\n\
         #[reset]\nfn main() {\n\
         \x20   let v: u16 = 1000;\n\
         \x20   let r: u16 = w3a(v, w2a(40000, 40001), v);\n\
         \x20   OUT0 = r.low;\n    OUT1 = r.high;\n    loop {}\n}\n");
    assert_eq!(e.mem16(0x0900), 1000);
}

/// A call in an argument that also does arithmetic: the operand spilled across
/// the JSR and the staged arguments share the software stack, so they have to
/// nest correctly with each other.
#[test]
fn arithmetic_around_a_nested_call_still_agrees() {
    let got = outs(
        &format!(
            "{PICKERS}#[reset]\nfn main() {{\n\
             \x20   let v: u8 = 5;\n\
             \x20   OUT0 = pick3a(v + 1, (v * 3) + pick2a(200, 201), v);\n\
             \x20   OUT1 = pick3b(v + 1, (v * 3) + pick2a(2, 3), v);\n\
             \x20   loop {{}}\n}}\n"
        ),
        2,
    );
    // 5*3 + 200 wraps to 215 for OUT1's sibling; OUT0 asks for the first
    // argument, which is 6, and OUT1 for 15 + 2 = 17.
    assert_eq!(got, vec![6, 17]);
}

/// Recursion through an argument list: the callee re-enters while its own
/// parameters are half-written, which is the same hazard one level deeper.
#[test]
fn recursion_through_an_argument_list() {
    let mut e = run("const OUT0: addr = 0x0900;\n\
         fn countdown(d: u8, acc: u8) -> u8 {\n\
         \x20   if d == 0 { return acc; }\n\
         \x20   return countdown(d - 1, acc + d);\n\
         }\n\
         fn pick2a(a: u8, b: u8) -> u8 { return a; }\n\
         #[reset]\nfn main() {\n\
         \x20   OUT0 = pick2a(42, countdown(10, 0));\n\
         \x20   loop {}\n}\n");
    assert_eq!(e.mem(0x0900), 42, "the first argument must survive");
}

/// More arguments than the fixed staging pool holds. Four 16-bit arguments
/// nested inside four more is sixteen bytes against the pool's eleven, so this
/// used to be a compile error naming a workaround. It now stages on the
/// software stack instead — one argument at a time, each parked as soon as it
/// is evaluated — and the depth is bounded by that stack rather than by the
/// pool. Every argument still has to arrive intact, which is what the weights
/// below check: a wrong slot shows up as a wrong multiple.
#[test]
fn arguments_beyond_the_staging_pool_still_arrive() {
    // `side` keeps `w` non-leaf so the inliner leaves it alone; only a real
    // call stages arguments.
    let src = "const LO: addr = 0x0900;\nconst HI: addr = 0x0901;\n\
               fn side(x: u16) -> u16 { return x + 1; }\n\
               fn w(a: u16, b: u16, c: u16, d: u16) -> u16 {\n\
               \x20   return a + (b * 3) + (c * 7) + (d * 11) + side(0);\n\
               }\n\
               #[reset]\nfn main() {\n\
               \x20   let r: u16 = w(1, 2, 3, w(4, 5, 6, 7));\n\
               \x20   LO = r.low;\n    HI = r.high;\n    loop {}\n}\n";
    let mut e = crate::common::exec::run(src);
    // inner: 4 + 15 + 42 + 77 + 1 = 139
    // outer: 1 + 6 + 21 + 1529 + 1 = 1558
    assert_eq!(e.mem16(0x0900), 1558);
}

/// The same overflow with a *recursive* callee. The frame save and the
/// arguments share one software stack, so with stack staging the save has to
/// happen before the arguments go on or it would bury them.
#[test]
fn stack_staged_arguments_survive_a_recursive_callee() {
    let mut e = crate::common::exec::run(
        "const LO: addr = 0x0900;\nconst HI: addr = 0x0901;\n\
         fn sum(d: u16, a: u16, b: u16, c: u16) -> u16 {\n\
         \x20   if d == 0 { return a + (b * 3) + (c * 7); }\n\
         \x20   return sum(d - 1, a, b, c) + 1;\n\
         }\n\
         #[reset]\nfn main() {\n\
         \x20   let r: u16 = sum(2, 1, 2, sum(1, 4, 5, 6));\n\
         \x20   LO = r.low;\n    HI = r.high;\n    loop {}\n}\n",
    );
    // inner sum(1,4,5,6): base 4 + 15 + 42 = 61, one level of +1 => 62
    // outer sum(2,1,2,62): base 1 + 6 + 434 = 441, two levels of +1 => 443
    assert_eq!(e.mem16(0x0900), 443);
}

/// A bit test whose object is a call, sitting in a later argument. `contains_call`
/// decides whether the earlier arguments get sheltered, and its `BitOp` arm used
/// to be covered by a catch-all that answered "no call". This shape already
/// worked — the answer was right before the arm was written — so it is pinned
/// rather than fixed: the arm now says so explicitly instead of by omission.
#[test]
fn a_bit_test_on_a_call_result_in_a_later_argument() {
    let got = outs(
        &format!(
            "{PICKERS}fn eight() -> u8 {{ return 8; }}\n\
             #[reset]\nfn main() {{\n\
             \x20   OUT0 = pick2a(200, eight().bit(3) as u8);\n\
             \x20   OUT1 = pick2b(200, eight().bit(3) as u8);\n\
             \x20   loop {{}}\n}}\n"
        ),
        2,
    );
    assert_eq!(got, vec![200, 1], "bit 3 of 8 is set, and 200 survives");
}

/// Arguments that are plain expressions — no call among them — go straight into
/// the callee's frame, skipping the argument pool. Each must still land in its
/// own slot; this passes three expressions of mixed width and reads every
/// parameter back.
#[test]
fn expression_arguments_reach_their_slots_directly() {
    let mut e = crate::common::exec::run(
        "const P: addr = 0x0900;\n\
         const Q0: addr = 0x0901;\n\
         const Q1: addr = 0x0902;\n\
         const R: addr = 0x0903;\n\
         fn take(p: u8, q: u16, r: u8) { P = p; Q0 = q.low; Q1 = q.high; R = r; }\n\
         #[reset]\n\
         fn main() {\n\
         \x20   let a: u8 = 5; let b: u16 = 1000; let c: u8 = 4;\n\
         \x20   take(a + 1, b + 7, c * 3);\n\
         \x20   take(a + 1, b + 7, c * 3);\n\
         \x20   loop {}\n\
         }",
    );
    assert_eq!(e.mem(0x0900), 6, "p = 5 + 1");
    assert_eq!(
        ((e.mem(0x0902) as u16) << 8) | e.mem(0x0901) as u16,
        1007,
        "q = 1000 + 7"
    );
    assert_eq!(e.mem(0x0903), 12, "r = 4 * 3");
}

/// The staging itself: a call whose arguments contain no call touches none of
/// the `$F4`-`$FE` argument pool — the values are produced straight into the
/// callee's frame. Two call sites keep `take` out of line so a real `JSR` with
/// staging is emitted.
#[test]
fn a_call_with_expression_arguments_skips_the_pool() {
    let asm = crate::common::harness::compile_success(
        "const OUT: addr = 0x0900;\n\
         fn take(p: u8, q: u8) -> u8 { OUT = p; return p + q; }\n\
         #[reset]\n\
         fn main() {\n\
         \x20   let a: u8 = 5;\n\
         \x20   let x: u8 = take(a + 1, a * 2);\n\
         \x20   let y: u8 = take(a + 2, a * 3);\n\
         \x20   OUT = x + y;\n\
         \x20   loop {}\n\
         }",
    );
    let main = asm.split("main:").nth(1).expect("main emitted");
    let main = main.split("\n; Function").next().unwrap_or(main);
    for pool in ["$F4", "$F5", "$F6", "$F7", "$F8"] {
        assert!(
            !main.contains(pool),
            "an expression argument was staged through the pool `{pool}`:\n{main}"
        );
    }
    assert!(
        main.contains("JSR take"),
        "the call was inlined away:\n{main}"
    );
}
