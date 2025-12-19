use wraith::codegen::generate;
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
    let asm = generate(&ast, &program).unwrap();

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
    let asm = generate(&ast, &program).unwrap();

    // Should load 42 into A, then store to SCREEN
    assert!(asm.contains("LDA #$2A"), "Should load immediate value 42 (0x2A)");
    assert!(asm.contains("STA $0400"), "Should store to address 0x0400");

    // Verify ordering: LDA must come before STA
    assert!(appears_before(&asm, "LDA #$2A", "STA $0400"),
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
    let asm = generate(&ast, &program).unwrap();

    // Should generate constant folded result
    assert!(asm.contains("; Constant folded"), "Should have constant folding comment");
    assert!(asm.contains("LDA #$1E"), "Should load folded constant 30 (0x1E)");
    assert!(asm.contains("STA $0400"), "Should store to RESULT");

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
    let asm = generate(&ast, &program).unwrap();

    // Verify proper binary operation sequence with variables:
    // 1. Load left operand (from X = 0x0401)
    // 2. Push to stack (PHA)
    // 3. Load right operand (from Y = 0x0402)
    // 4. Store right in TEMP
    // 5. Restore left (PLA)
    // 6. Add with carry (ADC)
    // 7. Store result to SCREEN
    assert!(asm.contains("LDA $0401"), "Should load left operand from X");
    assert!(asm.contains("LDA $0402"), "Should load right operand from Y");
    assert!(asm.contains("PHA"), "Should push left to stack");
    assert!(asm.contains("PLA"), "Should restore left from stack");
    assert!(asm.contains("STA $20"), "Should store right to TEMP");
    assert!(asm.contains("CLC"), "Should clear carry");
    assert!(asm.contains("ADC $20"), "Should add from TEMP");

    // Verify ordering
    assert!(appears_before(&asm, "PHA", "PLA"), "PHA before PLA");
    assert!(appears_before(&asm, "CLC", "ADC"), "CLC before ADC");
    assert!(appears_before(&asm, "ADC", "STA $0400"), "ADC before final STA");
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
    let asm = generate(&ast, &program).unwrap();

    assert!(asm.contains("BEQ"));
    assert!(asm.contains("JMP"));
    // Check for generated labels
    assert!(asm.contains("else_"));
    assert!(asm.contains("while_start_"));
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
    let asm = generate(&ast, &program).unwrap();

    assert!(asm.contains("JSR add"));
    // Arguments pushed
    assert!(asm.contains("PHA"));
    // Stack cleanup
    assert!(asm.contains("PLA"));
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
    let asm = generate(&ast, &program).unwrap();

    // String should have:
    // 1. Jump over data
    // 2. Label
    // 3. Length word (5, 0 for "Hello")
    // 4. String bytes
    // 5. Skip label
    // 6. Load address
    assert!(asm.contains("JMP"), "Should jump over string data");
    assert!(asm.contains("str_"), "Should have string label");
    assert!(asm.contains(".byte $05, $00"), "Should have length prefix (5)");
    assert!(asm.contains("$48"), "Should contain 'H' (0x48)");
    assert!(asm.contains("LDA #<str_"), "Should load low byte of address");
    assert!(asm.contains("LDX #>str_"), "Should load high byte of address");

    // Verify ordering
    assert!(appears_before(&asm, "JMP", ".byte $05"), "Jump before data");
    assert!(appears_before(&asm, ".byte $05", "LDA #<str_"), "Data before address load");
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
    let asm = generate(&ast, &program).unwrap();

    // Comparison should:
    // 1. Load operands and compare
    // 2. Branch on condition
    // 3. Set result to 0 or 1
    assert!(asm.contains("CMP $20"), "Should compare with TEMP");
    assert!(asm.contains("BEQ eq_true_"), "Should branch if equal");
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
    let asm = generate(&ast, &program).unwrap();

    // Short-circuit AND:
    // 1. Evaluate left (from X)
    // 2. If false (0), skip right and jump to end
    // 3. Otherwise evaluate right
    assert!(asm.contains("LDA $0401"), "Should load left operand from X");
    assert!(asm.contains("CMP #$00"), "Should check if false");
    assert!(asm.contains("BEQ and_end_"), "Should short-circuit if false");

    // CMP should come before BEQ
    let first_cmp = asm.find("CMP #$00").unwrap();
    let first_beq = asm.find("BEQ and_end_").unwrap();
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
    let asm = generate(&ast, &program).unwrap();

    // Multiplication uses repeated addition:
    // 1. Save multiplicand to X register
    // 2. Initialize result to 0
    // 3. Loop: add multiplicand to result
    assert!(asm.contains("TAX"), "Should save multiplicand to X");
    assert!(asm.contains("TAY"), "Should save multiplier to Y");
    assert!(asm.contains("mul_loop_"), "Should have multiply loop");
    assert!(asm.contains("TXA"), "Should load multiplicand");
    assert!(asm.contains("DEY"), "Should decrement counter");
    assert!(asm.contains("BNE mul_loop_"), "Should loop until done");

    // Verify ordering
    assert!(appears_before(&asm, "TAX", "mul_loop_"), "Setup before loop");
    assert!(appears_before(&asm, "mul_loop_", "BNE mul_loop_"), "Loop label before branch");
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
    let asm = generate(&ast, &program).unwrap();

    // Division uses repeated subtraction
    assert!(asm.contains("CPX #$00"), "Should check for division by zero");
    assert!(asm.contains("BEQ div_end_"), "Should skip if divisor is zero");
    assert!(asm.contains("div_loop_"), "Should have division loop");
    assert!(asm.contains("SBC $20"), "Should subtract divisor");
    assert!(asm.contains("INC $22"), "Should increment quotient");
    assert!(asm.contains("BCC div_end_"), "Should exit when dividend < divisor");

    // Verify ordering
    assert!(appears_before(&asm, "CPX", "BEQ div_end_"), "Zero check before skip");
    assert!(appears_before(&asm, "div_loop_", "SBC"), "Loop before subtraction");
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
    let asm = generate(&ast, &program).unwrap();

    // Shift uses iterative ASL
    assert!(asm.contains("LDX $20"), "Should load shift count");
    assert!(asm.contains("shl_loop_"), "Should have shift loop");
    assert!(asm.contains("ASL A"), "Should arithmetic shift left");
    assert!(asm.contains("DEX"), "Should decrement counter");
    assert!(asm.contains("BNE shl_loop_"), "Should loop");

    // Verify ordering
    assert!(appears_before(&asm, "LDX", "shl_loop_"), "Load count before loop");
    assert!(appears_before(&asm, "ASL A", "DEX"), "Shift before decrement");
}

