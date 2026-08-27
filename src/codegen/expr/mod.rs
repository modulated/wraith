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
mod bitop;
mod call;
mod cast;
pub(crate) mod compare;
mod literal;
mod unary;

// Import functions from submodules
use aggregate::{
    generate_enum_variant, generate_field_access, generate_index, generate_struct_init,
};
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
pub use aggregate::generate_struct_init_runtime;
pub(crate) use aggregate::{
    StaticBase, array_field_base, check_runtime_index_range, emit_array_struct_field_indexed,
    emit_struct_place_address, high_byte_in_x, is_call, is_two_byte_value, resolve_static_addr,
    resolve_static_struct_lvalue, type_byte_size,
};
pub(crate) use aggregate::{
    emit_aggregate_base, emit_element_address_into_ptr, yields_struct_pointer,
};
pub(crate) use bitop::bit_test_zp;
pub use call::generate_tail_recursive_update;
pub(crate) use cast::{emit_widen_a_into_y, implicit_widening};

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
                let is_16bit = expr_type.is_some_and(|ty| {
                    matches!(
                        ty,
                        crate::sema::types::Type::Primitive(crate::ast::PrimitiveType::U16)
                            | crate::sema::types::Type::Primitive(crate::ast::PrimitiveType::I16)
                            | crate::sema::types::Type::Primitive(crate::ast::PrimitiveType::B16)
                    )
                });

                // A folded pointer — `0xD012 as &u8` — is still two bytes, but
                // its high byte belongs in X, not Y. Loading only the low byte
                // leaves whatever X happened to hold, which in a zeroed
                // emulator reads as a plausible zero-page address.
                let is_pointer = matches!(expr_type, Some(crate::sema::types::Type::Pointer(_)));

                // Load the constant value
                let val = *n as u64;
                emitter.emit_inst("LDA", &format!("#${:02X}", val & 0xFF));

                if is_16bit {
                    // For 16-bit types, also load high byte into Y
                    emitter.emit_inst("LDY", &format!("#${:02X}", (val >> 8) & 0xFF));
                } else if is_pointer {
                    emitter.emit_inst("LDX", &format!("#${:02X}", (val >> 8) & 0xFF));
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
                let display = s
                    .chars()
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
        Expr::Literal(lit) => {
            // Array literals need the element width so u16/i16/b16 elements emit
            // two little-endian bytes rather than a single truncated byte.
            let elem_size = match info.resolved_types.get(&expr.span) {
                Some(crate::sema::types::Type::Array(elem, _)) => elem.size(),
                _ => 1,
            };
            generate_literal(lit, elem_size, emitter, string_collector)
        }
        Expr::Variable(name) => generate_variable(name, expr.span, emitter, info),
        Expr::Binary { left, op, right } => {
            generate_binary(left, *op, right, emitter, info, string_collector)
        }
        Expr::Unary { op, operand } => {
            generate_unary(*op, operand, emitter, info, string_collector)
        }
        Expr::Call { function, args } => {
            generate_call(function, args, emitter, info, string_collector)
        }
        Expr::Paren(inner) => generate_expr(inner, emitter, info, string_collector), // Just unwrap
        Expr::Cast {
            expr: inner,
            target_type,
        } => generate_type_cast(inner, target_type, emitter, info, string_collector),
        Expr::Index { object, index } => {
            generate_index(object, index, emitter, info, string_collector)
        }
        Expr::CallIndirect { callee, args } => {
            call::generate_call_indirect(callee, args, emitter, info, string_collector)
        }
        Expr::Slice { .. } => {
            // Slices are only valid as assignment targets, not as expressions
            Err(CodegenError::UnsupportedOperation(
                "Slice expressions can only be used as assignment targets".to_string(),
            ))
        }
        Expr::StructInit { name, fields } => {
            generate_struct_init(name, fields, expr.span, emitter, info, string_collector)
        }
        Expr::AnonStructInit { fields } => {
            // Look up the resolved struct name from sema
            let struct_name = info.resolved_struct_names.get(&expr.span).ok_or_else(|| {
                CodegenError::UnsupportedOperation(
                    "Anonymous struct init missing resolved name".to_string(),
                )
            })?;
            // Create a synthetic Spanned<String> for the struct name
            let name = crate::ast::Spanned::new(struct_name.clone(), expr.span);
            generate_struct_init(&name, fields, expr.span, emitter, info, string_collector)
        }
        Expr::Field { object, field } => {
            generate_field_access(object, field, emitter, info, string_collector)
        }
        Expr::BitOp { object, kind, bit } => {
            bitop::generate_bitop(object, *kind, bit, emitter, info, string_collector)
        }
        Expr::EnumVariant {
            enum_name,
            variant,
            data,
        } => generate_enum_variant(enum_name, variant, data, emitter, info, string_collector),
        Expr::SliceLen(object) => {
            // Sema resolved this `.len` as a struct field access rather than
            // the built-in accessor (the parser chose before types were known).
            if info.accessor_fields.contains(&expr.span) {
                let field = Spanned::new("len".to_string(), expr.span);
                return generate_field_access(object, &field, emitter, info, string_collector);
            }
            // Get the type of the object to determine how to access its length
            if let Some(obj_ty) = info.resolved_types.get(&object.span) {
                match obj_ty {
                    crate::sema::types::Type::String => {
                        // String .len access
                        // String is a pointer to length-prefixed data: [u8 length][bytes...]
                        // Strings are limited to 256 bytes max (u8 length)
                        emitter.emit_comment("String .len access (u8 length)");
                        if emitter.is_verbose() {
                            emitter.emit_comment("Load 1-byte length prefix");
                        }

                        // Get string address in A:X
                        generate_expr(object, emitter, info, string_collector)?;

                        // Stage the pointer through the allocator: the staging
                        // is brief, but an enclosing construct (an index
                        // assignment's parked value, a u16 binary op's left
                        // save) may already own $F0/$F1, and the old hardcoded
                        // store overwrote it.
                        let stage = emitter.temp_alloc.alloc_high(2).ok_or_else(|| {
                            emitter.pool_error("temporary storage exhausted in string .len")
                        })?;
                        emitter.emit_inst("STA", &format!("${:02X}", stage));
                        emitter.emit_inst("STX", &format!("${:02X}", stage + 1));

                        // Load length (single byte) via indirect indexed
                        // Result is u8 in A, zero-extended to u16 in Y:A
                        emitter.emit_inst("LDY", "#$00");
                        emitter.emit_inst("LDA", &format!("(${:02X}),Y", stage)); // Load length byte
                        emitter.temp_alloc.free_high(stage, 2);
                        // Length is always <= 255, so high byte is 0
                        emitter.emit_inst("LDY", "#$00"); // High byte = 0
                        // Result: length in A (low byte), Y = 0 (high byte)

                        Ok(())
                    }
                    crate::sema::types::Type::Array(_, n) => {
                        // Array length is a compile-time constant: emit it as a
                        // u16 immediate (A = low byte, Y = high byte). This makes
                        // `arr.len` and idioms like `for i in 0..arr.len` work.
                        let n = *n as u16;
                        emitter.emit_comment(&format!("Array .len (constant {})", n));
                        emitter.emit_lda_immediate((n & 0xFF) as i64);
                        emitter.emit_inst("LDY", &format!("#${:02X}", (n >> 8) & 0xFF));
                        emitter.mark_a_unknown();
                        Ok(())
                    }
                    crate::sema::types::Type::Slice(_) => {
                        // A slice's length is the u16 stored at descriptor bytes
                        // 2..3 (slot[0..1] is the base pointer). Load it into A:Y.
                        if let Expr::Variable(name) = &object.node
                            && let Some(sym) = info
                                .resolved_symbols
                                .get(&object.span)
                                .or_else(|| info.table.lookup(name))
                            && let crate::sema::table::SymbolLocation::ZeroPage(addr) = sym.location
                        {
                            emitter.emit_comment("Slice .len (from descriptor)");
                            emitter.emit_inst("LDA", &format!("${:02X}", addr + 2));
                            emitter.emit_inst("LDY", &format!("${:02X}", addr + 3));
                            emitter.mark_a_unknown();
                            Ok(())
                        } else {
                            Err(CodegenError::UnsupportedOperation(
                                ".len is only supported on slice variables".to_string(),
                            ))
                        }
                    }
                    _ => {
                        // Slices are not yet first-class values, so a runtime
                        // slice length is unreachable here; other types have no
                        // length.
                        Err(CodegenError::UnsupportedOperation(format!(
                            "Length access (.len) not yet implemented for type: {}",
                            obj_ty.display_name()
                        )))
                    }
                }
            } else {
                // No type information available - this shouldn't happen if semantic analysis passed
                Err(CodegenError::UnsupportedOperation(
                    "Length access (.len) missing type information (compiler bug)".to_string(),
                ))
            }
        }

        Expr::U16Low(operand) => {
            // Sema resolved this `.low` as a struct field access.
            if info.accessor_fields.contains(&expr.span) {
                let field = Spanned::new("low".to_string(), expr.span);
                return generate_field_access(operand, &field, emitter, info, string_collector);
            }
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
                                emitter.emit_comment(&format!("Load low byte from ${:02X}", addr));
                            }
                        }
                        SymbolLocation::Absolute(addr) => {
                            emitter.emit_lda_abs(addr);
                            if emitter.is_verbose() {
                                emitter.emit_comment(&format!("Load low byte from ${:04X}", addr));
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
            // Sema resolved this `.high` as a struct field access.
            if info.accessor_fields.contains(&expr.span) {
                let field = Spanned::new("high".to_string(), expr.span);
                return generate_field_access(operand, &field, emitter, info, string_collector);
            }
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
                                emitter.emit_comment(&format!(
                                    "Load high byte from ${:02X}",
                                    addr + 1
                                ));
                            }
                        }
                        SymbolLocation::Absolute(addr) => {
                            emitter.emit_inst("LDA", &format!("${:04X}", addr + 1));
                            if emitter.is_verbose() {
                                emitter.emit_comment(&format!(
                                    "Load high byte from ${:04X}",
                                    addr + 1
                                ));
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

            emitter.emit_inst("BCS", &set_label); // Branch if carry set
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
            emitter.emit_inst("PHP", ""); // Push processor status
            emitter.emit_inst("PLA", ""); // Pull to A
            emitter.emit_inst("AND", "#$02"); // Mask zero flag (bit 1)
            // Now A = 0 if zero clear, 2 if zero set
            // Convert 2 to 1
            let end_label = emitter.next_label("zx");
            emitter.emit_inst("BEQ", &end_label); // If zero, A already = 0
            emitter.emit_inst("LDA", "#$01");
            emitter.emit_label(&end_label);

            Ok(())
        }

        Expr::CpuFlagOverflow => {
            emitter.emit_comment("Read overflow flag");
            // Convert overflow flag to boolean (0 or 1)
            let set_label = emitter.next_label("vf");
            let end_label = emitter.next_label("vx");

            emitter.emit_inst("BVS", &set_label); // Branch if overflow set
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

            emitter.emit_inst("BMI", &set_label); // Branch if minus (negative set)
            // Negative clear
            emitter.emit_inst("LDA", "#$00");
            emitter.emit_inst("JMP", &end_label);
            // Negative set
            emitter.emit_label(&set_label);
            emitter.emit_inst("LDA", "#$01");
            emitter.emit_label(&end_label);

            Ok(())
        }

        Expr::Match {
            expr: match_expr,
            arms,
        } => {
            // The unified result type. A narrow arm body reaching a 16-bit
            // result has to be extended so its high byte in Y is defined, and
            // by which extension depends on the *arm's* type, so the arm needs
            // the result type rather than a yes/no.
            let result_ty = info.resolved_types.get(&expr.span);
            generate_match_expr(match_expr, arms, result_ty, emitter, info, string_collector)
        }
    }
}

