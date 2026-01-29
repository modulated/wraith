//! Aggregate data structure code generation
//!
//! This module handles:
//! - Array indexing
//! - String indexing (with length prefix)
//! - Struct initialization
//! - Struct field access
//! - Enum variant construction

use crate::Spanned;
use crate::ast::Expr;
use crate::codegen::{CodegenError, Emitter, StringCollector};
use crate::sema::ProgramInfo;

// Import generate_expr from parent module for recursive calls
use super::generate_expr;

pub(super) fn generate_index(
    object: &Spanned<Expr>,
    index: &Spanned<Expr>,
    emitter: &mut Emitter,
    info: &ProgramInfo,
    string_collector: &mut StringCollector,
) -> Result<(), CodegenError> {
    // Check if we're indexing a string
    if let Some(obj_ty) = info.resolved_types.get(&object.span)
        && matches!(obj_ty, crate::sema::types::Type::String)
    {
        // String indexing: s[i]
        // String format: [u8 length][bytes...] (max 256 bytes)
        // Strategy:
        // 1. Get string pointer in A:X
        // 2. Store to temp ($F0-$F1)
        // 3. Add 1 to pointer (skip u8 length prefix)
        // 4. Get index in Y register
        // 5. Load byte: LDA ($F0),Y

        emitter.emit_comment("String indexing: s[i]");
        if emitter.is_verbose() {
            emitter.emit_comment("Skip 1-byte length header to access character data");
        }

        // Get string pointer
        generate_expr(object, emitter, info, string_collector)?;
        emitter.emit_inst("STA", "$F0");
        emitter.emit_inst("STX", "$F1");

        // Skip length prefix (add 1 to pointer for u8 length)
        if emitter.is_verbose() {
            emitter.emit_comment("Add 1 to pointer to skip u8 length prefix");
        }
        let skip_label = emitter.next_label("si");
        emitter.emit_inst("INC", "$F0");
        emitter.emit_inst("BNE", &skip_label);
        emitter.emit_inst("INC", "$F1");
        emitter.emit_label(&skip_label);

        // Get index in Y
        generate_expr(index, emitter, info, string_collector)?;
        emitter.emit_inst("TAY", "");

        // Load byte
        emitter.emit_inst("LDA", "($F0),Y");
        emitter.reg_state.modify_a();

        return Ok(());
    }

    // For array indexing: array[index]
    // Strategy:
    // 1. Get the base address of the array
    // 2. Generate index into Y register
    // 3. Use absolute indexed addressing: LDA base,Y

    if !emitter.is_minimal() {
        emitter.emit_comment("Array access: base[index]");
    }

    // Currently only supporting variable arrays
    // For local arrays, we need to get the address where they're stored
    match &object.node {
        Expr::Variable(name) => {
            // Look up the array variable to get its location
            let sym = info
                .resolved_symbols
                .get(&object.span)
                .or_else(|| info.table.lookup(name))
                .ok_or_else(|| CodegenError::SymbolNotFound(name.clone()))?;

            match sym.location {
                crate::sema::table::SymbolLocation::Absolute(addr) => {
                    // Array variables store a pointer to the array data
                    // Need to use indirect indexed addressing: LDA (ptr),Y
                    // But indirect indexed requires zero-page pointer
                    // So we need to copy the pointer to zero page first (if not already there)

                    // For now, assume array pointers are in zero page (address < 256)
                    if addr >= 256 {
                        return Err(CodegenError::UnsupportedOperation(
                            "array variables must be in zero page for indexing".to_string(),
                        ));
                    }

                    if emitter.is_verbose() {
                        emitter.emit_comment("Use indirect indexed addressing: (ptr),Y");
                    }

                    // Generate index expression -> A, then transfer to Y
                    generate_expr(index, emitter, info, string_collector)?;
                    emitter.emit_inst("TAY", "");

                    // Use indirect indexed addressing: LDA (ptr),Y
                    // The array variable holds a 2-byte pointer in zero page
                    emitter.emit_inst("LDA", &format!("(${:02X}),Y", addr));
                    emitter.reg_state.modify_a();
                    Ok(())
                }
                crate::sema::table::SymbolLocation::ZeroPage(addr) => {
                    // Array in zero page - use indirect indexed addressing
                    generate_expr(index, emitter, info, string_collector)?;
                    emitter.emit_inst("TAY", "");
                    emitter.emit_inst("LDA", &format!("(${:02X}),Y", addr));
                    emitter.reg_state.modify_a();
                    Ok(())
                }
                crate::sema::table::SymbolLocation::None => {
                    // Compile-time constants don't have runtime storage
                    Err(CodegenError::UnsupportedOperation(
                        "cannot index compile-time constant".to_string(),
                    ))
                }
            }
        }
        _ => {
            // Complex array expressions not yet supported
            Err(CodegenError::UnsupportedOperation(
                "only variable array indexing is currently supported".to_string(),
            ))
        }
    }
}

