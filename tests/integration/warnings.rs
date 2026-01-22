//! Compiler warning tests
//!
//! Tests the warning system for non-fatal diagnostics

use crate::common::*;

// ============================================================================
// Unused Variable Warnings
// ============================================================================

#[test]
fn warn_unused_local_variable() {
    let result = compile(
        r#"
        fn main() {
            let x: u8 = 10;
        }
    "#,
    );

    match result {
        CompileResult::Success(warnings, _) => {
            assert!(!warnings.is_empty(), "Expected warning for unused variable");
            assert!(
                warnings.contains("unused variable"),
                "Should warn about unused variable 'x'"
            );
        }
        _ => panic!("Expected successful compilation with warnings"),
    }
}

#[test]
fn no_warn_when_variable_used() {
    let result = compile(
        r#"
        const OUT: addr = 0x6000;
        fn main() {
            let x: u8 = 10;
            OUT = x;
        }
    "#,
    );

    match result {
        CompileResult::Success(warnings, _) => {
            assert!(
                !warnings.contains("unused variable"),
                "Should not warn when variable is used"
            );
        }
        _ => panic!("Expected successful compilation"),
    }
}

#[test]
fn warn_multiple_unused_variables() {
    let result = compile(
        r#"
        fn main() {
            let x: u8 = 10;
            let y: u8 = 20;
            let z: u8 = 30;
        }
    "#,
    );

    match result {
        CompileResult::Success(warnings, _) => {
            assert!(warnings.contains("x"), "Should warn about unused 'x'");
            assert!(warnings.contains("y"), "Should warn about unused 'y'");
            assert!(warnings.contains("z"), "Should warn about unused 'z'");
        }
        _ => panic!("Expected successful compilation with warnings"),
    }
}

// ============================================================================
// Unused Parameter Warnings
// ============================================================================

#[test]
fn warn_unused_function_parameter() {
    let result = compile(
        r#"
        fn foo(x: u8) {
        }
        fn main() {
            foo(10);
        }
    "#,
    );

    match result {
        CompileResult::Success(warnings, _) => {
            assert!(
                warnings.contains("unused parameter"),
                "Should warn about unused parameter"
            );
            assert!(warnings.contains("x"), "Should mention parameter name 'x'");
        }
        _ => panic!("Expected successful compilation with warnings"),
    }
}

#[test]
fn no_warn_underscore_prefix_parameter() {
    let result = compile(
        r#"
        fn foo(_unused: u8) {
        }
        fn main() {
            foo(10);
        }
    "#,
    );

    match result {
        CompileResult::Success(warnings, _) => {
            assert!(
                !warnings.contains("_unused"),
                "Should not warn for _unused parameter"
            );
        }
        _ => panic!("Expected successful compilation"),
    }
}

#[test]
fn no_warn_when_parameter_used() {
    let result = compile(
        r#"
        fn add(x: u8) -> u8 {
            return x + 1;
        }
        fn main() {
            let y: u8 = add(10);
        }
    "#,
    );

    match result {
        CompileResult::Success(warnings, _) => {
            assert!(
                !warnings.contains("unused parameter"),
                "Should not warn when parameter is used"
            );
        }
        _ => panic!("Expected successful compilation"),
    }
}

// ============================================================================
// Unreachable Code Warnings
// ============================================================================

#[test]
fn warn_unreachable_after_return() {
    let result = compile(
        r#"
        fn foo() -> u8 {
            return 42;
            let x: u8 = 10;
        }
        fn main() {}
    "#,
    );

    match result {
        CompileResult::Success(warnings, _) => {
            assert!(
                warnings.contains("unreachable"),
                "Should warn about unreachable code"
            );
        }
        _ => panic!("Expected successful compilation with warnings"),
    }
}

#[test]
fn warn_unreachable_after_break() {
    let result = compile(
        r#"
        fn main() {
            for i: u8 in 0..10 {
                break;
                let x: u8 = 10;
            }
        }
    "#,
    );

    match result {
        CompileResult::Success(warnings, _) => {
            assert!(
                warnings.contains("unreachable"),
                "Should warn about unreachable code after break"
            );
        }
        _ => panic!("Expected successful compilation with warnings"),
    }
}

#[test]
fn warn_unreachable_after_continue() {
    let result = compile(
        r#"
        fn main() {
            for i: u8 in 0..10 {
                continue;
                let x: u8 = 10;
            }
        }
    "#,
    );

    match result {
        CompileResult::Success(warnings, _) => {
            assert!(
                warnings.contains("unreachable"),
                "Should warn about unreachable code after continue"
            );
        }
        _ => panic!("Expected successful compilation with warnings"),
    }
}

// ============================================================================
// Non-Exhaustive Match Warnings
// ============================================================================

