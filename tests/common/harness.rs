//! Test harness for compiling Wraith programs
//!
//! Provides functions to compile programs at different stages
//! and handle errors appropriately.

use wraith::ast::SourceFile;
use wraith::codegen::generate;
use wraith::lex;
use wraith::parser::Parser;
use wraith::sema::{analyze, ProgramInfo};

/// Result of compiling a Wraith program
#[derive(Debug)]
pub enum CompileResult {
    Success { asm: String },
    LexError(String),
    ParseError(String),
    SemaError(String),
    CodegenError(String),
}

/// Compile a Wraith source string through all phases
pub fn compile(source: &str) -> CompileResult {
    // Lex
    let tokens = match lex(source) {
        Ok(tokens) => tokens,
        Err(e) => return CompileResult::LexError(format!("{:?}", e)),
    };

    // Parse
    let ast = match Parser::parse(&tokens) {
        Ok(ast) => ast,
        Err(e) => return CompileResult::ParseError(e.format_with_source_and_file(source, None)),
    };

    // Semantic analysis
    let program = match analyze(&ast) {
        Ok(program) => program,
        Err(e) => return CompileResult::SemaError(e.format_with_source_and_file(source, None)),
    };

    // Code generation
    match generate(&ast, &program) {
        Ok(asm) => CompileResult::Success { asm },
        Err(e) => CompileResult::CodegenError(format!("{:?}", e)),
    }
}

/// Compile source and return assembly, panicking on any error
pub fn compile_success(source: &str) -> String {
    match compile(source) {
        CompileResult::Success { asm } => asm,
        CompileResult::LexError(e) => panic!("Lex error: {}", e),
        CompileResult::ParseError(e) => panic!("Parse error: {}", e),
        CompileResult::SemaError(e) => panic!("Semantic error: {}", e),
        CompileResult::CodegenError(e) => panic!("Codegen error: {}", e),
    }
}

/// Compile source to AST only
pub fn compile_to_ast(source: &str) -> Result<SourceFile, String> {
    let tokens = lex(source).map_err(|e| format!("{:?}", e))?;
    Parser::parse(&tokens).map_err(|e| e.format_with_source_and_file(source, None))
}

/// Compile source to semantic analysis
pub fn compile_to_sema(source: &str) -> Result<(SourceFile, ProgramInfo), String> {
    let ast = compile_to_ast(source)?;
    let program = analyze(&ast).map_err(|e| e.format_with_source_and_file(source, None))?;
    Ok((ast, program))
}

/// Lex source only
pub fn lex_only(source: &str) -> Result<Vec<wraith::lexer::SpannedToken>, String> {
    lex(source).map_err(|e| format!("{:?}", e))
}

/// Parse source only (lex + parse)
pub fn parse_only(source: &str) -> Result<SourceFile, String> {
    compile_to_ast(source)
}

/// Analyze source only (lex + parse + sema)
pub fn analyze_only(source: &str) -> Result<ProgramInfo, String> {
    let (_, program) = compile_to_sema(source)?;
    Ok(program)
}

// Legacy aliases for backward compatibility
pub use compile_success as assert_compiles;

/// Assert that source fails to compile with specific error phase
pub fn assert_fails_at(source: &str, expected_phase: &str) {
    match compile(source) {
        CompileResult::Success { .. } => {
            panic!("Expected compilation to fail at {} but it succeeded", expected_phase)
        }
        CompileResult::LexError(_) if expected_phase == "lex" => {}
        CompileResult::ParseError(_) if expected_phase == "parse" => {}
        CompileResult::SemaError(_) if expected_phase == "sema" => {}
        CompileResult::CodegenError(_) if expected_phase == "codegen" => {}
        other => panic!(
            "Expected failure at {} but got: {:?}",
            expected_phase, other
        ),
    }
}

/// Assert that source fails with error containing specific text
pub fn assert_error_contains(source: &str, needle: &str) {
    let error_msg = match compile(source) {
        CompileResult::Success { .. } => {
            panic!("Expected compilation to fail but it succeeded")
        }
        CompileResult::LexError(e) => e,
        CompileResult::ParseError(e) => e,
        CompileResult::SemaError(e) => e,
        CompileResult::CodegenError(e) => e,
    };

    assert!(
        error_msg.contains(needle),
        "Expected error to contain '{}' but got:\n{}",
        needle,
        error_msg
    );
}

/// Assert that lex phase fails
pub fn assert_lex_error(source: &str) {
    assert_fails_at(source, "lex");
}

/// Assert that parse phase fails
pub fn assert_parse_error(source: &str) {
    assert_fails_at(source, "parse");
}

/// Assert that semantic analysis fails
pub fn assert_sema_error(source: &str) {
    assert_fails_at(source, "sema");
}

/// Assert that code generation fails
pub fn assert_codegen_error(source: &str) {
    assert_fails_at(source, "codegen");
}

/// Extract instructions from assembly (ignoring comments and labels)
pub fn extract_instructions(asm: &str) -> Vec<String> {
    asm.lines()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty() && !line.starts_with(';') && !line.ends_with(':'))
        .map(|line| line.to_string())
        .collect()
}

/// Count occurrences of a pattern in assembly
pub fn count_pattern(asm: &str, pattern: &str) -> usize {
    asm.matches(pattern).count()
}