pub(super) fn generate_struct_init(
    name: &Spanned<String>,
    fields: &[crate::ast::FieldInit],
    emitter: &mut Emitter,
    info: &ProgramInfo,
) -> Result<(), CodegenError> {
    emitter.emit_comment(&format!("Struct init: {}", name.node));

    // Look up the struct definition
    let struct_def = info.type_registry.get_struct(&name.node).ok_or_else(|| {
        CodegenError::UnsupportedOperation(format!(
            "struct '{}' not found in type registry",
            name.node
        ))
    })?;

    // Generate labels for struct data
    let struct_label = emitter.next_label(&format!("struct_{}", name.node));
    let skip_label = emitter.next_label("ks");

    // Jump over the data
    emitter.emit_inst("JMP", &skip_label);

    // Emit struct data
    emitter.emit_label(&struct_label);

    // Create a map of field values for quick lookup
    let field_values: std::collections::HashMap<String, &Spanned<crate::ast::Expr>> = fields
        .iter()
        .map(|f| (f.name.node.clone(), &f.value))
        .collect();

    // Initialize each field in order (respecting struct layout)
    for field_info in &struct_def.fields {
        if let Some(value_expr) = field_values.get(&field_info.name) {
            // Evaluate the field value expression and emit as data
            // For now, we only support constant expressions
            if let crate::ast::Expr::Literal(lit) = &value_expr.node {
                match lit {
                    crate::ast::Literal::Integer(val) => {
                        // Emit the appropriate number of bytes based on field type
                        let size = field_info.ty.size();
                        if size == 1 {
                            emitter.emit_byte(*val as u8);
                        } else if size == 2 {
                            // Emit as little-endian u16
                            emitter.emit_byte((*val & 0xFF) as u8);
                            emitter.emit_byte(((*val >> 8) & 0xFF) as u8);
                        } else {
                            return Err(CodegenError::UnsupportedOperation(format!(
                                "struct field type with size {} not yet supported",
                                size
                            )));
                        }
                    }
                    crate::ast::Literal::Bool(b) => {
                        emitter.emit_byte(if *b { 1 } else { 0 });
                    }
                    _ => {
                        return Err(CodegenError::UnsupportedOperation(
                            "only integer and bool literals supported in struct initialization"
                                .to_string(),
                        ));
                    }
                }
            } else {
                return Err(CodegenError::UnsupportedOperation(
                    "only constant expressions supported in struct initialization".to_string(),
                ));
            }
        } else {
            // Field not provided - initialize to zero
            for _ in 0..field_info.ty.size() {
                emitter.emit_byte(0);
            }
        }
    }

    emitter.emit_label(&skip_label);

    // Load the address of the struct into A (low byte) and X (high byte)
    emitter.emit_inst("LDA", &format!("#<{}", struct_label));
    emitter.emit_inst("LDX", &format!("#>{}", struct_label));

    Ok(())
}

