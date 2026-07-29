//! Semantic Analysis module
//!
//! This module is responsible for:
//! - Symbol resolution (mapping names to declarations)
//! - Type checking
//! - Memory layout assignment (Zero Page vs Stack)
//! - Constant expression evaluation

pub mod analyze;
pub mod const_eval;
pub mod init;
pub mod table;
pub mod type_defs;
pub mod types;

use crate::ast::SourceFile;
use analyze::SemanticAnalyzer;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub enum SemaError {
    /// Symbol not found in scope
    UndefinedSymbol { name: String, span: Span },

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
    ImmutableAssignment { symbol: String, span: Span },

    /// Circular import detected
    CircularImport { path: String, chain: Vec<String> },

    /// Return type mismatch
    ReturnTypeMismatch {
        expected: String,
        found: String,
        span: Span,
    },

    /// Return outside of function
    ReturnOutsideFunction { span: Span },

    /// Break/continue outside of loop
    BreakOutsideLoop { span: Span },

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

    /// A pointer that would outlive the storage it names.
    EscapingPointer { reason: String, span: Span },

    /// Import error
    ImportError {
        path: String,
        reason: String,
        span: Span,
    },

    /// A failure inside an imported module, already rendered against that
    /// module's own source. Its spans index a file the driver never read, so it
    /// cannot be rendered later — it is formatted where it is caught and then
    /// carried verbatim, with the `import` that pulled the module in shown as
    /// the trail leading to it.
    InModule {
        path: String,
        rendered: String,
        /// Intermediate hops of the import chain, each already rendered by the
        /// level that held the relevant source, innermost first.
        trail: Vec<String>,
        import_span: Span,
    },

    /// Out of zero page memory
    OutOfZeroPage { span: Span },

    /// The colored per-function frames do not fit in the zero-page frame region.
    /// `chain` describes the deepest call chain and its cumulative frame usage.
    FrameRegionOverflow { chain: String, needed: usize },

    /// Identifier conflicts with 6502 instruction mnemonic
    InstructionConflict { name: String, span: Span },

    /// Generic error with custom message
    Custom { message: String, span: Span },

    /// Constant value overflow for declared type
    ConstantOverflow { value: i64, ty: String, span: Span },

    /// Cannot read from write-only address
    WriteOnlyRead { name: String, span: Span },

    /// Cannot write to read-only address
    ReadOnlyWrite { name: String, span: Span },

    /// addr type can only be used in const declarations
    InvalidAddrUsage { context: String, span: Span },

    /// Array index out of bounds (compile-time check)
    ArrayIndexOutOfBounds {
        index: i64,
        array_size: usize,
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
        self.format_with_source_of_file(source, filename, crate::ast::ROOT_FILE)
    }

    /// Render against the text of a specific file.
    ///
    /// Errors raised while analyzing an imported module carry that module's
    /// spans, so they must be rendered against that module's source — not the
    /// root's, which is all the driver has.
    pub fn format_with_source_of_file(
        &self,
        source: &str,
        filename: Option<&str>,
        file: u32,
    ) -> String {
        match self {
            SemaError::UndefinedSymbol { name, span } => {
                let msg = format!("undefined symbol '{}'", name);
                format!(
                    "error: {}\n{}",
                    msg,
                    span.format_error_context_of(source, filename, &msg, file)
                )
            }
            SemaError::TypeMismatch {
                expected,
                found,
                span,
            } => {
                let msg = format!("expected {}, found {}", expected, found);
                format!(
                    "error: type mismatch\n{}",
                    span.format_error_context_of(source, filename, &msg, file)
                )
            }
            SemaError::InvalidBinaryOp {
                op,
                left_ty,
                right_ty,
                span,
            } => {
                let mut msg = format!(
                    "cannot apply '{}' to types {} and {}",
                    op, left_ty, right_ty
                );
                // Wraith performs no implicit conversions, so a mixed-width
                // integer operation (e.g. u16 + u8) is rejected. Point the user
                // at the explicit cast, which is the intended way to do it.
                let is_int = |t: &str| matches!(t, "u8" | "i8" | "u16" | "i16");
                if left_ty != right_ty && is_int(left_ty) && is_int(right_ty) {
                    let wider = if matches!(left_ty.as_str(), "u16" | "i16") {
                        left_ty
                    } else {
                        right_ty
                    };
                    msg.push_str(&format!(
                        "\n  = help: Wraith has no implicit conversions; cast one operand, \
                         e.g. `(x as {})`",
                        wider
                    ));
                }
                format!(
                    "error: invalid binary operation\n{}",
                    span.format_error_context_of(source, filename, &msg, file)
                )
            }
            SemaError::InvalidUnaryOp {
                op,
                operand_ty,
                span,
            } => {
                let msg = format!("cannot apply '{}' to type {}", op, operand_ty);
                format!(
                    "error: invalid unary operation\n{}",
                    span.format_error_context_of(source, filename, &msg, file)
                )
            }
            SemaError::ArityMismatch {
                expected,
                found,
                span,
            } => {
                let msg = format!("expected {} argument(s), found {}", expected, found);
                format!(
                    "error: function call\n{}",
                    span.format_error_context_of(source, filename, &msg, file)
                )
            }
            SemaError::ImmutableAssignment { symbol, span } => {
                let msg = format!("cannot assign to immutable variable '{}'", symbol);
                format!(
                    "error: {}\n{}",
                    msg,
                    span.format_error_context_of(source, filename, &msg, file)
                )
            }
            SemaError::CircularImport { path, chain } => {
                format!(
                    "error: circular import detected: {} -> {}",
                    chain.join(" -> "),
                    path
                )
            }
            SemaError::ReturnTypeMismatch {
                expected,
                found,
                span,
            } => {
                let msg = format!("expected {}, found {}", expected, found);
                format!(
                    "error: return type mismatch\n{}",
                    span.format_error_context_of(source, filename, &msg, file)
                )
            }
            SemaError::ReturnOutsideFunction { span } => {
                let msg = "return statement outside function".to_string();
                format!(
                    "error: {}\n{}",
                    msg,
                    span.format_error_context_of(source, filename, &msg, file)
                )
            }
            SemaError::BreakOutsideLoop { span } => {
                let msg = "break/continue outside loop".to_string();
                format!(
                    "error: {}\n{}",
                    msg,
                    span.format_error_context_of(source, filename, &msg, file)
                )
            }
            SemaError::DuplicateSymbol {
                name,
                span,
                previous_span,
            } => {
                let msg = if let Some(prev) = previous_span {
                    format!(
                        "duplicate symbol '{}' (previously defined at {})",
                        name,
                        prev.format_position(source)
                    )
                } else {
                    format!("duplicate symbol '{}'", name)
                };
                format!(
                    "error: {}\n{}",
                    msg,
                    span.format_error_context_of(source, filename, &msg, file)
                )
            }
            SemaError::FieldNotFound {
                struct_name,
                field_name,
                span,
            } => {
                let msg = format!(
                    "field '{}' not found in struct '{}'",
                    field_name, struct_name
                );
                format!(
                    "error: {}\n{}",
                    msg,
                    span.format_error_context_of(source, filename, &msg, file)
                )
            }
            SemaError::EscapingPointer { reason, span } => {
                format!(
                    "error: pointer escapes its frame\n{}",
                    span.format_error_context_of(source, filename, reason, file)
                )
            }
            SemaError::ImportError { path, reason, span } => {
                let msg = format!("failed to import '{}': {}", path, reason);
                format!(
                    "error: import error\n{}",
                    span.format_error_context_of(source, filename, &msg, file)
                )
            }
            SemaError::InModule {
                path,
                rendered,
                trail,
                import_span,
            } => {
                // The child diagnostic already says what and where. All this
                // adds is the trail leading to it. Each hop was rendered by the
                // level that held the matching source, so the chain reads from
                // the failing module outward to the file being compiled.
                let mut out = rendered.clone();
                for hop in trail {
                    out.push_str("\n\n");
                    out.push_str(hop);
                }
                out.push_str(&format!(
                    "\n\nnote: in module '{}', imported here\n{}",
                    path,
                    import_span.format_error_context_of(source, filename, "", file)
                ));
                out
            }
            SemaError::OutOfZeroPage { span } => {
                let msg = "no more zero page addresses available".to_string();
                format!(
                    "error: out of zero page memory\n{}",
                    span.format_error_context_of(source, filename, &msg, file)
                )
            }
            SemaError::FrameRegionOverflow { chain, needed } => {
                format!(
                    "error: zero-page frame region overflow\n  \
                     the deepest call chain needs {} bytes but only the frame region \
                     ($40-$CF, 144 bytes) is available:\n  {}",
                    needed, chain
                )
            }
            SemaError::InstructionConflict { name, span } => {
                let msg = format!("identifier '{}' conflicts with instruction mnemonic", name);
                format!(
                    "error: {}\n{}",
                    msg,
                    span.format_error_context_of(source, filename, &msg, file)
                )
            }
            SemaError::Custom { message, span } => {
                format!(
                    "error: {}\n{}",
                    message,
                    span.format_error_context_of(source, filename, message, file)
                )
            }
            SemaError::ConstantOverflow { value, ty, span } => {
                let msg = format!("constant value {} does not fit in type {}", value, ty);
                format!(
                    "error: constant overflow\n{}",
                    span.format_error_context_of(source, filename, &msg, file)
                )
            }
            SemaError::WriteOnlyRead { name, span } => {
                let msg = format!("cannot read from write-only address '{}'", name);
                format!(
                    "error: {}\n{}",
                    msg,
                    span.format_error_context_of(source, filename, &msg, file)
                )
            }
            SemaError::ReadOnlyWrite { name, span } => {
                let msg = format!("cannot write to read-only address '{}'", name);
                format!(
                    "error: {}\n{}",
                    msg,
                    span.format_error_context_of(source, filename, &msg, file)
                )
            }
            SemaError::InvalidAddrUsage { context, span } => {
                let msg = format!(
                    "addr type can only be used in const declarations, not {}",
                    context
                );
                format!(
                    "error: invalid addr usage\n{}",
                    span.format_error_context_of(source, filename, &msg, file)
                )
            }
            SemaError::ArrayIndexOutOfBounds {
                index,
                array_size,
                span,
            } => {
                let msg = format!(
                    "array index {} is out of bounds for array of length {}",
                    index, array_size
                );
                format!(
                    "error: array index out of bounds\n{}",
                    span.format_error_context_of(source, filename, &msg, file)
                )
            }
        }
    }
}

