use std::fs;
use std::path::PathBuf;

use wraith::{Parser, codegen, lex};

fn main() {
    let args = std::env::args().collect::<Vec<String>>();

    if args.len() != 2 {
        eprintln!("Usage: {} <file>", args[0]);
        std::process::exit(1);
    }
    let file = &args[1];
    let source = fs::read_to_string(file).unwrap_or_else(|e| {
        eprintln!("Error reading file '{}': {}", file, e);
        std::process::exit(1);
    });

    let tokens = match lex(&source) {
        Ok(tokens) => tokens,
        Err(e) => {
            eprintln!("Lexer error: {:?}", e);
            std::process::exit(1);
        }
    };

    let ast = match Parser::parse(&tokens) {
        Ok(ast) => ast,
        Err(e) => {
            eprintln!("Parse error: {}", e.format_with_source(&source));
            std::process::exit(1);
        }
    };

    let file_path = PathBuf::from(file);
    let program_info = match wraith::sema::analyze_with_path(&ast, file_path) {
        Ok(info) => info,
        Err(e) => {
            eprintln!("Semantic error: {}", e.format_with_source(&source));
            std::process::exit(1);
        }
    };

    let code = match codegen::generate(&ast, &program_info) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("Codegen error: {:?}", e);
            std::process::exit(1);
        }
    };

    let out_file = file.replace(".wr", ".asm");
    fs::write(&out_file, &code).unwrap_or_else(|e| {
        eprintln!("Error writing output file '{}': {}", out_file, e);
        std::process::exit(1);
    });

    println!("Successfully compiled '{}' to '{}'", file, out_file);
}