/// Generate runtime struct initialization directly to a zero page address.
/// This stores field values directly to ZP memory instead of creating ROM data.
/// Returns with A containing the base address (for chained operations).
pub fn generate_struct_init_runtime(
    struct_name: &str,
    fields: &[crate::ast::FieldInit],
    dest_addr: u8,
    emitter: &mut Emitter,
    info: &ProgramInfo,
    string_collector: &mut StringCollector,
) -> Result<(), CodegenError> {
    emitter.emit_comment(&format!(
        "Struct init (runtime): {} at ${:02X}",
        struct_name, dest_addr
    ));

    // Look up the struct definition
    let struct_def = info.type_registry.get_struct(struct_name).ok_or_else(|| {
        CodegenError::UnsupportedOperation(format!(
            "struct '{}' not found in type registry",
            struct_name
        ))
    })?;

    // Create a map of field values for quick lookup
    let field_values: std::collections::HashMap<String, &Spanned<crate::ast::Expr>> = fields
        .iter()
        .map(|f| (f.name.node.clone(), &f.value))
        .collect();

    // Initialize each field in order (respecting struct layout)
    for field_info in &struct_def.fields {
        let field_addr = dest_addr + field_info.offset as u8;

        if let Some(value_expr) = field_values.get(&field_info.name) {
            // Generate the field value expression
            generate_expr(value_expr, emitter, info, string_collector)?;

            // Store to field address
            let size = field_info.ty.size();
            if size == 1 {
                emitter.emit_inst("STA", &format!("${:02X}", field_addr));
            } else if size == 2 {
                // For u16: A has low byte, Y has high byte
                emitter.emit_inst("STA", &format!("${:02X}", field_addr));
                emitter.emit_inst("STY", &format!("${:02X}", field_addr + 1));
            } else {
                return Err(CodegenError::UnsupportedOperation(format!(
                    "struct field type with size {} not yet supported",
                    size
                )));
            }
        } else {
            // Field not provided - initialize to zero
            emitter.emit_inst("LDA", "#$00");
            for i in 0..field_info.ty.size() {
                emitter.emit_inst("STA", &format!("${:02X}", field_addr + i as u8));
            }
        }
    }

    // Return base address in A (for use in expressions)
    emitter.emit_inst("LDA", &format!("#${:02X}", dest_addr));

    Ok(())
}

pub(super) fn generate_field_access(
    object: &Spanned<crate::ast::Expr>,
    field: &Spanned<String>,
    emitter: &mut Emitter,
    info: &ProgramInfo,
) -> Result<(), CodegenError> {
    use crate::ast::Expr;

    // Get the base object (must be a variable for now)
    if let Expr::Variable(var_name) = &object.node {
        // Look up the variable using span from resolved_symbols, fallback to global table
        let sym = info
            .resolved_symbols
            .get(&object.span)
            .or_else(|| info.table.lookup(var_name));

        if let Some(sym) = sym {
            // Get the base address of the struct
            let base_addr = match sym.location {
                crate::sema::table::SymbolLocation::ZeroPage(addr) => addr as u16,
                crate::sema::table::SymbolLocation::Absolute(addr) => addr,
                _ => {
                    return Err(CodegenError::UnsupportedOperation(format!(
                        "Cannot access field of variable with location: {:?}",
                        sym.location
                    )));
                }
            };

            emitter.emit_comment(&format!("Field access: {}.{}", var_name, field.node));

            // Get the struct type name from the symbol's type
            let struct_name = if let crate::sema::types::Type::Named(name) = &sym.ty {
                name
            } else {
                return Err(CodegenError::UnsupportedOperation(format!(
                    "variable '{}' is not a struct type",
                    var_name
                )));
            };

            // Look up the struct definition
            let struct_def = info.type_registry.get_struct(struct_name).ok_or_else(|| {
                CodegenError::UnsupportedOperation(format!(
                    "struct '{}' not found in type registry",
                    struct_name
                ))
            })?;

            // Find the field and get its offset
            let field_info = struct_def.get_field(&field.node).ok_or_else(|| {
                CodegenError::UnsupportedOperation(format!(
                    "field '{}' not found in struct '{}'",
                    field.node, struct_name
                ))
            })?;

            // Check if this is a parameter (pass-by-reference)
            // Parameters are in the param region ($80-$BF)
            let param_base = emitter.memory_layout.param_base;
            let param_end = emitter.memory_layout.param_end;
            let is_parameter = base_addr >= param_base as u16 && base_addr <= param_end as u16;

            if is_parameter {
                // Check if this struct param has a local pointer copy
                // (prevents clobbering on nested calls)
                let local_ptr_addr = emitter
                    .current_function()
                    .and_then(|fn_name| info.function_metadata.get(fn_name))
                    .and_then(|meta| meta.struct_param_locals.get(var_name))
                    .copied();

                let ptr_addr = local_ptr_addr.unwrap_or(base_addr as u8);

                // Use indirect indexed addressing: LDA ($ptr),Y
                let offset = field_info.offset;
                emitter.emit_inst("LDY", &format!("#${:02X}", offset));
                emitter.emit_inst("LDA", &format!("(${:02X}),Y", ptr_addr));
            } else {
                // Local struct - direct access
                let field_addr = base_addr + field_info.offset as u16;
                if field_addr < 0x100 {
                    emitter.emit_inst("LDA", &format!("${:02X}", field_addr));
                } else {
                    emitter.emit_inst("LDA", &format!("${:04X}", field_addr));
                }
            }

            Ok(())
        } else {
            Err(CodegenError::SymbolNotFound(var_name.clone()))
        }
    } else {
        Err(CodegenError::UnsupportedOperation(
            "Field access only supported on variables (not expressions)".to_string(),
        ))
    }
}