impl std::fmt::Display for SemaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SemaError::UndefinedSymbol { name, span } => {
                write!(
                    f,
                    "undefined symbol '{}' at {}..{}",
                    name, span.start, span.end
                )
            }
            SemaError::TypeMismatch {
                expected,
                found,
                span,
            } => {
                write!(
                    f,
                    "type mismatch at {}..{}: expected {}, found {}",
                    span.start, span.end, expected, found
                )
            }
            SemaError::InvalidBinaryOp {
                op,
                left_ty,
                right_ty,
                span,
            } => {
                write!(
                    f,
                    "invalid binary operation '{}' at {}..{}: cannot apply to types {} and {}",
                    op, span.start, span.end, left_ty, right_ty
                )
            }
            SemaError::InvalidUnaryOp {
                op,
                operand_ty,
                span,
            } => {
                write!(
                    f,
                    "invalid unary operation '{}' at {}..{}: cannot apply to type {}",
                    op, span.start, span.end, operand_ty
                )
            }
            SemaError::ArityMismatch {
                expected,
                found,
                span,
            } => {
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
            SemaError::ReturnTypeMismatch {
                expected,
                found,
                span,
            } => {
                write!(
                    f,
                    "return type mismatch at {}..{}: expected {}, found {}",
                    span.start, span.end, expected, found
                )
            }
            SemaError::ReturnOutsideFunction { span } => {
                write!(
                    f,
                    "return statement outside function at {}..{}",
                    span.start, span.end
                )
            }
            SemaError::BreakOutsideLoop { span } => {
                write!(
                    f,
                    "break/continue outside loop at {}..{}",
                    span.start, span.end
                )
            }
            SemaError::DuplicateSymbol {
                name,
                span,
                previous_span,
            } => {
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
            SemaError::FieldNotFound {
                struct_name,
                field_name,
                span,
            } => {
                write!(
                    f,
                    "field '{}' not found in struct '{}' at {}..{}",
                    field_name, struct_name, span.start, span.end
                )
            }
            SemaError::EscapingPointer { reason, span } => {
                write!(
                    f,
                    "pointer escapes its frame at {}..{}: {}",
                    span.start, span.end, reason
                )
            }
            SemaError::InModule { path, rendered, .. } => {
                write!(f, "in module '{}':\n{}", path, rendered)
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
            SemaError::FrameRegionOverflow { chain, needed } => {
                write!(
                    f,
                    "zero-page frame region overflow: deepest call chain needs {} bytes (max 144): {}",
                    needed, chain
                )
            }
            SemaError::InstructionConflict { name, span } => {
                write!(
                    f,
                    "identifier '{}' at {}..{} conflicts with instruction mnemonic",
                    name, span.start, span.end
                )
            }
            SemaError::Custom { message, span } => {
                write!(f, "{} at {}..{}", message, span.start, span.end)
            }
            SemaError::ConstantOverflow { value, ty, span } => {
                write!(
                    f,
                    "constant overflow at {}..{}: value {} does not fit in type {}",
                    span.start, span.end, value, ty
                )
            }
            SemaError::WriteOnlyRead { name, span } => {
                write!(
                    f,
                    "cannot read from write-only address '{}' at {}..{}",
                    name, span.start, span.end
                )
            }
            SemaError::ReadOnlyWrite { name, span } => {
                write!(
                    f,
                    "cannot write to read-only address '{}' at {}..{}",
                    name, span.start, span.end
                )
            }
            SemaError::InvalidAddrUsage { context, span } => {
                write!(
                    f,
                    "invalid addr usage at {}..{}: addr type can only be used in const declarations, not {}",
                    span.start, span.end, context
                )
            }
            SemaError::ArrayIndexOutOfBounds {
                index,
                array_size,
                span,
            } => {
                write!(
                    f,
                    "array index out of bounds at {}..{}: index {} is out of bounds for array of length {}",
                    span.start, span.end, index, array_size
                )
            }
        }
    }
}

