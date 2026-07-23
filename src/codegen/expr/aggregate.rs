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

/// Byte size of a type, resolving `Named` struct/enum types via the registry
/// (unlike `Type::size()`, which returns 0 for `Named`).
pub(super) fn type_byte_size(ty: &crate::sema::types::Type, info: &ProgramInfo) -> usize {
    use crate::sema::types::Type;
    match ty {
        Type::Primitive(p) => p.size_bytes(),
        Type::Array(el, n) => type_byte_size(el, info) * n,
        Type::Named(name) => info
            .type_registry
            .get_struct(name)
            .map(|s| s.total_size)
            .or_else(|| info.type_registry.get_enum(name).map(|e| e.total_size))
            .unwrap_or(0),
        other => other.size(),
    }
}

/// Emit an indirect-indexed array element load through a zero-page pointer at
/// `ptr`. For u8 elements this is `LDA (ptr),Y`; for u16 elements the index is
/// scaled ×2 and both bytes are loaded, ending with the low byte in A and the
/// high byte in Y (the compiler's u16 register convention). Mirrors the store
/// path in `stmt::generate_index_assignment`.
fn emit_indexed_load(
    emitter: &mut Emitter,
    info: &ProgramInfo,
    object: &Spanned<Expr>,
    index: &Spanned<Expr>,
    ptr: u8,
    string_collector: &mut StringCollector,
) -> Result<(), CodegenError> {
    use crate::ast::PrimitiveType;
    use crate::sema::types::Type;

    // Determine whether the element type occupies two bytes.
    let is_multibyte = matches!(
        info.resolved_types.get(&object.span),
        Some(Type::Array(elem, _)) if matches!(
            &**elem,
            Type::Primitive(PrimitiveType::U16)
                | Type::Primitive(PrimitiveType::I16)
                | Type::Primitive(PrimitiveType::B16)
        )
    );

    // Index expression -> A.
    generate_expr(index, emitter, info, string_collector)?;

    if is_multibyte {
        emitter.emit_comment("Scale index for u16 array (multiply by 2)");
        emitter.emit_inst("ASL", "A"); // index * 2
        emitter.emit_inst("TAY", "");
        emitter.emit_inst("LDA", &format!("(${:02X}),Y", ptr)); // low byte
        emitter.emit_inst("PHA", ""); // stash low byte
        emitter.emit_inst("INY", "");
        emitter.emit_inst("LDA", &format!("(${:02X}),Y", ptr)); // high byte
        emitter.emit_inst("TAY", ""); // Y = high byte
        emitter.emit_inst("PLA", ""); // A = low byte
    } else {
        emitter.emit_inst("TAY", "");
        emitter.emit_inst("LDA", &format!("(${:02X}),Y", ptr));
    }
    emitter.reg_state.modify_a();
    Ok(())
}

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
                crate::sema::table::SymbolLocation::FrameOffset(_) => Err(CodegenError::Internal(
                    "unresolved FrameOffset reached codegen (frame finalization skipped)"
                        .to_string(),
                )),
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

                    emit_indexed_load(emitter, info, object, index, addr as u8, string_collector)
                }
                crate::sema::table::SymbolLocation::ZeroPage(addr) => {
                    // Array in zero page - use indirect indexed addressing
                    emit_indexed_load(emitter, info, object, index, addr, string_collector)
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

    // Clone the field layout so the type_registry borrow is released before the
    // recursive calls below (which also borrow `info`).
    let field_layout: Vec<crate::sema::type_defs::FieldInfo> = struct_def.fields.clone();

    // Initialize each field in order (respecting struct layout)
    for field_info in &field_layout {
        let field_addr = dest_addr + field_info.offset as u8;
        let field_size = type_byte_size(&field_info.ty, info);

        if let Some(value_expr) = field_values.get(&field_info.name) {
            // A struct-typed field holds an inline nested struct: recurse into
            // its literal, writing directly at this field's address.
            if let crate::sema::types::Type::Named(inner) = &field_info.ty
                && info.type_registry.get_struct(inner).is_some()
            {
                let inner_fields = match &value_expr.node {
                    crate::ast::Expr::StructInit { fields, .. }
                    | crate::ast::Expr::AnonStructInit { fields } => fields,
                    _ => {
                        return Err(CodegenError::UnsupportedOperation(format!(
                            "nested struct field '{}' must be initialized with a struct literal",
                            field_info.name
                        )));
                    }
                };
                generate_struct_init_runtime(
                    inner,
                    inner_fields,
                    field_addr,
                    emitter,
                    info,
                    string_collector,
                )?;
                continue;
            }

            // Generate the scalar field value expression.
            generate_expr(value_expr, emitter, info, string_collector)?;

            // Store to field address
            if field_size == 1 {
                emitter.emit_inst("STA", &format!("${:02X}", field_addr));
            } else if field_size == 2 {
                // For u16: A has low byte, Y has high byte
                emitter.emit_inst("STA", &format!("${:02X}", field_addr));
                emitter.emit_inst("STY", &format!("${:02X}", field_addr + 1));
            } else {
                return Err(CodegenError::UnsupportedOperation(format!(
                    "struct field type with size {} not yet supported",
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
    }

    // Return base address in A (for use in expressions)
    emitter.emit_inst("LDA", &format!("#${:02X}", dest_addr));

    Ok(())
}

/// Resolve a chain of *local* (inline, non-pointer) struct accesses to a static
/// `(base_address, struct_type_name)`. Handles `Variable` and nested `Field`
/// objects (and constant array indices). Returns `None` when any link is not
/// statically addressable — a by-reference struct parameter, a runtime array
/// index, or a non-struct — so callers can fall back to another path.
fn resolve_static_struct_lvalue(
    expr: &Spanned<crate::ast::Expr>,
    info: &ProgramInfo,
) -> Option<(u16, String)> {
    use crate::ast::Expr;
    use crate::sema::table::SymbolLocation;
    use crate::sema::types::Type;

    match &expr.node {
        Expr::Variable(name) => {
            let sym = info
                .resolved_symbols
                .get(&expr.span)
                .or_else(|| info.table.lookup(name))?;
            if sym.is_param {
                return None; // parameter slot holds a pointer, not the struct
            }
            let addr = match sym.location {
                SymbolLocation::ZeroPage(a) => a as u16,
                SymbolLocation::Absolute(a) => a,
                _ => return None,
            };
            match &sym.ty {
                Type::Named(n) => Some((addr, n.clone())),
                _ => None,
            }
        }
        Expr::Field { object, field } => {
            let (base, struct_name) = resolve_static_struct_lvalue(object, info)?;
            let sdef = info.type_registry.get_struct(&struct_name)?;
            let finfo = sdef.get_field(&field.node)?;
            match &finfo.ty {
                Type::Named(inner) => Some((base + finfo.offset as u16, inner.clone())),
                _ => None, // scalar field: not a further struct
            }
        }
        Expr::Index { object, index } => {
            // Array element as struct: only a constant index is statically
            // addressable here (runtime indices need pointer arithmetic).
            let (base, elem_struct) = array_of_struct_base(object, info)?;
            let idx = const_index(index, info)?;
            let sdef = info.type_registry.get_struct(&elem_struct)?;
            Some((base + (idx * sdef.total_size) as u16, elem_struct))
        }
        _ => None,
    }
}

/// If `expr` is a local array-of-struct variable, return its base address and
/// the element struct's type name.
fn array_of_struct_base(
    expr: &Spanned<crate::ast::Expr>,
    info: &ProgramInfo,
) -> Option<(u16, String)> {
    use crate::ast::Expr;
    use crate::sema::table::SymbolLocation;
    use crate::sema::types::Type;

    let Expr::Variable(name) = &expr.node else {
        return None;
    };
    let sym = info
        .resolved_symbols
        .get(&expr.span)
        .or_else(|| info.table.lookup(name))?;
    let addr = match sym.location {
        SymbolLocation::ZeroPage(a) => a as u16,
        SymbolLocation::Absolute(a) => a,
        _ => return None,
    };
    match &sym.ty {
        Type::Array(elem, _) => match &**elem {
            Type::Named(n) => Some((addr, n.clone())),
            _ => None,
        },
        _ => None,
    }
}

/// Evaluate an index expression as a compile-time constant, if possible.
fn const_index(expr: &Spanned<crate::ast::Expr>, info: &ProgramInfo) -> Option<usize> {
    use crate::ast::{Expr, Literal};
    if let Expr::Literal(Literal::Integer(n)) = &expr.node {
        return usize::try_from(*n).ok();
    }
    if let Some(crate::sema::const_eval::ConstValue::Integer(n)) =
        info.folded_constants.get(&expr.span)
    {
        return usize::try_from(*n).ok();
    }
    None
}

/// Emit a load of `base.field` (a scalar struct field at a static address) into
/// A (low) / Y (high for u16). Shared by the local single-level and nested paths.
fn emit_static_field_load(
    base_addr: u16,
    struct_name: &str,
    field: &Spanned<String>,
    emitter: &mut Emitter,
    info: &ProgramInfo,
) -> Result<(), CodegenError> {
    let sdef = info.type_registry.get_struct(struct_name).ok_or_else(|| {
        CodegenError::UnsupportedOperation(format!(
            "struct '{}' not found in type registry",
            struct_name
        ))
    })?;
    let finfo = sdef.get_field(&field.node).ok_or_else(|| {
        CodegenError::UnsupportedOperation(format!(
            "field '{}' not found in struct '{}'",
            field.node, struct_name
        ))
    })?;
    let is_multibyte = matches!(
        &finfo.ty,
        crate::sema::types::Type::Primitive(
            crate::ast::PrimitiveType::U16
                | crate::ast::PrimitiveType::I16
                | crate::ast::PrimitiveType::B16
        )
    );
    let field_addr = base_addr + finfo.offset as u16;
    if field_addr < 0x100 {
        emitter.emit_inst("LDA", &format!("${:02X}", field_addr));
        if is_multibyte {
            emitter.emit_inst("LDY", &format!("${:02X}", field_addr + 1));
        }
    } else {
        emitter.emit_inst("LDA", &format!("${:04X}", field_addr));
        if is_multibyte {
            emitter.emit_inst("LDY", &format!("${:04X}", field_addr + 1));
        }
    }
    emitter.mark_a_unknown();
    Ok(())
}

pub(super) fn generate_field_access(
    object: &Spanned<crate::ast::Expr>,
    field: &Spanned<String>,
    emitter: &mut Emitter,
    info: &ProgramInfo,
) -> Result<(), CodegenError> {
    use crate::ast::Expr;

    // Nested field access (a.b.c) or array-of-struct element (arr[i].f): the
    // object is not a plain variable. Resolve a static address for local chains.
    if !matches!(&object.node, Expr::Variable(_)) {
        if let Some((base, struct_name)) = resolve_static_struct_lvalue(object, info) {
            emitter.emit_comment(&format!("Nested field access: .{}", field.node));
            return emit_static_field_load(base, &struct_name, field, emitter, info);
        }
        return Err(CodegenError::UnsupportedOperation(
            "Field access only supported on variables and local struct/array chains".to_string(),
        ));
    }

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
            // Parameters are pass-by-reference (the slot holds a pointer); locals
            // hold the struct inline. Distinguish via the explicit is_param flag
            // (frames put params and locals in the same region, so an address
            // range test no longer works).
            let sym_is_param = sym.is_param;

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
            let is_parameter = sym_is_param;

            // Multi-byte fields (u16/i16/b16) must load both bytes, ending with
            // the low byte in A and the high byte in Y (the u16 register
            // convention). A single LDA would truncate to the low byte.
            let is_multibyte = matches!(
                &field_info.ty,
                crate::sema::types::Type::Primitive(
                    crate::ast::PrimitiveType::U16
                        | crate::ast::PrimitiveType::I16
                        | crate::ast::PrimitiveType::B16
                )
            );

            if is_parameter {
                // The struct pointer lives directly in this parameter's frame slot;
                // frame coloring guarantees nested calls cannot clobber it.
                let ptr_addr = base_addr as u8;

                // Use indirect indexed addressing: LDA ($ptr),Y
                let offset = field_info.offset;
                emitter.emit_inst("LDY", &format!("#${:02X}", offset));
                emitter.emit_inst("LDA", &format!("(${:02X}),Y", ptr_addr));
                if is_multibyte {
                    emitter.emit_inst("PHA", ""); // stash low byte
                    emitter.emit_inst("INY", ""); // next field byte
                    emitter.emit_inst("LDA", &format!("(${:02X}),Y", ptr_addr));
                    emitter.emit_inst("TAY", ""); // Y = high byte
                    emitter.emit_inst("PLA", ""); // A = low byte
                }
            } else {
                // Local struct - direct access
                let field_addr = base_addr + field_info.offset as u16;
                if field_addr < 0x100 {
                    emitter.emit_inst("LDA", &format!("${:02X}", field_addr));
                    if is_multibyte {
                        emitter.emit_inst("LDY", &format!("${:02X}", field_addr + 1));
                    }
                } else {
                    emitter.emit_inst("LDA", &format!("${:04X}", field_addr));
                    if is_multibyte {
                        emitter.emit_inst("LDY", &format!("${:04X}", field_addr + 1));
                    }
                }
            }
            emitter.mark_a_unknown();

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
