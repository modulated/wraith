use wraith::codegen::{generate, CommentVerbosity};
use wraith::lex;
use wraith::parser::Parser;
use wraith::sema::analyze;

// Helper function to extract instructions from assembly output
#[allow(dead_code)]
fn extract_instructions(asm: &str) -> Vec<String> {
    asm.lines()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty() && !line.starts_with(';'))
        .map(|line| line.to_string())
        .collect()
}

// Helper to find instruction sequence
#[allow(dead_code)]
fn find_sequence(instructions: &[String], pattern: &[&str]) -> Option<usize> {
    instructions.windows(pattern.len()).position(|window| {
        window
            .iter()
            .zip(pattern.iter())
            .all(|(inst, pat)| inst.contains(pat))
    })
}

// Helper to verify instruction appears before another
fn appears_before(asm: &str, first: &str, second: &str) -> bool {
    if let (Some(first_pos), Some(second_pos)) = (asm.find(first), asm.find(second)) {
        first_pos < second_pos
    } else {
        false
    }
}

#[test]
fn test_codegen_empty_function() {
    let source = r#"
        fn main() {}
    "#;

    let tokens = lex(source).unwrap();
    let ast = Parser::parse(&tokens).unwrap();
    let program = analyze(&ast).unwrap();
    let (asm, _) = generate(&ast, &program, CommentVerbosity::Normal).unwrap();

    // Verify structure
    assert!(asm.contains("main:"), "Should have main label");
    assert!(asm.contains("RTS"), "Should have RTS instruction");
    assert!(
        appears_before(&asm, "main:", "RTS"),
        "Label should appear before RTS"
    );
}

#[test]
fn test_codegen_simple_assignment() {
    let source = r#"
        const SCREEN: addr = 0x0400;
        fn main() {
            SCREEN = 42;
        }
    "#;

    let tokens = lex(source).unwrap();
    let ast = Parser::parse(&tokens).unwrap();
    let program = analyze(&ast).unwrap();
    let (asm, _) = generate(&ast, &program, CommentVerbosity::Normal).unwrap();

    // Should load 42 into A, then store to SCREEN (using symbolic name)
    assert!(asm.contains("SCREEN = $0400"), "Should have address label");
    assert!(
        asm.contains("LDA #$2A"),
        "Should load immediate value 42 (0x2A)"
    );
    assert!(
        asm.contains("STA SCREEN"),
        "Should store to SCREEN using symbolic name"
    );

    // Verify ordering: LDA must come before STA
    assert!(
        appears_before(&asm, "LDA #$2A", "STA SCREEN"),
        "LDA should appear before STA"
    );
}

#[test]
fn test_codegen_constant_folding() {
    let source = r#"
        const RESULT: addr = 0x0400;
        fn main() {
            RESULT = 10 + 20;  // Should fold to 30 (0x1E)
        }
    "#;

    let tokens = lex(source).unwrap();
    let ast = Parser::parse(&tokens).unwrap();
    let program = analyze(&ast).unwrap();
    let (asm, _) = generate(&ast, &program, CommentVerbosity::Normal).unwrap();

    // Should generate constant folded result
    assert!(asm.contains("RESULT = $0400"), "Should have address label");
    assert!(
        asm.contains("LDA #$1E"),
        "Should load folded constant 30 (0x1E)"
    );
    assert!(
        asm.contains("STA RESULT"),
        "Should store to RESULT using symbolic name"
    );

    // Should NOT have addition instructions
    assert!(!asm.contains("ADC"), "Should not have ADC instruction");
    assert!(!asm.contains("PHA"), "Should not push to stack");
}

