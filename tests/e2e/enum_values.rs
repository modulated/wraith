//! Enum discriminants and casting them to integers.
//!
//! A unit variant *is* its discriminant at runtime — the value a `Direction`
//! carries is the byte the jump table indexes by — so `d as u8` should hand
//! back exactly the number written in the declaration. Register maps are the
//! reason this matters: `DDRA = direction as u8` is how a driver writes a
//! port, and the constants involved (`0xFF`, `0x00`) sit at the ends of the
//! byte range where the tag arithmetic used to break.

use crate::common::exec::run;
use crate::common::harness::{CompileResult, compile};

fn expect_error(src: &str) -> String {
    match compile(src) {
        CompileResult::SemaError(e) => e,
        CompileResult::Success(..) => panic!("expected a compile error, but it compiled"),
        other => panic!("expected a semantic error, got {other:?}"),
    }
}

// ============================================================================
// Casting to an integer
// ============================================================================

#[test]
fn a_unit_variant_casts_to_its_declared_discriminant() {
    let pick = |variant: &str| {
        run(&format!(
            r#"
            enum Direction {{ OUTPUT = 0xFF, INPUT = 0x00 }}
            const OUT: addr = 0x0900;
            #[reset]
            fn main() {{
                let d: Direction = Direction::{variant};
                OUT = d as u8;
                loop {{}}
            }}
        "#
        ))
        .mem(0x0900)
    };
    assert_eq!(pick("OUTPUT"), 0xFF);
    assert_eq!(pick("INPUT"), 0x00);
}

#[test]
fn implicit_discriminants_count_from_zero() {
    let pick = |variant: &str| {
        run(&format!(
            r#"
            enum Step {{ First, Second, Third }}
            const OUT: addr = 0x0900;
            #[reset]
            fn main() {{ let s: Step = Step::{variant}; OUT = s as u8; loop {{}} }}
        "#
        ))
        .mem(0x0900)
    };
    assert_eq!(pick("First"), 0);
    assert_eq!(pick("Second"), 1);
    assert_eq!(pick("Third"), 2);
}

#[test]
fn implicit_discriminants_continue_from_the_last_explicit_one() {
    let pick = |variant: &str| {
        run(&format!(
            r#"
            enum Code {{ A = 10, B, C = 20, D }}
            const OUT: addr = 0x0900;
            #[reset]
            fn main() {{ let c: Code = Code::{variant}; OUT = c as u8; loop {{}} }}
        "#
        ))
        .mem(0x0900)
    };
    assert_eq!(pick("A"), 10);
    assert_eq!(pick("B"), 11);
    assert_eq!(pick("C"), 20);
    assert_eq!(pick("D"), 21);
}

#[test]
fn a_cast_discriminant_can_be_written_to_a_hardware_register() {
    // The shape this is all for: an enum naming the two states of a port
    // direction register, written through a cast.
    assert_eq!(
        run(r#"
            enum Direction { OUTPUT = 0xFF, INPUT = 0x00 }
            const DDRA: addr = 0x0900;
            fn set_direction(d: Direction) { DDRA = d as u8; }
            #[reset]
            fn main() { set_direction(Direction::OUTPUT); loop {} }
        "#)
        .mem(0x0900),
        0xFF
    );
}

// ============================================================================
// Discriminants that do not fit
// ============================================================================

#[test]
fn a_variant_after_the_last_discriminant_is_rejected() {
    // `A = 0xFF` leaves nothing for `B`. This used to panic the compiler with
    // an arithmetic overflow; before that it would have wrapped `B` to 0 and
    // silently collided with whatever held that tag.
    let err = expect_error(
        r#"
        enum Full { A = 0xFF, B }
        const OUT: addr = 0x0900;
        #[reset]
        fn main() { let f: Full = Full::A; OUT = f as u8; loop {} }
    "#,
    );
    assert!(err.contains("discriminant"), "{err}");
    assert!(
        err.contains('B'),
        "the error should name the variant: {err}"
    );
}

#[test]
fn a_discriminant_outside_a_byte_is_rejected() {
    let err = expect_error(
        r#"
        enum Big { A = 300 }
        const OUT: addr = 0x0900;
        #[reset]
        fn main() { let a: Big = Big::A; OUT = a as u8; loop {} }
    "#,
    );
    assert!(err.contains("out of range"), "{err}");
}

#[test]
fn two_variants_sharing_a_discriminant_are_rejected() {
    // The tag is the whole value of a unit variant and is what match dispatch
    // indexes by, so a duplicate makes one of the two unreachable.
    let err = expect_error(
        r#"
        enum Clash { A = 5, B = 5 }
        const OUT: addr = 0x0900;
        #[reset]
        fn main() { let c: Clash = Clash::A; OUT = c as u8; loop {} }
    "#,
    );
    assert!(err.contains('A') && err.contains('B'), "{err}");
}

#[test]
fn an_implicit_discriminant_colliding_with_an_explicit_one_is_rejected() {
    // `A` takes 0 implicitly, then `B = 0` asks for the same tag.
    let err = expect_error(
        r#"
        enum Clash { A, B = 0 }
        const OUT: addr = 0x0900;
        #[reset]
        fn main() { let c: Clash = Clash::A; OUT = c as u8; loop {} }
    "#,
    );
    assert!(err.contains("discriminant"), "{err}");
}

#[test]
fn the_last_usable_discriminant_is_still_allowed() {
    // 0xFF itself is fine; only a variant *after* it has nowhere to go.
    assert_eq!(
        run(r#"
            enum Edge { A = 0xFE, B = 0xFF }
            const OUT: addr = 0x0900;
            #[reset]
            fn main() { let e: Edge = Edge::B; OUT = e as u8; loop {} }
        "#)
        .mem(0x0900),
        0xFF
    );
}

// ============================================================================
// Matching still sees the same values
// ============================================================================

#[test]
fn explicit_discriminants_match_correctly() {
    // Dispatch indexes by tag, so sparse discriminants have to keep working.
    let pick = |variant: &str| {
        run(&format!(
            r#"
            enum Code {{ Low = 1, Mid = 40, High = 200 }}
            const OUT: addr = 0x0900;
            #[reset]
            fn main() {{
                let c: Code = Code::{variant};
                match c {{
                    Code::Low  => {{ OUT = 11; }}
                    Code::Mid  => {{ OUT = 22; }}
                    Code::High => {{ OUT = 33; }}
                }}
                loop {{}}
            }}
        "#
        ))
        .mem(0x0900)
    };
    assert_eq!(pick("Low"), 11);
    assert_eq!(pick("Mid"), 22);
    assert_eq!(pick("High"), 33);
}
