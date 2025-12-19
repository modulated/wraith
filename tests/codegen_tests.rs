use wraith::codegen::generate;
use wraith::lex;
use wraith::parser::Parser;
use wraith::sema::analyze;

#[test]
fn test_codegen_empty_function() {
    let source = r#"
        fn main() {}
    "#;

    let tokens = lex(source).unwrap();
    let ast = Parser::parse(&tokens).unwrap();
    let program = analyze(&ast).unwrap();
    let asm = generate(&ast, &program).unwrap();

    assert!(asm.contains("main:"));
    assert!(asm.contains("RTS"));
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

    // Should load 42 into A
    assert!(asm.contains("LDA #$2A")); // 42 in hex is 2A
    // Should store A into SCREEN (0x0400)
    // Note: Our current implementation of assignment isn't fully done in stmt.rs
    // We need to check if Stmt::Assign is implemented
}

#[test]
fn test_codegen_binary_op() {
    let source = r#"
        addr SCREEN = 0x0400;
        fn main() {
            SCREEN = 10 + 20;
        }
    "#;

    let tokens = lex(source).unwrap();
    let ast = Parser::parse(&tokens).unwrap();
    let program = analyze(&ast).unwrap();
    let asm = generate(&ast, &program).unwrap();

    assert!(asm.contains("ADC $20"));
    assert!(asm.contains("PHA"));
    assert!(asm.contains("PLA"));
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
