use wraith::lex;
use wraith::parser::Parser;
use wraith::sema::analyze;
use wraith::sema::table::SymbolKind;

#[test]
fn test_analyze_simple_program() {
    let source = r#"
        const SCREEN: addr = 0x0400;

        fn main() {
            let x: u8 = 42;
        }
    "#;

    let tokens = lex(source).unwrap();
    let ast = Parser::parse(&tokens).unwrap();
    let program = analyze(&ast).unwrap();

    // Check global symbols
    let screen = program.table.lookup("SCREEN").expect("SCREEN not found");
    assert_eq!(screen.kind, SymbolKind::Address);  // Address declarations are marked as Address kind
    assert!(screen.mutable);

    let main = program.table.lookup("main").expect("main not found");
    assert_eq!(main.kind, SymbolKind::Function);
}

#[test]
fn test_analyze_function_params() {
    let source = r#"
        fn add(a: u8, b: u8) -> u8 {
            return a + b;
        }
    "#;

    let tokens = lex(source).unwrap();
    let ast = Parser::parse(&tokens).unwrap();
    let program = analyze(&ast).unwrap();

    let add = program.table.lookup("add").expect("add not found");
    assert_eq!(add.kind, SymbolKind::Function);

    // Note: We can't easily check local symbols (params) from the global table
    // because they are in a local scope that is exited after analysis.
    // To test locals, we'd need to inspect the table *during* analysis or
    // have the analyzer return more detailed info.
    // For now, we just check that analysis succeeds.
}
