//! Wraith - A programming language for the 6502 processor
//!
//! This crate provides a compiler for the Wraith programming language,
//! which compiles to 6502 assembly and binary formats.

pub mod ast;

// Re-export commonly used types
pub use ast::{SourceFile, Span, Spanned};
