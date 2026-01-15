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
///
/// Generate code for unary operations
pub(super) fn generate_unary(
    op: UnaryOp,
    operand: &Spanned<Expr>,
    expr_span: crate::ast::Span,
    emitter: &mut Emitter,
    info: &ProgramInfo,
    string_collector: &mut StringCollector,
) -> Result<(), CodegenError> {
    match op {
        UnaryOp::Deref => {
            // Dereference: *ptr
            // 1. Evaluate operand to get the pointer (Address)
            generate_expr(operand, emitter, info, string_collector)?;

            // Result is in A:X (Pointer convention)
            // Store to Zero Page Scratch ($30-$31) for indirect addressing
            emitter.emit_inst("STA", "$30");
            emitter.emit_inst("STX", "$31");

            // 2. Determine type of the result (the pointee type)
            let result_ty = info.resolved_types.get(&expr_span).ok_or_else(|| {
                CodegenError::UnsupportedOperation("Missing type info for dereference".to_string())
            })?;

            // 3. Load value based on type
            match result_ty {
                // 1-byte types
                crate::sema::types::Type::Primitive(crate::ast::PrimitiveType::U8)
                | crate::sema::types::Type::Primitive(crate::ast::PrimitiveType::I8)
                | crate::sema::types::Type::Primitive(crate::ast::PrimitiveType::Bool)
                | crate::sema::types::Type::Primitive(crate::ast::PrimitiveType::B8) => {
                    emitter.emit_inst("LDY", "#$00");
                    emitter.emit_inst("LDA", "($30),Y");
                }

                // 2-byte Arithmetic types (A:Y convention)
                crate::sema::types::Type::Primitive(crate::ast::PrimitiveType::U16)
                | crate::sema::types::Type::Primitive(crate::ast::PrimitiveType::I16)
                | crate::sema::types::Type::Primitive(crate::ast::PrimitiveType::B16) => {
                    // Safe sequence: Load Low->X, Load High->Y, Move X->A
                    emitter.emit_inst("LDY", "#$00");
                    emitter.emit_inst("LDA", "($30),Y"); // Low byte -> A
                    emitter.emit_inst("TAX", ""); // Save Low in X

                    emitter.emit_inst("INY", "");
                    emitter.emit_inst("LDA", "($30),Y"); // High byte -> A
                    emitter.emit_inst("TAY", ""); // High byte -> Y

                    emitter.emit_inst("TXA", ""); // Restore Low byte to A
                }

                // 2-byte Pointer types / Enums (A:X convention)
                crate::sema::types::Type::Pointer(..)
                | crate::sema::types::Type::String
                | crate::sema::types::Type::Array(..)
                | crate::sema::types::Type::Named(_) => {
                    // Optimized sequence: High->X, Low->A

                    // High byte first (Y=1)
                    emitter.emit_inst("LDY", "#$01");
                    emitter.emit_inst("LDA", "($30),Y");
                    emitter.emit_inst("TAX", ""); // High -> X

                    // Low byte second (Y=0)
                    emitter.emit_inst("LDY", "#$00");
                    emitter.emit_inst("LDA", "($30),Y"); // Low -> A
                }

                crate::sema::types::Type::Void => {
                    // No load needed
                }

                _ => {
                    // Function pointers?
                    emitter.emit_inst("LDY", "#$01");
                    emitter.emit_inst("LDA", "($30),Y");
                    emitter.emit_inst("TAX", "");
                    emitter.emit_inst("LDY", "#$00");
                    emitter.emit_inst("LDA", "($30),Y");
                }
            }

            return Ok(());
        }
        UnaryOp::AddrOf | UnaryOp::AddrOfMut => {
            // Address-of: &var
            if let Expr::Variable(name) = &operand.node {
                // Determine symbol location
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

                    // Load Address into A:X (Pointer Convention)
                    // Low byte -> A
                    emitter.emit_inst("LDA", &format!("#${:02X}", addr & 0xFF));

                    // High byte -> X
                    if addr > 0xFF {
                        emitter.emit_inst("LDX", &format!("#${:02X}", (addr >> 8) & 0xFF));
                    } else {
                        // Zero page, high byte is 0
                        emitter.emit_inst("LDX", "#$00");
                    }

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
