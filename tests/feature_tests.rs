//! Feature tests - verify each language feature works correctly
//!
//! Tests are organized by feature:
//! - Variables and types
//! - Operators
//! - Control flow
//! - Functions
//! - Structs and enums
//! - Arrays
//! - Pointers
//! - Inline assembly

mod test_harness;
use test_harness::*;

// ============================================================================
// VARIABLES AND TYPES
// ============================================================================

#[test]
fn test_variable_declaration_u8() {
    let asm = compile_success(
        r#"
        const OUT: addr = 0x400;
        fn main() {
            let x: u8 = 42;
            OUT = x;
        }
        "#,
    );
    assert_asm_contains(&asm, "LDA #$2A"); // Load 42
}

#[test]
fn test_variable_declaration_u16() {
    let asm = compile_success(
        r#"
        const OUT: addr = 0x400;
        fn main() {
            let x: u16 = 0x1234;
            OUT = x as u8;
        }
        "#,
    );
    assert_asm_contains(&asm, "LDA #$34"); // Low byte of 0x1234
}

#[test]
fn test_hex_literals() {
    let asm = compile_success(
        r#"
        const OUT: addr = 0xC000;
        fn main() {
            let a: u8 = 0xFF;
            let b: u16 = 0xFA00;
            OUT = a;
        }
        "#,
    );
    assert_asm_contains(&asm, "LDA #$FF");
}

#[test]
fn test_binary_literals() {
    let asm = compile_success(
        r#"
        const OUT: addr = 0x400;
        fn main() {
            let flags: u8 = 0b11010010;
            OUT = flags;
        }
        "#,
    );
    assert_asm_contains(&asm, "LDA #$D2"); // 0b11010010 = 0xD2
}

#[test]
fn test_mutable_variable() {
    let asm = compile_success(
        r#"
        const OUT: addr = 0x400;
        fn main() {
            let x: u8 = 10;
            x = 20;
            OUT = x;
        }
        "#,
    );
    assert_asm_contains(&asm, "LDA #$0A"); // Initial value
    assert_asm_contains(&asm, "LDA #$14"); // New value 20
}

// ============================================================================
// OPERATORS
// ============================================================================

#[test]
fn test_arithmetic_addition() {
    let asm = compile_success(
        r#"
        const OUT: addr = 0x400;
        fn main() {
            let x: u8 = 10 + 20;
            OUT = x;
        }
        "#,
    );
    // Should constant fold to 30 (0x1E)
    assert_asm_contains(&asm, "LDA #$1E");
}

#[test]
fn test_arithmetic_multiplication() {
    let asm = compile_success(
        r#"
        const OUT: addr = 0x400;
        fn main() {
            let x: u8 = 5;
            let y: u8 = x * 3;
            OUT = y;
        }
        "#,
    );
    // Should have multiplication code
    assert!(asm.contains("LDA") && asm.contains("STA"));
}

#[test]
fn test_bitwise_and() {
    let asm = compile_success(
        r#"
        const OUT: addr = 0x400;
        fn main() {
            let x: u8 = 0xFF & 0x0F;
            OUT = x;
        }
        "#,
    );
    // Should constant fold to 0x0F
    assert_asm_contains(&asm, "LDA #$0F");
}

#[test]
fn test_bitwise_or() {
    let asm = compile_success(
        r#"
        const OUT: addr = 0x400;
        fn main() {
            let x: u8 = 0xF0 | 0x0F;
            OUT = x;
        }
        "#,
    );
    // Should constant fold to 0xFF
    assert_asm_contains(&asm, "LDA #$FF");
}

#[test]
fn test_shift_left() {
    let asm = compile_success(
        r#"
        const OUT: addr = 0x400;
        fn main() {
            let x: u8 = 1 << 4;
            OUT = x;
        }
        "#,
    );
    // Should constant fold to 16 (0x10)
    assert_asm_contains(&asm, "LDA #$10");
}

// ============================================================================
// CONTROL FLOW
// ============================================================================