#[test]
fn test_codegen_binary_op() {
    let source = r#"
        const SCREEN: addr = 0x0400;
        const X: addr = 0x0401;
        const Y: addr = 0x0402;
        fn main() {
            X = 10;
            Y = 20;
            SCREEN = X + Y;
        }
    "#;

    let tokens = lex(source).unwrap();
    let ast = Parser::parse(&tokens).unwrap();
    let program = analyze(&ast).unwrap();
    let (asm, _) = generate(&ast, &program, CommentVerbosity::Normal).unwrap();

    // Verify optimized binary operation sequence:
    // For X + Y where both are addr variables:
    // 1. Assignments to X and Y happen first
    // 2. For the addition, right operand (Y) is evaluated first (already in A from assignment)
    // 3. Store to TEMP (store-load optimization: no LDA Y needed, value already in A)
    // 4. Load left operand (X)
    // 5. Add with carry (ADC)
    // 6. Store result to SCREEN

    // Check address labels
    assert!(asm.contains("X = $0401"), "Should have X address label");
    assert!(asm.contains("Y = $0402"), "Should have Y address label");
    assert!(
        asm.contains("SCREEN = $0400"),
        "Should have SCREEN address label"
    );

    // Check that assignments happen (using symbolic names)
    assert!(asm.contains("STA X"), "Should store to X");
    assert!(asm.contains("STA Y"), "Should store to Y");

    // Check the addition operations (using symbolic names)
    assert!(asm.contains("LDA X"), "Should load from X");
    assert!(asm.contains("STA $20"), "Should use TEMP for binary op");
    assert!(asm.contains("CLC"), "Should clear carry");
    assert!(asm.contains("ADC $20"), "Should add from TEMP");
    assert!(asm.contains("STA SCREEN"), "Should store result to SCREEN");

    // Verify ordering of the addition
    assert!(
        appears_before(&asm, "STA $20", "LDA X"),
        "Store temp before load X"
    );
    assert!(appears_before(&asm, "CLC", "ADC"), "CLC before ADC");
    assert!(
        appears_before(&asm, "ADC", "STA SCREEN"),
        "ADC before final STA"
    );
}

#[test]
fn test_codegen_control_flow() {
    let source = r#"
        fn main() {
            if true {
                // do something
            }
            while true {
                // loop
            }
        }
    "#;

    let tokens = lex(source).unwrap();
    let ast = Parser::parse(&tokens).unwrap();
    let program = analyze(&ast).unwrap();
    let (asm, _) = generate(&ast, &program, CommentVerbosity::Normal).unwrap();

    // Check for branch instructions (BEQ or BNE - optimizer may invert branches)
    assert!(
        asm.contains("BEQ") || asm.contains("BNE"),
        "Should have branch instruction"
    );
    // Check for while loop labels
    assert!(asm.contains("wh_"), "While loop should have wh_ label");
}

#[test]
fn test_codegen_function_call() {
    let source = r#"
        fn add(a: u8, b: u8) -> u8 {
            return a + b;
        }

        fn main() {
            add(10, 20);
        }
    "#;

    let tokens = lex(source).unwrap();
    let ast = Parser::parse(&tokens).unwrap();
    let program = analyze(&ast).unwrap();
    let (asm, _) = generate(&ast, &program, CommentVerbosity::Normal).unwrap();

    // Function call should:
    // 1. Store arguments to zero page argument area (not using hardware stack)
    // 2. JSR to function (or JMP if tail-call optimized)
    // 3. Function accesses args from parameter locations
    assert!(
        asm.contains("JSR add") || asm.contains("JMP add"),
        "Should call function with JSR or JMP (tail-call optimized)"
    );

    // Our calling convention uses zero page, not hardware stack (PHA/PLA)
    // Arguments are stored to zero page locations before JSR
    assert!(asm.contains("STA $"), "Should store arguments to zero page");
    assert!(asm.contains("LDA #$0A"), "Should load first argument (10)");
    assert!(asm.contains("LDA #$14"), "Should load second argument (20)");
}

// ============================================================================
// NEW COMPREHENSIVE TESTS WITH ORDERING VERIFICATION
// ============================================================================

#[test]
fn test_codegen_string_literal() {
    let source = r#"
        fn main() {
            "Hello";
        }
    "#;

    let tokens = lex(source).unwrap();
    let ast = Parser::parse(&tokens).unwrap();
    let program = analyze(&ast).unwrap();
    let (asm, _) = generate(&ast, &program, CommentVerbosity::Normal).unwrap();

    // String should have:
    // 1. String label in DATA section
    // 2. Length byte (5 for "Hello") - now u8 instead of u16
    // 3. String bytes
    // 4. Load address in code
    assert!(asm.contains("str_"), "Should have string label");
    assert!(
        asm.contains(".BYTE $05  ; length = 5"),
        "Should have length prefix (5)"
    );
    assert!(asm.contains("$48"), "Should contain 'H' (0x48)");
    assert!(
        asm.contains("LDA #<str_"),
        "Should load low byte of address"
    );
    assert!(
        asm.contains("LDX #>str_"),
        "Should load high byte of address"
    );

    // Verify ordering - string data comes after code (in DATA section)
    // Note: With content-based labels, string labels are now hashes like "str_a1b2c3d4:"
    assert!(
        asm.find("main:").unwrap() < asm.find("str_").unwrap(),
        "Code before data section"
    );
}

