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

/// The staging pool is finite, and a nested call needs room for both argument
/// lists at once. Exceeding it must fail loudly at compile time rather than
/// emit code that reuses the bytes — the failure this whole file is about.
#[test]
fn exhausting_the_staging_pool_is_a_compile_error() {
    // `side` keeps `w` non-leaf so the inliner leaves it alone; only a real
    // call stages arguments through the pool.
    let src = "const OUT0: addr = 0x0900;\n\
               fn side(x: u16) -> u16 { return x + 1; }\n\
               fn w(a: u16, b: u16, c: u16, d: u16) -> u16 { return a + side(b); }\n\
               #[reset]\nfn main() {\n\
               \x20   let v: u16 = 1;\n\
               \x20   let r: u16 = w(v, v, v, w(v, v, v, v));\n\
               \x20   OUT0 = r.low;\n    loop {}\n}\n";
    crate::common::assert_error_contains(src, "argument-evaluation pool exhausted");
    crate::common::assert_error_contains(src, "Bind the inner call to a `let` first");
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