#[test]
fn test_if_statement() {
    let asm = compile_success(
        r#"
        const OUT: addr = 0x400;
        fn main() {
            let x: u8 = 10;
            if x == 10 {
                OUT = 0xFF;
            }
        }
        "#,
    );
    assert_asm_contains(&asm, "CMP");
    assert_asm_contains(&asm, "BEQ");
}

#[test]
fn test_if_else() {
    let asm = compile_success(
        r#"
        const OUT: addr = 0x400;
        fn main() {
            let x: u8 = 10;
            if x == 10 {
                OUT = 0xFF;
            } else {
                OUT = 0x00;
            }
        }
        "#,
    );
    assert_asm_contains(&asm, "BEQ");
    assert_asm_contains(&asm, "JMP");
}

#[test]
fn test_while_loop() {
    let asm = compile_success(
        r#"
        const OUT: addr = 0x400;
        fn main() {
            let i: u8 = 0;
            while i < 10 {
                i = i + 1;
            }
            OUT = i;
        }
        "#,
    );
    assert_asm_contains(&asm, "wb_"); // While body label
    assert_asm_contains(&asm, "wc_"); // While check label
    assert_asm_contains(&asm, "JMP");
}

#[test]
fn test_for_range_loop() {
    let asm = compile_success(
        r#"
        const OUT: addr = 0x400;
        fn main() {
            let sum: u8 = 0;
            for i in 0..10 {
                sum = sum + i;
            }
            OUT = sum;
        }
        "#,
    );
    assert_asm_contains(&asm, "fl_");
}

// ============================================================================
// FUNCTIONS
// ============================================================================

#[test]
fn test_function_call_no_args() {
    let asm = compile_success(
        r#"
        const OUT: addr = 0x400;
        fn helper() {
            OUT = 42;
        }
        fn main() {
            helper();
        }
        "#,
    );
    // Tail call optimization may convert JSR+RTS to JMP
    assert!(
        asm.contains("JSR helper") || asm.contains("JMP helper"),
        "Expected JSR or JMP helper (tail-call optimized)"
    );
}

#[test]
fn test_function_call_with_args() {
    let asm = compile_success(
        r#"
        const OUT: addr = 0x400;
        fn add(a: u8, b: u8) {
            OUT = a + b;
        }
        fn main() {
            add(10, 20);
        }
        "#,
    );
    // Tail call optimization may convert JSR+RTS to JMP
    assert!(
        asm.contains("JSR add") || asm.contains("JMP add"),
        "Expected JSR or JMP add (tail-call optimized)"
    );
}

#[test]
fn test_function_return_value() {
    let asm = compile_success(
        r#"
        const OUT: addr = 0x400;
        fn get_value() -> u8 {
            return 42;
        }
        fn main() {
            let x: u8 = get_value();
            OUT = x;
        }
        "#,
    );
    assert_asm_contains(&asm, "JSR get_value");
    assert_asm_contains(&asm, "RTS");
}

// ============================================================================
// ARRAYS
// ============================================================================

#[test]
fn test_array_literal() {
    let asm = compile_success(
        r#"
        fn main() {
            let data: [u8; 3] = [1, 2, 3];
        }
        "#,
    );
    assert_asm_contains(&asm, ".BYTE $01");
    assert_asm_contains(&asm, ".BYTE $02");
    assert_asm_contains(&asm, ".BYTE $03");
}

#[test]
fn test_array_fill() {
    let asm = compile_success(
        r#"
        fn main() {
            let buffer: [u8; 5] = [0; 5];
        }
        "#,
    );
    // Should have 5 zero bytes
    assert_eq!(count_pattern(&asm, ".BYTE $00"), 5);
}

// ============================================================================
// STRUCTS AND ENUMS
// ============================================================================

#[test]
fn test_struct_definition() {
    let asm = compile_success(
        r#"
        struct Point {
            x: u8,
            y: u8,
        }
        fn main() {}
        "#,
    );
    assert!(asm.contains("main:"));
}

