use std::fs;

use wraith::{Parser, codegen, lex};

fn main() {
    let args = std::env::args().collect::<Vec<String>>();

    if args.len() != 2 {
        eprintln!("Usage: {} <file>", args[0]);
        std::process::exit(1);
    }
    let file = &args[1];
    let source = fs::read_to_string(file).expect("Failed to read file");

    let tokens = lex(&source).expect("Lexer Error");
    let ast = Parser::parse(&tokens).expect("Parser Error");
    let program_info = wraith::sema::analyze(&ast).expect("Semantic Analysis Error");
    let code = codegen::generate(&ast, &program_info).expect("Codegen Error");
    let out_file = file.replace(".wr", ".asm");
    fs::write(&out_file, code).expect("Failed to write file");
}
