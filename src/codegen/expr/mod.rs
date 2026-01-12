//! Expression Code Generation
//!
//! Compiles expressions into assembly instructions.
//! Result is typically left in the Accumulator (A).

use crate::ast::{Expr, Spanned};
use crate::codegen::{CodegenError, Emitter, StringCollector};
use crate::sema::ProgramInfo;
use crate::sema::table::SymbolLocation;

// Submodules
mod aggregate;
mod binary;
mod call;
mod cast;
mod compare;
mod literal;
mod unary;

// Import functions from submodules
use aggregate::{generate_enum_variant, generate_field_access, generate_index, generate_struct_init};
use binary::generate_binary;
use call::generate_call;
use cast::generate_type_cast;
use compare::{
    generate_compare_eq, generate_compare_ge, generate_compare_gt, generate_compare_le,
    generate_compare_lt, generate_compare_ne, generate_logical_and, generate_logical_or,
};
use literal::{generate_literal, generate_variable};
use unary::generate_unary;

// Re-export for use in other codegen modules
pub use call::generate_tail_recursive_update;
pub use aggregate::generate_struct_init_runtime;

pub fn generate_expr(
    expr: &Spanned<Expr>,
    emitter: &mut Emitter,
    info: &ProgramInfo,
    string_collector: &mut StringCollector,
) -> Result<(), CodegenError> {
    // Check if this expression was constant-folded
    if let Some(const_val) = info.folded_constants.get(&expr.span) {
        match const_val {
            crate::sema::const_eval::ConstValue::Integer(n) => {
                // Check if this is a 16-bit type
                let expr_type = info.resolved_types.get(&expr.span);
                let is_16bit = expr_type.is_some_and(|ty| matches!(ty,
                    crate::sema::types::Type::Primitive(crate::ast::PrimitiveType::U16) |
                    crate::sema::types::Type::Primitive(crate::ast::PrimitiveType::I16) |
                    crate::sema::types::Type::Primitive(crate::ast::PrimitiveType::B16)
                ));

                // Load the constant value
                let val = *n as u64;
                emitter.emit_inst("LDA", &format!("#${:02X}", val & 0xFF));

                if is_16bit {
                    // For 16-bit types, also load high byte into Y
                    emitter.emit_inst("LDY", &format!("#${:02X}", (val >> 8) & 0xFF));
                }

                return Ok(());
            }
            crate::sema::const_eval::ConstValue::Bool(b) => {
                emitter.emit_inst("LDA", if *b { "#$01" } else { "#$00" });
                return Ok(());
            }
            crate::sema::const_eval::ConstValue::String(s) => {
                // Register string with collector (deduplicated automatically)
                let str_label = string_collector.add_string(s.clone());

                // Escape special characters for display in comment
                let display = s.chars()
                    .map(|c| match c {
                        '\n' => "\\n".to_string(),
                        '\r' => "\\r".to_string(),
                        '\t' => "\\t".to_string(),
                        '\0' => "\\0".to_string(),
                        '\\' => "\\\\".to_string(),
                        '"' => "\\\"".to_string(),
                        c if c.is_ascii_graphic() || c == ' ' => c.to_string(),
                        c => format!("\\x{:02X}", c as u8),
                    })
                    .collect::<String>();

                // Load address of string into A (low byte) and X (high byte)
                emitter.emit_comment(&format!("Const string: \"{}\" -> {}", display, str_label));
                emitter.emit_inst("LDA", &format!("#<{}", str_label));
                emitter.emit_inst("LDX", &format!("#>{}", str_label));
                return Ok(());
            }
        }
    }

    match &expr.node {
        Expr::Literal(lit) => generate_literal(lit, emitter, string_collector),
        Expr::Variable(name) => generate_variable(name, expr.span, emitter, info),
        Expr::Binary { left, op, right } => generate_binary(left, *op, right, emitter, info, string_collector),
        Expr::Unary { op, operand } => generate_unary(*op, operand, emitter, info, string_collector),
        Expr::Call { function, args } => generate_call(function, args, emitter, info, string_collector),
        Expr::Paren(inner) => generate_expr(inner, emitter, info, string_collector), // Just unwrap
        Expr::Cast { expr: inner, target_type } => {
            generate_type_cast(inner, target_type, emitter, info, string_collector)
        }
        Expr::Index { object, index } => generate_index(object, index, emitter, info, string_collector),
        Expr::Slice { .. } => {
            // Slices are only valid as assignment targets, not as expressions
            Err(CodegenError::UnsupportedOperation(
                "Slice expressions can only be used as assignment targets".to_string()
            ))
        }
        Expr::StructInit { name, fields } => generate_struct_init(name, fields, emitter, info),
        Expr::AnonStructInit { fields } => {
            // Look up the resolved struct name from sema
            let struct_name = info.resolved_struct_names.get(&expr.span)
                .ok_or_else(|| CodegenError::UnsupportedOperation(
                    "Anonymous struct init missing resolved name".to_string()
                ))?;
            // Create a synthetic Spanned<String> for the struct name
            let name = crate::ast::Spanned::new(struct_name.clone(), expr.span);
            generate_struct_init(&name, fields, emitter, info)
        }
        Expr::Field { object, field } => generate_field_access(object, field, emitter, info),
        Expr::EnumVariant {
            enum_name,
            variant,
            data,
        } => generate_enum_variant(enum_name, variant, data, emitter, info),
        Expr::SliceLen(object) => {
            // Get the type of the object to determine how to access its length
            if let Some(obj_ty) = info.resolved_types.get(&object.span) {
                match obj_ty {
                    crate::sema::types::Type::String => {
                        // String .len access
                        // String is a pointer to length-prefixed data: [u16 length][bytes...]
                        emitter.emit_comment("String .len access");
                        if emitter.is_verbose() {
                            emitter.emit_comment("Load 2-byte length prefix (little-endian u16)");
                        }

                        // Get string address in A:X
                        generate_expr(object, emitter, info, string_collector)?;

                        // Store pointer to temp location ($F0-$F1)
                        emitter.emit_inst("STA", "$F0");
                        emitter.emit_inst("STX", "$F1");

                        // Load length (first 2 bytes) via indirect indexed
                        emitter.emit_inst("LDY", "#$00");
                        emitter.emit_inst("LDA", "($F0),Y"); // Low byte of length
                        emitter.emit_inst("TAX", "");  // Save low byte in X temporarily
                        emitter.emit_inst("INY", "");
                        emitter.emit_inst("LDA", "($F0),Y"); // High byte of length
                        // Result: length in A (high) and X (low)
                        // Swap them so A has low byte, Y has high byte (standard u16 convention)
                        emitter.emit_inst("TAY", "");  // High byte to Y
                        emitter.emit_inst("TXA", "");  // Low byte to A

                        Ok(())
                    }
                    _ => {
                        // Other types not yet supported
                        Err(CodegenError::UnsupportedOperation(
                            format!("Length access (.len) not yet implemented for type: {}", obj_ty.display_name())
                        ))
                    }
                }
            } else {
                // No type information available - this shouldn't happen if semantic analysis passed
                Err(CodegenError::UnsupportedOperation(
                    "Length access (.len) missing type information (compiler bug)".to_string()
                ))
            }
        }

        Expr::U16Low(operand) => {
            emitter.emit_comment("u16/i16 .low access");

            // Optimize for simple variable access (most common case)
            if let Expr::Variable(name) = &operand.node {
                if let Some(sym) = info
                    .resolved_symbols
                    .get(&operand.span)
                    .or_else(|| info.table.lookup(name))
                {
                    match sym.location {
                        SymbolLocation::ZeroPage(addr) => {
                            emitter.emit_lda_zp(addr);
                            if emitter.is_verbose() {
                                emitter
                                    .emit_comment(&format!("Load low byte from ${:02X}", addr));
                            }
                        }
                        SymbolLocation::Absolute(addr) => {
                            emitter.emit_lda_abs(addr);
                            if emitter.is_verbose() {
                                emitter
                                    .emit_comment(&format!("Load low byte from ${:04X}", addr));
                            }
                        }
                        _ => {
                            return Err(CodegenError::UnsupportedOperation(format!(
                                "Cannot access .low of variable '{}'",
                                name
                            )));
                        }
                    }
                } else {
                    return Err(CodegenError::SymbolNotFound(name.clone()));
                }
            } else {
                // For expressions: evaluate (result in A=low, Y=high), low already in A
                generate_expr(operand, emitter, info, string_collector)?;
                if emitter.is_verbose() {
                    emitter.emit_comment("Expression result: low byte already in A");
                }
            }

            Ok(())
        }

        Expr::U16High(operand) => {
            emitter.emit_comment("u16/i16 .high access");

            // Optimize for simple variable access
            if let Expr::Variable(name) = &operand.node {
                if let Some(sym) = info
                    .resolved_symbols
                    .get(&operand.span)
                    .or_else(|| info.table.lookup(name))
                {
                    match sym.location {
                        SymbolLocation::ZeroPage(addr) => {
                            emitter.emit_inst("LDA", &format!("${:02X}", addr + 1));
                            if emitter.is_verbose() {
                                emitter
                                    .emit_comment(&format!("Load high byte from ${:02X}", addr + 1));
                            }
                        }
                        SymbolLocation::Absolute(addr) => {
                            emitter.emit_inst("LDA", &format!("${:04X}", addr + 1));
                            if emitter.is_verbose() {
                                emitter
                                    .emit_comment(&format!("Load high byte from ${:04X}", addr + 1));
                            }
                        }
                        _ => {
                            return Err(CodegenError::UnsupportedOperation(format!(
                                "Cannot access .high of variable '{}'",
                                name
                            )));
                        }
                    }
                } else {
                    return Err(CodegenError::SymbolNotFound(name.clone()));
                }
            } else {
                // For expressions: evaluate (result in A=low, Y=high), transfer Y to A
                generate_expr(operand, emitter, info, string_collector)?;
                emitter.emit_inst("TYA", "");
                if emitter.is_verbose() {
                    emitter.emit_comment("Transfer high byte from Y to A");
                }
            }

            Ok(())
        }

        // CPU status flags - read current processor status
        Expr::CpuFlagCarry => {
            emitter.emit_comment("Read carry flag");
            // Convert carry flag to boolean (0 or 1)
            let set_label = emitter.next_label("cf");
            let end_label = emitter.next_label("cx");

            emitter.emit_inst("BCS", &set_label);  // Branch if carry set
            // Carry clear
            emitter.emit_inst("LDA", "#$00");
            emitter.emit_inst("JMP", &end_label);
            // Carry set
            emitter.emit_label(&set_label);
            emitter.emit_inst("LDA", "#$01");
            emitter.emit_label(&end_label);

            Ok(())
        }

        Expr::CpuFlagZero => {
            emitter.emit_comment("Read zero flag");
            // Convert zero flag to boolean (0 or 1)
            // Note: We need a value to test. Use a register that's likely unchanged
            // or better: use PHP (push processor status) and PLA
            emitter.emit_inst("PHP", "");  // Push processor status
            emitter.emit_inst("PLA", "");  // Pull to A
            emitter.emit_inst("AND", "#$02");  // Mask zero flag (bit 1)
            // Now A = 0 if zero clear, 2 if zero set
            // Convert 2 to 1
            let end_label = emitter.next_label("zx");
            emitter.emit_inst("BEQ", &end_label);  // If zero, A already = 0
            emitter.emit_inst("LDA", "#$01");
            emitter.emit_label(&end_label);

            Ok(())
        }

        Expr::CpuFlagOverflow => {
            emitter.emit_comment("Read overflow flag");
            // Convert overflow flag to boolean (0 or 1)
            let set_label = emitter.next_label("vf");
            let end_label = emitter.next_label("vx");

            emitter.emit_inst("BVS", &set_label);  // Branch if overflow set
            // Overflow clear
            emitter.emit_inst("LDA", "#$00");
            emitter.emit_inst("JMP", &end_label);
            // Overflow set
            emitter.emit_label(&set_label);
            emitter.emit_inst("LDA", "#$01");
            emitter.emit_label(&end_label);

            Ok(())
        }

        Expr::CpuFlagNegative => {
            emitter.emit_comment("Read negative flag");
            // Convert negative flag to boolean (0 or 1)
            let set_label = emitter.next_label("nf");
            let end_label = emitter.next_label("nx");

            emitter.emit_inst("BMI", &set_label);  // Branch if minus (negative set)
            // Negative clear
            emitter.emit_inst("LDA", "#$00");
            emitter.emit_inst("JMP", &end_label);
            // Negative set
            emitter.emit_label(&set_label);
            emitter.emit_inst("LDA", "#$01");
            emitter.emit_label(&end_label);

            Ok(())
        }
    }
}