#[test]
fn warn_non_exhaustive_match() {
    let result = compile(
        r#"
        enum Color {
            Red,
            Green,
            Blue,
        }
        fn main() {
            let c: Color = Color::Red;
            match c {
                Color::Red => { let x: u8 = 1; }
            }
        }
    "#,
    );

    match result {
        CompileResult::Success(warnings, _) => {
            assert!(
                warnings.contains("non-exhaustive"),
                "Should warn about non-exhaustive match"
            );
            assert!(
                warnings.contains("Green") || warnings.contains("Blue"),
                "Should mention missing variants"
            );
        }
        _ => panic!("Expected successful compilation with warnings"),
    }
}

#[test]
fn no_warn_exhaustive_match() {
    let result = compile(
        r#"
        enum Color {
            Red,
            Green,
        }
        fn main() {
            let c: Color = Color::Red;
            match c {
                Color::Red => { let x: u8 = 1; }
                Color::Green => { let x: u8 = 2; }
            }
        }
    "#,
    );

    match result {
        CompileResult::Success(warnings, _) => {
            assert!(
                !warnings.contains("non-exhaustive"),
                "Should not warn when match is exhaustive"
            );
        }
        _ => panic!("Expected successful compilation"),
    }
}

#[test]
fn no_warn_match_with_wildcard() {
    let result = compile(
        r#"
        enum Color {
            Red,
            Green,
            Blue,
        }
        fn main() {
            let c: Color = Color::Red;
            match c {
                Color::Red => { let x: u8 = 1; }
                _ => { let x: u8 = 0; }
            }
        }
    "#,
    );

    match result {
        CompileResult::Success(warnings, _) => {
            assert!(
                !warnings.contains("non-exhaustive"),
                "Wildcard should cover all cases"
            );
        }
        _ => panic!("Expected successful compilation"),
    }
}

// ============================================================================
// Unused Import Warnings (when import system is implemented)
// ============================================================================

#[test]
fn warn_unused_import() {
    let result = compile_with_base_path(
        r#"
        import {LED, BUTTON} from "addresses.wr";
        fn main() {
            // Only use LED, not BUTTON
            let x: u8 = LED;
        }
    "#,
        "tests/integration/addresses.wr",
    );

    match result {
        CompileResult::Success(warnings, _) => {
            assert!(
                warnings.contains("unused import") || warnings.contains("BUTTON"),
                "Should warn about unused import BUTTON. Got warnings: {}",
                warnings
            );
        }
        CompileResult::LexError(e) => panic!("Lex error: {}", e),
        CompileResult::ParseError(e) => panic!("Parse error: {}", e),
        CompileResult::SemaError(e) => panic!("Semantic error: {}", e),
        CompileResult::CodegenError(e) => panic!("Codegen error: {}", e),
    }
}

// ============================================================================
// Unused Function Warnings
// ============================================================================

#[test]
fn test_warn_unused_function() {
    let result = compile(
        r#"
        fn unused_helper() {
            // This function is never called
        }

        #[reset]
        fn main() {
            // main does not call unused_helper
        }
    "#,
    );

    match result {
        CompileResult::Success(warnings, _) => {
            assert!(
                warnings.contains("unused function"),
                "Should warn about unused function"
            );
            assert!(
                warnings.contains("unused_helper"),
                "Should mention function name 'unused_helper'"
            );
        }
        _ => panic!("Expected successful compilation with warnings"),
    }
}

#[test]
fn test_no_warn_when_function_called() {
    let result = compile(
        r#"
        fn helper() -> u8 {
            return 42;
        }

        #[reset]
        fn main() {
            let x: u8 = helper();
        }
    "#,
    );

    match result {
        CompileResult::Success(warnings, _) => {
            assert!(
                !warnings.contains("unused function"),
                "Should not warn when function is called"
            );
        }
        _ => panic!("Expected successful compilation"),
    }
}

#[test]
fn test_no_warn_main_function() {
    let result = compile(
        r#"
        #[reset]
        fn main() {
            // Entry point - should never warn even if not explicitly called
        }
    "#,
    );

    match result {
        CompileResult::Success(warnings, _) => {
            assert!(
                !warnings.contains("unused function"),
                "Should not warn about main function"
            );
        }
        _ => panic!("Expected successful compilation"),
    }
}

#[test]
fn test_no_warn_interrupt_handlers() {
    let result = compile(
        r#"
        #[irq]
        fn irq_handler() {
            // IRQ handler - called by hardware
        }

        #[nmi]
        fn nmi_handler() {
            // NMI handler - called by hardware
        }

        #[reset]
        fn main() {
            // Don't call handlers from code
        }
    "#,
    );

    match result {
        CompileResult::Success(warnings, _) => {
            assert!(
                !warnings.contains("irq_handler"),
                "Should not warn about IRQ handler"
            );
            assert!(
                !warnings.contains("nmi_handler"),
                "Should not warn about NMI handler"
            );
        }
        _ => panic!("Expected successful compilation"),
    }
}

