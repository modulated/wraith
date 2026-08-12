//! Escape analysis: the rules that stop a pointer outliving what it points at.
//!
//! Locals live in zero-page frames that are colour-allocated by the call graph:
//! a callee's frame never overlaps a live caller's, which is exactly why passing
//! `&local` *down* is safe. Nothing protects a pointer that goes the other way.
//! The moment a function returns, an unrelated function may occupy the same
//! bytes, so a pointer to its local now names someone else's variable.
//!
//! This is not a borrow checker and does not try to be one. It rejects the four
//! shapes that actually dangle and leaves everything else alone, which fits the
//! language's stated position of trusting the programmer while still refusing to
//! silently miscompile.
//!
//! The subtle one is laundering. A pointer *parameter* has unknown provenance —
//! the callee cannot see where it came from — so storing one in a global has to
//! be allowed for a registration function to be writable at all. That would be a
//! hole big enough to drive the whole analysis through, except that the caller
//! is checked instead: passing `&local` to a parameter that is known to escape
//! is rejected at the call site.

use crate::common::exec::run;
use crate::common::harness::{CompileResult, compile};

fn expect_error(src: &str) -> String {
    match compile(src) {
        CompileResult::SemaError(e) | CompileResult::CodegenError(e) => e,
        CompileResult::Success(..) => panic!("expected a compile error, but it compiled"),
        other => panic!("expected an error, got {other:?}"),
    }
}

fn expect_ok(src: &str) {
    match compile(src) {
        CompileResult::Success(..) => {}
        other => panic!("expected this to compile, got {other:?}"),
    }
}

// ============================================================================
// Returning a pointer to a local
// ============================================================================

#[test]
fn returning_the_address_of_a_local_is_rejected() {
    let err = expect_error(
        r#"
        const OUT: addr = 0x0900;
        fn leak() -> &u8 { let x: u8 = 42; return &x; }
        #[reset]
        fn main() { let p: &u8 = leak(); OUT = *p; loop {} }
    "#,
    );
    assert!(err.contains("local"), "{err}");
    assert!(
        err.contains("frame") || err.contains("outlive"),
        "the reason should be given: {err}"
    );
}

#[test]
fn returning_a_local_pointer_through_a_variable_is_rejected() {
    // The same escape, laundered through a binding. Provenance has to follow
    // the value, not just the syntax of the `return`.
    let err = expect_error(
        r#"
        const OUT: addr = 0x0900;
        fn leak() -> &u8 { let x: u8 = 42; let p: &u8 = &x; return p; }
        #[reset]
        fn main() { let p: &u8 = leak(); OUT = *p; loop {} }
    "#,
    );
    assert!(err.contains("local"), "{err}");
}

#[test]
fn returning_a_pointer_to_a_static_is_allowed() {
    // A static has a fixed address for the life of the program, so there is
    // nothing to outlive.
    expect_ok(
        r#"
        const OUT: addr = 0x0900;
        static COUNT: u8 = 7;
        fn get() -> &u8 { return &COUNT; }
        #[reset]
        fn main() { let p: &u8 = get(); OUT = *p; loop {} }
    "#,
    );
}

#[test]
fn returning_a_pointer_parameter_is_allowed() {
    // Its provenance is unknown, and by the rules below nothing `Local` can
    // have reached it, so passing it back out is safe.
    expect_ok(
        r#"
        const OUT: addr = 0x0900;
        fn pass(p: &u8) -> &u8 { return p; }
        #[reset]
        fn main() { let x: u8 = 3; let q: &u8 = pass(&x); OUT = *q; loop {} }
    "#,
    );
}

// ============================================================================
// Storing a pointer somewhere that outlives the frame
// ============================================================================

#[test]
fn storing_the_address_of_a_local_in_a_static_is_rejected() {
    let err = expect_error(
        r#"
        const OUT: addr = 0x0900;
        static SAVED: &u8 = 0x0400 as &u8;
        #[reset]
        fn main() { let x: u8 = 1; SAVED = &x; OUT = *SAVED; loop {} }
    "#,
    );
    assert!(err.contains("SAVED") || err.contains("global"), "{err}");
}