#[test]
fn test_codegen_comparison_eq() {
    let source = r#"
        const RESULT: addr = 0x0400;
        const X: addr = 0x0401;
        fn main() {
            X = 5;
            RESULT = X == 5;
        }
    "#;

    let tokens = lex(source).unwrap();
    let ast = Parser::parse(&tokens).unwrap();
    let program = analyze(&ast).unwrap();
    let (asm, _) = generate(&ast, &program, CommentVerbosity::Normal).unwrap();

    // Comparison should:
    // 1. Load operands and compare
    // 2. Branch on condition
    // 3. Set result to 0 or 1
    assert!(asm.contains("CMP $20"), "Should compare with TEMP");
    assert!(asm.contains("BEQ et_"), "Should branch if equal");
    assert!(asm.contains("LDA #$00"), "Should load false value");
    assert!(asm.contains("LDA #$01"), "Should load true value");

    // Verify ordering
    assert!(appears_before(&asm, "CMP", "BEQ"), "CMP before BEQ");
}

#[test]
fn test_codegen_logical_and_short_circuit() {
    let source = r#"
        const RESULT: addr = 0x0400;
        const X: addr = 0x0401;
        const Y: addr = 0x0402;
        fn main() {
            X = 0;
            Y = 1;
            RESULT = X && Y;
        }
    "#;

    let tokens = lex(source).unwrap();
    let ast = Parser::parse(&tokens).unwrap();
    let program = analyze(&ast).unwrap();
    let (asm, _) = generate(&ast, &program, CommentVerbosity::Normal).unwrap();

    // Check address labels
    assert!(asm.contains("X = $0401"), "Should have X address label");
    assert!(asm.contains("Y = $0402"), "Should have Y address label");

    // Short-circuit AND:
    // 1. Evaluate left (from X)
    // 2. If false (0), skip right and jump to end
    // 3. Otherwise evaluate right
    assert!(asm.contains("LDA X"), "Should load left operand from X");
    // Note: CMP #$00 is optimized away because LDA already sets the Z flag
    assert!(asm.contains("BEQ ax_"), "Should short-circuit if false");

    // LDA should come before BEQ
    let first_lda_x = asm.find("LDA X").unwrap();
    let first_beq = asm.find("BEQ ax_").unwrap();
    assert!(
        first_lda_x < first_beq,
        "LDA X before BEQ for short-circuit"
    );
}

#[test]
fn test_codegen_multiplication() {
    let source = r#"
        const RESULT: addr = 0x0400;
        const X: addr = 0x0401;
        const Y: addr = 0x0402;
        fn main() {
            X = 5;
            Y = 3;
            RESULT = X * Y;
        }
    "#;

    let tokens = lex(source).unwrap();
    let ast = Parser::parse(&tokens).unwrap();
    let program = analyze(&ast).unwrap();
    let (asm, _) = generate(&ast, &program, CommentVerbosity::Normal).unwrap();

    // Multiplication uses shift-and-add algorithm:
    // 1. Save multiplicand to memory
    // 2. Initialize result to 0
    // 3. Loop 8 times: check multiplier bit, add if set, shift both values
    assert!(
        asm.contains("LDX #$08"),
        "Should initialize loop counter to 8"
    );
    assert!(asm.contains("ml_"), "Should have multiply loop label");
    assert!(asm.contains("LSR"), "Should shift multiplier right");
    assert!(asm.contains("BCC"), "Should branch if bit clear (skip add)");
    assert!(asm.contains("ASL"), "Should shift multiplicand left");
    assert!(asm.contains("DEX"), "Should decrement loop counter");
    assert!(asm.contains("BNE ml_"), "Should loop until done");

    // Verify ordering
    assert!(appears_before(&asm, "LDX #$08", "ml_"), "Setup before loop");
    assert!(
        appears_before(&asm, "ml_", "BNE ml_"),
        "Loop label before branch"
    );
}

