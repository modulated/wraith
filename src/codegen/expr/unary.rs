//! Unary operation code generation
//!
//! This module handles all unary operators:
//! - Negation (`-x`): Two's complement negation
//! - Bitwise NOT (`~x`): Bitwise complement
//! - Logical NOT (`!x`): Boolean negation (converts to 0 or 1)
//! - Dereference (`*ptr`): Load value from address
//! - Address-of (`&x`, `&mut x`): Get variable address

use crate::ast::{Expr, Spanned, UnaryOp};
use crate::codegen::{CodegenError, Emitter, StringCollector};
use crate::sema::ProgramInfo;

// Import generate_expr from parent module for recursive calls
use super::generate_expr;

/// Generate code for unary operations
///
/// Handles all unary operators with different evaluation strategies:
///
/// **Address operations** (evaluated before operand):
/// - `&x`, `&mut x`: Address-of operators - load variable address into A+Y
///
/// **Value operations** (evaluate operand first):
/// - `-x`: Negation (two's complement: `~x + 1`)
/// - `~x`: Bitwise NOT (XOR with $FF)
/// - `!x`: Logical NOT (converts to boolean 0/1 and inverts)
/// - `*ptr`: Dereference (load value from address using indirect addressing)
pub(super) fn generate_unary(
    op: UnaryOp,
    operand: &Spanned<Expr>,
    emitter: &mut Emitter,
    info: &ProgramInfo,
    string_collector: &mut StringCollector,
) -> Result<(), CodegenError> {
    match op {
        UnaryOp::Deref => {
            // Dereference: *ptr - Load value from address stored in operand
            generate_expr(operand, emitter, info, string_collector)?;

            // A now contains the address (low byte for u8 pointers)
            // For 6502, we need to set up indirect addressing
            // Store address in zero page for indirect access
            emitter.emit_inst("STA", "$30"); // Store low byte at $30

            // For u16 pointers, we'd also load high byte:
            // For now, assume u8 addresses or low byte only
            emitter.emit_inst("LDA", "#$00");
            emitter.emit_inst("STA", "$31"); // High byte = 0 for zero page

            // Load value using indirect addressing
            emitter.emit_inst("LDY", "#$00");
            emitter.emit_inst("LDA", "($30),Y"); // Indirect indexed addressing

            return Ok(());
        }
        UnaryOp::AddrOf | UnaryOp::AddrOfMut => {
            // Address-of: &var or &mut var - Get the address of a variable
            // Don't evaluate the operand, just get its address
            if let Expr::Variable(name) = &operand.node {
                // Try resolved_symbols first, fallback to global table
                let sym = info
                    .resolved_symbols
                    .get(&operand.span)
                    .or_else(|| info.table.lookup(name));

                if let Some(sym) = sym {
                    let addr = match sym.location {
                        crate::sema::table::SymbolLocation::ZeroPage(zp) => zp as u16,
                        crate::sema::table::SymbolLocation::Absolute(abs) => abs,
                        _ => {
                            return Err(CodegenError::UnsupportedOperation(format!(
                                "Cannot take address of variable with location: {:?}",
                                sym.location
                            )));
                        }
                    };

                    // Load the address into A (low byte) and X (high byte) for 16-bit addresses
                    // For zero page addresses (< 256), high byte is 0
                    // For absolute addresses (>= 256), load both bytes
                    emitter.emit_inst("LDA", &format!("#${:02X}", addr & 0xFF));

                    if addr > 0xFF {
                        // 16-bit address: load high byte into Y
                        emitter.emit_inst("LDY", &format!("#${:02X}", (addr >> 8) & 0xFF));
                        emitter.reg_state.set_y(
                            crate::codegen::regstate::RegisterValue::Immediate(
                                ((addr >> 8) & 0xFF) as i64,
                            ),
                        );
                    } else {
                        // Zero page address: high byte is 0
                        emitter.emit_inst("LDY", "#$00");
                        emitter
                            .reg_state
                            .set_y(crate::codegen::regstate::RegisterValue::Immediate(0));
                    }

                    emitter.reg_state.set_a(
                        crate::codegen::regstate::RegisterValue::Immediate((addr & 0xFF) as i64),
                    );

                    return Ok(());
                } else {
                    return Err(CodegenError::SymbolNotFound(name.clone()));
                }
            } else {
                return Err(CodegenError::UnsupportedOperation(
                    "Address-of (&) only supported on variables".to_string(),
                ));
            }
        }
        _ => {}
    }

    // For other operations, evaluate operand first
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
        UnaryOp::Deref | UnaryOp::AddrOf | UnaryOp::AddrOfMut => {
            // Already handled above
        }
    }

    Ok(())
}