#[test]
fn storing_a_pointer_to_a_static_in_a_static_is_allowed() {
    expect_ok(
        r#"
        const OUT: addr = 0x0900;
        static COUNT: u8 = 7;
        static SAVED: &u8 = 0x0400 as &u8;
        #[reset]
        fn main() { SAVED = &COUNT; OUT = *SAVED; loop {} }
    "#,
    );
}

// ============================================================================
// Recursion
// ============================================================================

#[test]
fn taking_the_address_of_a_local_in_a_recursive_function_is_rejected() {
    // A recursive call bulk-copies the callee's frame to the software stack and
    // copies it back on return, so the same bytes serve every depth. A pointer
    // taken at one depth names a different invocation's variable at the next,
    // and a write through it is discarded when the frame is restored.
    let err = expect_error(
        r#"
        const OUT: addr = 0x0900;
        fn down(n: u8) -> u8 {
            let here: u8 = n;
            if n == 0 { return 0; }
            let p: &u8 = &here;
            return down(n - 1) + *p;
        }
        #[reset]
        fn main() { OUT = down(3); loop {} }
    "#,
    );
    assert!(err.contains("recursi"), "{err}");
}

#[test]
fn mutual_recursion_is_caught_too() {
    // The rule is about the strongly-connected component, not a self-edge.
    let err = expect_error(
        r#"
        const OUT: addr = 0x0900;
        fn ping(n: u8) -> u8 { let h: u8 = n; let p: &u8 = &h; if n == 0 { return 0; } return pong(n - 1) + *p; }
        fn pong(n: u8) -> u8 { if n == 0 { return 0; } return ping(n - 1); }
        #[reset]
        fn main() { OUT = ping(3); loop {} }
    "#,
    );
    assert!(err.contains("recursi"), "{err}");
}

#[test]
fn a_pointer_to_a_static_in_a_recursive_function_is_allowed() {
    // Only frame storage is at risk; a static keeps its address at every depth.
    expect_ok(
        r#"
        const OUT: addr = 0x0900;
        static ACC: u8 = 0;
        fn down(n: u8) -> u8 {
            if n == 0 { return 0; }
            let p: &u8 = &ACC;
            *p = *p + 1;
            return down(n - 1);
        }
        #[reset]
        fn main() { down(3); OUT = ACC; loop {} }
    "#,
    );
}

// ============================================================================
// Laundering through a parameter
// ============================================================================

#[test]
fn passing_a_local_to_a_parameter_that_escapes_is_rejected_at_the_call_site() {
    // `keep` itself is fine: its parameter's provenance is unknown, and a
    // library that registers a device handle has to be writable. The escape is
    // only visible where a *local* is handed to it, so that is where it is
    // reported.
    let err = expect_error(
        r#"
        const OUT: addr = 0x0900;
        static SAVED: &u8 = 0x0400 as &u8;
        fn keep(d: &u8) { SAVED = d; }
        #[reset]
        fn main() { let x: u8 = 1; keep(&x); OUT = *SAVED; loop {} }
    "#,
    );
    assert!(err.contains("keep"), "the callee should be named: {err}");
    assert!(err.contains("escape") || err.contains("outlive"), "{err}");
}

#[test]
fn a_registration_function_still_compiles_on_its_own() {
    // The half that must keep working: storing a parameter is allowed, and
    // callers that pass a global are fine.
    expect_ok(
        r#"
        const OUT: addr = 0x0900;
        static COUNT: u8 = 7;
        static SAVED: &u8 = 0x0400 as &u8;
        fn keep(d: &u8) { SAVED = d; }
        #[reset]
        fn main() { keep(&COUNT); OUT = *SAVED; loop {} }
    "#,
    );
}