#[test]
fn test_codegen_division() {
    let source = r#"
        const RESULT: addr = 0x0400;
        const X: addr = 0x0401;
        const Y: addr = 0x0402;
        fn main() {
            X = 10;
            Y = 3;
            RESULT = X / Y;
        }
    "#;

    let tokens = lex(source).unwrap();
    let ast = Parser::parse(&tokens).unwrap();
    let program = analyze(&ast).unwrap();
    let (asm, _) = generate(&ast, &program, CommentVerbosity::Normal).unwrap();

    // Division uses repeated subtraction
    assert!(
        asm.contains("CPX #$00"),
        "Should check for division by zero"
    );
    assert!(asm.contains("BEQ dx_"), "Should skip if divisor is zero");
    assert!(asm.contains("dl_"), "Should have division loop");
    assert!(asm.contains("SBC $20"), "Should subtract divisor");
    assert!(asm.contains("INC $22"), "Should increment quotient");
    assert!(
        asm.contains("BCC dx_"),
        "Should exit when dividend < divisor"
    );

    // Verify ordering
    assert!(
        appears_before(&asm, "CPX", "BEQ dx_"),
        "Zero check before skip"
    );
    assert!(
        appears_before(&asm, "dl_", "SBC"),
        "Loop before subtraction"
    );
}

#[test]
fn test_codegen_shift_operations() {
    let source = r#"
        const RESULT: addr = 0x0400;
        const X: addr = 0x0401;
        fn main() {
            X = 8;
            RESULT = X << 2;
        }
    "#;

    let tokens = lex(source).unwrap();
    let ast = Parser::parse(&tokens).unwrap();
    let program = analyze(&ast).unwrap();
    let (asm, _) = generate(&ast, &program, CommentVerbosity::Normal).unwrap();

    // Shift uses iterative ASL
    assert!(asm.contains("LDX $20"), "Should load shift count");
    assert!(asm.contains("sl_"), "Should have shift loop");
    assert!(asm.contains("ASL A"), "Should arithmetic shift left");
    assert!(asm.contains("DEX"), "Should decrement counter");
    assert!(asm.contains("BNE sl_"), "Should loop");

    // Verify ordering
    assert!(appears_before(&asm, "LDX", "sl_"), "Load count before loop");
    assert!(
        appears_before(&asm, "ASL A", "DEX"),
        "Shift before decrement"
    );
}

#[test]
fn test_codegen_for_loop() {
    let source = r#"
        fn main() {
            for i in 0..10 {
                // body
            }
        }
    "#;

    let tokens = lex(source).unwrap();
    let ast = Parser::parse(&tokens).unwrap();
    let program = analyze(&ast).unwrap();
    let (asm, _) = generate(&ast, &program, CommentVerbosity::Normal).unwrap();

    // Optimized for loop using X register:
    // 1. Initialize counter in X register (TAX)
    // 2. Store end value in temp location
    // 3. Loop: check condition with CPX, execute body, increment with INX
    assert!(asm.contains("TAX"), "Should transfer counter to X");
    assert!(asm.contains("fl_"), "Should have loop label");
    assert!(asm.contains("fx_"), "Should have end label");
    assert!(asm.contains("CPX $22"), "Should compare X with end value");
    assert!(asm.contains("BCS fx_"), "Should exit if counter >= end");
    assert!(asm.contains("INX"), "Should increment X register");
    assert!(asm.contains("JMP fl_"), "Should jump back to start");

    // Verify ordering
    assert!(
        appears_before(&asm, "TAX", "fl_"),
        "Transfer to X before loop"
    );
    assert!(
        appears_before(&asm, "STA $22", "fl_"),
        "Store end before loop"
    );
    assert!(
        appears_before(&asm, "fl_", "CPX"),
        "Loop label before check"
    );
    assert!(
        appears_before(&asm, "INX", "JMP fl_"),
        "Increment before jump"
    );
}

#[test]
fn test_codegen_unary_operations() {
    let source = r#"
        const RESULT: addr = 0x0400;
        const X: addr = 0x0401;
        fn main() {
            X = 5;
            RESULT = -X;
        }
    "#;

    let tokens = lex(source).unwrap();
    let ast = Parser::parse(&tokens).unwrap();
    let program = analyze(&ast).unwrap();
    let (asm, _) = generate(&ast, &program, CommentVerbosity::Normal).unwrap();

    // Negation uses two's complement: ~A + 1
    assert!(asm.contains("EOR #$FF"), "Should invert bits");
    assert!(asm.contains("CLC"), "Should clear carry");
    assert!(asm.contains("ADC #$01"), "Should add 1");

    // Verify ordering
    assert!(
        appears_before(&asm, "EOR #$FF", "CLC"),
        "Invert before clear"
    );
    assert!(appears_before(&asm, "CLC", "ADC #$01"), "Clear before add");
}

