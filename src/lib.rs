//! Wraith - A programming language for the 6502 processor
//!
//! This crate provides a compiler for the Wraith programming language,
//! which compiles to 6502 assembly and binary formats.

#![warn(clippy::all)]

pub mod ast;
pub mod codegen;
pub mod config;
pub mod lexer;
pub mod parser;
pub mod sema;

// Re-export commonly used types
pub use ast::{SourceFile, Span, Spanned};
pub use lexer::{Token, lex};
pub use parser::{ParseError, Parser};