#[test]
fn escaping_through_two_levels_of_call_is_caught() {
    // `outer` does not store anything itself; it forwards to `inner`, which
    // does. The summary has to propagate along the call graph to see it.
    let err = expect_error(
        r#"
        const OUT: addr = 0x0900;
        static SAVED: &u8 = 0x0400 as &u8;
        fn inner(d: &u8) { SAVED = d; }
        fn outer(d: &u8) { inner(d); }
        #[reset]
        fn main() { let x: u8 = 1; outer(&x); OUT = *SAVED; loop {} }
    "#,
    );
    assert!(err.contains("outer"), "{err}");
}

#[test]
fn passing_a_local_to_a_parameter_that_does_not_escape_is_allowed() {
    // The common case, and the one the whole feature exists for.
    expect_ok(
        r#"
        const OUT: addr = 0x0900;
        fn fill(d: &u8, v: u8) { *d = v; }
        #[reset]
        fn main() { let slot: u8 = 0; fill(&slot, 9); OUT = slot; loop {} }
    "#,
    );
}

// ============================================================================
// The permitted cases still run
// ============================================================================

#[test]
fn the_allowed_shapes_still_execute_correctly() {
    assert_eq!(
        run(r#"
            const OUT: addr = 0x0900;
            static TOTAL: u8 = 0;
            fn add_into(dest: &u8, v: u8) { *dest = *dest + v; }
            #[reset]
            fn main() {
                let local: u8 = 1;
                add_into(&local, 2);
                add_into(&TOTAL, 10);
                OUT = local + TOTAL;
                loop {}
            }
        "#)
        .mem(0x0900),
        13
    );
}

// ============================================================================
// Walker coverage
//
// The escape walkers enumerate AST forms by hand, and three of them forgot
// forms: ForEach bodies were never descended into, `for` range bounds and
// match scrutinees were never visited, and a `.len` re-resolved as a struct
// field wasn't peeled like a field. Each hole let `&local` through.
// ============================================================================

#[test]
fn returning_a_local_pointer_from_a_for_each_body_is_rejected() {
    let err = expect_error(
        r#"
        const OUT: addr = 0x0900;
        fn leak(arr: [u8; 4]) -> &u8 {
            for x in arr {
                let y: u8 = x;
                return &y;
            }
            // A `for` may run zero times, so without this the function could
            // fall off the end and would be rejected for that instead of for
            // the escape this test is about.
            loop {}
        }
        #[reset]
        fn main() { let a: [u8; 4] = [1, 2, 3, 4]; let p: &u8 = leak(a); OUT = *p; loop {} }
    "#,
    );
    assert!(err.contains("returns a pointer"), "{err}");
}

#[test]
fn address_of_a_local_in_a_for_bound_of_a_recursive_function_is_rejected() {
    let err = expect_error(
        r#"
        const OUT: addr = 0x0900;
        fn take(p: &u8) -> u8 { return *p; }
        fn rec(n: u8) -> u8 {
            let local: u8 = 3;
            let acc: u8 = 0;
            for i in 0..take(&local) { acc = acc + i; }
            if n == 0 { return acc; }
            return rec(n - 1);
        }
        #[reset]
        fn main() { OUT = rec(2); loop {} }
    "#,
    );
    assert!(err.contains("recursive"), "{err}");
}

#[test]
fn storing_a_local_pointer_through_an_accessor_named_field_is_rejected() {
    // `len` is a field here, not the built-in accessor — the store must be
    // checked exactly like `G1.other = &y`, which was always rejected.
    let err = expect_error(
        r#"
        const OUT: addr = 0x0900;
        static DUMMY: u8 = 0;
        struct Rec { len: &u8, other: &u8 }
        static G1: Rec = Rec { len: &DUMMY, other: &DUMMY };
        #[reset]
        fn main() {
            let y: u8 = 5;
            G1.len = &y;
            OUT = *G1.len;
            loop {}
        }
    "#,
    );
    assert!(err.contains("storing a pointer to a local"), "{err}");
}