#[test]
fn test_codegen_nested_expressions() {
    let source = r#"
        const RESULT: addr = 0x0400;
        const A: addr = 0x0401;
        const B: addr = 0x0402;
        const C: addr = 0x0403;
        fn main() {
            A = 5;
            B = 3;
            C = 2;
            RESULT = (A + B) * C;
        }
    "#;

    let tokens = lex(source).unwrap();
    let ast = Parser::parse(&tokens).unwrap();
    let program = analyze(&ast).unwrap();
    let (asm, _) = generate(&ast, &program, CommentVerbosity::Normal).unwrap();

    // Should have both addition and multiplication
    assert!(asm.contains("ADC $20"), "Should have addition");
    assert!(asm.contains("ml_"), "Should have multiplication");

    // Addition should come before multiplication
    let add_pos = asm.find("ADC $20").unwrap();
    let mul_pos = asm.find("ml_").unwrap();
    assert!(
        add_pos < mul_pos,
        "Addition (inner) should come before multiplication (outer)"
    );
}

#[test]
fn test_codegen_enum_unit_variant() {
    let source = r#"
        enum Direction {
            North,
            South,
            East,
            West,
        }

        fn main() {
            // Just call the enum constructor (don't store)
            Direction::North;
        }
    "#;

    let tokens = lex(source).unwrap();
    let ast = Parser::parse(&tokens).unwrap();
    let program = analyze(&ast).unwrap();
    let (asm, _) = generate(&ast, &program, CommentVerbosity::Normal).unwrap();

    // Should generate enum data with tag
    assert!(
        asm.contains("; Enum variant: Direction::North"),
        "Should have enum comment"
    );
    assert!(asm.contains("JMP es_"), "Should jump over data");
    assert!(asm.contains("en_"), "Should have enum label");
    assert!(
        asm.contains(".BYTE $00"),
        "Should emit tag byte 0 for first variant"
    );

    // Should load address into A:X
    assert!(asm.contains("LDA #<en_"), "Should load low byte of address");
    assert!(
        asm.contains("LDX #>en_"),
        "Should load high byte of address"
    );
}

#[test]
fn test_codegen_enum_tuple_variant() {
    let source = r#"
        enum Color {
            RGB(u8, u8, u8),
        }

        fn main() {
            Color::RGB(255, 128, 64);
        }
    "#;

    let tokens = lex(source).unwrap();
    let ast = Parser::parse(&tokens).unwrap();
    let program = analyze(&ast).unwrap();
    let (asm, _) = generate(&ast, &program, CommentVerbosity::Normal).unwrap();

    // Should generate enum data with tag and fields
    assert!(
        asm.contains("; Enum variant: Color::RGB"),
        "Should have enum comment"
    );
    assert!(asm.contains("en_"), "Should have enum label");
    assert!(asm.contains(".BYTE $00"), "Should emit tag byte");
    assert!(
        asm.contains(".BYTE $FF"),
        "Should emit 255 (0xFF) for first field"
    );
    assert!(
        asm.contains(".BYTE $80"),
        "Should emit 128 (0x80) for second field"
    );
    assert!(
        asm.contains(".BYTE $40"),
        "Should emit 64 (0x40) for third field"
    );

    // Should load address into A:X
    assert!(asm.contains("LDA #<en_"), "Should load low byte of address");
    assert!(
        asm.contains("LDX #>en_"),
        "Should load high byte of address"
    );
}

#[test]
fn test_codegen_enum_struct_variant() {
    let source = r#"
        enum Message {
            Point { x: u8, y: u8 },
        }

        fn main() {
            Message::Point { x: 10, y: 20 };
        }
    "#;

    let tokens = lex(source).unwrap();
    let ast = Parser::parse(&tokens).unwrap();
    let program = analyze(&ast).unwrap();
    let (asm, _) = generate(&ast, &program, CommentVerbosity::Normal).unwrap();

    // Should generate enum data with tag and named fields
    assert!(
        asm.contains("; Enum variant: Message::Point"),
        "Should have enum comment"
    );
    assert!(asm.contains("en_"), "Should have enum label");
    assert!(asm.contains(".BYTE $00"), "Should emit tag byte");
    assert!(
        asm.contains(".BYTE $0A"),
        "Should emit 10 (0x0A) for x field"
    );
    assert!(
        asm.contains(".BYTE $14"),
        "Should emit 20 (0x14) for y field"
    );
}

