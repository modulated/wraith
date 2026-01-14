//! Literal and variable loading code generation
//!
//! This module handles:
//! - Integer literals (8-bit and 16-bit)
//! - Boolean literals
//! - String literals
//! - Array literals (compile-time)
//! - Array fill literals `[value; count]`
//! - Variable loading (zero-page and absolute addressing)

use crate::ast::{Expr, Span};
use crate::codegen::{CodegenError, Emitter, StringCollector};
use crate::sema::ProgramInfo;
use crate::sema::table::SymbolLocation;
use crate::sema::types::Type;

/// Generate code for literal values
///
/// Literals are compile-time constants that are directly embedded in the code.
/// - Integers: Load into A (8-bit) or A+Y (16-bit)
/// - Booleans: Load 0 or 1 into A
/// - Strings: Load address into A+X (pointer to length-prefixed string)
/// - Arrays: Emit data inline and load address into A+X
pub(super) fn generate_literal(
    lit: &crate::ast::Literal,
    emitter: &mut Emitter,
    string_collector: &mut StringCollector,
) -> Result<(), CodegenError> {
    match lit {
        crate::ast::Literal::Integer(val) => {
            // Handle values based on size
            // For 8-bit values (0-255 or -128 to 127), load into A only
            // For 16-bit values, load low byte into A and high byte into X
            let value = *val as u64; // Convert to unsigned for bit manipulation

            if value <= 0xFF {
                // 8-bit value: load into A only
                emitter.emit_lda_immediate(*val);
            } else if value <= 0xFFFF {
                // 16-bit value: load low byte into A, high byte into Y
                emitter.emit_inst("LDA", &format!("#${:02X}", value & 0xFF));
                emitter.emit_inst("LDY", &format!("#${:02X}", (value >> 8) & 0xFF));
                emitter
                    .reg_state
                    .set_a(crate::codegen::regstate::RegisterValue::Immediate(
                        (value & 0xFF) as i64,
                    ));
                emitter
                    .reg_state
                    .set_y(crate::codegen::regstate::RegisterValue::Immediate(
                        ((value >> 8) & 0xFF) as i64,
                    ));
            } else {
                // Values larger than 16-bit not supported on 6502
                return Err(CodegenError::UnsupportedOperation(format!(
                    "Integer literal {} too large for 6502 (max 65535)",
                    val
                )));
            }
            Ok(())
        }
        crate::ast::Literal::Bool(val) => {
            let v = if *val { 1 } else { 0 };
            emitter.emit_lda_immediate(v);
            Ok(())
        }
        crate::ast::Literal::String(s) => {
            // Register string with collector (deduplicated automatically)
            let str_label = string_collector.add_string(s.clone());

            // Load address of string into A (low byte) and X (high byte)
            emitter.emit_comment(&format!("String literal: \"{}\" -> {}", s, str_label));
            emitter.emit_inst("LDA", &format!("#<{}", str_label));
            emitter.emit_inst("LDX", &format!("#>{}", str_label));

            // Result: A = low byte of address, X = high byte of address
            Ok(())
        }
        crate::ast::Literal::Array(elements) => {
            // Generate array literal - store data in memory and return address
            let arr_label = emitter.next_label("arr");
            let skip_label = emitter.next_label("as");

            // Jump over the array data
            emitter.emit_inst("JMP", &skip_label);

            // Emit array data with label
            emitter.emit_label(&arr_label);

            // Emit each element
            // Try to evaluate each element as a constant expression
            for elem in elements {
                match &elem.node {
                    // Fast path for simple literals
                    Expr::Literal(crate::ast::Literal::Integer(val)) => {
                        emitter.emit_byte(*val as u8);
                    }
                    Expr::Literal(crate::ast::Literal::Bool(b)) => {
                        emitter.emit_byte(if *b { 1 } else { 0 });
                    }
                    // For complex expressions, try to evaluate as constant
                    _ => {
                        // Try to evaluate as constant expression
                        use crate::sema::const_eval::eval_const_expr;
                        match eval_const_expr(elem) {
                            Ok(const_val) => {
                                // Successfully evaluated as constant
                                if let Some(int_val) = const_val.as_integer() {
                                    emitter.emit_byte(int_val as u8);
                                } else {
                                    return Err(CodegenError::UnsupportedOperation(
                                        "Array elements must evaluate to integer constants"
                                            .to_string(),
                                    ));
                                }
                            }
                            Err(_) => {
                                // Not a constant expression - runtime array construction not supported in literals
                                return Err(CodegenError::UnsupportedOperation(
                                    "Array literals must contain constant expressions (literals or compile-time constants)"
                                        .to_string(),
                                ));
                            }
                        }
                    }
                }
            }

            // Skip label
            emitter.emit_label(&skip_label);

            // Load address of array into A (low byte) and X (high byte)
            emitter.emit_comment(&format!(
                "Load address of array ({} elements)",
                elements.len()
            ));
            emitter.emit_inst("LDA", &format!("#<{}", arr_label));
            emitter.emit_inst("LDX", &format!("#>{}", arr_label));

            Ok(())
        }
        crate::ast::Literal::ArrayFill { value, count } => {
            // Generate array filled with repeated value
            let arr_label = emitter.next_label("af");
            let skip_label = emitter.next_label("af");

            // Jump over the array data
            emitter.emit_inst("JMP", &skip_label);

            // Emit array data with label
            emitter.emit_label(&arr_label);

            // Get the fill value (must be a constant expression)
            let byte_val = match &value.node {
                Expr::Literal(crate::ast::Literal::Integer(val)) => *val as u8,
                Expr::Literal(crate::ast::Literal::Bool(b)) => {
                    if *b {
                        1
                    } else {
                        0
                    }
                }
                _ => {
                    // Try to evaluate as constant expression
                    use crate::sema::const_eval::eval_const_expr;
                    match eval_const_expr(value) {
                        Ok(const_val) => {
                            if let Some(int_val) = const_val.as_integer() {
                                int_val as u8
                            } else {
                                return Err(CodegenError::UnsupportedOperation(
                                    "Array fill value must evaluate to an integer constant"
                                        .to_string(),
                                ));
                            }
                        }
                        Err(_) => {
                            return Err(CodegenError::UnsupportedOperation(
                                "Array fill value must be a constant expression".to_string(),
                            ));
                        }
                    }
                }
            };

            // Optimization: For large zero-filled arrays, use .RES directive
            const ZERO_FILL_THRESHOLD: usize = 16;
            if byte_val == 0 && *count >= ZERO_FILL_THRESHOLD {
                // Use efficient zero-fill directive
                emitter.emit_comment(&format!("Zero-filled array optimized: {} bytes", count));
                emitter.emit_raw(&format!("    .RES {}", count));
            } else {
                // Emit the value 'count' times
                for _ in 0..*count {
                    emitter.emit_byte(byte_val);
                }
            }

            // Skip label
            emitter.emit_label(&skip_label);

            // Load address of array into A (low byte) and X (high byte)
            emitter.emit_comment(&format!(
                "Load address of filled array ({} elements)",
                count
            ));
            emitter.emit_inst("LDA", &format!("#<{}", arr_label));
            emitter.emit_inst("LDX", &format!("#>{}", arr_label));

            Ok(())
        }
    }
}

