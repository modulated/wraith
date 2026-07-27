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

    // Determine operand width so 16-bit values negate/complement both bytes.
    let is_u16 = matches!(
        info.resolved_types.get(&operand.span),
        Some(crate::sema::types::Type::Primitive(
            crate::ast::PrimitiveType::U16
                | crate::ast::PrimitiveType::I16
                | crate::ast::PrimitiveType::B16
        ))
    );

    // Apply unary operation to A (low) / Y (high for u16)
    match op {
        UnaryOp::AddrOf | UnaryOp::Deref => {
            return Err(CodegenError::UnsupportedOperation(
                "pointer operations are not supported yet".to_string(),
            ));
        }
        UnaryOp::Neg => {
            if is_u16 {
                // 16-bit two's complement: ~value + 1 across both bytes, carrying
                // from low into high. Low in A, high in Y; $22 holds the low result.
                let tmp = emitter.memory_layout.loop_end_temp(); // $22
                emitter.emit_inst("EOR", "#$FF"); // ~low
                emitter.emit_inst("CLC", "");
                emitter.emit_inst("ADC", "#$01"); // ~low + 1 (carry out)
                emitter.emit_inst("STA", &format!("${:02X}", tmp));
                emitter.emit_inst("TYA", ""); // high
                emitter.emit_inst("EOR", "#$FF"); // ~high
                emitter.emit_inst("ADC", "#$00"); // + carry from low
                emitter.emit_inst("TAY", ""); // Y = high result
                emitter.emit_inst("LDA", &format!("${:02X}", tmp)); // A = low result
            } else {
                // 8-bit two's complement: ~A + 1
                emitter.emit_inst("EOR", "#$FF"); // Bitwise NOT
                emitter.emit_inst("CLC", "");
                emitter.emit_inst("ADC", "#$01"); // Add 1
            }
        }
        UnaryOp::BitNot => {
            if is_u16 {
                // Complement both bytes; stash the low result while doing the high.
                let tmp = emitter.memory_layout.loop_end_temp(); // $22
                emitter.emit_inst("EOR", "#$FF"); // ~low
                emitter.emit_inst("STA", &format!("${:02X}", tmp));
                emitter.emit_inst("TYA", ""); // high
                emitter.emit_inst("EOR", "#$FF"); // ~high
                emitter.emit_inst("TAY", ""); // Y = ~high
                emitter.emit_inst("LDA", &format!("${:02X}", tmp)); // A = ~low
            } else {
                // Bitwise NOT
                emitter.emit_inst("EOR", "#$FF");
            }
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

    // Unary ops rewrite A (and Y for u16) via raw instructions the tracker does
    // not follow; drop cached register beliefs.
    emitter.mark_a_unknown();
    Ok(())
}