#[test]
fn test_codegen_enum_pattern_matching() {
    let source = r#"
        enum Status {
            Off,
            On,
            Error,
        }

        const RESULT: addr = 0x0401;

        fn main() {
            match Status::On {
                Status::Off => {
                    RESULT = 0;
                }
                Status::On => {
                    RESULT = 1;
                }
                Status::Error => {
                    RESULT = 255;
                }
            }
        }
    "#;

    let tokens = lex(source).unwrap();
    let ast = Parser::parse(&tokens).unwrap();
    let program = analyze(&ast).unwrap();
    let (asm, _) = generate(&ast, &program, CommentVerbosity::Normal).unwrap();

    // Should have match statement structure (jump table for 3+ arms)
    assert!(
        asm.contains("; Match statement (jump table)"),
        "Should have jump table match comment"
    );

    // Should store enum pointer and load tag
    assert!(asm.contains("STA $30"), "Should store pointer low byte");
    assert!(asm.contains("STX $31"), "Should store pointer high byte");
    assert!(asm.contains("LDY #$00"), "Should set Y to 0");
    assert!(
        asm.contains("LDA ($30),Y"),
        "Should load tag byte using indirect indexed"
    );
    assert!(asm.contains("STA $32"), "Should store tag at $32");

    // Should have jump table dispatch
    assert!(
        asm.contains("ASL"),
        "Should double tag for address indexing"
    );
    assert!(asm.contains("TAX"), "Should transfer to X for indexing");
    assert!(
        asm.contains("LDA match_0_jt,X"),
        "Should load jump address low byte"
    );
    assert!(
        asm.contains("STA $30"),
        "Should store jump address low byte"
    );
    assert!(
        asm.contains("LDA match_0_jt+1,X"),
        "Should load jump address high byte"
    );
    assert!(
        asm.contains("STA $31"),
        "Should store jump address high byte"
    );
    assert!(asm.contains("JMP ($30)"), "Should jump indirect");

    // Should have jump table with .WORD entries
    assert!(asm.contains("match_0_jt:"), "Should have jump table label");
    assert!(
        asm.contains(".WORD match_0_arm_0"),
        "Should have arm 0 in jump table"
    );
    assert!(
        asm.contains(".WORD match_0_arm_1"),
        "Should have arm 1 in jump table"
    );
    assert!(
        asm.contains(".WORD match_0_arm_2"),
        "Should have arm 2 in jump table"
    );

    // Should have match arm labels
    assert!(asm.contains("match_0_arm_0:"), "Should have arm 0 label");
    assert!(asm.contains("match_0_arm_1:"), "Should have arm 1 label");
    assert!(asm.contains("match_0_arm_2:"), "Should have arm 2 label");
    assert!(asm.contains("match_0_end:"), "Should have match end label");

    // Should have different values for each arm
    assert!(asm.contains("LDA #$00"), "Should have value 0");
    assert!(asm.contains("LDA #$01"), "Should have value 1");
    assert!(asm.contains("LDA #$FF"), "Should have value 255");
}

#[test]
fn test_codegen_enum_multiple_variants() {
    let source = r#"
        enum Option {
            None,
            Some(u8),
        }

        fn main() {
            Option::Some(42);
        }
    "#;

    let tokens = lex(source).unwrap();
    let ast = Parser::parse(&tokens).unwrap();
    let program = analyze(&ast).unwrap();
    let (asm, _) = generate(&ast, &program, CommentVerbosity::Normal).unwrap();

    // Should generate enum with tag 1 (second variant)
    assert!(
        asm.contains("; Enum variant: Option::Some"),
        "Should have enum comment"
    );
    assert!(asm.contains("en_"), "Should have enum label");
    assert!(
        asm.contains(".BYTE $01"),
        "Should emit tag byte 1 for second variant"
    );
    assert!(asm.contains(".BYTE $2A"), "Should emit 42 (0x2A) for data");
}

