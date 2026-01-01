//! Function call code generation
//!
//! This module handles:
//! - Normal function calls (JSR-based with zero-page parameter passing)
//! - Inline function expansion (body substitution)
//! - Parameter storage and register invalidation
//! - Return value handling

use crate::ast::{Expr, Spanned};
use crate::codegen::{CodegenError, Emitter, StringCollector};
use crate::sema::types::Type;
use crate::sema::ProgramInfo;

// Import generate_expr from parent module for recursive calls
use super::generate_expr;

/// Generate code for function calls
///
/// Dispatches to either:
/// - `generate_inline_call` for `#[inline]` functions
/// - Regular JSR-based call for normal functions
///
/// Regular calling convention:
/// - Arguments passed in zero-page starting at `param_base`
/// - 16-bit arguments take 2 consecutive bytes
/// - Return value in A (8-bit) or A+Y (16-bit)
/// - All registers invalidated after call
pub(super) fn generate_call(
    function: &Spanned<String>,
    args: &[Spanned<Expr>],
    emitter: &mut Emitter,
    info: &ProgramInfo,
    string_collector: &mut StringCollector,
) -> Result<(), CodegenError> {
    // Check if function should be inlined
    if let Some(metadata) = info.function_metadata.get(&function.node)
        && metadata.is_inline
    {
        // Inline the function call
        return generate_inline_call(function, args, emitter, info, metadata, string_collector);
    }

    // 6502 calling convention: Arguments are passed in zero page locations
    // Parameters are allocated starting at param_base (from memory layout)
    // This avoids using the hardware stack which is limited and slow to access

    let param_base = emitter.memory_layout.param_base;

    // Emit descriptive call comment
    if args.is_empty() {
        emitter.emit_comment(&format!("Call: {}()", function.node));
    } else {
        emitter.emit_comment(&format!(
            "Call: {}(...) [{} arg{}]",
            function.node,
            args.len(),
            if args.len() == 1 { "" } else { "s" }
        ));
    }

    // Document parameter storage in verbose mode
    if emitter.is_verbose() && !args.is_empty() {
        emitter.emit_comment(&format!(
            "Parameters: [${:02X}-${:02X}] = {} arg{}",
            param_base,
            param_base + args.len() as u8 - 1,
            args.len(),
            if args.len() == 1 { "" } else { "s" }
        ));
    }

    // Store each argument to its corresponding parameter location
    // Track byte offset (16-bit params take 2 bytes)
    let mut byte_offset = 0u8;
    for arg in args.iter() {
        let param_addr = param_base + byte_offset;

        // Check if this argument is a 16-bit type
        let arg_type = info.resolved_types.get(&arg.span);
        let is_16bit = arg_type.is_some_and(|ty| {
            matches!(
                ty,
                crate::sema::types::Type::Primitive(crate::ast::PrimitiveType::U16)
                    | crate::sema::types::Type::Primitive(crate::ast::PrimitiveType::I16)
                    | crate::sema::types::Type::Primitive(crate::ast::PrimitiveType::B16)
            )
        });

        // Generate argument expression (result in A for 8-bit, A+Y for 16-bit)
        generate_expr(arg, emitter, info, string_collector)?;

        // Store to parameter location
        emitter.emit_inst("STA", &format!("${:02X}", param_addr));
        if is_16bit {
            // For 16-bit types, also store high byte (in Y) to next location
            emitter.emit_inst("STY", &format!("${:02X}", param_addr + 1));
            byte_offset += 2; // 16-bit param takes 2 bytes
        } else {
            byte_offset += 1; // 8-bit param takes 1 byte
        }
    }

    // Call the function
    emitter.emit_inst("JSR", &function.node);

    // Invalidate register state after function call
    // (called function may modify any register; only A/Y contain known return value)
    emitter.reg_state.invalidate_all();

    // Result is returned in A register (no cleanup needed)
    if !emitter.is_minimal() {
        // Look up the function's return type to generate accurate comment
        if let Some(sym) = info.table.lookup(&function.node) {
            if let Type::Function(_, ret_type) = &sym.ty {
                match ret_type.as_ref() {
                    Type::Void => {
                        // No return value
                    }
                    Type::Primitive(crate::ast::PrimitiveType::U16)
                    | Type::Primitive(crate::ast::PrimitiveType::I16)
                    | Type::Primitive(crate::ast::PrimitiveType::B16) => {
                        emitter.emit_comment(&format!(
                            "Returns: A=result_low, Y=result_high ({})",
                            ret_type.display_name()
                        ));
                    }
                    ty => {
                        emitter.emit_comment(&format!("Returns: A=result ({})", ty.display_name()));
                    }
                }
            } else {
                // Fallback for non-function types
                emitter.emit_comment("Returns: A=result");
            }
        } else {
            // Fallback if function not in symbol table
            emitter.emit_comment("Returns: A=result");
        }
    }

    Ok(())
}

