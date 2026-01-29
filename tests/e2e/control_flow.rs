//! End-to-end tests for control flow constructs

use crate::common::*;

#[test]
fn if_statement() {
    let asm = compile_success(
        r#"
        fn main() {
            if true {
                let x: u8 = 10;
            }
        }
    "#,
    );

    assert_asm_contains(&asm, "BNE"); // Branch to then when condition is true
}

#[test]
fn if_else() {
    let asm = compile_success(
        r#"
        fn main() {
            if false {
                let x: u8 = 10;
            } else {
                let x: u8 = 20;
            }
        }
    "#,
    );

    assert_asm_contains(&asm, "BNE"); // Branch to then when condition is true
    assert_asm_contains(&asm, "JMP");
}

#[test]
fn while_loop() {
    let asm = compile_success(
        r#"
        fn main() {
            let x: u8 = 0;
            while x < 10 {
                x = x + 1;
            }
        }
    "#,
    );

    assert_asm_contains(&asm, "CMP");
    assert_asm_contains(&asm, "JMP");
}

#[test]
fn for_range_loop() {
    let asm = compile_success(
        r#"
        fn main() {
            for i: u8 in 0..10 {
                let x: u8 = i;
            }
        }
    "#,
    );

    assert_asm_contains(&asm, "INX");
    assert_asm_contains(&asm, "CPX");
}

// ============================================================
// Break and Continue Tests
// ============================================================

#[test]
fn while_loop_with_break() {
    let asm = compile_success(
        r#"
        fn main() {
            let x: u8 = 0;
            while x < 10 {
                if x == 5 {
                    break;
                }
                x = x + 1;
            }
        }
    "#,
    );

    // Should have loop labels and JMP for break
    assert_asm_contains(&asm, "JMP");
    assert_asm_contains(&asm, "CMP");
}

#[test]
fn while_loop_with_continue() {
    let asm = compile_success(
        r#"
        fn main() {
            let x: u8 = 0;
            while x < 10 {
                x = x + 1;
                if x == 5 {
                    continue;
                }
                let y: u8 = x;
            }
        }
    "#,
    );

    // Continue should jump back to loop condition
    assert_asm_contains(&asm, "JMP");
    assert_asm_contains(&asm, "CMP");
}

#[test]
fn for_loop_with_break() {
    let asm = compile_success(
        r#"
        fn main() {
            for i: u8 in 0..10 {
                if i == 5 {
                    break;
                }
            }
        }
    "#,
    );

    // Should exit loop early via JMP
    assert_asm_contains(&asm, "INX"); // For loop uses X register
    assert_asm_contains(&asm, "JMP");
    assert_asm_contains(&asm, "BEQ"); // Conditional branch for if
}

#[test]
fn for_loop_with_continue() {
    let asm = compile_success(
        r#"
        fn main() {
            for i: u8 in 0..10 {
                if i == 5 {
                    continue;
                }
                let x: u8 = i;
            }
        }
    "#,
    );

    // Continue should jump to loop increment
    assert_asm_contains(&asm, "INX");
    assert_asm_contains(&asm, "JMP");
}

#[test]
fn nested_loop_break() {
    let asm = compile_success(
        r#"
        fn main() {
            for i: u8 in 0..10 {
                for j: u8 in 0..10 {
                    if j == 2 {
                        break;
                    }
                }
            }
        }
    "#,
    );

    // Should have distinct loop labels for nested loops
    assert_asm_contains(&asm, "INX");
    assert_asm_contains(&asm, "JMP");
}

// ============================================================
// Enhanced Match Statement Tests
// ============================================================

#[test]
fn match_literal_patterns() {
    let asm = compile_success(
        r#"
        fn main() {
            let x: u8 = 5;
            match x {
                0 => { let y: u8 = 10; }
                5 => { let y: u8 = 20; }
                _ => { let y: u8 = 30; }
            }
        }
    "#,
    );

    // Should generate comparisons and branches
    assert_asm_contains(&asm, "CMP");
    assert_asm_contains(&asm, "BEQ");
}

#[test]
fn match_enum_variants() {
    let asm = compile_success(
        r#"
        enum Direction {
            North,
            South,
            East,
            West,
        }
        fn main() {
            let d: Direction = Direction::North;
            match d {
                Direction::North => { let val: u8 = 1; }
                Direction::South => { let val: u8 = 2; }
                Direction::East => { let val: u8 = 3; }
                Direction::West => { let val: u8 = 4; }
            }
        }
    "#,
    );

    // With 4 variants, uses jump table dispatch (ASL/TAX/JMP indirect)
    assert_asm_contains(&asm, "LDA");
    assert_asm_contains(&asm, "ASL"); // Double tag for address indexing
    assert_asm_contains(&asm, "TAX"); // Transfer to X for table indexing
    assert_asm_contains(&asm, "JMP"); // Jump to arm
}

#[test]
fn match_expression_bodies() {
    let asm = compile_success(
        r#"
        fn main() {
            let x: u8 = 2;
            let y: u8 = 0;
            match x {
                1 => { y = 10; }
                2 => { y = 20; }
                _ => { y = 30; }
            }
        }
    "#,
    );

    // Match arms with assignment statements
    assert_asm_contains(&asm, "CMP");
    assert_asm_contains(&asm, "BEQ");
}

#[test]
fn match_multiple_arms() {
    let asm = compile_success(
        r#"
        fn main() {
            let x: u8 = 5;
            match x {
                1 => { let y: u8 = 10; }
                2 => { let y: u8 = 20; }
                3 => { let y: u8 = 30; }
                _ => { let y: u8 = 0; }
            }
        }
    "#,
    );

    // Should have labels for each arm
    assert_asm_contains(&asm, "CMP");
    assert_asm_contains(&asm, "JMP");
}