#[test]
fn test_no_warn_inline_functions() {
    let result = compile(
        r#"
        #[inline]
        fn inline_helper(x: u8) -> u8 {
            return x + 1;
        }

        #[reset]
        fn main() {
            // Inline functions may be used from other modules
            // Don't warn even if not called in this file
        }
    "#,
    );

    match result {
        CompileResult::Success(warnings, _) => {
            assert!(
                !warnings.contains("inline_helper"),
                "Should not warn about inline function"
            );
        }
        _ => panic!("Expected successful compilation"),
    }
}
// ============================================================================
// Address Overlap Warnings
// ============================================================================

#[test]
fn warn_address_overlaps_code_section() {
    // Default CODE section is 0x8000-0xBFFF
    let result = compile(
        r#"
        const OVERLAP: addr = 0x9000;
        fn main() {}
    "#,
    );

    match result {
        CompileResult::Success(warnings, _) => {
            assert!(
                warnings.contains("overlap"),
                "Should warn about address overlap"
            );
            assert!(
                warnings.contains("0x9000") || warnings.contains("$9000"),
                "Should mention the address"
            );
            assert!(warnings.contains("CODE"), "Should mention CODE section");
        }
        _ => panic!("Expected successful compilation with warnings"),
    }
}

#[test]
fn warn_address_overlaps_data_section() {
    // Default DATA section is 0xD000-0xEFFF
    let result = compile(
        r#"
        const OVERLAP: addr = 0xD500;
        fn main() {}
    "#,
    );

    match result {
        CompileResult::Success(warnings, _) => {
            assert!(
                warnings.contains("overlap"),
                "Should warn about address overlap"
            );
            assert!(
                warnings.contains("0xD500") || warnings.contains("$D500"),
                "Should mention the address"
            );
            assert!(warnings.contains("DATA"), "Should mention DATA section");
        }
        _ => panic!("Expected successful compilation with warnings"),
    }
}

#[test]
fn warn_address_at_section_start() {
    // Test address exactly at CODE section start (0x8000)
    let result = compile(
        r#"
        const AT_START: addr = 0x8000;
        fn main() {}
    "#,
    );

    match result {
        CompileResult::Success(warnings, _) => {
            assert!(
                warnings.contains("overlap"),
                "Should warn about overlap at section start"
            );
            assert!(warnings.contains("CODE"), "Should mention CODE section");
        }
        _ => panic!("Expected successful compilation with warnings"),
    }
}

#[test]
fn warn_address_at_section_end() {
    // Test address exactly at CODE section end (0xBFFF)
    let result = compile(
        r#"
        const AT_END: addr = 0xBFFF;
        fn main() {}
    "#,
    );

    match result {
        CompileResult::Success(warnings, _) => {
            assert!(
                warnings.contains("overlap"),
                "Should warn about overlap at section end"
            );
            assert!(warnings.contains("CODE"), "Should mention CODE section");
        }
        _ => panic!("Expected successful compilation with warnings"),
    }
}

#[test]
fn no_warn_address_outside_sections() {
    // Address in zero page (0x0000-0x00FF) - outside CODE/DATA sections
    let result = compile(
        r#"
        const ZERO_PAGE: addr = 0x0040;
        fn main() {}
    "#,
    );

    match result {
        CompileResult::Success(warnings, _) => {
            assert!(
                !warnings.contains("overlap"),
                "Should not warn for zero page address"
            );
        }
        _ => panic!("Expected successful compilation"),
    }
}

#[test]
fn no_warn_address_between_sections() {
    // Address between CODE (ends at 0xBFFF) and DATA (starts at 0xD000)
    let result = compile(
        r#"
        const BETWEEN: addr = 0xC000;
        fn main() {}
    "#,
    );

    match result {
        CompileResult::Success(warnings, _) => {
            assert!(
                !warnings.contains("overlap"),
                "Should not warn for address between sections"
            );
        }
        _ => panic!("Expected successful compilation"),
    }
}

#[test]
fn no_warn_address_after_sections() {
    // Address above DATA section (ends at 0xEFFF)
    let result = compile(
        r#"
        const HIGH_MEM: addr = 0xF000;
        fn main() {}
    "#,
    );

    match result {
        CompileResult::Success(warnings, _) => {
            assert!(
                !warnings.contains("overlap"),
                "Should not warn for address above sections"
            );
        }
        _ => panic!("Expected successful compilation"),
    }
}

#[test]
fn warn_multiple_overlapping_addresses() {
    let result = compile(
        r#"
        const OVERLAP1: addr = 0x8000;
        const OVERLAP2: addr = 0x9000;
        const OVERLAP3: addr = 0xD000;
        fn main() {}
    "#,
    );

    match result {
        CompileResult::Success(warnings, _) => {
            // Should have warnings for all three overlapping addresses
            let overlap_count = warnings.matches("overlap").count();
            assert!(
                overlap_count >= 3,
                "Should warn about all overlapping addresses, got {} warnings",
                overlap_count
            );
        }
        _ => panic!("Expected successful compilation with warnings"),
    }
}
