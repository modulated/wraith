use std::fs;
use std::path::PathBuf;

use wraith::{Parser, codegen, lex};

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() {
    let args = std::env::args().collect::<Vec<String>>();

    // Handle flags
    if args.len() == 2 {
        match args[1].as_str() {
            "--version" | "-v" => {
                println!("Wraith Compiler {} - modulated", VERSION);
                return;
            }
            "--help" | "-h" => {
                print_usage(&args[0]);
                return;
            }
            _ => {}
        }
    }

    if args.len() != 2 {
        print_usage(&args[0]);
        std::process::exit(1);
    }

    let file = &args[1];

    // Read source file
    let source = match fs::read_to_string(file) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {}: {}", file, e);
            std::process::exit(1);
        }
    };

    // Lex
    let tokens = match lex(&source) {
        Ok(tokens) => tokens,
        Err(e) => {
            eprintln!("error: lexical analysis failed");
            eprintln!("{:?}", e);
            std::process::exit(1);
        }
    };

    // Parse
    let ast = match Parser::parse(&tokens) {
        Ok(ast) => ast,
        Err(e) => {
            eprintln!("{}", e.format_with_source_and_file(&source, Some(file)));
            std::process::exit(1);
        }
    };

    // Semantic analysis
    let file_path = PathBuf::from(file);
    let program_info = match wraith::sema::analyze_with_path(&ast, file_path) {
        Ok(info) => info,
        Err(e) => {
            eprintln!("{}", e.format_with_source_and_file(&source, Some(file)));
            std::process::exit(1);
        }
    };

    // Code generation
    let code = match codegen::generate(&ast, &program_info) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: code generation failed");
            eprintln!("{:?}", e);
            std::process::exit(1);
        }
    };

    // Write output
    let out_file = file.replace(".wr", ".asm");
    if let Err(e) = fs::write(&out_file, &code) {
        eprintln!("error: could not write to {}: {}", out_file, e);
        std::process::exit(1);
    }

    // Success - no output unless there were warnings
}

fn print_usage(program: &str) {
    eprintln!("Usage: {} <input.wr>", program);
    eprintln!();
    eprintln!("Options:");
    eprintln!("  -h, --help       Print this help message");
    eprintln!("  -v, --version    Print version information");
}