#[test]
fn test_codegen_inline_function() {
    let source = r#"
        #[inline]
        fn add(a: u8, b: u8) -> u8 {
            return a + b;
        }

        fn regular_fn(x: u8) -> u8 {
            return x;
        }

        fn main() {
            let result: u8 = add(5, 3);
            let other: u8 = regular_fn(result);
        }
    "#;

    let tokens = lex(source).unwrap();
    let ast = Parser::parse(&tokens).unwrap();
    let program = analyze(&ast).unwrap();
    let (asm, _) = generate(&ast, &program, CommentVerbosity::Normal).unwrap();

    // Inline functions are NOT emitted as separate functions
    assert!(
        !asm.contains("add:"),
        "Should NOT have add label (inline function)"
    );

    // Regular function should be emitted normally
    assert!(asm.contains("regular_fn:"), "Should have regular_fn label");

    // Main function should inline the add call (no JSR to add)
    let main_start = asm.find("main:").unwrap();
    let main_section = &asm[main_start..];

    // Should have comment indicating inline expansion
    assert!(
        main_section.contains("; Inline: add"),
        "Should have inline comment"
    );

    // Should NOT have JSR to add in main
    assert!(
        !main_section.contains("JSR add"),
        "Should not have JSR to add (inlined)"
    );

    // Should have JSR to regular_fn (not inlined)
    assert!(
        main_section.contains("JSR regular_fn"),
        "Should have JSR to regular_fn"
    );

    // Verify the inline expansion actually happened:
    // The add function loads params from $40, $41 and does ADC
    // The inlined version should do the same without JSR/RTS
    assert!(main_section.contains("LDA #$05"), "Should load immediate 5");
    assert!(main_section.contains("LDA #$03"), "Should load immediate 3");
    assert!(
        main_section.contains("ADC"),
        "Should have ADC instruction from inlined add"
    );
}

#[test]
fn test_codegen_recursive_function() {
    let source = r#"
        fn fib(n: u8) -> u16 {
            if (n <= 1) {
                return n as u16;
            }
            return fib(n - 1) + fib(n - 2);
        }

        fn main() {
            let result: u16 = fib(4);
        }
    "#;

    let tokens = lex(source).unwrap();
    let ast = Parser::parse(&tokens).unwrap();
    let program = analyze(&ast).unwrap();
    let (asm, _) = generate(&ast, &program, CommentVerbosity::Normal).unwrap();

    // Recursive functions must save/restore parameter area to prevent
    // nested calls from overwriting the caller's parameters

    // Should have fib function
    assert!(asm.contains("fib:"), "Should have fib function label");

    // Should have recursive calls to fib
    assert!(asm.contains("JSR fib"), "Should have JSR to fib");

    // Critical: Parameters must be preserved across recursive calls
    // Parameters are now saved using software stack (push/pop to $0200+ using $FF pointer)
    let fib_start = asm.find("fib:").unwrap();
    let fib_section = &asm[fib_start..];

    // Check if parameters are saved using software stack
    // Push saves to $0200,X where X is loaded from $FF (stack pointer)
    let push_count = fib_section.matches("STA $0200,X").count();
    assert!(
        push_count >= 1,
        "Should push parameters to software stack (found {})",
        push_count
    );

    // Must also restore parameters before evaluating right operand
    // Pop loads from $0200,X and stores back to $80+
    let pop_count = fib_section.matches("LDA $0200,X").count();
    assert!(
        pop_count >= 1,
        "Should pop parameters from software stack (found {})",
        pop_count
    );
}

#[test]
fn test_codegen_tail_call_optimization() {
    let source = r#"
        fn factorial(n: u8, acc: u16) -> u16 {
            if n == 0 {
                return acc;
            }
            return factorial(n - 1, acc * (n as u16));
        }

        fn main() {
            let result: u16 = factorial(5, 1);
        }
    "#;

    let tokens = lex(source).unwrap();
    let ast = Parser::parse(&tokens).unwrap();
    let program = analyze(&ast).unwrap();
    let (asm, _) = generate(&ast, &program, CommentVerbosity::Normal).unwrap();

    // Tail recursive functions should have loop restart label
    assert!(
        asm.contains("factorial_loop_start:"),
        "Should have loop restart label"
    );

    // Should have tail call optimization comment
    assert!(
        asm.contains("Tail recursive function - loop optimization enabled"),
        "Should have tail recursion comment"
    );

    // Should optimize tail call to JMP instead of JSR
    assert!(
        asm.contains("JMP factorial_loop_start"),
        "Should have JMP to loop start for tail recursive call"
    );

    // The tail call should not use JSR factorial inside the factorial function
    // (JSR from main calling factorial is fine and expected)
    let factorial_start = asm.find("factorial:").unwrap();

    // Find where the factorial function ends (next function starts with "; Function: ")
    let next_function = asm[factorial_start..]
        .find("\n; Function: ")
        .map(|pos| factorial_start + pos)
        .unwrap_or(asm.len());

    let factorial_section = &asm[factorial_start..next_function];

    // Count JSR factorial calls within the factorial function
    // Should be 0 (the tail call is converted to JMP)
    let jsr_count = factorial_section.matches("JSR factorial").count();
    assert_eq!(
        jsr_count, 0,
        "Tail recursive call should not use JSR (found {} JSR calls)",
        jsr_count
    );

    // Verify the function has a normal RTS for the base case
    assert!(
        factorial_section.contains("RTS"),
        "Should still have RTS for base case return"
    );
}

