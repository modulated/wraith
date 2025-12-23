//! Test harness utilities for Wraith compiler testing
//!
//! Provides helper functions and macros for testing:
//! - Compilation success/failure
//! - Assembly output verification
//! - Error message checking
//! - Snapshot testing

use wraith::codegen::generate;
use wraith::lex;
use wraith::parser::Parser;
use wraith::sema::analyze;

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

/// Assert that source compiles successfully
pub fn assert_compiles(source: &str) -> String {
    match compile(source) {
        CompileResult::Success { asm } => asm,
        CompileResult::LexError(e) => panic!("Lex error: {}", e),
        CompileResult::ParseError(e) => panic!("Parse error: {}", e),
        CompileResult::SemaError(e) => panic!("Semantic error: {}", e),
        CompileResult::CodegenError(e) => panic!("Codegen error: {}", e),
    }
}

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

/// Assert that assembly contains specific instruction
pub fn assert_asm_contains(asm: &str, pattern: &str) {
    assert!(
        asm.contains(pattern),
        "Expected assembly to contain '{}' but it didn't.\nAssembly:\n{}",
        pattern,
        asm
    );
}

/// Assert that assembly does NOT contain specific instruction
#[allow(dead_code)]
pub fn assert_asm_not_contains(asm: &str, pattern: &str) {
    assert!(
        !asm.contains(pattern),
        "Expected assembly to NOT contain '{}' but it did.\nAssembly:\n{}",
        pattern,
        asm
    );
}

/// Assert that first pattern appears before second in assembly
pub fn assert_asm_order(asm: &str, first: &str, second: &str) {
    let first_pos = asm
        .find(first)
        .unwrap_or_else(|| panic!("Could not find '{}' in assembly", first));
    let second_pos = asm
        .find(second)
        .unwrap_or_else(|| panic!("Could not find '{}' in assembly", second));

    assert!(
        first_pos < second_pos,
        "Expected '{}' to appear before '{}' but it didn't.\nAssembly:\n{}",
        first,
        second,
        asm
    );
}

/// Extract instructions from assembly (ignoring comments and labels)
#[allow(dead_code)]
pub fn extract_instructions(asm: &str) -> Vec<String> {
    asm.lines()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty() && !line.starts_with(';') && !line.ends_with(':'))
        .map(|line| line.to_string())
        .collect()
}

/// Count occurrences of a pattern in assembly
#[allow(dead_code)]
pub fn count_pattern(asm: &str, pattern: &str) -> usize {
    asm.matches(pattern).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_harness_compile_success() {
        let asm = assert_compiles("fn main() {}");
        assert!(asm.contains("main:"));
        assert!(asm.contains("RTS"));
    }

    #[test]
    fn test_harness_parse_error() {
        assert_fails_at("fn main() {", "parse");
    }

    #[test]
    fn test_harness_error_contains() {
        assert_error_contains("fn main() {", "expected");
    }

    #[test]
    fn test_harness_asm_contains() {
        let asm = assert_compiles("addr X = 0x400; fn main() { X = 42; }");
        assert_asm_contains(&asm, "X = $0400");  // Address label
        assert_asm_contains(&asm, "LDA #$2A");
        assert_asm_contains(&asm, "STA X");  // Symbolic name
    }

    #[test]
    fn test_harness_asm_order() {
        let asm = assert_compiles("addr X = 0x400; fn main() { X = 42; }");
        assert_asm_order(&asm, "LDA #$2A", "STA X");  // Symbolic name
    }
}