impl std::error::Error for SemaError {}

/// Compiler warnings (non-fatal diagnostics)
#[derive(Debug, Clone)]
pub enum Warning {
    /// Unused variable
    UnusedVariable { name: String, span: Span },

    /// Unused import
    UnusedImport { name: String, span: Span },

    /// Unreachable code after return/break/continue
    UnreachableCode { span: Span },

    /// Unused function parameter
    UnusedParameter { name: String, span: Span },

    /// Unused function
    UnusedFunction { name: String, span: Span },

    /// Unused `static` or `const` that nothing reaches
    UnusedStatic { name: String, span: Span },

    /// Non-exhaustive match (missing enum variants)
    NonExhaustiveMatch {
        missing_patterns: Vec<String>,
        span: Span,
    },

    /// Constant with non-uppercase name
    NonUppercaseConstant { name: String, span: Span },

    /// Function parameters exceed available zero-page space
    ParameterOverflow {
        function_name: String,
        bytes_used: u8,
        bytes_available: u8,
        span: Span,
    },

    /// Address declaration overlaps with compiler-managed memory section
    AddressOverlap {
        name: String,
        address: u16,
        section_name: String,
        section_start: u16,
        section_end: u16,
        span: Span,
    },

    /// A recursive function has a large frame, so each recursive call consumes a
    /// large slice of the fixed 256-byte software stack used to save/restore
    /// frames across recursion. Deep recursion will silently overflow it.
    DeepRecursionRisk {
        function: String,
        frame_bytes: u8,
        safe_depth: usize,
        span: Span,
    },
}