#[test]
fn test_struct_initialization() {
    let asm = compile_success(
        r#"
        struct Point {
            x: u8,
            y: u8,
        }
        const OUT: addr = 0x400;
        fn main() {
            let p: Point = Point { x: 10, y: 20 };
        }
        "#,
    );
    assert!(asm.contains("main:"));
}

#[test]
fn test_enum_definition() {
    let asm = compile_success(
        r#"
        enum Status {
            Idle,
            Active,
            Done
        }
        fn main() {}
        "#,
    );
    assert!(asm.contains("main:"));
}

// ============================================================================
// MEMORY OPERATIONS
// ============================================================================

#[test]
fn test_address_declaration() {
    let asm = compile_success(
        r#"
        const SCREEN: addr = 0xC000;
        fn main() {
            SCREEN = 42;
        }
        "#,
    );
    assert_asm_contains(&asm, "SCREEN = $C000"); // Address label
    assert_asm_contains(&asm, "STA SCREEN"); // Symbolic name
}

#[test]
fn test_constant_address_expression() {
    let asm = compile_success(
        r#"
        const BASE: addr = 0xC000;
        const SCREEN: addr = BASE + 0x100;
        fn main() {
            SCREEN = 42;
        }
        "#,
    );
    assert_asm_contains(&asm, "SCREEN = $C100"); // 0xC000 + 0x100
    assert_asm_contains(&asm, "STA SCREEN"); // Symbolic name
}

// ============================================================================
// INTERRUPT HANDLERS
// ============================================================================

#[test]
fn test_nmi_handler() {
    let asm = compile_success(
        r#"
        const OUT: addr = 0x400;
        #[nmi]
        fn nmi_handler() {
            OUT = 0xFF;
        }
        fn main() {}
        "#,
    );
    // Should have prologue
    assert_asm_contains(&asm, "PHA");
    assert_asm_contains(&asm, "TXA");
    assert_asm_contains(&asm, "TYA");
    // Should have epilogue
    assert_asm_contains(&asm, "TAY");
    assert_asm_contains(&asm, "TAX");
    assert_asm_contains(&asm, "RTI");
    // Should have vector table
    assert_asm_contains(&asm, ".ORG $FFFA");
    assert_asm_contains(&asm, ".WORD nmi_handler");
}

#[test]
fn test_irq_handler() {
    let asm = compile_success(
        r#"
        const OUT: addr = 0x400;
        #[irq]
        fn irq_handler() {
            OUT = 0x42;
        }
        fn main() {}
        "#,
    );
    // Should use RTI
    assert_asm_contains(&asm, "RTI");
    // Should have IRQ vector
    assert_asm_contains(&asm, ".WORD irq_handler");
}

#[test]
fn test_reset_handler() {
    let asm = compile_success(
        r#"
        #[reset]
        fn start() {
        }
        fn main() {}
        "#,
    );
    // Should have RESET vector
    assert_asm_contains(&asm, ".WORD start");
}

#[test]
fn test_all_interrupt_vectors() {
    let asm = compile_success(
        r#"
        const OUT: addr = 0x400;
        #[nmi]
        fn nmi_handler() {
            OUT = 1;
        }
        #[reset]
        fn reset_handler() {
            OUT = 2;
        }
        #[irq]
        fn irq_handler() {
            OUT = 3;
        }
        fn main() {}
        "#,
    );
    // Verify vector table exists at correct location
    assert_asm_contains(&asm, ".ORG $FFFA");
    // Verify all three vectors are present
    assert_asm_contains(&asm, ".WORD nmi_handler");
    assert_asm_contains(&asm, ".WORD reset_handler");
    assert_asm_contains(&asm, ".WORD irq_handler");
}

// ============================================================================
// STRING FEATURES
// ============================================================================

#[test]
fn test_string_concatenation() {
    let asm = compile_success(
        r#"
        const GREETING: str = "Hello, " + "World!";
        fn main() {
            let msg: str = GREETING;
        }
        "#,
    );
    // Should contain the concatenated string "Hello, World!"
    assert_asm_contains(&asm, "Hello, World!");
    // Should have single string literal (deduplicated)
    assert_asm_contains(&asm, "; \"Hello, World!\"");
}