#[test]
fn test_codegen_match_dead_code_elimination() {
    // Test that match arms with return statements don't emit unreachable JMP instructions
    let source = r#"
        enum State { A, B, C }

        fn get_value(s: State) -> u8 {
            match s {
                State::A => { return 1; }
                State::B => { return 2; }
                State::C => { return 3; }
            }
        }

        fn main() {
            let x: u8 = get_value(State::A);
        }
    "#;

    let tokens = lex(source).unwrap();
    let ast = Parser::parse(&tokens).unwrap();
    let program = analyze(&ast).unwrap();
    let (asm, _) = generate(&ast, &program, CommentVerbosity::Normal).unwrap();

    // Find the get_value function section
    let fn_start = asm
        .find("get_value:")
        .expect("Should have get_value function");
    let fn_end = asm[fn_start..]
        .find("\n; Function: ")
        .map(|pos| fn_start + pos)
        .unwrap_or(asm.len());
    let fn_section = &asm[fn_start..fn_end];

    // Should have RTS instructions (one per match arm)
    let rts_count = fn_section.matches("RTS").count();
    assert!(
        rts_count >= 3,
        "Should have at least 3 RTS instructions (one per arm), found {}",
        rts_count
    );

    // Should NOT have unreachable JMP after RTS pattern
    // Check that no JMP immediately follows RTS (with only whitespace/comments between)
    let lines: Vec<&str> = fn_section.lines().collect();
    for i in 0..lines.len().saturating_sub(1) {
        let line = lines[i].trim();
        let next_line = lines[i + 1].trim();
        if line == "RTS" && next_line.starts_with("JMP match_") {
            panic!("Found unreachable JMP after RTS: {} -> {}", line, next_line);
        }
    }
}

#[test]
fn test_codegen_match_no_jmp_after_break() {
    // Test that match arms in loops with break don't emit unreachable JMP
    let source = r#"
        enum Cmd { Stop, Continue }

        fn main() {
            let cmd: Cmd = Cmd::Stop;
            loop {
                match cmd {
                    Cmd::Stop => { break; }
                    Cmd::Continue => { }
                }
            }
        }
    "#;

    let tokens = lex(source).unwrap();
    let ast = Parser::parse(&tokens).unwrap();
    let program = analyze(&ast).unwrap();
    let (asm, _) = generate(&ast, &program, CommentVerbosity::Normal).unwrap();

    // The Stop arm ends with break, so no JMP match_X_end should follow
    // The Continue arm doesn't terminate, so it SHOULD have JMP match_X_end

    // Should have exactly 1 JMP to match end (from Continue arm only)
    // The Stop arm with break should NOT have a JMP
    let lines: Vec<&str> = asm.lines().collect();
    let mut found_break_with_jmp = false;
    for i in 0..lines.len().saturating_sub(1) {
        let line = lines[i].trim();
        // Look for the pattern where we jump out of loop (break) followed by JMP match_end
        if line.starts_with("JMP lp_") || line.starts_with("JMP lx_") {
            // This is a break - check if next non-comment line is JMP match_end
            for next_line in lines.iter().skip(i + 1) {
                let next = next_line.trim();
                if next.is_empty() || next.starts_with(';') {
                    continue;
                }
                if next.starts_with("JMP match_") && next.contains("_end") {
                    found_break_with_jmp = true;
                }
                break;
            }
        }
    }
    assert!(
        !found_break_with_jmp,
        "Should not have JMP match_end after break"
    );
}