pub(super) fn generate_enum_variant(
    enum_name: &Spanned<String>,
    variant: &Spanned<String>,
    data: &crate::ast::VariantData,
    emitter: &mut Emitter,
    info: &ProgramInfo,
    string_collector: &mut crate::codegen::StringCollector,
) -> Result<(), CodegenError> {
    emitter.emit_comment(&format!(
        "Enum variant: {}::{}",
        enum_name.node, variant.node
    ));

    // Look up the enum definition
    let enum_def = info
        .type_registry
        .get_enum(&enum_name.node)
        .ok_or_else(|| {
            CodegenError::UnsupportedOperation(format!(
                "enum '{}' not found in type registry",
                enum_name.node
            ))
        })?;

    // Find the variant
    let variant_info = enum_def.get_variant(&variant.node).ok_or_else(|| {
        CodegenError::UnsupportedOperation(format!(
            "variant '{}' not found in enum '{}'",
            variant.node, enum_name.node
        ))
    })?;

    // Check if all values are constant expressions
    // If so, we can use inline data (efficient, data in ROM)
    // Otherwise, we need runtime construction (data in temp storage)
    let all_constant = is_variant_data_constant(data);

    if all_constant {
        // Use inline data approach (original implementation)
        generate_enum_variant_inline(enum_name, variant, data, emitter, variant_info)
    } else {
        // Use runtime construction approach
        generate_enum_variant_runtime(
            enum_name,
            variant,
            data,
            emitter,
            info,
            variant_info,
            string_collector,
        )
    }
}

/// Check if all values in variant data are constant expressions
fn is_variant_data_constant(data: &crate::ast::VariantData) -> bool {
    match data {
        crate::ast::VariantData::Unit => true,
        crate::ast::VariantData::Tuple(values) => values.iter().all(|v| is_expr_constant(&v.node)),
        crate::ast::VariantData::Struct(fields) => {
            fields.iter().all(|f| is_expr_constant(&f.value.node))
        }
    }
}

/// Check if an expression is a compile-time constant
fn is_expr_constant(expr: &crate::ast::Expr) -> bool {
    matches!(
        expr,
        crate::ast::Expr::Literal(crate::ast::Literal::Integer(_))
            | crate::ast::Expr::Literal(crate::ast::Literal::Bool(_))
    )
}

