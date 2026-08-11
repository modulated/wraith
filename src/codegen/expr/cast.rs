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

/// Move the high byte from X to Y (pointer convention -> u16 convention).
///
/// The 6502 has no `TXY`, and going through A (`PHA/TXA/TAY/PLA`) is four
/// instructions and 11 cycles because A is holding the low byte. A round trip
/// through a zero-page scratch byte is two instructions and 6 cycles.
fn move_x_to_y(emitter: &mut Emitter) {
    let tmp = emitter.memory_layout.loop_end_temp();
    emitter.emit_inst("STX", &format!("${:02X}", tmp));
    emitter.emit_inst("LDY", &format!("${:02X}", tmp));
}

/// Move the high byte from Y to X (u16 convention -> pointer convention).
fn move_y_to_x(emitter: &mut Emitter) {
    let tmp = emitter.memory_layout.loop_end_temp();
    emitter.emit_inst("STY", &format!("${:02X}", tmp));
    emitter.emit_inst("LDX", &format!("${:02X}", tmp));
}

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

    // Check if source is an enum type
    let source_is_enum = source_type.is_some_and(|ty| {
        if let crate::sema::types::Type::Named(type_name) = ty {
            info.type_registry.get_enum(type_name).is_some()
        } else {
            false
        }
    });

    // A pointer arrives in A:X; a 16-bit scalar in A:Y. Casting between them is
    // therefore a register shuffle, not a value change — and the 6502 has no
    // X↔Y transfer, so it has to go through memory. (A function name is NOT
    // this shape: code addresses use the A:Y scalar convention.)
    let source_is_pointer = matches!(source_type, Some(crate::sema::types::Type::Pointer(_)));

    // Evaluate the source expression
    generate_expr(expr, emitter, info, string_collector)?;

    // If source is an enum, dereference the pointer to get the discriminant
    // Enums are represented as pointers (A:X = low:high) to their data,
    // where the first byte is the discriminant tag
    if source_is_enum {
        emitter.emit_comment("Dereference enum pointer to get discriminant");
        // A = low byte of pointer, X = high byte
        emitter.emit_inst("STA", "$20");
        emitter.emit_inst("STX", "$21");
        emitter.emit_inst("LDY", "#$00");
        emitter.emit_inst("LDA", "($20),Y");
        // Now A contains the discriminant value
    }

    // Determine target type
    match &target_type.node {
        TypeExpr::Primitive(target_prim) => {
            match target_prim {
                PrimitiveType::U16 | PrimitiveType::I16 => {
                    // `p as u16`: the bytes are already right, but the high one
                    // is in X and a u16 wants it in Y.
                    if source_is_pointer {
                        emitter.emit_comment(&format!("Cast pointer to {:?}", target_prim));
                        move_x_to_y(emitter);
                        return Ok(());
                    }

                    // Check if source is already 16-bit. A function name
                    // counts: a code address is 2 bytes and arrives in A:Y
                    // like any other 16-bit scalar, so `f as u16` is a pure
                    // type change. (Zero-extending it used to clobber the
                    // high byte — mem_jump landed in nowhere.)
                    let source_is_16bit = source_type.is_some_and(|ty| {
                        matches!(
                            ty,
                            crate::sema::types::Type::Primitive(PrimitiveType::U16)
                                | crate::sema::types::Type::Primitive(PrimitiveType::I16)
                                | crate::sema::types::Type::Primitive(PrimitiveType::B16)
                                | crate::sema::types::Type::Function(_, _)
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

                    // Casting from 8-bit to 16-bit. Which extension applies is
                    // a property of the *source*, not the destination: a signed
                    // source carries its sign into the high byte, an unsigned
                    // one carries zero. Reading it off the destination instead
                    // gets both mixed cases backwards — `200u8 as i16` came out
                    // −56 rather than 200, and `-1i8 as u16` came out 255 rather
                    // than 65535. With no resolved source type to consult, fall
                    // back to the destination, which is right whenever the two
                    // agree.
                    let source_is_signed = match source_type {
                        Some(crate::sema::types::Type::Primitive(p)) => {
                            matches!(p, PrimitiveType::I8 | PrimitiveType::I16)
                        }
                        None => matches!(target_prim, PrimitiveType::I16),
                        _ => false,
                    };

                    emitter.emit_comment(&format!("Cast to {:?}", target_prim));

                    if source_is_signed {
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
                PrimitiveType::U8 | PrimitiveType::I8 | PrimitiveType::Char => {
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
        // `n as &T` — reinterpret an address as a pointer. Only the register
        // convention changes: a pointer's high byte lives in X.
        TypeExpr::Pointer { .. } => {
            if source_is_pointer {
                // `p as &U` is a pure retype; the bytes are already in A:X.
                emitter.emit_comment("Cast between pointer types (no change)");
                return Ok(());
            }
            let source_is_16bit = source_type.is_some_and(|ty| {
                matches!(
                    ty,
                    crate::sema::types::Type::Primitive(
                        PrimitiveType::U16 | PrimitiveType::I16 | PrimitiveType::B16
                    )
                )
            });
            emitter.emit_comment("Cast to pointer");
            if source_is_16bit {
                move_y_to_x(emitter);
            } else {
                // An 8-bit source addresses zero page, so the high byte is $00.
                emitter.emit_inst("LDX", "#$00");
            }
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

    // Casts rewrite A via raw instructions (the bool conversion, the enum-pointer
    // dereference) that don't update register tracking, so drop cached beliefs —
    // otherwise a following load of the cast's source could be wrongly elided.
    emitter.mark_a_unknown();
    Ok(())
}
