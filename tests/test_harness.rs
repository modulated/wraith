//! Test harness utilities for Wraith compiler testing
//!
//! This module re-exports the common test infrastructure for backward compatibility.
//! New tests should use `mod common` directly.

#[path = "common/mod.rs"]
mod common;

// Re-export everything for backward compatibility
pub use common::*;

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
        assert_asm_contains(&asm, "X = $0400");
        assert_asm_contains(&asm, "LDA #$2A");
        assert_asm_contains(&asm, "STA X");
    }

    #[test]
    fn test_harness_asm_order() {
        let asm = assert_compiles("addr X = 0x400; fn main() { X = 42; }");
        assert_asm_order(&asm, "LDA #$2A", "STA X");
    }
}
