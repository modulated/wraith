//! Semantic Analysis module
//!
//! This module is responsible for:
//! - Symbol resolution (mapping names to declarations)
//! - Type checking
//! - Memory layout assignment (Zero Page vs Stack)

pub mod analyze;
pub mod table;
pub mod types;

use crate::ast::SourceFile;
use analyze::SemanticAnalyzer;

#[derive(Debug, Clone)]
pub enum SemaError {
    ImmutableAssign,
    TypeMismatch,
    ArgMismatch,
    SymbolNotFound, // TODO: Add specific errors
}

use crate::ast::Span;
use crate::sema::table::SymbolInfo;
use std::collections::HashMap;

pub struct ProgramInfo {
    // Placeholder for analyzed program data
    pub table: table::SymbolTable,
    pub resolved_symbols: HashMap<Span, SymbolInfo>,
}

pub fn analyze(ast: &SourceFile) -> Result<ProgramInfo, SemaError> {
    let mut analyzer = SemanticAnalyzer::new();
    analyzer.analyze(ast)
}
