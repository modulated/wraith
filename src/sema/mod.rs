//! Semantic Analysis module
//!
//! This module is responsible for:
//! - Symbol resolution (mapping names to declarations)
//! - Type checking
//! - Memory layout assignment (Zero Page vs Stack)
//! - Constant expression evaluation

pub mod analyze;
pub mod const_eval;
pub mod table;
pub mod type_defs;
pub mod types;

use crate::ast::SourceFile;
use analyze::SemanticAnalyzer;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub enum SemaError {
    /// Symbol not found in scope
    UndefinedSymbol {
        name: String,
        span: Span,
    },

    /// Type mismatch in expression or assignment
    TypeMismatch {
        expected: String,
        found: String,
        span: Span,
    },

    /// Invalid operation for given types
    InvalidBinaryOp {
        op: String,
        left_ty: String,
        right_ty: String,
        span: Span,
    },

    /// Invalid unary operation for type
    InvalidUnaryOp {
        op: String,
        operand_ty: String,
        span: Span,
    },

    /// Function call with wrong number of arguments
    ArityMismatch {
        expected: usize,
        found: usize,
        span: Span,
    },

    /// Attempting to assign to immutable variable
    ImmutableAssignment {
        symbol: String,
        span: Span,
    },

    /// Circular import detected
    CircularImport {
        path: String,
        chain: Vec<String>,
    },

    /// Return type mismatch
    ReturnTypeMismatch {
        expected: String,
        found: String,
        span: Span,
    },

    /// Return outside of function
    ReturnOutsideFunction {
        span: Span,
    },

    /// Break/continue outside of loop
    BreakOutsideLoop {
        span: Span,
    },

    /// Duplicate symbol definition
    DuplicateSymbol {
        name: String,
        span: Span,
        previous_span: Option<Span>,
    },

    /// Field not found in struct
    FieldNotFound {
        struct_name: String,
        field_name: String,
        span: Span,
    },

    /// Import error
    ImportError {
        path: String,
        reason: String,
        span: Span,
    },

    /// Out of zero page memory
    OutOfZeroPage {
        span: Span,
    },

    /// Generic error with custom message
    Custom {
        message: String,
        span: Span,
    },
}

impl SemaError {
    /// Format error with source code context showing the actual line and error marker
    pub fn format_with_source(&self, source: &str) -> String {
        self.format_with_source_and_file(source, None)
    }

    /// Format error with source code context and filename
    pub fn format_with_source_and_file(&self, source: &str, filename: Option<&str>) -> String {
        match self {
            SemaError::UndefinedSymbol { name, span } => {
                let msg = format!("undefined symbol '{}'", name);
                format!("error: {}\n{}", msg, span.format_error_context(source, filename, &msg))
            }
            SemaError::TypeMismatch { expected, found, span } => {
                let msg = format!("expected {}, found {}", expected, found);
                format!("error: type mismatch\n{}", span.format_error_context(source, filename, &msg))
            }
            SemaError::InvalidBinaryOp { op, left_ty, right_ty, span } => {
                let msg = format!("cannot apply '{}' to types {} and {}", op, left_ty, right_ty);
                format!("error: invalid binary operation\n{}", span.format_error_context(source, filename, &msg))
            }
            SemaError::InvalidUnaryOp { op, operand_ty, span } => {
                let msg = format!("cannot apply '{}' to type {}", op, operand_ty);
                format!("error: invalid unary operation\n{}", span.format_error_context(source, filename, &msg))
            }
            SemaError::ArityMismatch { expected, found, span } => {
                let msg = format!("expected {} argument(s), found {}", expected, found);
                format!("error: function call\n{}", span.format_error_context(source, filename, &msg))
            }
            SemaError::ImmutableAssignment { symbol, span } => {
                let msg = format!("cannot assign to immutable variable '{}'", symbol);
                format!("error: {}\n{}", msg, span.format_error_context(source, filename, &msg))
            }
            SemaError::CircularImport { path, chain } => {
                format!(
                    "error: circular import detected: {} -> {}",
                    chain.join(" -> "),
                    path
                )
            }
            SemaError::ReturnTypeMismatch { expected, found, span } => {
                let msg = format!("expected {}, found {}", expected, found);
                format!("error: return type mismatch\n{}", span.format_error_context(source, filename, &msg))
            }
            SemaError::ReturnOutsideFunction { span } => {
                let msg = "return statement outside function".to_string();
                format!("error: {}\n{}", msg, span.format_error_context(source, filename, &msg))
            }
            SemaError::BreakOutsideLoop { span } => {
                let msg = "break/continue outside loop".to_string();
                format!("error: {}\n{}", msg, span.format_error_context(source, filename, &msg))
            }
            SemaError::DuplicateSymbol { name, span, previous_span } => {
                let msg = if let Some(prev) = previous_span {
                    format!(
                        "duplicate symbol '{}' (previously defined at {})",
                        name,
                        prev.format_position(source)
                    )
                } else {
                    format!("duplicate symbol '{}'", name)
                };
                format!("error: {}\n{}", msg, span.format_error_context(source, filename, &msg))
            }
            SemaError::FieldNotFound { struct_name, field_name, span } => {
                let msg = format!("field '{}' not found in struct '{}'", field_name, struct_name);
                format!("error: {}\n{}", msg, span.format_error_context(source, filename, &msg))
            }
            SemaError::ImportError { path, reason, span } => {
                let msg = format!("failed to import '{}': {}", path, reason);
                format!("error: import error\n{}", span.format_error_context(source, filename, &msg))
            }
            SemaError::OutOfZeroPage { span } => {
                let msg = "no more zero page addresses available".to_string();
                format!("error: out of zero page memory\n{}", span.format_error_context(source, filename, &msg))
            }
            SemaError::Custom { message, span } => {
                format!("error: {}\n{}", message, span.format_error_context(source, filename, message))
            }
        }
    }
}

