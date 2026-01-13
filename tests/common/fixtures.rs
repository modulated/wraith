//! Common test fixtures and example programs
//!
//! Provides reusable test programs for various scenarios

#![allow(dead_code)]

/// Minimal valid program
pub const MINIMAL_PROGRAM: &str = "fn main() {}";

/// Simple variable declaration
pub const SIMPLE_VARIABLE: &str = r#"
fn main() {
    let x: u8 = 42;
}
"#;

/// Simple function call
pub const SIMPLE_FUNCTION_CALL: &str = r#"
fn foo() -> u8 {
    return 10;
}

fn main() {
    let x: u8 = foo();
}
"#;

/// Inline function
pub const INLINE_FUNCTION: &str = r#"
#[inline]
fn add(a: u8, b: u8) -> u8 {
    return a + b;
}

fn main() {
    let result: u8 = add(5, 10);
}
"#;

/// If statement
pub const IF_STATEMENT: &str = r#"
fn main() {
    let x: u8 = 10;
    if x == 10 {
        let y: u8 = 20;
    }
}
"#;

/// While loop
pub const WHILE_LOOP: &str = r#"
fn main() {
    let x: u8 = 0;
    while x < 10 {
        x = x + 1;
    }
}
"#;

/// For loop
pub const FOR_LOOP: &str = r#"
fn main() {
    for i: u8 in 0..10 {
        let x: u8 = i;
    }
}
"#;

/// Struct definition and usage
pub const STRUCT_USAGE: &str = r#"
struct Point {
    x: u8,
    y: u8,
}

fn main() {
    let p: Point = Point { x: 10, y: 20 };
    let x: u8 = p.x;
}
"#;

/// Enum definition and match
pub const ENUM_MATCH: &str = r#"
enum Direction {
    North,
    South,
    East,
    West,
}

fn main() {
    let d: Direction = Direction::North;
    match d {
        Direction::North => { let x: u8 = 1; }
        Direction::South => { let x: u8 = 2; }
        Direction::East => { let x: u8 = 3; }
        Direction::West => { let x: u8 = 4; }
    }
}
"#;

/// Address declaration
pub const ADDRESS_DECL: &str = r#"
const SCREEN: addr = 0x0400;

fn main() {
    SCREEN = 42;
}
"#;

/// Array literal
pub const ARRAY_LITERAL: &str = r#"
fn main() {
    let arr: [u8; 3] = [1, 2, 3];
}
"#;

/// String literal
pub const STRING_LITERAL: &str = r#"
fn main() {
    "Hello, World!";
}
"#;

/// Arithmetic operations
pub const ARITHMETIC: &str = r#"
fn main() {
    let a: u8 = 10;
    let b: u8 = 20;
    let sum: u8 = a + b;
    let diff: u8 = a - b;
    let prod: u8 = a * b;
    let quot: u8 = a / b;
}
"#;

/// Bitwise operations
pub const BITWISE: &str = r#"
fn main() {
    let a: u8 = 0xFF;
    let b: u8 = 0x0F;
    let and_result: u8 = a & b;
    let or_result: u8 = a | b;
    let xor_result: u8 = a ^ b;
    let shift_left: u8 = a << 1;
    let shift_right: u8 = a >> 1;
}
"#;

/// Interrupt handler
pub const INTERRUPT_HANDLER: &str = r#"
const OUT: addr = 0x400;

#[nmi]
fn nmi_handler() {
    OUT = 0xFF;
}

fn main() {}
"#;

/// Program with warnings (unused variable)
pub const UNUSED_VARIABLE: &str = r#"
fn main() {
    let x: u8 = 10;
    let y: u8 = 20;
}
"#;

/// Program with multiple functions
pub const MULTIPLE_FUNCTIONS: &str = r#"
fn add(a: u8, b: u8) -> u8 {
    return a + b;
}

fn sub(a: u8, b: u8) -> u8 {
    return a - b;
}

fn main() {
    let x: u8 = add(10, 20);
    let y: u8 = sub(30, 10);
}
"#;