/// Generate enum variant with inline data (for constant values)
fn generate_enum_variant_inline(
    _enum_name: &Spanned<String>,
    variant: &Spanned<String>,
    data: &crate::ast::VariantData,
    emitter: &mut Emitter,
    variant_info: &crate::sema::type_defs::VariantInfo,
) -> Result<(), CodegenError> {
    // Generate labels for enum data
    let enum_label = emitter.next_label("en");
    let skip_label = emitter.next_label("es");

    // Jump over the data
    emitter.emit_inst("JMP", &skip_label);

    // Emit enum data
    emitter.emit_label(&enum_label);

    // Emit discriminant tag
    emitter.emit_byte(variant_info.tag);

    // Emit variant data based on type
    match (&variant_info.data, data) {
        (crate::sema::type_defs::VariantData::Unit, crate::ast::VariantData::Unit) => {
            // Unit variant - just the tag, no data
        }
        (
            crate::sema::type_defs::VariantData::Tuple(field_types),
            crate::ast::VariantData::Tuple(values),
        ) => {
            if values.len() != field_types.len() {
                return Err(CodegenError::UnsupportedOperation(format!(
                    "variant '{}' expects {} fields, got {}",
                    variant.node,
                    field_types.len(),
                    values.len()
                )));
            }

            for (value_expr, field_type) in values.iter().zip(field_types.iter()) {
                emit_constant_value(&value_expr.node, field_type.size(), emitter)?;
            }
        }
        (
            crate::sema::type_defs::VariantData::Struct(field_infos),
            crate::ast::VariantData::Struct(field_inits),
        ) => {
            let field_values: std::collections::HashMap<String, &Spanned<crate::ast::Expr>> =
                field_inits
                    .iter()
                    .map(|f| (f.name.node.clone(), &f.value))
                    .collect();

            for field_info in field_infos {
                if let Some(value_expr) = field_values.get(&field_info.name) {
                    emit_constant_value(&value_expr.node, field_info.ty.size(), emitter)?;
                } else {
                    // Field not provided - initialize to zero
                    for _ in 0..field_info.ty.size() {
                        emitter.emit_byte(0);
                    }
                }
            }
        }
        _ => {
            return Err(CodegenError::UnsupportedOperation(format!(
                "variant data mismatch for '{}'",
                variant.node
            )));
        }
    }

    emitter.emit_label(&skip_label);

    // Load the address of the enum into A (low byte) and X (high byte)
    emitter.emit_inst("LDA", &format!("#<{}", enum_label));
    emitter.emit_inst("LDX", &format!("#>{}", enum_label));

    Ok(())
}

/// Emit a constant value as inline data bytes
fn emit_constant_value(
    expr: &crate::ast::Expr,
    size: usize,
    emitter: &mut Emitter,
) -> Result<(), CodegenError> {
    if let crate::ast::Expr::Literal(lit) = expr {
        match lit {
            crate::ast::Literal::Integer(val) => {
                if size == 1 {
                    emitter.emit_byte(*val as u8);
                } else if size == 2 {
                    emitter.emit_byte((*val & 0xFF) as u8);
                    emitter.emit_byte(((*val >> 8) & 0xFF) as u8);
                } else {
                    return Err(CodegenError::UnsupportedOperation(format!(
                        "field type with size {} not yet supported",
                        size
                    )));
                }
            }
            crate::ast::Literal::Bool(b) => {
                emitter.emit_byte(if *b { 1 } else { 0 });
            }
            _ => {
                return Err(CodegenError::UnsupportedOperation(
                    "only integer and bool literals supported in enum variant data".to_string(),
                ));
            }
        }
    }
    Ok(())
}

