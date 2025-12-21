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
    /// Format error with source code context (line:col instead of byte offsets)
    pub fn format_with_source(&self, source: &str) -> String {
        match self {
            SemaError::UndefinedSymbol { name, span } => {
                format!("undefined symbol '{}' at {}", name, span.format_position(source))
            }
            SemaError::TypeMismatch { expected, found, span } => {
                format!(
                    "type mismatch at {}: expected {}, found {}",
                    span.format_position(source),
                    expected,
                    found
                )
            }
            SemaError::InvalidBinaryOp { op, left_ty, right_ty, span } => {
                format!(
                    "invalid binary operation '{}' at {}: cannot apply to types {} and {}",
                    op,
                    span.format_position(source),
                    left_ty,
                    right_ty
                )
            }
            SemaError::InvalidUnaryOp { op, operand_ty, span } => {
                format!(
                    "invalid unary operation '{}' at {}: cannot apply to type {}",
                    op,
                    span.format_position(source),
                    operand_ty
                )
            }
            SemaError::ArityMismatch { expected, found, span } => {
                format!(
                    "function call at {}: expected {} argument(s), found {}",
                    span.format_position(source),
                    expected,
                    found
                )
            }
            SemaError::ImmutableAssignment { symbol, span } => {
                format!(
                    "cannot assign to immutable variable '{}' at {}",
                    symbol,
                    span.format_position(source)
                )
            }
            SemaError::CircularImport { path, chain } => {
                format!(
                    "circular import detected: {} -> {}",
                    chain.join(" -> "),
                    path
                )
            }
            SemaError::ReturnTypeMismatch { expected, found, span } => {
                format!(
                    "return type mismatch at {}: expected {}, found {}",
                    span.format_position(source),
                    expected,
                    found
                )
            }
            SemaError::ReturnOutsideFunction { span } => {
                format!("return statement outside function at {}", span.format_position(source))
            }
            SemaError::BreakOutsideLoop { span } => {
                format!("break/continue outside loop at {}", span.format_position(source))
            }
            SemaError::DuplicateSymbol { name, span, previous_span } => {
                if let Some(prev) = previous_span {
                    format!(
                        "duplicate symbol '{}' at {} (previously defined at {})",
                        name,
                        span.format_position(source),
                        prev.format_position(source)
                    )
                } else {
                    format!(
                        "duplicate symbol '{}' at {}",
                        name,
                        span.format_position(source)
                    )
                }
            }
            SemaError::FieldNotFound { struct_name, field_name, span } => {
                format!(
                    "field '{}' not found in struct '{}' at {}",
                    field_name,
                    struct_name,
                    span.format_position(source)
                )
            }
            SemaError::ImportError { path, reason, span } => {
                format!(
                    "import error at {}: failed to import '{}': {}",
                    span.format_position(source),
                    path,
                    reason
                )
            }
            SemaError::OutOfZeroPage { span } => {
                format!(
                    "out of zero page memory at {}: no more zero page addresses available",
                    span.format_position(source)
                )
            }
            SemaError::Custom { message, span } => {
                format!("{} at {}", message, span.format_position(source))
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