/// Generate code for match expression
/// Unlike match statements, match expressions must return a value
fn generate_match_expr(
    match_expr: &Spanned<Expr>,
    arms: &[crate::ast::ExprMatchArm],
    result_ty: Option<&crate::sema::types::Type>,
    emitter: &mut Emitter,
    info: &ProgramInfo,
    string_collector: &mut StringCollector,
) -> Result<(), CodegenError> {
    use crate::ast::Pattern;

    let result_is_u16 = matches!(
        result_ty,
        Some(crate::sema::types::Type::Primitive(
            crate::ast::PrimitiveType::U16
                | crate::ast::PrimitiveType::I16
                | crate::ast::PrimitiveType::B16
        ))
    );

    // Generate an arm body, then extend its high byte (Y) when the unified
    // result is 16-bit but this arm produced only an 8-bit value — otherwise Y
    // would carry a garbage high byte into the u16 result.
    //
    // *Which* extension is the arm's own question, not the result's: an `i8`
    // arm of an `i16` match keeps its sign. This zero-extended every narrow
    // arm, so `match k { 0 => neg, _ => 300 }` turned −59 into 197 — the same
    // defect fixed at the six other widening sites, in a seventh they did not
    // reach, because a match *statement* widens through the assignment and
    // only the expression form comes through here.
    let gen_body = |body: &Spanned<Expr>,
                    emitter: &mut Emitter,
                    info: &ProgramInfo,
                    sc: &mut StringCollector|
     -> Result<(), CodegenError> {
        generate_expr(body, emitter, info, sc)?;
        if result_is_u16 {
            let body_ty = info.resolved_types.get(&body.span);
            let body_is_u16 = matches!(
                body_ty,
                Some(crate::sema::types::Type::Primitive(
                    crate::ast::PrimitiveType::U16
                        | crate::ast::PrimitiveType::I16
                        | crate::ast::PrimitiveType::B16
                ))
            );
            if !body_is_u16 {
                match result_ty.and_then(|rt| implicit_widening(body_ty, rt)) {
                    Some(signed) => emit_widen_a_into_y(emitter, signed),
                    // No widening rule applies (a `bool` arm, or a type sema
                    // did not resolve): the high byte still has to be defined.
                    None => emitter.emit_inst("LDY", "#$00"),
                }
            }
        }
        Ok(())
    };

    let match_id = emitter.next_match_id();
    let end_label = format!("mx_{}", match_id);

    emitter.emit_comment("Match expression");

    // Check if we're matching on an enum
    let is_enum_match = arms
        .iter()
        .any(|arm| matches!(arm.pattern.node, Pattern::EnumVariant { .. }));

    // Whether the scrutinee is 16-bit (low in A/$20, high in Y/$21) and/or
    // signed — mirrors the match-statement path so literal/range patterns and
    // variable bindings see the full value with correct comparison semantics.
    let scrutinee_ty = info.resolved_types.get(&match_expr.span);
    let scrutinee_is_u16 = matches!(
        scrutinee_ty,
        Some(crate::sema::types::Type::Primitive(
            crate::ast::PrimitiveType::U16
                | crate::ast::PrimitiveType::I16
                | crate::ast::PrimitiveType::B16
        ))
    );
    let scrutinee_is_signed = scrutinee_ty.is_some_and(|t| t.is_signed());

    // Enum pointer lives in the pointer-ops area (not $20, which arm bodies use
    // as scratch); the tag is cached at ptr_base+2. This matches the statement
    // path and lets extract_enum_bindings read payloads from a stable pointer.
    let ptr_base = emitter.memory_layout.pointer_ops_start;

    // Evaluate the matched expression
    generate_expr(match_expr, emitter, info, string_collector)?;

    if is_enum_match {
        // For enum matching, expression returns a pointer in A:X
        emitter.emit_inst("STA", &format!("${:02X}", ptr_base));
        emitter.emit_inst("STX", &format!("${:02X}", ptr_base + 1));

        // Load the discriminant tag from the enum (first byte)
        emitter.emit_inst("LDY", "#$00");
        emitter.emit_inst("LDA", &format!("(${:02X}),Y", ptr_base));
        emitter.emit_inst("STA", &format!("${:02X}", ptr_base + 2)); // cache tag
    } else {
        // For simple value matching, store the low byte at $20 (and the high
        // byte at $21 for u16 so patterns/bindings see the full value).
        emitter.emit_inst("STA", "$20");
        if scrutinee_is_u16 {
            emitter.emit_inst("STY", "$21");
        }
    }

    // A non-matching arm runs no body (it branches to its skip label before the
    // body), so the enum pointer / value scratch survives across arms.
    for (i, arm) in arms.iter().enumerate() {
        let next_label = format!("mn_{}_{}", match_id, i);

        match &arm.pattern.node {
            Pattern::EnumVariant {
                enum_name,
                variant,
                bindings,
            } => {
                // Look up the enum and get the tag for this variant
                if let Some(enum_def) = info.type_registry.enums.get(&enum_name.node)
                    && let Some(tag) = enum_def
                        .variants
                        .iter()
                        .position(|v| v.name == variant.node)
                {
                    emitter.emit_inst("LDA", &format!("${:02X}", ptr_base + 2));
                    emitter.emit_inst("CMP", &format!("#${:02X}", tag));
                    emitter.emit_inst("BNE", &next_label);

                    // Copy any payload bindings into their storage (every byte,
                    // so multi-byte payloads keep their high byte).
                    crate::codegen::stmt::extract_enum_bindings(
                        enum_name, variant, bindings, ptr_base, emitter, info,
                    )?;

                    gen_body(&arm.body, emitter, info, string_collector)?;
                    emitter.emit_inst("JMP", &end_label);
                }
                emitter.emit_label(&next_label);
            }

            Pattern::Wildcard => {
                // Wildcard matches everything - just generate the body
                gen_body(&arm.body, emitter, info, string_collector)?;
                emitter.emit_inst("JMP", &end_label);
            }

            Pattern::Variable(name) => {
                // Variable pattern binds the whole value: copy it (both bytes
                // for u16) into the binding's storage, recorded by sema under
                // the pattern span, before running the body.
                copy_scrutinee_to_binding(
                    &arm.pattern.span,
                    name,
                    scrutinee_is_u16,
                    emitter,
                    info,
                )?;
                gen_body(&arm.body, emitter, info, string_collector)?;
                emitter.emit_inst("JMP", &end_label);
            }

            Pattern::Literal(lit_expr) => {
                // Compare against literal (both bytes when the scrutinee is u16).
                if let Expr::Literal(crate::ast::Literal::Integer(n)) = &lit_expr.node {
                    let val = *n as u16;
                    if scrutinee_is_u16 {
                        emitter.emit_inst("LDA", "$20");
                        emitter.emit_inst("CMP", &format!("#${:02X}", val & 0xFF));
                        emitter.emit_inst("BNE", &next_label);
                        emitter.emit_inst("LDA", "$21");
                        emitter.emit_inst("CMP", &format!("#${:02X}", (val >> 8) & 0xFF));
                        emitter.emit_inst("BNE", &next_label);
                    } else {
                        emitter.emit_inst("LDA", "$20");
                        emitter.emit_inst("CMP", &format!("#${:02X}", val & 0xFF));
                        emitter.emit_inst("BNE", &next_label);
                    }
                    gen_body(&arm.body, emitter, info, string_collector)?;
                    emitter.emit_inst("JMP", &end_label);
                }
                emitter.emit_label(&next_label);
            }

            Pattern::Range {
                start,
                end,
                inclusive,
            } => {
                // value >= start && value <= end (or < end+1 for inclusive),
                // over the low byte. Signed ranges use the same folded
                // (N eor V) sign test as the statement path.
                if let (
                    Expr::Literal(crate::ast::Literal::Integer(start_val)),
                    Expr::Literal(crate::ast::Literal::Integer(end_val)),
                ) = (&start.node, &end.node)
                {
                    let upper_bound = if *inclusive { end_val + 1 } else { *end_val };
                    if scrutinee_is_signed {
                        let emit_signed_lt =
                            |emitter: &mut Emitter, bound: i64, target: &str, tag: &str| {
                                let nov = format!("mnr_{}_{}_{}", match_id, i, tag);
                                compare::emit_signed_lt_flag(emitter, bound, &nov);
                                emitter.emit_inst("BMI", target);
                            };
                        // value < start -> skip this arm.
                        emit_signed_lt(emitter, *start_val, &next_label, "v1");
                        // value < end+1 -> in range; else fall through to skip.
                        let body_label = format!("mnrb_{}_{}", match_id, i);
                        emit_signed_lt(emitter, upper_bound, &body_label, "v2");
                        emitter.emit_inst("JMP", &next_label);
                        emitter.emit_label(&body_label);
                    } else {
                        emitter.emit_inst("LDA", "$20");
                        emitter.emit_inst("CMP", &format!("#${:02X}", *start_val as u8));
                        emitter.emit_inst("BCC", &next_label); // value < start
                        emitter.emit_inst("CMP", &format!("#${:02X}", upper_bound as u8));
                        emitter.emit_inst("BCS", &next_label); // value >= end+1
                    }
                    gen_body(&arm.body, emitter, info, string_collector)?;
                    emitter.emit_inst("JMP", &end_label);
                }
                emitter.emit_label(&next_label);
            }
        }
    }

    emitter.emit_label(&end_label);
    Ok(())
}

