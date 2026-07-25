//! Variable declaration and literal tests, verified by execution.
//!
//! These previously asserted that an `LDA` appeared, or that the text `$FF`
//! occurred somewhere in the output — true of a program that stored the wrong
//! value, or the right value in the wrong place. They now read the variable back.
//!
//! The arithmetic and bitwise tests that used to live here (addition,
//! multiplication, `&`, `|`, `<<`) were duplicates of `operators.rs`, which
//! covers each operator with boundary and wrap-around cases; they were removed
//! rather than migrated twice.

use crate::common::exec::run;

/// Run a program body and read back the u8 left in `OUT` ($0900).
fn eval(body: &str) -> u8 {
    let src = format!(
        r#"
        const OUT: addr = 0x0900;
        #[reset]
        fn main() {{
            {body}
            loop {{}}
        }}
    "#
    );
    run(&src).mem(0x0900)
}

/// Run a program body and read back the u16 left in LO/HI ($0900/$0901).
fn eval16(body: &str) -> u16 {
    let src = format!(
        r#"
        const LO: addr = 0x0900;
        const HI: addr = 0x0901;
        #[reset]
        fn main() {{
            {body}
            loop {{}}
        }}
    "#
    );
    run(&src).mem16(0x0900)
}

// ============================================================================
// Declarations
// ============================================================================

#[test]
fn variable_declaration_u8() {
    assert_eq!(eval("let x: u8 = 42; OUT = x;"), 42);
}

#[test]
fn variable_declaration_u16() {
    // 1000 is $03E8: both bytes must survive, which an `LDA` check never showed.
    assert_eq!(eval16("let x: u16 = 1000; LO = x.low; HI = x.high;"), 1000);
}

#[test]
fn variable_declaration_u16_high_byte_only() {
    // $0100 has a zero low byte; a low-byte-only store would read back as 0.
    assert_eq!(
        eval16("let x: u16 = 0x0100; LO = x.low; HI = x.high;"),
        0x0100
    );
}

#[test]
fn multiple_variables_are_independent() {
    let src = r#"
        const A0: addr = 0x0900;
        const A1: addr = 0x0901;
        const A2: addr = 0x0902;
        #[reset]
        fn main() {
            let a: u8 = 1;
            let b: u8 = 2;
            let c: u8 = 3;
            b = 99;
            A0 = a; A1 = b; A2 = c;
            loop {}
        }
    "#;
    let mut e = run(src);
    assert_eq!(
        (e.mem(0x0900), e.mem(0x0901), e.mem(0x0902)),
        (1, 99, 3),
        "writing b must not disturb a or c"
    );
}

// ============================================================================
// Literal forms — all three notations must produce the same value
// ============================================================================

#[test]
fn hex_literals() {
    assert_eq!(eval("let x: u8 = 0xFF; OUT = x;"), 0xFF);
    assert_eq!(eval("let x: u8 = 0x2A; OUT = x;"), 42);
}

#[test]
fn binary_literals() {
    assert_eq!(eval("let x: u8 = 0b10101010; OUT = x;"), 0xAA);
    assert_eq!(eval("let x: u8 = 0b00001111; OUT = x;"), 0x0F);
}

#[test]
fn literal_notations_agree() {
    assert_eq!(eval("let x: u8 = 255; OUT = x;"), 0xFF);
    assert_eq!(eval("let x: u8 = 0xFF; OUT = x;"), 0xFF);
    assert_eq!(eval("let x: u8 = 0b11111111; OUT = x;"), 0xFF);
}

#[test]
fn hex_literal_u16() {
    assert_eq!(
        eval16("let x: u16 = 0xBEEF; LO = x.low; HI = x.high;"),
        0xBEEF
    );
}

// ============================================================================
// Mutation
// ============================================================================

#[test]
fn mutable_variable() {
    assert_eq!(eval("let x: u8 = 10; x = 20; OUT = x;"), 20);
}

#[test]
fn mutable_variable_u16() {
    assert_eq!(
        eval16("let x: u16 = 1; x = 0x1234; LO = x.low; HI = x.high;"),
        0x1234
    );
}

#[test]
fn variable_reassigned_from_itself() {
    assert_eq!(eval("let x: u8 = 5; x = x + x; OUT = x;"), 10);
}