/// Generate enum variant with runtime construction (for non-constant values)
fn generate_enum_variant_runtime(
    _enum_name: &Spanned<String>,
    variant: &Spanned<String>,
    data: &crate::ast::VariantData,
    emitter: &mut Emitter,
    info: &ProgramInfo,
    variant_info: &crate::sema::type_defs::VariantInfo,
    string_collector: &mut crate::codegen::StringCollector,
) -> Result<(), CodegenError> {
    emitter.emit_comment("Runtime enum construction");

    // Calculate total size needed: 1 byte tag + field data
    let data_size = match &variant_info.data {
        crate::sema::type_defs::VariantData::Unit => 0,
        crate::sema::type_defs::VariantData::Tuple(types) => {
            types.iter().map(|t| t.size()).sum::<usize>()
        }
        crate::sema::type_defs::VariantData::Struct(fields) => {
            fields.iter().map(|f| f.ty.size()).sum::<usize>()
        }
    };
    let total_size = 1 + data_size; // tag + data

    // Allocate temp storage from primary pool ($20-$3F)
    let temp_base = emitter
        .temp_alloc
        .alloc_primary(total_size as u8)
        .ok_or_else(|| {
            CodegenError::UnsupportedOperation(
                "not enough temp storage for runtime enum construction".to_string(),
            )
        })?;

    // Write the tag byte
    emitter.emit_inst("LDA", &format!("#${:02X}", variant_info.tag));
    emitter.emit_inst("STA", &format!("${:02X}", temp_base));

    // Invalidate register state - the tag load clobbered A, so any subsequent
    // field value expressions must reload their values
    emitter.reg_state.invalidate_all();

    // Write field values
    let mut offset = 1u8; // Start after the tag byte

    match (&variant_info.data, data) {
        (crate::sema::type_defs::VariantData::Unit, crate::ast::VariantData::Unit) => {
            // Unit variant - no data to write
        }
        (
            crate::sema::type_defs::VariantData::Tuple(field_types),
            crate::ast::VariantData::Tuple(values),
        ) => {
            if values.len() != field_types.len() {
                return Err(CodegenError::UnsupportedOperation(format!(
                    "variant '{}' expects {} fields, got {}",
                    variant.node,
                    field_types.len(),
                    values.len()
                )));
            }

            for (value_expr, field_type) in values.iter().zip(field_types.iter()) {
                let field_size = field_type.size();
                let field_addr = temp_base + offset;

                // Generate code to evaluate the expression
                generate_expr(value_expr, emitter, info, string_collector)?;

                // Store the result to temp storage
                if field_size == 1 {
                    emitter.emit_inst("STA", &format!("${:02X}", field_addr));
                } else if field_size == 2 {
                    // 16-bit: A has low byte, Y has high byte
                    emitter.emit_inst("STA", &format!("${:02X}", field_addr));
                    emitter.emit_inst("STY", &format!("${:02X}", field_addr + 1));
                } else {
                    return Err(CodegenError::UnsupportedOperation(format!(
                        "field type with size {} not yet supported",
                        field_size
                    )));
                }

                offset += field_size as u8;
            }
        }
        (
            crate::sema::type_defs::VariantData::Struct(field_infos),
            crate::ast::VariantData::Struct(field_inits),
        ) => {
            let field_values: std::collections::HashMap<String, &Spanned<crate::ast::Expr>> =
                field_inits
                    .iter()
                    .map(|f| (f.name.node.clone(), &f.value))
                    .collect();

            for field_info in field_infos {
                let field_size = field_info.ty.size();
                let field_addr = temp_base + offset;

                if let Some(value_expr) = field_values.get(&field_info.name) {
                    // Generate code to evaluate the expression
                    generate_expr(value_expr, emitter, info, string_collector)?;

                    // Store the result to temp storage
                    if field_size == 1 {
                        emitter.emit_inst("STA", &format!("${:02X}", field_addr));
                    } else if field_size == 2 {
                        emitter.emit_inst("STA", &format!("${:02X}", field_addr));
                        emitter.emit_inst("STY", &format!("${:02X}", field_addr + 1));
                    } else {
                        return Err(CodegenError::UnsupportedOperation(format!(
                            "field type with size {} not yet supported",
                            field_size
                        )));
                    }
                } else {
                    // Field not provided - initialize to zero
                    emitter.emit_inst("LDA", "#$00");
                    for i in 0..field_size {
                        emitter.emit_inst("STA", &format!("${:02X}", field_addr + i as u8));
                    }
                }

                offset += field_size as u8;
            }
        }
        _ => {
            return Err(CodegenError::UnsupportedOperation(format!(
                "variant data mismatch for '{}'",
                variant.node
            )));
        }
    }

    // Load the address of the temp storage into A (low byte) and X (high byte)
    // Since temp storage is in zero page, high byte is always 0
    emitter.emit_inst("LDA", &format!("#${:02X}", temp_base));
    emitter.emit_inst("LDX", "#$00");

    // Note: We don't free the temp storage here because the caller needs
    // to use the pointer. The temp allocator will be reset at function boundaries.

    Ok(())
}