/// Generate code to load a variable's value
///
/// Variables can be stored in different locations:
/// - Zero-page (0x00-0xFF): Fast, uses optimized instructions
/// - Absolute (0x0100-0xFFFF): Slower, uses 16-bit addressing
/// - Symbolic: Address declarations, uses label names
///
/// For 16-bit variables, loads low byte into A and high byte into Y
pub(super) fn generate_variable(
    name: &str,
    span: Span,
    emitter: &mut Emitter,
    info: &ProgramInfo,
) -> Result<(), CodegenError> {
    use crate::sema::table::SymbolKind;

    if let Some(sym) = info.resolved_symbols.get(&span) {
        // Check if this is a u16/i16/b16 variable that needs both bytes loaded
        let is_u16 = matches!(
            sym.ty,
            Type::Primitive(crate::ast::PrimitiveType::U16)
                | Type::Primitive(crate::ast::PrimitiveType::I16)
                | Type::Primitive(crate::ast::PrimitiveType::B16)
        );

        // Check if this is an enum variable (needs 2-byte pointer in A:X)
        let is_enum = if let Type::Named(type_name) = &sym.ty {
            info.type_registry.get_enum(type_name).is_some()
        } else {
            false
        };

        match sym.location {
            SymbolLocation::Absolute(_addr) => {
                // Check if this is an address declaration - use symbolic name
                if sym.kind == SymbolKind::Address {
                    emitter.emit_lda_symbol(name);
                } else {
                    // Regular variable at absolute address - use numeric
                    emitter.emit_lda_abs(_addr);
                    // For u16/i16, also load high byte into X
                    if is_u16 {
                        emitter.emit_inst("LDX", &format!("${:04X}", _addr + 1));
                    }
                    // For enums, load high byte into X (pointer convention)
                    if is_enum {
                        emitter.emit_inst("LDX", &format!("${:04X}", _addr + 1));
                    }
                }
                Ok(())
            }
            SymbolLocation::ZeroPage(addr) => {
                // Use optimized load that skips if value already in A
                emitter.emit_lda_zp(addr);
                // For u16/i16, also load high byte into Y
                if is_u16 {
                    emitter.emit_inst("LDY", &format!("${:02X}", addr + 1));
                }
                // For enums, load high byte into X (pointer convention: A=low, X=high)
                if is_enum {
                    emitter.emit_inst("LDX", &format!("${:02X}", addr + 1));
                }
                Ok(())
            }
            SymbolLocation::None => Err(CodegenError::UnsupportedOperation(format!(
                "Variable '{}' has no storage location",
                name
            ))),
        }
    } else {
        // Fallback to global lookup if not found in resolved (shouldn't happen if analyzed correctly)
        if let Some(sym) = info.table.lookup(name) {
            match sym.location {
                SymbolLocation::Absolute(addr) => {
                    emitter.emit_inst("LDA", &format!("${:04X}", addr));
                    Ok(())
                }
                _ => Err(CodegenError::UnsupportedOperation(format!(
                    "Variable '{}' has unsupported location type: {:?}",
                    name, sym.location
                ))),
            }
        } else {
            Err(CodegenError::SymbolNotFound(name.to_string()))
        }
    }
}
