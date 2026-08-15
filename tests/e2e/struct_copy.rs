//! Whole-struct copies: `let q: P = p;`, `q = p;`, and `f(p)` where `p` names
//! storage rather than producing it.
//!
//! Binding and assignment used to have cases for a struct *literal* and a
//! struct-returning *call* and nothing else, so a struct-valued place fell
//! through to the scalar path: one `LDA`/`STA`, and an array index scaled by 1
//! instead of the element size. `let q: P = PS[1]` bound `PS[0].y` into `q.x`
//! and left every later field zero. It compiled without a word.
//!
//! Passing one to a function had the same hole at a different site — the
//! pass-by-reference path matched a zero-page local and nothing else, so a
//! `static`, a nested field and an array element staged one byte of the
//! struct's *contents* as though it were the pointer.
//!
//! Two conventions meet here and the tests keep them apart:
//!
//!   * **binding and assignment copy.** `let q: P = p` gives `q` its own bytes;
//!     writing `q` must leave `p` alone.
//!   * **arguments are by reference.** `f(p)` hands over the address, so the
//!     callee's field writes land on the caller's struct. That is what the
//!     specification says, and changing the argument path must not change it.
//!
//! Found while working out how to add a device to `examples/device_drivers.wr`,
//! where the natural spelling of registration is `CONSOLE = DRIVERS[id]`.

use crate::common::exec::run;

/// Three-byte structs, so a copy exercises the loop form rather than the
/// unrolled one, and a wrong element stride is visible in every field.
const PRE: &str = r#"
    const OUT0: addr = 0x0900;
    const OUT1: addr = 0x0901;
    const OUT2: addr = 0x0902;
    struct P { x: u8, y: u8, z: u8 }
    static PS: [P; 3] = [
        P { x: 1, y: 2, z: 3 },
        P { x: 4, y: 5, z: 6 },
        P { x: 7, y: 8, z: 9 },
    ];
    const CPS: [P; 2] = [P { x: 10, y: 11, z: 12 }, P { x: 13, y: 14, z: 15 }];
    static S: P = P { x: 21, y: 22, z: 23 };
"#;

/// Run `body`, which must leave a `P` named `q`, and report its three fields.
fn bind(body: &str) -> (u8, u8, u8) {
    let mut e = run(&format!(
        "{PRE}#[reset]\nfn main() {{\n{body}\n\
         \x20   OUT0 = q.x; OUT1 = q.y; OUT2 = q.z;\n    loop {{}}\n}}\n"
    ));
    (e.mem(0x0900), e.mem(0x0901), e.mem(0x0902))
}

/// As [`bind`], with `q` already declared so `body` can assign to it.
fn assign(body: &str) -> (u8, u8, u8) {
    bind(&format!("    let q: P = P {{ x: 0, y: 0, z: 0 }};\n{body}"))
}

// ---------------------------------------------------------------------------
// Binding: `let q: P = <place>;`
// ---------------------------------------------------------------------------

#[test]
fn bind_from_a_local() {
    assert_eq!(
        bind("    let a: P = P { x: 4, y: 5, z: 6 };\n    let q: P = a;"),
        (4, 5, 6)
    );
}

#[test]
fn bind_from_a_static() {
    assert_eq!(bind("    let q: P = S;"), (21, 22, 23));
}

#[test]
fn bind_from_a_constant_index() {
    assert_eq!(bind("    let q: P = PS[2];"), (7, 8, 9));
}

#[test]
fn bind_from_a_runtime_index() {
    // The shape that gave this file its name: before the fix, (2, 0, 0).
    assert_eq!(bind("    let i: u8 = 1;\n    let q: P = PS[i];"), (4, 5, 6));
}

#[test]
fn bind_from_a_const_table_at_a_runtime_index() {
    // A `const` table lives in ROM under an assembler label, so its address is
    // `#<label`/`#>label` rather than a number to do arithmetic on.
    assert_eq!(
        bind("    let i: u8 = 1;\n    let q: P = CPS[i];"),
        (13, 14, 15)
    );
}

