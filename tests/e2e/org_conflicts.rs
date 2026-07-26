//! `#[org]` placement tests.
//!
//! Pinning a function to an address is a promise the compiler cannot verify by
//! looking at one declaration: whether it is safe depends on how big the
//! function turns out to be and on what else claims that range. Overlaps were
//! detected only between functions, so an `#[org]` landing on the interrupt
//! vectors, on a string table or on a RAM global was accepted silently — and
//! the failure surfaces as a machine that will not boot rather than as a
//! diagnostic.
//!
//! These check that every kind of overlap is rejected, and, just as importantly,
//! that legal placements still compile.

use crate::common::harness::{CompileResult, compile};

/// Compile and return the error text, failing the test if it compiled.
fn expect_error(src: &str) -> String {
    match compile(src) {
        CompileResult::CodegenError(e) => e,
        CompileResult::SemaError(e) => panic!("expected a codegen error, got a sema error: {e}"),
        CompileResult::Success(..) => panic!("expected a compile error, but it compiled"),
        other => panic!("expected a codegen error, got {other:?}"),
    }
}

fn expect_success(src: &str) {
    match compile(src) {
        CompileResult::Success(..) => {}
        other => panic!("expected this to compile, got {other:?}"),
    }
}

// ============================================================================
// Function against function
// ============================================================================

#[test]
fn two_functions_at_the_same_org_conflict() {
    let err = expect_error(
        r#"
        const OUT: addr = 0x0900;
        #[org(0x8100)]
        fn alpha() -> u8 { return 1; }
        #[org(0x8100)]
        fn beta() -> u8 { return 2; }
        #[reset]
        fn main() { OUT = alpha() + beta(); loop {} }
    "#,
    );
    assert!(err.contains("alpha") && err.contains("beta"), "{err}");
}

#[test]
fn a_function_overrunning_into_the_next_org_conflicts() {
    // The addresses differ, so only the measured size reveals the overlap.
    let err = expect_error(
        r#"
        const OUT: addr = 0x0900;
        #[org(0x8100)]
        fn alpha() -> u8 { let a: u8 = 1; return a + 2 + 3 + 4 + 5; }
        #[org(0x8105)]
        fn beta() -> u8 { return 2; }
        #[reset]
        fn main() { OUT = alpha() + beta(); loop {} }
    "#,
    );
    assert!(err.contains("alpha") && err.contains("beta"), "{err}");
}

// ============================================================================
// Function against everything else that occupies space
// ============================================================================

#[test]
fn an_org_over_the_interrupt_vectors_conflicts() {
    // The worst case: the 6502 fetches its reset vector from $FFFC, so code
    // written over $FFFA-$FFFF stops the machine booting at all.
    let err = expect_error(
        r#"
        const OUT: addr = 0x0900;
        #[org(0xFFF8)]
        fn alpha() -> u8 { let a: u8 = 1; return a + 2 + 3; }
        #[reset]
        fn main() { OUT = alpha(); loop {} }
    "#,
    );
    assert!(
        err.contains("interrupt vector table"),
        "the vectors should be named in the error: {err}"
    );
}

#[test]
fn an_org_over_a_mutable_static_conflicts() {
    let err = expect_error(
        r#"
        const OUT: addr = 0x0900;
        static COUNTER: u8 = 0;
        #[org(0x0400)]
        fn alpha() -> u8 { return 7; }
        #[reset]
        fn main() { COUNTER = 1; OUT = alpha() + COUNTER; loop {} }
    "#,
    );
    assert!(err.contains("COUNTER"), "{err}");
}

#[test]
fn data_is_allocated_around_a_pinned_function() {
    // A function pinned into DATA reserves its range, so the string table is
    // placed after it rather than on top of it. Before placement reserved
    // ranges this was a conflict; now it simply works.
    let asm = crate::common::harness::compile_success(
        r#"
        const OUT: addr = 0x0900;
        const MSG: str = "hello world";
        #[org(0xD000)]
        fn alpha() -> u8 { return 7; }
        #[reset]
        fn main() { let s: str = MSG; OUT = alpha(); loop {} }
    "#,
    );
    assert!(asm.contains(".ORG $D000"), "alpha keeps its address: {asm}");
    // The string starts past alpha's range rather than at the section base.
    let str_org = asm
        .lines()
        .zip(asm.lines().skip(1))
        .find(|(_, next)| next.starts_with("str_"))
        .map(|(org, _)| org.to_string())
        .expect("string literal should be emitted");
    assert_ne!(
        str_org.trim(),
        ".ORG $D000",
        "the string must not be placed on top of the pinned function"
    );
}

// ============================================================================
// Placement outside the memory map
// ============================================================================

#[test]
fn an_org_outside_every_section_is_rejected() {
    // $0100 is the 6502 hardware stack page and belongs to no section. Nothing
    // would have been checked against it, and on a ROM image the bytes are not
    // there to execute.
    let err = expect_error(
        r#"
        const OUT: addr = 0x0900;
        #[org(0x0100)]
        fn alpha() -> u8 { return 7; }
        #[reset]
        fn main() { OUT = alpha(); loop {} }
    "#,
    );
    assert!(err.contains("not inside any configured section"), "{err}");
    assert!(
        err.contains("CODE"),
        "the error should list where it could go: {err}"
    );
}

#[test]
fn an_org_that_overruns_the_end_of_its_section_is_rejected() {
    // The start is inside CODE ($8000-$BFFF) but the body runs past $BFFF.
    // Checking only the start address would let this through.
    let err = expect_error(
        r#"
        const OUT: addr = 0x0900;
        #[org(0xBFFA)]
        fn alpha() -> u8 { let a: u8 = 1; return a + 2 + 3 + 4; }
        #[reset]
        fn main() { OUT = alpha(); loop {} }
    "#,
    );
    assert!(err.contains("not inside any configured section"), "{err}");
}

// ============================================================================
// Legal placements must keep working
// ============================================================================

#[test]
fn a_well_placed_org_still_compiles() {
    expect_success(
        r#"
        const OUT: addr = 0x0900;
        #[org(0xA000)]
        fn alpha() -> u8 { return 7; }
        #[reset]
        fn main() { OUT = alpha(); loop {} }
    "#,
    );
}

#[test]
fn adjacent_orgs_that_merely_touch_do_not_conflict() {
    // `alpha` ends exactly where `beta` begins. An off-by-one in the overlap
    // test would reject this.
    expect_success(
        r#"
        const OUT: addr = 0x0900;
        #[org(0xA000)]
        fn alpha() -> u8 { return 7; }
        #[org(0xA100)]
        fn beta() -> u8 { return 8; }
        #[reset]
        fn main() { OUT = alpha() + beta(); loop {} }
    "#,
    );
}

#[test]
fn programs_without_any_org_are_unaffected() {
    // Auto-allocated functions are handed sequential addresses and can never
    // overlap each other; the check must not invent a conflict among them.
    expect_success(
        r#"
        const OUT: addr = 0x0900;
        static COUNTER: u8 = 0;
        const MSG: str = "hi";
        fn a() -> u8 { return 1; }
        fn b() -> u8 { return 2; }
        fn c() -> u8 { return 3; }
        #[reset]
        fn main() {
            let s: str = MSG;
            COUNTER = a() + b() + c();
            OUT = COUNTER;
            loop {}
        }
    "#,
    );
}
