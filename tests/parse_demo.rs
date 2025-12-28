//! Test parsing example files

use std::fs;
use wraith::{Parser, lex};

#[test]
fn test_parse_demo_file() {
    let source = fs::read_to_string("examples/via.wr").expect("failed to read example file");

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