/// Copy the match scrutinee (held at $20, and $21 when u16) into a variable
/// pattern's binding storage, which sema records under the pattern span.
fn copy_scrutinee_to_binding(
    pattern_span: &crate::ast::Span,
    name: &str,
    scrutinee_is_u16: bool,
    emitter: &mut Emitter,
    info: &ProgramInfo,
) -> Result<(), CodegenError> {
    use crate::sema::table::SymbolLocation;
    let loc = info
        .resolved_symbols
        .get(pattern_span)
        .map(|sym| sym.location.clone())
        .ok_or_else(|| CodegenError::SymbolNotFound(name.to_string()))?;
    match loc {
        SymbolLocation::ZeroPage(addr) => {
            emitter.emit_inst("LDA", "$20");
            emitter.emit_inst("STA", &format!("${:02X}", addr));
            if scrutinee_is_u16 {
                emitter.emit_inst("LDA", "$21");
                emitter.emit_inst("STA", &format!("${:02X}", addr + 1));
            }
        }
        SymbolLocation::Absolute(addr) => {
            emitter.emit_inst("LDA", "$20");
            emitter.emit_inst("STA", &format!("${:04X}", addr));
            if scrutinee_is_u16 {
                emitter.emit_inst("LDA", "$21");
                emitter.emit_inst("STA", &format!("${:04X}", addr + 1));
            }
        }
        _ => {
            return Err(CodegenError::UnsupportedOperation(format!(
                "match binding '{}' has unsupported storage location",
                name
            )));
        }
    }
    emitter.invalidate_registers();
    Ok(())
}