impl Warning {
    /// Format warning with source context (similar to error formatting)
    pub fn format_with_source_and_file(&self, source: &str, filename: Option<&str>) -> String {
        let (message, span) = match self {
            Warning::UnusedVariable { name, span } => {
                (format!("unused variable: `{}`", name), span)
            }
            Warning::UnusedImport { name, span } => (format!("unused import: `{}`", name), span),
            Warning::UnreachableCode { span } => ("unreachable code".to_string(), span),
            Warning::UnusedParameter { name, span } => {
                (format!("unused parameter: `{}`", name), span)
            }
            Warning::UnusedFunction { name, span } => {
                (format!("unused function: `{}`", name), span)
            }
            Warning::UnusedStatic { name, span } => {
                (format!("unused constant or static: `{}`", name), span)
            }
            Warning::NonExhaustiveMatch {
                missing_patterns,
                span,
            } => {
                let patterns = missing_patterns.join("`, `");
                (
                    format!("non-exhaustive match, missing: `{}`", patterns),
                    span,
                )
            }
            Warning::NonUppercaseConstant { name, span } => (
                format!("constant `{}` should have an uppercase name", name),
                span,
            ),
            Warning::ParameterOverflow {
                function_name,
                bytes_used,
                bytes_available,
                span,
            } => (
                format!(
                    "function `{}` parameters use {} bytes, exceeds available {} bytes",
                    function_name, bytes_used, bytes_available
                ),
                span,
            ),
            Warning::AddressOverlap {
                name,
                address,
                section_name,
                section_start,
                section_end,
                span,
            } => (
                format!(
                    "address declaration `{}` at ${:04X} overlaps with {} section (${:04X}-${:04X})",
                    name, address, section_name, section_start, section_end
                ),
                span,
            ),
            Warning::DeepRecursionRisk {
                function,
                frame_bytes,
                safe_depth,
                span,
            } => (
                format!(
                    "recursive function `{}` has a {}-byte frame; each recursive call saves it \
                     to the 256-byte software stack, so recursion deeper than ~{} levels will \
                     silently overflow it and corrupt data. Use tail recursion (which compiles \
                     to a loop) or an explicit loop for deep recursion.",
                    function, frame_bytes, safe_depth
                ),
                span,
            ),
        };

        format!(
            "warning: {}\n{}",
            message,
            span.format_error_context(source, filename, &message)
        )
    }
}

