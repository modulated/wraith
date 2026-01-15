//! Unary operation code generation
//!
//! This module handles all unary operators:
//! - Negation (`-x`): Two's complement negation
//! - Bitwise NOT (`~x`): Bitwise complement
//! - Logical NOT (`!x`): Boolean negation (converts to 0 or 1)

use crate::ast::{Spanned, UnaryOp};
use crate::codegen::{CodegenError, Emitter, StringCollector};
use crate::sema::ProgramInfo;

// Import generate_expr from parent module for recursive calls
use super::generate_expr;

/// Generate code for unary operations
///
/// Handles all unary operators:
/// - `-x`: Negation (two's complement: `~x + 1`)
/// - `~x`: Bitwise NOT (XOR with $FF)
/// - `!x`: Logical NOT (converts to boolean 0/1 and inverts)
pub(super) fn generate_unary(
    op: UnaryOp,
    operand: &Spanned<crate::ast::Expr>,
    emitter: &mut Emitter,
    info: &ProgramInfo,
    string_collector: &mut StringCollector,
) -> Result<(), CodegenError> {
    // Evaluate operand first
    generate_expr(operand, emitter, info, string_collector)?;

    // Apply unary operation to A
    match op {
        UnaryOp::Neg => {
            // Two's complement: ~A + 1
            emitter.emit_inst("EOR", "#$FF"); // Bitwise NOT
            emitter.emit_inst("CLC", "");
            emitter.emit_inst("ADC", "#$01"); // Add 1
        }
        UnaryOp::BitNot => {
            // Bitwise NOT
            emitter.emit_inst("EOR", "#$FF");
        }
        UnaryOp::Not => {
            // Logical NOT: convert to boolean (0 or 1) and invert
            let true_label = emitter.next_label("nt");
            let end_label = emitter.next_label("nx");

            emitter.emit_inst("CMP", "#$00");
            emitter.emit_inst("BEQ", &true_label); // If zero, result is true (1)

            // False case (input was non-zero)
            emitter.emit_inst("LDA", "#$00");
            emitter.emit_inst("JMP", &end_label);

            // True case (input was zero)
            emitter.emit_label(&true_label);
            emitter.emit_inst("LDA", "#$01");

            emitter.emit_label(&end_label);
        }
    }

    Ok(())
}