/// Generate inline function call expansion
///
/// Expands the function body inline, substituting arguments for parameters.
/// - Arguments evaluated and stored in parameter zero-page locations
/// - Function body generated inline (no JSR)
/// - Return statements jump to end instead of RTS
/// - Parameter symbols merged into current context
fn generate_inline_call(
    function: &Spanned<String>,
    args: &[Spanned<Expr>],
    emitter: &mut Emitter,
    info: &ProgramInfo,
    metadata: &crate::sema::FunctionMetadata,
    string_collector: &mut StringCollector,
) -> Result<(), CodegenError> {
    // Emit inline expansion comment
    if args.is_empty() {
        emitter.emit_comment(&format!("Inline: {}()", function.node));
    } else {
        emitter.emit_comment(&format!(
            "Inline: {}(...) [{} arg{}]",
            function.node,
            args.len(),
            if args.len() == 1 { "" } else { "s" }
        ));
    }

    // Get inline function body and parameters
    let body = metadata.inline_body.as_ref().ok_or_else(|| {
        CodegenError::UnsupportedOperation(format!("Inline function {} missing body", function.node))
    })?;

    let params = metadata.inline_params.as_ref().ok_or_else(|| {
        CodegenError::UnsupportedOperation(format!(
            "Inline function {} missing parameters",
            function.node
        ))
    })?;

    // Verify argument count matches parameter count
    if args.len() != params.len() {
        return Err(CodegenError::UnsupportedOperation(format!(
            "Inline function {} expects {} args, got {}",
            function.node,
            params.len(),
            args.len()
        )));
    }

    // Store arguments to the parameter locations that were allocated during semantic analysis
    // Each parameter has a specific zero-page address that was assigned when the function was defined
    // We need to store the argument values at those exact addresses
    for (i, arg) in args.iter().enumerate() {
        generate_expr(arg, emitter, info, string_collector)?;

        // Get the parameter info for this position
        let param = &params[i];

        // Look up the parameter's allocated location from inline_param_symbols
        if let Some(ref param_symbols) = metadata.inline_param_symbols {
            if let Some(param_info) = param_symbols.get(&param.name.span) {
                match param_info.location {
                    crate::sema::table::SymbolLocation::ZeroPage(addr) => {
                        emitter.emit_inst("STA", &format!("${:02X}", addr));
                    }
                    _ => {
                        return Err(CodegenError::UnsupportedOperation(format!(
                            "Inline function parameter '{}' must be in zero page",
                            param.name.node
                        )));
                    }
                }
            } else {
                return Err(CodegenError::UnsupportedOperation(format!(
                    "Parameter symbol '{}' not found for inline function",
                    param.name.node
                )));
            }
        } else {
            return Err(CodegenError::UnsupportedOperation(format!(
                "No parameter symbols for inline function {}",
                function.node
            )));
        }
    }

    // Generate the function body inline
    // Push inline context so return statements won't emit RTS
    emitter.push_inline();

    // For inline functions from imported modules, we need to merge the parameter symbols
    // from the original module into the current ProgramInfo so the function body can
    // reference its parameters correctly
    let result = if let Some(ref param_symbols) = metadata.inline_param_symbols {
        // Create a modified ProgramInfo with merged resolved_symbols
        let mut merged_resolved = info.resolved_symbols.clone();
        for (span, symbol) in param_symbols {
            merged_resolved.insert(*span, symbol.clone());
        }

        let modified_info = crate::sema::ProgramInfo {
            table: info.table.clone(),
            resolved_symbols: merged_resolved,
            function_metadata: info.function_metadata.clone(),
            folded_constants: info.folded_constants.clone(),
            type_registry: info.type_registry.clone(),
            resolved_types: info.resolved_types.clone(),
            imported_items: info.imported_items.clone(),
            warnings: info.warnings.clone(),
        };

        use crate::codegen::stmt::generate_stmt;
        generate_stmt(body, emitter, &modified_info, string_collector)
    } else {
        // No parameter symbols stored - this indicates a bug in semantic analysis
        // Inline functions should always have parameter symbols populated
        return Err(CodegenError::UnsupportedOperation(format!(
            "Inline function {} has no parameter symbols (compiler bug)",
            function.node
        )));
    };

    // Pop inline context
    emitter.pop_inline();

    result
}