use crate::ast::{FnParam, Span, Spanned, Stmt};
use crate::sema::table::SymbolInfo;
use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};

#[derive(Debug, Clone)]
pub struct FunctionMetadata {
    pub org_address: Option<u16>,
    pub section: Option<String>,
    pub is_inline: bool,
    /// True if this function is an interrupt handler (#[irq]/#[nmi]/#[interrupt]).
    /// Handlers can preempt main code, so `finalize_frames` computes the zero-page
    /// state they must save; reset is NOT a handler.
    pub is_interrupt_handler: bool,
    /// For inline functions, store the body and parameters for expansion
    pub inline_body: Option<Spanned<Stmt>>,
    pub inline_params: Option<Vec<FnParam>>,
    /// For inline functions, store resolved symbols for parameters
    /// This allows inline expansion across module boundaries
    pub inline_param_symbols: Option<HashMap<Span, SymbolInfo>>,
    /// Whether this function has tail recursive calls
    pub has_tail_recursion: bool,
    /// Total bytes used by function parameters in zero page
    /// Used for optimized parameter save/restore in recursive calls
    pub param_bytes_used: u8,
}

/// Tail call information for a function
#[derive(Debug, Clone, Default)]
pub struct TailCallInfo {
    /// Set of return statement spans that contain tail recursive calls
    pub tail_recursive_returns: HashSet<Span>,
}

/// A function's zero-page frame, assigned by `finalize_frames`.
///
/// Frames are colored by the call graph so that functions which cannot be
/// simultaneously active share addresses. A function reads its parameters and
/// locals at `base + offset`; a caller writes a callee's arguments starting at
/// the callee's `base`.
#[derive(Debug, Clone, Copy)]
pub struct FrameInfo {
    /// Absolute zero-page base address of the frame.
    pub base: u8,
    /// Total frame size in bytes (parameters + locals + temp slots).
    pub size: u8,
    /// Size in bytes of the contiguous parameter block at the bottom of the frame.
    pub param_size: u8,
}