#[test]
fn test_codegen_for_loop() {
    let source = r#"
        fn main() {
            for u8 i in 0..10 {
                // body
            }
        }
    "#;

    let tokens = lex(source).unwrap();
    let ast = Parser::parse(&tokens).unwrap();
    let program = analyze(&ast).unwrap();
    let asm = generate(&ast, &program).unwrap();

    // For loop should:
    // 1. Initialize counter
    // 2. Store end value
    // 3. Loop: check condition, execute body, increment
    assert!(asm.contains("for_loop_"), "Should have loop label");
    assert!(asm.contains("for_end_"), "Should have end label");
    assert!(asm.contains("CMP $21"), "Should compare with end value");
    assert!(asm.contains("BCS for_end_"), "Should exit if counter >= end");
    assert!(asm.contains("INC"), "Should increment counter");
    assert!(asm.contains("JMP for_loop_"), "Should jump back to start");

    // Verify ordering
    assert!(appears_before(&asm, "STA $21", "for_loop_"), "Store end before loop");
    assert!(appears_before(&asm, "for_loop_", "CMP"), "Loop label before check");
    assert!(appears_before(&asm, "INC", "JMP for_loop_"), "Increment before jump");
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
    let asm = generate(&ast, &program).unwrap();

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
    let asm = generate(&ast, &program).unwrap();

    // Should have both addition and multiplication
    assert!(asm.contains("ADC $20"), "Should have addition");
    assert!(asm.contains("mul_loop_"), "Should have multiplication");

    // Addition should come before multiplication
    let add_pos = asm.find("ADC $20").unwrap();
    let mul_pos = asm.find("mul_loop_").unwrap();
    assert!(add_pos < mul_pos, "Addition (inner) should come before multiplication (outer)");
}
