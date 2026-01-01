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
    instructions
        .windows(pattern.len())
        .position(|window| {
            window.iter().zip(pattern.iter()).all(|(inst, pat)| {
                inst.contains(pat)
            })
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
    assert!(appears_before(&asm, "main:", "RTS"), "Label should appear before RTS");
}

#[test]
fn test_codegen_simple_assignment() {
    let source = r#"
        addr SCREEN = 0x0400;
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
    assert!(asm.contains("LDA #$2A"), "Should load immediate value 42 (0x2A)");
    assert!(asm.contains("STA SCREEN"), "Should store to SCREEN using symbolic name");

    // Verify ordering: LDA must come before STA
    assert!(appears_before(&asm, "LDA #$2A", "STA SCREEN"),
            "LDA should appear before STA");
}

#[test]
fn test_codegen_constant_folding() {
    let source = r#"
        addr RESULT = 0x0400;
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
    assert!(asm.contains("LDA #$1E"), "Should load folded constant 30 (0x1E)");
    assert!(asm.contains("STA RESULT"), "Should store to RESULT using symbolic name");

    // Should NOT have addition instructions
    assert!(!asm.contains("ADC"), "Should not have ADC instruction");
    assert!(!asm.contains("PHA"), "Should not push to stack");
}

#[test]
fn test_codegen_binary_op() {
    let source = r#"
        addr SCREEN = 0x0400;
        addr X = 0x0401;
        addr Y = 0x0402;
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
    assert!(asm.contains("SCREEN = $0400"), "Should have SCREEN address label");

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
    assert!(appears_before(&asm, "STA $20", "LDA X"), "Store temp before load X");
    assert!(appears_before(&asm, "CLC", "ADC"), "CLC before ADC");
    assert!(appears_before(&asm, "ADC", "STA SCREEN"), "ADC before final STA");
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

    assert!(asm.contains("BEQ"));
    assert!(asm.contains("JMP"));
    // Check for generated labels
    assert!(asm.contains("else_"));
    assert!(asm.contains("wh_"));
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
    // 2. JSR to function
    // 3. Function accesses args from parameter locations
    assert!(asm.contains("JSR add"), "Should call function with JSR");

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
    // 2. Length word (5, 0 for "Hello")
    // 3. String bytes
    // 4. Load address in code
    assert!(asm.contains("str_"), "Should have string label");
    assert!(asm.contains(".BYTE $05, $00"), "Should have length prefix (5)");
    assert!(asm.contains("$48"), "Should contain 'H' (0x48)");
    assert!(asm.contains("LDA #<str_"), "Should load low byte of address");
    assert!(asm.contains("LDX #>str_"), "Should load high byte of address");

    // Verify ordering - string data comes after code (in DATA section)
    assert!(appears_before(&asm, "main:", "str_0:"), "Code before data section");
}

#[test]
fn test_codegen_comparison_eq() {
    let source = r#"
        addr RESULT = 0x0400;
        addr X = 0x0401;
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
        addr RESULT = 0x0400;
        addr X = 0x0401;
        addr Y = 0x0402;
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
    assert!(asm.contains("CMP #$00"), "Should check if false");
    assert!(asm.contains("BEQ ax_"), "Should short-circuit if false");

    // CMP should come before BEQ
    let first_cmp = asm.find("CMP #$00").unwrap();
    let first_beq = asm.find("BEQ ax_").unwrap();
    assert!(first_cmp < first_beq, "CMP before BEQ for short-circuit");
}

#[test]
fn test_codegen_multiplication() {
    let source = r#"
        addr RESULT = 0x0400;
        addr X = 0x0401;
        addr Y = 0x0402;
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

    // Multiplication uses repeated addition:
    // 1. Save multiplicand to X register
    // 2. Initialize result to 0
    // 3. Loop: add multiplicand to result
    assert!(asm.contains("TAX"), "Should save multiplicand to X");
    assert!(asm.contains("TAY"), "Should save multiplier to Y");
    assert!(asm.contains("ml_"), "Should have multiply loop");
    assert!(asm.contains("TXA"), "Should load multiplicand");
    assert!(asm.contains("DEY"), "Should decrement counter");
    assert!(asm.contains("BNE ml_"), "Should loop until done");

    // Verify ordering
    assert!(appears_before(&asm, "TAX", "ml_"), "Setup before loop");
    assert!(appears_before(&asm, "ml_", "BNE ml_"), "Loop label before branch");
}

#[test]
fn test_codegen_division() {
    let source = r#"
        addr RESULT = 0x0400;
        addr X = 0x0401;
        addr Y = 0x0402;
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
    assert!(asm.contains("CPX #$00"), "Should check for division by zero");
    assert!(asm.contains("BEQ dx_"), "Should skip if divisor is zero");
    assert!(asm.contains("dl_"), "Should have division loop");
    assert!(asm.contains("SBC $20"), "Should subtract divisor");
    assert!(asm.contains("INC $22"), "Should increment quotient");
    assert!(asm.contains("BCC dx_"), "Should exit when dividend < divisor");

    // Verify ordering
    assert!(appears_before(&asm, "CPX", "BEQ dx_"), "Zero check before skip");
    assert!(appears_before(&asm, "dl_", "SBC"), "Loop before subtraction");
}

#[test]
fn test_codegen_shift_operations() {
    let source = r#"
        addr RESULT = 0x0400;
        addr X = 0x0401;
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
    assert!(appears_before(&asm, "ASL A", "DEX"), "Shift before decrement");
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
    assert!(appears_before(&asm, "TAX", "fl_"), "Transfer to X before loop");
    assert!(appears_before(&asm, "STA $22", "fl_"), "Store end before loop");
    assert!(appears_before(&asm, "fl_", "CPX"), "Loop label before check");
    assert!(appears_before(&asm, "INX", "JMP fl_"), "Increment before jump");
}

#[test]
fn test_codegen_unary_operations() {
    let source = r#"
        addr RESULT = 0x0400;
        addr X = 0x0401;
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
    assert!(appears_before(&asm, "EOR #$FF", "CLC"), "Invert before clear");
    assert!(appears_before(&asm, "CLC", "ADC #$01"), "Clear before add");
}

#[test]
fn test_codegen_nested_expressions() {
    let source = r#"
        addr RESULT = 0x0400;
        addr A = 0x0401;
        addr B = 0x0402;
        addr C = 0x0403;
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
    assert!(add_pos < mul_pos, "Addition (inner) should come before multiplication (outer)");
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
    assert!(asm.contains("; Enum variant: Direction::North"), "Should have enum comment");
    assert!(asm.contains("JMP es_"), "Should jump over data");
    assert!(asm.contains("en_"), "Should have enum label");
    assert!(asm.contains(".BYTE $00"), "Should emit tag byte 0 for first variant");

    // Should load address into A:X
    assert!(asm.contains("LDA #<en_"), "Should load low byte of address");
    assert!(asm.contains("LDX #>en_"), "Should load high byte of address");
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
    assert!(asm.contains("; Enum variant: Color::RGB"), "Should have enum comment");
    assert!(asm.contains("en_"), "Should have enum label");
    assert!(asm.contains(".BYTE $00"), "Should emit tag byte");
    assert!(asm.contains(".BYTE $FF"), "Should emit 255 (0xFF) for first field");
    assert!(asm.contains(".BYTE $80"), "Should emit 128 (0x80) for second field");
    assert!(asm.contains(".BYTE $40"), "Should emit 64 (0x40) for third field");

    // Should load address into A:X
    assert!(asm.contains("LDA #<en_"), "Should load low byte of address");
    assert!(asm.contains("LDX #>en_"), "Should load high byte of address");
}

#[test]
fn test_codegen_enum_struct_variant() {
    let source = r#"
        enum Message {
            Point { u8 x, u8 y },
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
    assert!(asm.contains("; Enum variant: Message::Point"), "Should have enum comment");
    assert!(asm.contains("en_"), "Should have enum label");
    assert!(asm.contains(".BYTE $00"), "Should emit tag byte");
    assert!(asm.contains(".BYTE $0A"), "Should emit 10 (0x0A) for x field");
    assert!(asm.contains(".BYTE $14"), "Should emit 20 (0x14) for y field");
}

#[test]
fn test_codegen_enum_pattern_matching() {
    let source = r#"
        enum Status {
            Off,
            On,
            Error,
        }

        addr RESULT = 0x0401;

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

    // Should have match statement structure
    assert!(asm.contains("; Match statement"), "Should have match comment");

    // Should store enum pointer and load tag
    assert!(asm.contains("STA $20"), "Should store pointer low byte");
    assert!(asm.contains("STX $21"), "Should store pointer high byte");
    assert!(asm.contains("LDY #$00"), "Should set Y to 0");
    assert!(asm.contains("LDA ($20),Y"), "Should load tag byte using indirect indexed");
    assert!(asm.contains("STA $22"), "Should store tag at $22");

    // Should compare with each variant tag
    assert!(asm.contains("CMP #$00"), "Should compare with tag 0 (Off)");
    assert!(asm.contains("CMP #$01"), "Should compare with tag 1 (On)");
    assert!(asm.contains("CMP #$02"), "Should compare with tag 2 (Error)");

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
    assert!(asm.contains("; Enum variant: Option::Some"), "Should have enum comment");
    assert!(asm.contains("en_"), "Should have enum label");
    assert!(asm.contains(".BYTE $01"), "Should emit tag byte 1 for second variant");
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
            result: u8 = add(5, 3);
            other: u8 = regular_fn(result);
        }
    "#;

    let tokens = lex(source).unwrap();
    let ast = Parser::parse(&tokens).unwrap();
    let program = analyze(&ast).unwrap();
    let (asm, _) = generate(&ast, &program, CommentVerbosity::Normal).unwrap();

    // Inline functions are NOT emitted as separate functions
    assert!(!asm.contains("add:"), "Should NOT have add label (inline function)");

    // Regular function should be emitted normally
    assert!(asm.contains("regular_fn:"), "Should have regular_fn label");

    // Main function should inline the add call (no JSR to add)
    let main_start = asm.find("main:").unwrap();
    let main_section = &asm[main_start..];

    // Should have comment indicating inline expansion
    assert!(main_section.contains("; Inline: add"), "Should have inline comment");

    // Should NOT have JSR to add in main
    assert!(!main_section.contains("JSR add"), "Should not have JSR to add (inlined)");

    // Should have JSR to regular_fn (not inlined)
    assert!(main_section.contains("JSR regular_fn"), "Should have JSR to regular_fn");

    // Verify the inline expansion actually happened:
    // The add function loads params from $40, $41 and does ADC
    // The inlined version should do the same without JSR/RTS
    assert!(main_section.contains("LDA #$05"), "Should load immediate 5");
    assert!(main_section.contains("LDA #$03"), "Should load immediate 3");
    assert!(main_section.contains("ADC"), "Should have ADC instruction from inlined add");
}
