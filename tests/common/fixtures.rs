//! Common test fixtures and example programs
//!
//! Provides reusable test programs for various scenarios

/// Minimal valid program
pub const MINIMAL_PROGRAM: &str = "fn main() {}";

/// Simple variable declaration
pub const SIMPLE_VARIABLE: &str = r#"
fn main() {
    x: u8 = 42;
}
"#;

/// Simple function call
pub const SIMPLE_FUNCTION_CALL: &str = r#"
fn foo() -> u8 {
    return 10;
}

fn main() {
    x: u8 = foo();
}
"#;

/// Inline function
pub const INLINE_FUNCTION: &str = r#"
inline fn add(a: u8, b: u8) -> u8 {
    return a + b;
}

fn main() {
    result: u8 = add(5, 10);
}
"#;

/// If statement
pub const IF_STATEMENT: &str = r#"
fn main() {
    x: u8 = 10;
    if x == 10 {
        y: u8 = 20;
    }
}
"#;

/// While loop
pub const WHILE_LOOP: &str = r#"
fn main() {
    x: u8 = 0;
    while x < 10 {
        x = x + 1;
    }
}
"#;

/// For loop
pub const FOR_LOOP: &str = r#"
fn main() {
    for i: u8 in 0..10 {
        x: u8 = i;
    }
}
"#;

/// Struct definition and usage
pub const STRUCT_USAGE: &str = r#"
struct Point {
    u8 x,
    u8 y,
}

fn main() {
    p: Point = Point { x: 10, y: 20 };
    x: u8 = p.x;
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
    d: Direction = Direction::North;
    match d {
        Direction::North => { x: u8 = 1; }
        Direction::South => { x: u8 = 2; }
        Direction::East => { x: u8 = 3; }
        Direction::West => { x: u8 = 4; }
    }
}
"#;

/// Address declaration
pub const ADDRESS_DECL: &str = r#"
addr SCREEN = 0x0400;

fn main() {
    SCREEN = 42;
}
"#;

/// Array literal
pub const ARRAY_LITERAL: &str = r#"
fn main() {
    arr: [u8; 3] = [1, 2, 3];
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
    a: u8 = 10;
    b: u8 = 20;
    sum: u8 = a + b;
    diff: u8 = a - b;
    prod: u8 = a * b;
    quot: u8 = a / b;
}
"#;

/// Bitwise operations
pub const BITWISE: &str = r#"
fn main() {
    a: u8 = 0xFF;
    b: u8 = 0x0F;
    and_result: u8 = a & b;
    or_result: u8 = a | b;
    xor_result: u8 = a ^ b;
    shift_left: u8 = a << 1;
    shift_right: u8 = a >> 1;
}
"#;

/// Interrupt handler
pub const INTERRUPT_HANDLER: &str = r#"
addr OUT = 0x400;

#[nmi]
fn nmi_handler() {
    OUT = 0xFF;
}

fn main() {}
"#;

/// Program with warnings (unused variable)
pub const UNUSED_VARIABLE: &str = r#"
fn main() {
    x: u8 = 10;
    y: u8 = 20;
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
    x: u8 = add(10, 20);
    y: u8 = sub(30, 10);
}
"#;