impl std::fmt::Display for SemaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SemaError::UndefinedSymbol { name, span } => {
                write!(f, "undefined symbol '{}' at {}..{}", name, span.start, span.end)
            }
            SemaError::TypeMismatch { expected, found, span } => {
                write!(
                    f,
                    "type mismatch at {}..{}: expected {}, found {}",
                    span.start, span.end, expected, found
                )
            }
            SemaError::InvalidBinaryOp { op, left_ty, right_ty, span } => {
                write!(
                    f,
                    "invalid binary operation '{}' at {}..{}: cannot apply to types {} and {}",
                    op, span.start, span.end, left_ty, right_ty
                )
            }
            SemaError::InvalidUnaryOp { op, operand_ty, span } => {
                write!(
                    f,
                    "invalid unary operation '{}' at {}..{}: cannot apply to type {}",
                    op, span.start, span.end, operand_ty
                )
            }
            SemaError::ArityMismatch { expected, found, span } => {
                write!(
                    f,
                    "function call at {}..{}: expected {} argument(s), found {}",
                    span.start, span.end, expected, found
                )
            }
            SemaError::ImmutableAssignment { symbol, span } => {
                write!(
                    f,
                    "cannot assign to immutable variable '{}' at {}..{}",
                    symbol, span.start, span.end
                )
            }
            SemaError::CircularImport { path, chain } => {
                write!(
                    f,
                    "circular import detected: {} -> {}",
                    chain.join(" -> "),
                    path
                )
            }
            SemaError::ReturnTypeMismatch { expected, found, span } => {
                write!(
                    f,
                    "return type mismatch at {}..{}: expected {}, found {}",
                    span.start, span.end, expected, found
                )
            }
            SemaError::ReturnOutsideFunction { span } => {
                write!(f, "return statement outside function at {}..{}", span.start, span.end)
            }
            SemaError::BreakOutsideLoop { span } => {
                write!(f, "break/continue outside loop at {}..{}", span.start, span.end)
            }
            SemaError::DuplicateSymbol { name, span, previous_span } => {
                if let Some(prev) = previous_span {
                    write!(
                        f,
                        "duplicate symbol '{}' at {}..{} (previously defined at {}..{})",
                        name, span.start, span.end, prev.start, prev.end
                    )
                } else {
                    write!(
                        f,
                        "duplicate symbol '{}' at {}..{}",
                        name, span.start, span.end
                    )
                }
            }
            SemaError::FieldNotFound { struct_name, field_name, span } => {
                write!(
                    f,
                    "field '{}' not found in struct '{}' at {}..{}",
                    field_name, struct_name, span.start, span.end
                )
            }
            SemaError::ImportError { path, reason, span } => {
                write!(
                    f,
                    "import error at {}..{}: failed to import '{}': {}",
                    span.start, span.end, path, reason
                )
            }
            SemaError::OutOfZeroPage { span } => {
                write!(
                    f,
                    "out of zero page memory at {}..{}: no more zero page addresses available",
                    span.start, span.end
                )
            }
            SemaError::Custom { message, span } => {
                write!(f, "{} at {}..{}", message, span.start, span.end)
            }
        }
    }
}

impl std::error::Error for SemaError {}

use crate::ast::Span;
use crate::sema::table::SymbolInfo;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct FunctionMetadata {
    pub org_address: Option<u16>,
}

pub struct ProgramInfo {
    // Placeholder for analyzed program data
    pub table: table::SymbolTable,
    pub resolved_symbols: HashMap<Span, SymbolInfo>,
    pub function_metadata: HashMap<String, FunctionMetadata>,
    /// Map of expression spans to their constant-folded values
    pub folded_constants: HashMap<Span, const_eval::ConstValue>,
    /// Registry of struct and enum type definitions
    pub type_registry: type_defs::TypeRegistry,
}

pub fn analyze(ast: &SourceFile) -> Result<ProgramInfo, SemaError> {
    let mut analyzer = SemanticAnalyzer::new();
    analyzer.analyze(ast)
}

pub fn analyze_with_path(ast: &SourceFile, file_path: PathBuf) -> Result<ProgramInfo, SemaError> {
    let mut analyzer = SemanticAnalyzer::with_base_path(file_path);
    analyzer.analyze(ast)
}