#[test]
fn bind_from_a_nested_field() {
    let mut e = run(r#"
        const OUT0: addr = 0x0900;
        const OUT1: addr = 0x0901;
        struct Inner { a: u8, b: u8 }
        struct Outer { tag: u8, inner: Inner }
        static O: Outer = Outer { tag: 9, inner: Inner { a: 3, b: 4 } };
        #[reset]
        fn main() {
            let q: Inner = O.inner;
            OUT0 = q.a;
            OUT1 = q.b;
            loop {}
        }
    "#);
    assert_eq!((e.mem(0x0900), e.mem(0x0901)), (3, 4));
}

#[test]
fn bind_from_a_by_reference_parameter() {
    // A struct parameter's slot holds a pointer to the caller's storage, so
    // the source address is a load rather than a constant.
    let mut e = run(r#"
        const OUT0: addr = 0x0900;
        const OUT1: addr = 0x0901;
        struct P { x: u8, y: u8 }
        static SEEN: u8 = 0;
        fn take(p: P) {
            let q: P = p;
            SEEN = q.x + q.y;
        }
        #[reset]
        fn main() {
            let a: P = P { x: 30, y: 12 };
            take(a);
            OUT0 = SEEN;
            OUT1 = a.x;
            loop {}
        }
    "#);
    assert_eq!((e.mem(0x0900), e.mem(0x0901)), (42, 30));
}

#[test]
fn the_bound_copy_is_independent_of_its_source() {
    // The copy convention itself: `q` owns its bytes.
    let mut e = run(r#"
        const OUT0: addr = 0x0900;
        const OUT1: addr = 0x0901;
        struct P { x: u8, y: u8 }
        static S: P = P { x: 1, y: 2 };
        #[reset]
        fn main() {
            let q: P = S;
            q.x = 99;
            OUT0 = q.x;
            OUT1 = S.x;
            loop {}
        }
    "#);
    assert_eq!(
        (e.mem(0x0900), e.mem(0x0901)),
        (99, 1),
        "writing the copy must not reach the source"
    );
}

// ---------------------------------------------------------------------------
// Assignment: `<place> = <place>;`
// ---------------------------------------------------------------------------

#[test]
fn assign_from_a_static() {
    assert_eq!(assign("    q = S;"), (21, 22, 23));
}

#[test]
fn assign_from_a_runtime_index() {
    assert_eq!(assign("    let i: u8 = 2;\n    q = PS[i];"), (7, 8, 9));
}

#[test]
fn assign_into_a_static() {
    let mut e = run(&format!(
        "{PRE}#[reset]\nfn main() {{\n\
         \x20   let i: u8 = 1;\n\
         \x20   S = PS[i];\n\
         \x20   OUT0 = S.x; OUT1 = S.y; OUT2 = S.z;\n\
         \x20   loop {{}}\n}}\n"
    ));
    assert_eq!((e.mem(0x0900), e.mem(0x0901), e.mem(0x0902)), (4, 5, 6));
}

#[test]
fn assign_into_a_runtime_index() {
    // A destination with no address until run time: both ends go through
    // pointers and the copy is indirect on each.
    let mut e = run(&format!(
        "{PRE}#[reset]\nfn main() {{\n\
         \x20   let i: u8 = 2;\n\
         \x20   PS[i] = S;\n\
         \x20   OUT0 = PS[2].x; OUT1 = PS[2].y; OUT2 = PS[2].z;\n\
         \x20   loop {{}}\n}}\n"
    ));
    assert_eq!((e.mem(0x0900), e.mem(0x0901), e.mem(0x0902)), (21, 22, 23));
}

#[test]
fn assign_between_two_runtime_indices() {
    let mut e = run(&format!(
        "{PRE}#[reset]\nfn main() {{\n\
         \x20   let i: u8 = 0;\n\
         \x20   let j: u8 = 2;\n\
         \x20   PS[i] = PS[j];\n\
         \x20   OUT0 = PS[0].x; OUT1 = PS[0].y; OUT2 = PS[0].z;\n\
         \x20   loop {{}}\n}}\n"
    ));
    assert_eq!((e.mem(0x0900), e.mem(0x0901), e.mem(0x0902)), (7, 8, 9));
}

#[test]
fn self_assignment_is_identity() {
    // Source and destination are the same bytes; a forward copy is a no-op
    // rather than a smear.
    let mut e = run(&format!(
        "{PRE}#[reset]\nfn main() {{\n\
         \x20   let i: u8 = 1;\n\
         \x20   PS[i] = PS[i];\n\
         \x20   OUT0 = PS[1].x; OUT1 = PS[1].y; OUT2 = PS[1].z;\n\
         \x20   loop {{}}\n}}\n"
    ));
    assert_eq!((e.mem(0x0900), e.mem(0x0901), e.mem(0x0902)), (4, 5, 6));
}

#[test]
fn writing_to_a_const_table_is_still_rejected() {
    crate::common::assert_error_contains(
        &format!("{PRE}#[reset]\nfn main() {{ let i: u8 = 0; CPS[i] = S; loop {{}} }}\n"),
        "a const lives in ROM",
    );
}

// ---------------------------------------------------------------------------
// Arguments: still by reference, from every place.
// ---------------------------------------------------------------------------

const ARGS: &str = r#"
    const OUT0: addr = 0x0900;
    const OUT1: addr = 0x0901;
    struct P { x: u8, y: u8 }
    struct Outer { tag: u8, inner: P }
    static PS: [P; 3] = [P { x: 1, y: 2 }, P { x: 4, y: 5 }, P { x: 7, y: 8 }];
    static O: Outer = Outer { tag: 9, inner: P { x: 30, y: 12 } };
    static S: P = P { x: 20, y: 22 };
    fn sum(p: P) -> u8 { return p.x + p.y; }
    fn bump(p: P) { p.x = p.x + 100; }
"#;

fn arg_sum(expr: &str) -> u8 {
    run(&format!(
        "{ARGS}#[reset]\nfn main() {{ let i: u8 = 1; OUT0 = sum({expr}); loop {{}} }}\n"
    ))
    .mem(0x0900)
}

#[test]
fn struct_argument_from_a_static() {
    assert_eq!(arg_sum("S"), 42);
}

#[test]
fn struct_argument_from_a_constant_index() {
    assert_eq!(arg_sum("PS[2]"), 15);
}

#[test]
fn struct_argument_from_a_runtime_index() {
    assert_eq!(arg_sum("PS[i]"), 9);
}

#[test]
fn struct_argument_from_a_nested_field() {
    assert_eq!(arg_sum("O.inner"), 42);
}

#[test]
fn a_struct_argument_is_still_passed_by_reference() {
    // The convention the specification states: the callee writes through to
    // the caller's storage. Widening which places can be passed must not turn
    // an argument into a copy.
    let mut e = run(&format!(
        "{ARGS}#[reset]\nfn main() {{\n\
         \x20   let i: u8 = 1;\n\
         \x20   bump(PS[i]);\n\
         \x20   bump(S);\n\
         \x20   OUT0 = PS[1].x;\n\
         \x20   OUT1 = S.x;\n\
         \x20   loop {{}}\n}}\n"
    ));
    assert_eq!(
        (e.mem(0x0900), e.mem(0x0901)),
        (104, 120),
        "the callee's writes must land on the caller's struct"
    );
}