#[test]
fn test_string_concatenation_multiple() {
    let asm = compile_success(
        r#"
        const PATH: str = "data/" + "level" + ".txt";
        fn main() {
            let p: str = PATH;
        }
        "#,
    );
    // Should contain the fully concatenated string
    assert_asm_contains(&asm, "data/level.txt");
}

#[test]
fn test_string_iteration_simple() {
    let asm = compile_success(
        r#"
        const MSG: str = "ABC";
        fn main() {
            for c in MSG {
                let ch: u8 = c;
            }
        }
        "#,
    );
    // Should generate a for-each loop
    assert_asm_contains(&asm, "ForEach loop");
    // Should iterate over the string
    assert_asm_contains(&asm, "String iteration");
}

#[test]
fn test_string_iteration_with_index() {
    let asm = compile_success(
        r#"
        const MSG: str = "ABC";
        fn main() {
            for (i, c) in MSG {
                let idx: u8 = i;
                let ch: u8 = c;
            }
        }
        "#,
    );
    // Should generate a for-each loop with index
    assert_asm_contains(&asm, "ForEach loop");
    // Should store index in variable
    assert_asm_contains(&asm, "Store index in i");
}

#[test]
fn test_string_slicing() {
    let asm = compile_success(
        r#"
        const FULL: str = "Hello, World!";
        const GREETING: str = FULL[0..5];
        fn main() {
            let msg: str = GREETING;
        }
        "#,
    );
    // Should contain the sliced string "Hello"
    assert_asm_contains(&asm, "Hello");
    // Should NOT contain the full string if not used
    // (The slice result should be the only string)
}

#[test]
fn test_string_slicing_middle() {
    let asm = compile_success(
        r#"
        const FULL: str = "Hello, World!";
        const NAME: str = FULL[7..12];
        fn main() {
            let msg: str = NAME;
        }
        "#,
    );
    // Should contain the sliced string "World"
    assert_asm_contains(&asm, "World");
}

#[test]
fn test_string_slicing_with_concatenation() {
    let asm = compile_success(
        r#"
        const FULL: str = "Hello, World!";
        const PART1: str = FULL[0..5];
        const PART2: str = FULL[7..12];
        const COMBINED: str = PART1 + " " + PART2;
        fn main() {
            let msg: str = COMBINED;
        }
        "#,
    );
    // Should contain the combined string
    assert_asm_contains(&asm, "Hello World");
}

#[test]
fn test_string_len_property() {
    let asm = compile_success(
        r#"
        const MSG: str = "Hello";
        fn main() {
            let len: u16 = MSG.len;
        }
        "#,
    );
    // Should access length
    assert_asm_contains(&asm, "String .len access");
}

// DISABLED: String caching temporarily disabled due to initialization order issues
// #[test]
// fn test_string_caching_hot_strings() {
//     let asm = compile_success(
//         r#"
//         fn process_string(s: str) -> u16 {
//             // Access the string 4 times to trigger caching (3+ accesses)
//             let len1: u16 = s.len;
//             let len2: u16 = s.len;
//             let len3: u16 = s.len;
//             let len4: u16 = s.len;
//             return len1 + len2 + len3 + len4;
//         }
//         fn main() {}
//         "#,
//     );
//     // Should initialize string pointer cache
//     assert_asm_contains(&asm, "Initialize string pointer cache");
//     // Should use cached string
//     assert_asm_contains(&asm, "Cached string");
// }

#[test]
fn test_string_indexing() {
    let asm = compile_success(
        r#"
        const MSG: str = "ABC";
        fn main() {
            let first: u8 = MSG[0];
            let second: u8 = MSG[1];
        }
        "#,
    );
    // Should generate string indexing code
    assert_asm_contains(&asm, "String indexing: s[i]");
}