/// Zero-page state an interrupt handler must save/restore so it cannot corrupt
/// interrupted main-thread code. Computed by `finalize_frames` per handler.
#[derive(Debug, Clone, Default)]
pub struct InterruptSaveInfo {
    /// Save the shared codegen scratch ($20-$3F, $F0-$FE) around the handler body.
    pub save_scratch: bool,
    /// Save the mul16/div16 working storage ($D0-$DC) — set if the handler graph
    /// performs 16-bit multiply/divide/modulo.
    pub save_math: bool,
    /// (base, size) of frames belonging to functions shared between this handler
    /// and main-thread code, which must be preserved across the handler.
    pub shared_frames: Vec<(u8, u8)>,
}

/// One byte of a mutable global's startup image. Function pointers cannot be
/// resolved during analysis (code addresses are assigned later, during section
/// allocation), so they are recorded as label references and emitted as
/// `#<label` / `#>label` operands for the assembler to resolve.
#[derive(Debug, Clone)]
pub enum InitByte {
    Byte(u8),
    /// Low byte of a function's address.
    FnLow(String),
    /// High byte of a function's address.
    FnHigh(String),
}

/// A mutable global's RAM address and the startup image its declaration gives
/// it. Written into RAM by the reset handler.
#[derive(Debug, Clone)]
pub struct StaticInit {
    pub name: String,
    pub addr: u16,
    pub bytes: Vec<InitByte>,
}

/// A local array's data block.
#[derive(Debug, Clone)]
pub struct LocalArray {
    /// Offset within the declaring function's block during analysis; an
    /// absolute RAM address after `finalize_frames`.
    pub addr: u16,
    /// Size of the data in bytes.
    pub size: u16,
    /// The function that declared it, used to find its block's base.
    pub function: String,
}

pub struct ProgramInfo {
    // Placeholder for analyzed program data
    pub table: table::SymbolTable,
    pub resolved_symbols: HashMap<Span, SymbolInfo>,
    pub function_metadata: HashMap<String, FunctionMetadata>,
    /// Map of expression spans to their constant-folded values
    pub folded_constants: HashMap<Span, const_eval::ConstValue>,
    /// Hidden per-loop frame slots holding non-constant for-loop range ends,
    /// keyed by the range end expression's span. Frame-allocated (and colored
    /// with the call graph) so nested loops, scratch-using expressions, and
    /// calls in the loop body cannot clobber a live bound.
    pub loop_bound_slots: HashMap<Span, SymbolInfo>,
    /// Where each local array's data lives in RAM, keyed by the declaration's
    /// name span. Local array *data* used to be emitted inline in the CODE
    /// section with only a pointer in the frame slot, which meant writing to a
    /// local array on a ROM board did nothing at all.
    pub local_arrays: HashMap<Span, LocalArray>,
    /// Where each enum-typed local's data lives in RAM, keyed like
    /// `local_arrays`; see `SemanticAnalyzer::enum_blocks`.
    pub enum_blocks: HashMap<Span, LocalArray>,
    /// Registry of struct and enum type definitions
    pub type_registry: type_defs::TypeRegistry,
    /// Map of expression spans to their resolved types
    pub resolved_types: HashMap<Span, types::Type>,
    /// Items from imported modules that need to be emitted in codegen
    pub imported_items: Vec<Spanned<crate::ast::Item>>,
    /// Compiler warnings collected during analysis
    pub warnings: Vec<Warning>,
    /// Statements identified as unreachable for dead code elimination
    pub unreachable_stmts: HashSet<Span>,
    /// Tail call information for all functions
    pub tail_call_info: HashMap<String, TailCallInfo>,
    /// Map of anonymous struct init spans to their resolved struct names
    pub resolved_struct_names: HashMap<Span, String>,
    /// Spans of `.len`/`.low`/`.high` expressions that sema resolved as struct
    /// field accesses rather than the built-in accessors. The parser emits the
    /// built-in node before types are known; sema re-decides by the object's
    /// type and records the choice here for codegen.
    pub accessor_fields: HashSet<Span>,
    /// Global string pool for cross-module string deduplication
    /// Maps string content to a unique label (e.g., "Hello" -> "str_0")
    pub string_pool: HashMap<String, String>,
    /// Initial values for mutable `static` globals, in declaration order. The
    /// reset handler writes these into RAM at startup, since RAM cannot be
    /// pre-loaded from ROM on a bare machine.
    pub static_inits: Vec<StaticInit>,
    /// Per-function zero-page frame assignment (base + size), colored by the
    /// call graph. Populated by `finalize_frames`.
    pub function_frames: HashMap<String, FrameInfo>,
    /// Memory layout from wraith.toml, so codegen can place the software stack
    /// and other regions where the board actually has RAM.
    pub memory_config: crate::config::MemoryConfig,
    /// Function signatures (Type::Function) keyed by name, covering local and
    /// imported functions (including imported-module functions this module did
    /// not name). Codegen falls back to this to marshal call arguments when the
    /// symbol table has no entry for the callee.
    pub function_signatures: HashMap<String, types::Type>,
    /// Call-graph edges (caller, callee) that lie within a recursion cycle.
    /// A call across such an edge must save/restore the callee's frame.
    pub recursive_call_edges: HashSet<(String, String)>,
    /// Per-interrupt-handler zero-page save requirements.
    pub interrupt_save_info: HashMap<String, InterruptSaveInfo>,
    /// Functions whose address is taken (used as function pointers). They pass
    /// arguments through the fixed indirect-arg staging block and copy them into
    /// their frame in a prologue, so indirect callers can pass args.
    pub address_taken_functions: HashSet<String>,
    /// Every symbol the program can reach from the root module, as a closure over
    /// calls and references. Imported items outside this set are dead and are not
    /// emitted: importing a module exposes its whole file, and without this a
    /// program that used one helper from a library dragged in all of it.
    pub reachable_symbols: HashSet<String>,
}

