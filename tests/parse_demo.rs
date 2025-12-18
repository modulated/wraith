//! Test parsing demo.wr file

use std::fs;
use wraith::{Parser, lex};

#[test]
fn test_parse_demo_file() {
    let source = fs::read_to_string("tests/demo.wr").expect("failed to read demo.wr");

    let tokens = lex(&source).expect("lexer error");
    println!("Tokens: {:?}", tokens);

    let result = Parser::parse(&tokens);
    match result {
        Ok(ast) => {
            println!("Parsed {} items", ast.items.len());
            for item in &ast.items {
                println!("  {:?}", item);
            }
        }
        Err(e) => {
            panic!("Parse error: {}", e);
        }
    }
}
