//! End-to-end tests for complex types

use crate::common::*;

#[test]
fn array_literal() {
    let asm = compile_success(r#"
        fn main() {
            data: [u8; 3] = [1, 2, 3];
        }
    "#);

    assert_asm_contains(&asm, ".BYTE $01");
    assert_asm_contains(&asm, ".BYTE $02");
    assert_asm_contains(&asm, ".BYTE $03");
}

#[test]
fn array_fill() {
    let asm = compile_success(r#"
        fn main() {
            buffer: [u8; 5] = [0; 5];
        }
    "#);

    assert_eq!(count_pattern(&asm, ".BYTE $00"), 5);
}

#[test]
fn struct_definition() {
    let asm = compile_success(r#"
        struct Point {
            u8 x,
            u8 y,
        }
        fn main() {}
    "#);

    // Struct definition doesn't generate code by itself
    assert_asm_contains(&asm, "main:");
}

#[test]
fn struct_initialization() {
    let asm = compile_success(r#"
        struct Point {
            u8 x,
            u8 y,
        }
        fn main() {
            p: Point = Point { x: 10, y: 20 };
        }
    "#);

    assert_asm_contains(&asm, "LDA");
    assert_asm_contains(&asm, "STA");
}

#[test]
fn enum_definition() {
    let asm = compile_success(r#"
        enum Direction {
            North,
            South,
            East,
            West,
        }
        fn main() {
            d: Direction = Direction::North;
        }
    "#);

    assert_asm_contains(&asm, "Direction::North");
    assert_asm_contains(&asm, ".BYTE $00");
}