/// 6502 and 65C02 instruction mnemonics
/// Using these as identifiers will cause assembly conflicts
const INSTRUCTION_MNEMONICS: &[&str] = &[
    // Standard 6502 instructions
    "ADC", "AND", "ASL", "BCC", "BCS", "BEQ", "BIT", "BMI", "BNE", "BPL", "BRK", "BVC", "BVS",
    "CLC", "CLD", "CLI", "CLV", "CMP", "CPX", "CPY", "DEC", "DEX", "DEY", "EOR", "INC", "INX",
    "INY", "JMP", "JSR", "LDA", "LDX", "LDY", "LSR", "NOP", "ORA", "PHA", "PHP", "PLA", "PLP",
    "ROL", "ROR", "RTI", "RTS", "SBC", "SEC", "SED", "SEI", "STA", "STX", "STY", "TAX", "TAY",
    "TSX", "TXA", "TXS", "TYA", // 65C02 extensions
    "BRA", "PHX", "PHY", "PLX", "PLY", "STZ", "TRB", "TSB", "WAI", "STP",
    // 65C02 bit manipulation (BBR0-7, BBS0-7, RMB0-7, SMB0-7)
    "BBR0", "BBR1", "BBR2", "BBR3", "BBR4", "BBR5", "BBR6", "BBR7", "BBS0", "BBS1", "BBS2", "BBS3",
    "BBS4", "BBS5", "BBS6", "BBS7", "RMB0", "RMB1", "RMB2", "RMB3", "RMB4", "RMB5", "RMB6", "RMB7",
    "SMB0", "SMB1", "SMB2", "SMB3", "SMB4", "SMB5", "SMB6", "SMB7",
];

/// Check if an identifier conflicts with a instruction mnemonic
pub fn is_instruction_conflict(name: &str) -> bool {
    let uppercase = name.to_uppercase();
    INSTRUCTION_MNEMONICS.contains(&uppercase.as_str())
}

pub fn analyze(ast: &SourceFile) -> Result<ProgramInfo, SemaError> {
    let mut analyzer = SemanticAnalyzer::new();
    analyzer.analyze(ast)
}

pub fn analyze_with_path(ast: &SourceFile, file_path: PathBuf) -> Result<ProgramInfo, SemaError> {
    let mut analyzer = SemanticAnalyzer::with_base_path(file_path);
    analyzer.analyze(ast)
}
