//! Type casting code generation
//!
//! This module handles all type conversions:
//! - 8-bit ↔ 16-bit conversions
//! - Sign extension (i8 → i16)
//! - Zero extension (u8 → u16)
//! - Truncation (16-bit → 8-bit)
//! - Boolean conversion (any type → bool)
//! - BCD type conversions (b8 ↔ b16)

use crate::ast::{Expr, PrimitiveType, Spanned, TypeExpr};
use crate::codegen::{CodegenError, Emitter, StringCollector};
use crate::sema::ProgramInfo;

// Import generate_expr from parent module for recursive calls
use super::generate_expr;

/// Generate code for type casting expressions
///
/// Handles all primitive type conversions:
/// - **8-bit → 16-bit**: Zero-extension (u8, b8) or sign-extension (i8)
/// - **16-bit → 8-bit**: Truncation (keeps low byte in A, discards high byte)
/// - **Any → bool**: Converts to canonical boolean (0 or 1)
/// - **BCD conversions**: b8 ↔ b16 (bit pattern unchanged, type safety enforced)
///
/// Complex type casts (structs, enums) are not supported.
pub(super) fn generate_type_cast(
    expr: &Spanned<Expr>,
    target_type: &Spanned<TypeExpr>,
    emitter: &mut Emitter,
    info: &ProgramInfo,
    string_collector: &mut StringCollector,
) -> Result<(), CodegenError> {
    // Get source type to determine what kind of cast is needed
    let source_type = info.resolved_types.get(&expr.span);

    // Evaluate the source expression
    generate_expr(expr, emitter, info, string_collector)?;

    // Determine target type
    match &target_type.node {
        TypeExpr::Primitive(target_prim) => {
            match target_prim {
                PrimitiveType::U16 | PrimitiveType::I16 => {
                    // Check if source is already 16-bit
                    let source_is_16bit = source_type.is_some_and(|ty| {
                        matches!(
                            ty,
                            crate::sema::types::Type::Primitive(PrimitiveType::U16)
                                | crate::sema::types::Type::Primitive(PrimitiveType::I16)
                                | crate::sema::types::Type::Primitive(PrimitiveType::B16)
                        )
                    });

                    // If source is already 16-bit, no extension needed (just type change)
                    if source_is_16bit {
                        emitter.emit_comment(&format!(
                            "Cast to {:?} (no extension needed)",
                            target_prim
                        ));
                        // A and Y already contain the 16-bit value
                        return Ok(());
                    }

                    // Casting from 8-bit to 16-bit: Need to handle high byte
                    // For u8 -> u16: zero-extend (A has low byte, Y should be 0)
                    // For i8 -> i16: sign-extend (A has low byte, Y should be sign-extended)

                    emitter.emit_comment(&format!("Cast to {:?}", target_prim));

                    if matches!(target_prim, PrimitiveType::I16) {
                        // Sign extension: if bit 7 of A is set, Y = $FF, else Y = $00
                        if emitter.is_verbose() {
                            emitter.emit_comment(
                                "Sign-extend i8 to i16: replicate sign bit to high byte",
                            );
                        }
                        emitter.emit_inst("TAX", ""); // Save value in X temporarily
                        emitter.emit_inst("AND", "#$80"); // Check sign bit
                        let neg_label = emitter.next_label("sn");
                        let end_label = emitter.next_label("sx");

                        emitter.emit_inst("BEQ", &neg_label); // If zero (positive), use 0
                        emitter.emit_inst("LDA", "#$FF"); // Negative: high byte = $FF
                        emitter.emit_inst("JMP", &end_label);
                        emitter.emit_label(&neg_label);
                        emitter.emit_inst("LDA", "#$00"); // Positive: high byte = $00
                        emitter.emit_label(&end_label);

                        // Now A has high byte, X has low byte - put high byte in Y
                        emitter.emit_inst("TAY", ""); // Y = high byte
                        emitter.emit_inst("TXA", ""); // A = low byte
                        if emitter.is_verbose() {
                            emitter.emit_comment("Result: A=low_byte, Y=sign_extended_high_byte");
                        }
                    } else {
                        // Zero extension: Y = 0
                        if emitter.is_verbose() {
                            emitter.emit_comment("Zero-extend u8 to u16: high byte = 0");
                        }
                        emitter.emit_inst("LDY", "#$00");
                        // A already has the low byte
                        if emitter.is_verbose() {
                            emitter.emit_comment("Result: A=low_byte, Y=$00");
                        }
                    }
                }
                PrimitiveType::Addr => {
                    // addr type cannot be used as a cast target - it's only for declarations
                    return Err(CodegenError::UnsupportedOperation(
                        "cannot cast to addr type (addr is only for memory-mapped I/O declarations)".to_string()
                    ));
                }
                PrimitiveType::U8 | PrimitiveType::I8 => {
                    // Casting to 8-bit: Just truncate (A already has the value)
                    emitter.emit_comment(&format!("Cast to {:?} (truncate)", target_prim));
                    // For u16/i16 -> u8, we just keep A (low byte), discard high byte
                    // A already contains the result
                    if emitter.is_verbose() {
                        emitter.emit_comment("Result: A=low_byte (high byte discarded)");
                    }
                }
                PrimitiveType::Bool => {
                    // Cast to bool: 0 = false, non-zero = true
                    // Convert to canonical boolean (0 or 1)
                    emitter.emit_comment("Cast to bool");
                    if emitter.is_verbose() {
                        emitter.emit_comment("Convert to canonical bool: 0=false, 1=true");
                    }
                    let true_label = emitter.next_label("bt");
                    let end_label = emitter.next_label("bx");

                    emitter.emit_inst("CMP", "#$00");
                    emitter.emit_inst("BNE", &true_label);
                    // False case
                    emitter.emit_inst("LDA", "#$00");
                    emitter.emit_inst("JMP", &end_label);
                    // True case
                    emitter.emit_label(&true_label);
                    emitter.emit_inst("LDA", "#$01");
                    emitter.emit_label(&end_label);
                    if emitter.is_verbose() {
                        emitter.emit_comment("Result: A=boolean (0 or 1)");
                    }
                }
                PrimitiveType::B16 => {
                    // Check if source is already 16-bit
                    let source_is_16bit = source_type.is_some_and(|ty| {
                        matches!(
                            ty,
                            crate::sema::types::Type::Primitive(PrimitiveType::U16)
                                | crate::sema::types::Type::Primitive(PrimitiveType::I16)
                                | crate::sema::types::Type::Primitive(PrimitiveType::B16)
                        )
                    });

                    // If source is already 16-bit, no extension needed (just type change)
                    if source_is_16bit {
                        emitter.emit_comment("Cast to b16 (no extension needed)");
                        // A and Y already contain the 16-bit value
                        return Ok(());
                    }

                    // Casting from 8-bit to b16: zero-extend
                    emitter.emit_comment("Cast to b16");
                    if emitter.is_verbose() {
                        emitter.emit_comment("Zero-extend to b16: high byte = 0");
                    }
                    emitter.emit_inst("LDY", "#$00");
                    if emitter.is_verbose() {
                        emitter.emit_comment("Result: A=low_byte, Y=$00");
                    }
                }
                PrimitiveType::B8 => {
                    // Casting to b8: truncate (same as u8)
                    emitter.emit_comment("Cast to b8 (truncate)");
                    if emitter.is_verbose() {
                        emitter.emit_comment("Result: A=low_byte (high byte discarded)");
                    }
                }
            }
        }
        TypeExpr::Pointer { .. } => {
            // Pointer cast: for now, just treat as address value
            emitter.emit_comment("Cast to pointer (no conversion)");
        }
        _ => {
            // Casting to/from complex types (structs, enums, etc.) is not supported
            // Only primitive type casts are part of the language
            return Err(CodegenError::UnsupportedOperation(format!(
                "cannot cast to complex type: {:?}",
                target_type.node
            )));
        }
    }

    Ok(())
}
