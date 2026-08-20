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
pub(crate) fn type_byte_size(ty: &crate::sema::types::Type, info: &ProgramInfo) -> usize {
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

/// Reject a *runtime* index into an array whose largest byte offset would
/// exceed the 8-bit index register.
///
/// Indexed addressing (`base,Y` / `(zp),Y`) reaches at most `base+255`, and the
/// pre-index `ASL` that scales a two-byte element drops its carry — so a runtime
/// index into, say, `[u16; 200]` silently reads the wrong element. A
/// compile-time-constant index is exempt: it is a fixed access whose offset is
/// known, not scaled through the 8-bit register at runtime.
pub(crate) fn check_runtime_index_range(
    elem_size: usize,
    len: usize,
    index: &Spanned<Expr>,
    info: &ProgramInfo,
) -> Result<(), CodegenError> {
    let max_offset = len.saturating_sub(1).saturating_mul(elem_size);
    if max_offset > 0xFF
        && crate::sema::const_eval::eval_const_expr(index).is_err()
        && !info.folded_constants.contains_key(&index.span)
    {
        let max_elems = 256usize.checked_div(elem_size).unwrap_or(len);
        return Err(CodegenError::UnsupportedOperation(format!(
            "array of {len} elements of {elem_size} bytes is too large to index by a \
             runtime value: the scaled offset would exceed the 8-bit index register \
             (at most {max_elems} elements are runtime-indexable). Use a \
             compile-time-constant index or a smaller array."
        )));
    }
    Ok(())
}

/// Load a value through a zero-page pointer at `ptr`, `offset` bytes in.
///
/// `LDY #offset / LDA (ptr),Y`, plus the `PHA/INY/LDA/TA?/PLA` dance for a
/// two-byte value so it ends up in the right register pair — A:Y for a 16-bit
/// scalar, A:X for an address, which is what `high_in_x` selects.
///
/// This is the one sequence that had been written out four times: for the enum
/// discriminant, for the slice descriptor copy, for `*p`, and for `p.field`
/// through a by-reference parameter. The last of those kept its own copy long
/// enough for the two to disagree — `*p` decided "two bytes" by re-listing
/// `u16`/`i16`/`b16` and so read one byte of a `&&u8`, which is the same
/// omission that had already been fixed on the store side.
pub(crate) fn emit_deref_load(
    emitter: &mut Emitter,
    ptr: u8,
    offset: u8,
    is_multibyte: bool,
    high_in_x: bool,
) {
    emitter.emit_inst("LDY", &format!("#${:02X}", offset));
    emitter.emit_inst("LDA", &format!("(${:02X}),Y", ptr));
    if is_multibyte {
        emitter.emit_inst("PHA", "");
        emitter.emit_inst("INY", "");
        emitter.emit_inst("LDA", &format!("(${:02X}),Y", ptr));
        emitter.emit_inst(if high_in_x { "TAX" } else { "TAY" }, "");
        emitter.emit_inst("PLA", "");
    }
    emitter.reg_state.modify_a();
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
    use crate::sema::types::Type;

    // Determine whether the element type occupies two bytes (arrays, slices and
    // pointers share the indirect-indexed load path; each of their slots holds
    // a base pointer rather than the data). `p[i]` on a pointer is the i-th
    // element from it, scaled by the element width — a pointer carries no
    // length, so there is nothing to bounds-check.
    let elem_ty = match info.resolved_types.get(&object.span) {
        Some(Type::Array(elem, _)) | Some(Type::Slice(elem)) | Some(Type::Pointer(elem)) => {
            Some(&**elem)
        }
        _ => None,
    };
    // Two-byte elements (u16/i16/b16, a function pointer, or a `&T`) are scaled
    // by the element width and loaded into A:Y — including a local
    // function-pointer table (`let handlers = [d0, d1]; handlers[i](x)`).
    let is_multibyte = elem_ty.is_some_and(is_two_byte_value);

    // A fixed-size array whose scaled offset would exceed the 8-bit index is
    // not runtime-indexable (a local array is frame-bounded well under this, but
    // the invariant holds wherever this path is reached). Slices and pointers
    // carry no length, so there is nothing to check.
    if let Some(Type::Array(elem, len)) = info.resolved_types.get(&object.span) {
        check_runtime_index_range(type_byte_size(elem, info), *len, index, info)?;
    }

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

/// The base address and element type of an array reached through struct fields
/// (`s.a`, `s.inner.a`), or `None` when it is not statically addressable.
pub(crate) fn array_field_base(
    object: &Spanned<Expr>,
    info: &ProgramInfo,
) -> Option<(StaticBase, crate::sema::types::Type)> {
    use crate::sema::types::Type;
    // Only a field chain; a bare variable is handled by the paths above.
    if !matches!(object.node, Expr::Field { .. }) {
        return None;
    }
    match resolve_static_addr(object, info)? {
        (base, Type::Array(elem, _)) => Some((base, *elem)),
        _ => None,
    }
}

/// Load `base[index]` where `base` is a fixed address: absolute indexed for a
/// runtime index, a plain load when the index folds to a constant.
fn emit_static_indexed_load(
    base: &StaticBase,
    elem_ty: &crate::sema::types::Type,
    index: &Spanned<Expr>,
    emitter: &mut Emitter,
    info: &ProgramInfo,
    string_collector: &mut StringCollector,
) -> Result<(), CodegenError> {
    let esize = type_byte_size(elem_ty, info).max(1);
    let is_multibyte = is_two_byte_value(elem_ty);

    // A constant index needs no index register at all.
    if let Some(idx) = const_index(index, info) {
        let at = base.plus((idx * esize) as u16);
        emitter.emit_inst("LDA", &at.operand(0));
        if is_multibyte {
            let hi = if high_byte_in_x(elem_ty) {
                "LDX"
            } else {
                "LDY"
            };
            emitter.emit_inst(hi, &at.operand(1));
        }
        emitter.reg_state.modify_a();
        return Ok(());
    }

    generate_expr(index, emitter, info, string_collector)?;
    emit_scale_index(emitter, esize);
    emitter.emit_inst("TAY", "");
    emitter.emit_inst("LDA", &format!("{},Y", base.operand_abs(0)));
    if is_multibyte {
        emitter.emit_inst("PHA", "");
        emitter.emit_inst("LDA", &format!("{},Y", base.operand_abs(1)));
        let hi = if high_byte_in_x(elem_ty) {
            "TAX"
        } else {
            "TAY"
        };
        emitter.emit_inst(hi, "");
        emitter.emit_inst("PLA", "");
    }
    emitter.reg_state.modify_a();
    Ok(())
}

/// Emit the address of `object[index]` into A:X — the base of whatever
/// `object` names, plus the index scaled by the element's width.
///
/// The index is evaluated *first* and parked, because both halves want A and
/// the base is the one that ends up in a register pair. Evaluating it first
/// also means it is evaluated exactly once: the load and the store paths both
/// come through here, and the store path used to have the index in `Y` before
/// it knew whether it could reach the base at all.
fn emit_element_address(
    object: &Spanned<Expr>,
    index: &Spanned<Expr>,
    emitter: &mut Emitter,
    info: &ProgramInfo,
    string_collector: &mut StringCollector,
) -> Result<Option<crate::sema::types::Type>, CodegenError> {
    use crate::sema::types::Type;

    let esize = match info.resolved_types.get(&object.span) {
        Some(Type::Array(elem, _)) | Some(Type::Slice(elem)) | Some(Type::Pointer(elem)) => {
            type_byte_size(elem, info).max(1)
        }
        _ => 1,
    };
    let parked = emitter.temp_alloc.alloc_high(1).ok_or_else(|| {
        CodegenError::Internal("temporary storage exhausted in an element address".into())
    })?;
    generate_expr(index, emitter, info, string_collector)?;
    emit_scale_index(emitter, esize);
    emitter.emit_inst("STA", &format!("${:02X}", parked));

    let base = emit_aggregate_base(object, emitter, info, string_collector);
    let elem = match base {
        Ok(Some(Type::Array(elem, _)))
        | Ok(Some(Type::Slice(elem)))
        | Ok(Some(Type::Pointer(elem))) => (*elem).clone(),
        Ok(_) => {
            emitter.temp_alloc.free_high(parked, 1);
            return Ok(None);
        }
        Err(e) => {
            emitter.temp_alloc.free_high(parked, 1);
            return Err(e);
        }
    };
    emit_add_zp_to_ax(emitter, parked);
    emitter.temp_alloc.free_high(parked, 1);
    Ok(Some(elem))
}

/// [`emit_element_address`], left in the shared deref pointer instead of A:X,
/// which is where `(zp),Y` needs it for the load or store that follows.
pub(crate) fn emit_element_address_into_ptr(
    object: &Spanned<Expr>,
    index: &Spanned<Expr>,
    emitter: &mut Emitter,
    info: &ProgramInfo,
    string_collector: &mut StringCollector,
) -> Result<Option<(u8, crate::sema::types::Type)>, CodegenError> {
    let Some(elem) = emit_element_address(object, index, emitter, info, string_collector)? else {
        return Ok(None);
    };
    let ptr = emitter.memory_layout.deref_ptr();
    emitter.emit_inst("STA", &format!("${:02X}", ptr));
    emitter.emit_inst("STX", &format!("${:02X}", ptr + 1));
    Ok(Some((ptr, elem)))
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

        // The pointer is staged across the index expression, which is
        // arbitrary code: it must come from the allocator so a nested user
        // (`.len`, u8 multiply, an enclosing index assignment's parked value)
        // gets different bytes. The old hardcoded $F0/$F1 collided with all
        // three.
        let stage = emitter.temp_alloc.alloc_high(2).ok_or_else(|| {
            CodegenError::Internal("temporary storage exhausted in string index".to_string())
        })?;

        // Get string pointer
        generate_expr(object, emitter, info, string_collector)?;
        emitter.emit_inst("STA", &format!("${:02X}", stage));
        emitter.emit_inst("STX", &format!("${:02X}", stage + 1));

        // Skip length prefix (add 1 to pointer for u8 length)
        if emitter.is_verbose() {
            emitter.emit_comment("Add 1 to pointer to skip u8 length prefix");
        }
        let skip_label = emitter.next_label("si");
        emitter.emit_inst("INC", &format!("${:02X}", stage));
        emitter.emit_inst("BNE", &skip_label);
        emitter.emit_inst("INC", &format!("${:02X}", stage + 1));
        emitter.emit_label(&skip_label);

        // Get index in Y
        generate_expr(index, emitter, info, string_collector)?;
        emitter.emit_inst("TAY", "");

        // Load byte
        emitter.emit_inst("LDA", &format!("(${:02X}),Y", stage));
        emitter.reg_state.modify_a();
        emitter.temp_alloc.free_high(stage, 2);

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

            // A global array lives inline at its own label — a `const` lookup
            // table in ROM, or a mutable `static` in RAM. Both are indexed with
            // absolute-indexed addressing through the label, not the
            // pointer-indirect path used for local (zero-page) arrays, whose
            // slot holds a pointer rather than the data.
            let is_global_inline_array = matches!(sym.ty, crate::sema::types::Type::Array(..))
                && (sym.kind == crate::sema::table::SymbolKind::Constant
                    || (sym.containing_function.is_none()
                        && matches!(
                            sym.location,
                            crate::sema::table::SymbolLocation::Absolute(_)
                        )));
            if is_global_inline_array {
                // A two-byte element (u16/i16/b16, a function pointer, or a
                // `&T`) is indexed with a scaled offset and loaded into A:Y.
                // Function pointers are what a `static` driver-dispatch table
                // (`handlers[i](x)`) is built from; omitting them here loaded a
                // single byte and left the index in Y as the "high byte".
                let is_multibyte = matches!(
                    &sym.ty,
                    crate::sema::types::Type::Array(elem, _) if is_two_byte_value(elem)
                );
                if let crate::sema::types::Type::Array(elem, len) = &sym.ty {
                    check_runtime_index_range(type_byte_size(elem, info), *len, index, info)?;
                }
                generate_expr(index, emitter, info, string_collector)?;
                if is_multibyte {
                    emitter.emit_inst("ASL", "A"); // scale index x2 for u16 elements
                }
                emitter.emit_inst("TAY", "");
                emitter.emit_inst("LDA", &format!("{},Y", name));
                if is_multibyte {
                    emitter.emit_inst("PHA", "");
                    emitter.emit_inst("LDA", &format!("{}+1,Y", name));
                    emitter.emit_inst("TAY", ""); // Y = high byte
                    emitter.emit_inst("PLA", ""); // A = low byte
                }
                emitter.reg_state.modify_a();
                return Ok(());
            }

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
            // An array reached through a field — `s.a[i]`, `s.inner.a[i]` — is
            // laid out inline in its owner, so it has a static base address
            // even though the object is not a bare variable.
            if let Some((base, elem_ty)) = array_field_base(object, info) {
                return emit_static_indexed_load(
                    &base,
                    &elem_ty,
                    index,
                    emitter,
                    info,
                    string_collector,
                );
            }

            // Otherwise the array's address is only known at run time —
            // `p.a[i]` through a pointer, an element of a nested array — so
            // compute it and read through it.
            if let Some((ptr, elem)) =
                emit_element_address_into_ptr(object, index, emitter, info, string_collector)?
            {
                emit_deref_load(
                    emitter,
                    ptr,
                    0,
                    is_two_byte_value(&elem),
                    high_byte_in_x(&elem),
                );
                return Ok(());
            }

            Err(CodegenError::UnsupportedOperation(
                "only an array reached through a name, a field or a pointer can be indexed"
                    .to_string(),
            ))
        }
    }
}

/// A struct literal in expression position — a `return`, a call argument,
/// anything without a destination of its own.
///
/// When every field is a constant the struct is emitted as bytes in the CODE
/// section and the expression evaluates to a pointer at them, which costs no
/// RAM and no cycles. When a field has to be *computed* there are no bytes
/// until the program runs, so sema reserved a block of writable RAM for this
/// literal site (`ProgramInfo::struct_temps`) and the fields are assembled into
/// it at run time. Either way the result is the same pointer convention: low
/// byte in A, high byte in X.
/// Emit one field of a *constant* struct as ROM data.
///
/// Split out of `generate_struct_init` because a field is not always a scalar:
/// an array field holds its elements inline, so it contributes `len * size`
/// bytes rather than `size`, and it is the one field kind that recurses. The
/// runtime path has always handled arrays; this one refused them with "only
/// integer and bool literals supported", which named the wrong thing — an
/// array literal *is* a literal — and made `return S { a: [1, 2, 3] }` fail
/// where the same struct bound to a local compiled.
fn emit_const_field(
    ty: &crate::sema::types::Type,
    value: &Spanned<Expr>,
    field_name: &str,
    emitter: &mut Emitter,
) -> Result<(), CodegenError> {
    use crate::ast::Literal;
    use crate::sema::types::Type;

    // An array field: its elements, then zero for any the literal leaves out.
    // Only the literal *forms* are read here, because the elements are what
    // has to be walked; each element is then a constant expression like any
    // scalar field.
    if let Type::Array(elem, len) = ty {
        let elems: Vec<&Spanned<Expr>> = match &value.node {
            Expr::Literal(Literal::Array(es)) => es.iter().collect(),
            Expr::Literal(Literal::ArrayFill { value, count }) => vec![&**value; *count],
            _ => {
                return Err(CodegenError::UnsupportedOperation(format!(
                    "field `{field_name}` is an array, so it needs an array literal"
                )));
            }
        };
        if elems.len() > *len {
            return Err(CodegenError::UnsupportedOperation(format!(
                "field `{field_name}` holds {len} elements but {} were given",
                elems.len()
            )));
        }
        for e in &elems {
            emit_const_field(elem, e, field_name, emitter)?;
        }
        for _ in elems.len()..*len {
            for _ in 0..elem.size() {
                emitter.emit_byte(0);
            }
        }
        return Ok(());
    }

    // A scalar field is whatever the constant evaluator can fold, not only a
    // bare literal token. Matching on `Expr::Literal` here made this path
    // disagree with sema's: sema decides a struct literal is *constant* — and
    // so gets no run-time block — by folding it, so `f0: (-1)` is constant
    // there and a unary minus here, and the struct compiled only when every
    // field happened to be a non-negative number.
    let val = crate::sema::const_eval::eval_const_expr(value)
        .ok()
        .and_then(|v| v.as_integer())
        .ok_or_else(|| {
            CodegenError::UnsupportedOperation(format!(
                "field `{field_name}` of a constant struct must be a constant expression: it \
                 becomes ROM data, so there is nothing to compute it with"
            ))
        })?;

    match ty.size() {
        1 => emitter.emit_byte(val as u8),
        2 => {
            emitter.emit_byte((val & 0xFF) as u8);
            emitter.emit_byte(((val >> 8) & 0xFF) as u8);
        }
        n => {
            return Err(CodegenError::UnsupportedOperation(format!(
                "field `{field_name}` is {n} bytes wide, which a constant struct cannot \
                 initialise from a number"
            )));
        }
    }
    Ok(())
}

pub(super) fn generate_struct_init(
    name: &Spanned<String>,
    fields: &[crate::ast::FieldInit],
    span: crate::ast::Span,
    emitter: &mut Emitter,
    info: &ProgramInfo,
    string_collector: &mut StringCollector,
) -> Result<(), CodegenError> {
    // A computed literal is built in RAM. Sema reserves the block for exactly
    // the literals that need one, so its presence is the signal.
    if let Some(block) = info.struct_temps.get(&span) {
        return generate_struct_init_runtime(
            &name.node,
            fields,
            block.addr,
            emitter,
            info,
            string_collector,
        );
    }

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

    // Initialize each field in order (respecting struct layout).
    for field_info in &struct_def.fields {
        match field_values.get(&field_info.name) {
            Some(value_expr) => {
                emit_const_field(&field_info.ty, value_expr, &field_info.name, emitter)?;
            }
            // Field not provided - initialize to zero.
            None => {
                for _ in 0..field_info.ty.size() {
                    emitter.emit_byte(0);
                }
            }
        }
    }

    emitter.emit_label(&skip_label);

    // Load the address of the struct into A (low byte) and X (high byte)
    emitter.emit_inst("LDA", &format!("#<{}", struct_label));
    emitter.emit_inst("LDX", &format!("#>{}", struct_label));

    Ok(())
}

/// Generate runtime struct initialization directly to a writable address.
/// This stores field values to RAM instead of creating ROM data.
///
/// `dest_addr` is a full 16-bit address: a zero-page frame slot when a variable
/// is being initialized in place, or a block in the call-graph-colored struct
/// scratch region when the literal appears in expression position (a `return`,
/// an argument) and so has no destination of its own.
pub fn generate_struct_init_runtime(
    struct_name: &str,
    fields: &[crate::ast::FieldInit],
    dest_addr: u16,
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
        let field_addr = dest_addr + field_info.offset as u16;
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

            // An array field of scalars is laid out inline, so its bytes are
            // written element by element at the field's address — the same job
            // a local array declaration does, at a different address.
            //
            // Without this an array field fell through to the scalar path,
            // where a two-byte one was stored as though it *were* a two-byte
            // value: the array literal evaluated to a pointer at its data and
            // the field ended up holding that pointer (with a garbage high
            // byte, since the scalar path reads Y while an address arrives in
            // X). Anything wider was rejected outright as "struct field type
            // with size N not yet supported".
            if let crate::sema::types::Type::Array(elem, n) = &field_info.ty
                && !matches!(&**elem, crate::sema::types::Type::Named(t)
                    if info.type_registry.get_struct(t).is_some())
            {
                let esize = type_byte_size(elem, info).max(1);
                crate::codegen::stmt::assign::generate_local_array_init(
                    field_addr,
                    (esize * n) as u16,
                    esize,
                    value_expr,
                    emitter,
                    info,
                )?;
                continue;
            }

            // An array-of-struct field holds inline elements: initialize each
            // struct-literal element at field_addr + j*element_size.
            if let crate::sema::types::Type::Array(elem, _) = &field_info.ty
                && let crate::sema::types::Type::Named(elem_struct) = &**elem
                && info.type_registry.get_struct(elem_struct).is_some()
                && let crate::ast::Expr::Literal(crate::ast::Literal::Array(elements)) =
                    &value_expr.node
            {
                let esize = type_byte_size(elem, info) as u8;
                for (j, el) in elements.iter().enumerate() {
                    let el_fields = match &el.node {
                        crate::ast::Expr::StructInit { fields, .. }
                        | crate::ast::Expr::AnonStructInit { fields } => fields,
                        _ => {
                            return Err(CodegenError::UnsupportedOperation(
                                "array-of-struct field elements must be struct literals"
                                    .to_string(),
                            ));
                        }
                    };
                    generate_struct_init_runtime(
                        elem_struct,
                        el_fields,
                        field_addr + (j as u16) * esize as u16,
                        emitter,
                        info,
                        string_collector,
                    )?;
                }
                continue;
            }

            // Generate the scalar field value expression.
            generate_expr(value_expr, emitter, info, string_collector)?;

            // Store to field address
            if field_size == 1 {
                emitter.emit_inst("STA", &addr_operand(field_addr));
            } else if field_size == 2 {
                // A holds the low byte; the high byte is in Y for a u16 or a
                // function pointer, but in X for a `&T` — the pointer
                // convention every other pointer-like value uses.
                emitter.emit_inst("STA", &addr_operand(field_addr));
                let hi = if high_byte_in_x(&field_info.ty) {
                    "STX"
                } else {
                    "STY"
                };
                emitter.emit_inst(hi, &addr_operand(field_addr + 1));
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
                emitter.emit_inst("STA", &addr_operand(field_addr + i as u16));
            }
        }
    }

    // The field initializers above write memory through raw STA/STY that bypass
    // register tracking, so drop all cached beliefs before returning — otherwise a
    // later load of one of these fields (or of A) could be wrongly elided. The
    // final LDA is raw and leaves A untracked, which is correct.
    emitter.invalidate_registers();

    // Return the base address the way every struct-valued expression does:
    // low byte in A, high byte in X. A zero-page block still needs the explicit
    // zero high byte — a caller copying through the pointer reads X whatever
    // page the block happens to be on.
    emitter.emit_inst("LDA", &format!("#${:02X}", dest_addr & 0xFF));
    emitter.emit_inst("LDX", &format!("#${:02X}", dest_addr >> 8));

    Ok(())
}

/// Whether evaluating `expr` leaves a *pointer* to struct bytes in A:X rather
/// than a value in A.
///
/// Two expressions do this: a call to a struct-returning function, and a struct
/// literal that had to be built at run time (a constant one is also a pointer,
/// but into ROM — same convention). Consumers that bind the result have to copy
/// `size` bytes through the pointer; storing A alone writes the pointer's low
/// byte into the destination's first field, which is what every one of these
/// sites did before the computed-literal path existed and only a call could
/// reach them.
pub(crate) fn yields_struct_pointer(expr: &Spanned<Expr>, info: &ProgramInfo) -> bool {
    match &expr.node {
        _ if is_call(expr) => true,
        Expr::StructInit { .. } | Expr::AnonStructInit { .. } => {
            info.struct_temps.contains_key(&expr.span)
        }
        // Enumerated rather than left to a catch-all: a new form that returns
        // an aggregate by pointer would otherwise be read as a scalar, which is
        // how a call through a function pointer came to store one byte of the
        // struct it returned.
        Expr::Call { .. }
        | Expr::CallIndirect { .. }
        | Expr::Paren(_)
        | Expr::Variable(_)
        | Expr::Field { .. }
        | Expr::Index { .. }
        | Expr::Unary { .. }
        | Expr::U16Low(_)
        | Expr::U16High(_)
        | Expr::SliceLen(_)
        | Expr::BitOp { .. }
        | Expr::Literal(_)
        | Expr::Binary { .. }
        | Expr::Cast { .. }
        | Expr::Slice { .. }
        | Expr::EnumVariant { .. }
        | Expr::CpuFlagCarry
        | Expr::CpuFlagZero
        | Expr::CpuFlagOverflow
        | Expr::CpuFlagNegative
        | Expr::Match { .. } => false,
    }
}

/// Whether an expression form can *name storage*, as opposed to producing a
/// value by being evaluated.
///
/// The resolvers below all answer questions about addresses, and all of them
/// used to say `None` for two different things: "this form has no address, and
/// never could" and "this form has one and nobody wrote the case". A caller
/// cannot tell those apart, so it reads both as "not applicable, use the
/// ordinary path" — and the ordinary path is the scalar one. Six bugs came
/// through that door.
///
/// This is the missing half of the question. A `Call` has no address because a
/// call produces its result; that is a fact about the form. A place that no
/// case claimed is a gap, and the resolver can say so rather than shrug.
///
/// **Conservative on purpose**: a form that *might* name storage counts as a
/// place. Over-reporting costs at most a false alarm on a shape that turns out
/// to be a value; under-reporting would let the original defect back in.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Denotes {
    /// Named storage. An address exists — though not necessarily one known at
    /// compile time, which is a separate question each resolver asks itself.
    Place,
    /// A value, produced by evaluating the expression. There is no storage to
    /// take the address of.
    Value,
}

/// Classify a form. Exhaustive over `Expr` with no catch-all, so a construct
/// added to the language is a compile error here rather than a silent `Value`.
pub(crate) fn denotes(expr: &Expr) -> Denotes {
    match expr {
        // Storage, directly or through something that is.
        Expr::Variable(_) => Denotes::Place,
        Expr::Field { object, .. } | Expr::Index { object, .. } => denotes(&object.node),
        Expr::Paren(inner) => denotes(&inner.node),
        // `*p` names what `p` points at, whatever `p` itself is.
        Expr::Unary {
            op: crate::ast::UnaryOp::Deref,
            ..
        } => Denotes::Place,
        // Byte and length accessors: assignable where their operand is, so they
        // follow it. (`.len` on a genuine slice is not, but a slice is never a
        // struct, so no resolver's answer turns on it.)
        Expr::U16Low(inner) | Expr::U16High(inner) | Expr::SliceLen(inner) => denotes(&inner.node),
        // Bit access reads or writes bits of its object.
        Expr::BitOp { object, .. } => denotes(&object.node),

        // Values. Each of these produces its result; asking for an address is
        // a category error, not an unimplemented case.
        Expr::Literal(_)
        | Expr::Binary { .. }
        | Expr::Unary { .. }
        | Expr::Cast { .. }
        | Expr::Slice { .. }
        | Expr::Call { .. }
        | Expr::CallIndirect { .. }
        | Expr::StructInit { .. }
        | Expr::AnonStructInit { .. }
        | Expr::EnumVariant { .. }
        | Expr::CpuFlagCarry
        | Expr::CpuFlagZero
        | Expr::CpuFlagOverflow
        | Expr::CpuFlagNegative
        | Expr::Match { .. } => Denotes::Value,
    }
}

/// Whether `expr` is a call — direct, or through a function pointer.
///
/// The two return identically: an aggregate comes back as a pointer in A:X. But
/// only `Call` was enumerated at the four sites that ask this question, so a
/// struct or a slice from `DEV.get()` matched none of them and fell through to
/// the scalar store, which writes one byte. Where the question is "does
/// evaluating this leave a returned value in the register convention", it is
/// this and not `Call` alone.
pub(crate) fn is_call(expr: &Spanned<Expr>) -> bool {
    let mut cur = expr;
    while let Expr::Paren(inner) = &cur.node {
        cur = inner;
    }
    matches!(cur.node, Expr::Call { .. } | Expr::CallIndirect { .. })
}

/// Format a store/load operand, picking zero-page or absolute form by address.
///
/// A struct block lives in the zero-page frame region when it *is* a variable's
/// storage, and in the colored RAM scratch region when it backs a computed
/// literal in expression position. Both go through here so a single store site
/// serves either.
fn addr_operand(addr: u16) -> String {
    if addr < 0x100 {
        format!("${:02X}", addr)
    } else {
        format!("${:04X}", addr)
    }
}

/// Which register holds the high byte of a two-byte value once loaded.
///
/// A `&T` and a `str` follow the pointer convention (A:X) — both are addresses,
/// and a string literal loads as `LDA #<label / LDX #>label`. Everything else
/// that is two bytes — 16-bit scalars, and function pointers — uses A:Y.
/// Storing one as though it were the other writes the wrong high byte, which is
/// how `*pp = q` on a `&&u8` used to lose half its address and how a `str`
/// field in a struct literal came to be initialised from whatever `Y` held.
pub(crate) fn high_byte_in_x(ty: &crate::sema::types::Type) -> bool {
    matches!(
        ty,
        crate::sema::types::Type::Pointer(_) | crate::sema::types::Type::String
    )
}

/// Whether a scalar value occupies two bytes (a field, array element, pointee,
/// or the value itself).
///
/// The one authoritative answer, so a load/store site cannot drift by
/// re-listing the variants and forgetting one. Beyond the 16-bit primitives it
/// must include a function pointer and a `&T` — both hold an address — because
/// loading one byte of an address and letting the caller supply a high byte
/// from whatever an index register held is how a dispatch table came to jump
/// into nowhere and `*pp = q` came to store half a pointer. Pair it with
/// [`high_byte_in_x`] for the register convention. (A slice is four bytes and is
/// handled separately; this predicate is for the two-byte-or-one question.)
pub(crate) fn is_two_byte_value(ty: &crate::sema::types::Type) -> bool {
    use crate::sema::types::Type;
    matches!(
        ty,
        Type::Function(..)
            | Type::Pointer(_)
            | Type::String
            | Type::Primitive(
                crate::ast::PrimitiveType::U16
                    | crate::ast::PrimitiveType::I16
                    | crate::ast::PrimitiveType::B16
            )
    )
}

/// Where a statically-addressable value lives.
///
/// Almost everything has a number: a zero-page frame slot, or a BSS static
/// whose address analysis assigned. A `const` array does not — its data is
/// placed in the DATA section during codegen and referenced by label, and its
/// symbol carries the `Absolute(0)` sentinel instead of an address. Treating
/// that sentinel as real is how `ROM[i].field` came to read `$0002`: the
/// element offset added to nothing.
#[derive(Clone, Debug)]
pub(crate) enum StaticBase {
    Addr(u16),
    /// A label the assembler places, plus a byte offset into it.
    Label(String, u16),
}

impl StaticBase {
    pub(crate) fn plus(&self, delta: u16) -> Self {
        match self {
            StaticBase::Addr(a) => StaticBase::Addr(a.wrapping_add(delta)),
            StaticBase::Label(l, off) => StaticBase::Label(l.clone(), off.wrapping_add(delta)),
        }
    }

    /// The assembler operand naming this address `extra` bytes further on.
    /// Zero-page form where it fits, since that is a byte and a cycle cheaper.
    pub(crate) fn operand(&self, extra: u16) -> String {
        match self {
            StaticBase::Addr(a) => {
                let a = a.wrapping_add(extra);
                if a < 0x100 {
                    format!("${:02X}", a)
                } else {
                    format!("${:04X}", a)
                }
            }
            StaticBase::Label(l, off) => match off.wrapping_add(extra) {
                0 => l.clone(),
                n => format!("{}+{}", l, n),
            },
        }
    }

    /// Always the two-byte form, for indexed addressing where zero-page,Y does
    /// not exist for `LDA`.
    pub(crate) fn operand_abs(&self, extra: u16) -> String {
        match self {
            StaticBase::Addr(a) => format!("${:04X}", a.wrapping_add(extra)),
            StaticBase::Label(..) => self.operand(extra),
        }
    }

    /// ROM data: writable only by mistake.
    pub(crate) fn is_read_only(&self) -> bool {
        matches!(self, StaticBase::Label(..))
    }

    /// Emit this address itself as a value, low byte in A and high byte in X —
    /// the pointer convention.
    ///
    /// A label has to go through `#<`/`#>` rather than arithmetic: an immutable
    /// `const` is ROM data whose address the linker chooses, so sema leaves it
    /// at `Absolute(0)` and there is no number to add an offset to. Sites that
    /// computed `base + offset` from that placeholder produced the *offset* as
    /// the address — `&A[1]` came out as `$0001`, a pointer into system-reserved
    /// zero page.
    pub(crate) fn emit_as_pointer(&self, emitter: &mut Emitter) {
        match self {
            StaticBase::Addr(a) => {
                emitter.emit_inst("LDA", &format!("#${:02X}", a & 0xFF));
                emitter.emit_inst("LDX", &format!("#${:02X}", a >> 8));
            }
            StaticBase::Label(..) => {
                let name = self.operand(0);
                emitter.emit_inst("LDA", &format!("#<{}", name));
                emitter.emit_inst("LDX", &format!("#>{}", name));
            }
        }
    }
}

/// Whether the root of a place chain is a slot *holding* an address rather
/// than storage itself — a pointer, a by-reference parameter, or a local array,
/// whose slot points at data allocated elsewhere.
///
/// This is the question [`resolve_static_addr`] does not ask, and the reason it
/// cannot be the only answer: a constant address exists for the slot either
/// way, and only one of the two is the address a caller wants.
fn chain_root_is_indirect(expr: &Spanned<Expr>, info: &ProgramInfo) -> bool {
    use crate::sema::table::{SymbolKind, SymbolLocation};
    use crate::sema::types::Type;

    let mut cur = expr;
    loop {
        match &cur.node {
            Expr::Field { object, .. } | Expr::Index { object, .. } => cur = object,
            Expr::Paren(inner) => cur = inner,
            Expr::Unary {
                op: crate::ast::UnaryOp::Deref,
                ..
            } => return true,
            Expr::Variable(name) => {
                let Some(sym) = info
                    .resolved_symbols
                    .get(&cur.span)
                    .or_else(|| info.table.lookup(name))
                else {
                    return false;
                };
                return sym.is_param
                    || matches!(sym.ty, Type::Pointer(_))
                    || (matches!(sym.ty, Type::Array(..))
                        && sym.kind != SymbolKind::Constant
                        && matches!(sym.location, SymbolLocation::ZeroPage(_))
                        && sym.containing_function.is_some());
            }
            _ => return false,
        }
    }
}

/// Emit the address of the aggregate `expr` denotes into A:X, and return the
/// type of what lives there.
///
/// This is the run-time counterpart of [`resolve_static_addr`], and the answer
/// for every place whose address is *not* a constant: an array field reached
/// through a pointer, an element of a nested array, the struct a call just
/// returned. It tries the constant answer first, so a place that has one still
/// gets the two immediate loads it always did.
///
/// A pointer is dereferenced on the way through, because that is what the
/// language means by `p.field` and `p[i]` — there is no `(*p).field` to write
/// instead. That makes this the *aggregate* base rather than "the address of
/// the storage named `expr`": for a pointer variable the two differ, and every
/// caller here wants the pointee.
///
/// Three answers, on the same contract as [`emit_struct_place_address`]:
///
/// * `Ok(Some(ty))` — the address is in A:X and `ty` is what it points at.
/// * `Ok(None)` — `expr` denotes no storage at all, so there is no address to
///   emit. A caller may have another path for it.
/// * `Err(..)` — it denotes storage and no case here produced an address.
pub(crate) fn emit_aggregate_base(
    expr: &Spanned<Expr>,
    emitter: &mut Emitter,
    info: &ProgramInfo,
    string_collector: &mut StringCollector,
) -> Result<Option<crate::sema::types::Type>, CodegenError> {
    use crate::sema::table::SymbolLocation;
    use crate::sema::types::Type;

    // The address is a constant wherever it can be: two immediate loads rather
    // than a load and an add.
    //
    // Only where the chain's *root* is inline storage, though.
    // `resolve_static_addr` answers `Some` for a local array too — its slot has
    // an address — but that slot holds a *pointer* to the data, so the constant
    // it returns is one indirection short and every offset added to it would be
    // added to the wrong number.
    if !chain_root_is_indirect(expr, info)
        && let Some((base, ty)) = resolve_static_addr(expr, info)
    {
        base.emit_as_pointer(emitter);
        emitter.invalidate_registers();
        return Ok(Some(ty));
    }

    match &expr.node {
        Expr::Paren(inner) => emit_aggregate_base(inner, emitter, info, string_collector),

        // A slot holding an address: a pointer, a by-reference struct
        // parameter, or a local array, whose slot points at its data.
        Expr::Variable(name) => {
            let Some(sym) = info
                .resolved_symbols
                .get(&expr.span)
                .or_else(|| info.table.lookup(name))
            else {
                return Ok(None);
            };
            let SymbolLocation::ZeroPage(slot) = sym.location else {
                return Ok(None);
            };
            let pointee = match &sym.ty {
                Type::Pointer(inner) => (**inner).clone(),
                // A parameter's slot holds the caller's address; a local array's
                // holds its data's. Either way the slot is not the value.
                t if sym.is_param || matches!(t, Type::Array(..)) => t.clone(),
                _ => return Ok(None),
            };
            emitter.emit_inst("LDA", &format!("${:02X}", slot));
            emitter.emit_inst("LDX", &format!("${:02X}", slot + 1));
            emitter.invalidate_registers();
            Ok(Some(pointee))
        }

        // `*p` names what `p` holds, which is already the address.
        Expr::Unary {
            op: crate::ast::UnaryOp::Deref,
            operand,
        } => {
            let Some(Type::Pointer(inner)) = info.resolved_types.get(&operand.span) else {
                return Ok(None);
            };
            let pointee = (**inner).clone();
            generate_expr(operand, emitter, info, string_collector)?;
            Ok(Some(pointee))
        }

        Expr::Field { object, field } => {
            let Some(obj_ty) = emit_aggregate_base(object, emitter, info, string_collector)? else {
                return Ok(None);
            };
            let sname = match obj_ty {
                Type::Named(n) => n,
                Type::Pointer(inner) => match *inner {
                    Type::Named(n) => n,
                    _ => return Ok(None),
                },
                _ => return Ok(None),
            };
            let sdef = info.type_registry.get_struct(&sname).ok_or_else(|| {
                CodegenError::UnsupportedOperation(format!("struct '{sname}' not found"))
            })?;
            let finfo = sdef.get_field(&field.node).ok_or_else(|| {
                CodegenError::UnsupportedOperation(format!(
                    "field '{}' not found in struct '{sname}'",
                    field.node
                ))
            })?;
            emit_add_const_to_ax(emitter, finfo.offset as u16);
            Ok(Some(finfo.ty.clone()))
        }

        Expr::Index { object, index } => {
            emit_element_address(object, index, emitter, info, string_collector)
        }

        // A call and a run-time struct literal hand back a pointer already.
        _ if yields_struct_pointer(expr, info) => {
            let ty = info.resolved_types.get(&expr.span).cloned();
            generate_expr(expr, emitter, info, string_collector)?;
            Ok(ty)
        }

        // Enumerated rather than left to a catch-all, so a new form has to be
        // classified here rather than silently reported as "no storage".
        Expr::Unary { .. }
        | Expr::U16Low(_)
        | Expr::U16High(_)
        | Expr::SliceLen(_)
        | Expr::BitOp { .. }
        | Expr::Literal(_)
        | Expr::Binary { .. }
        | Expr::Cast { .. }
        | Expr::Slice { .. }
        | Expr::Call { .. }
        | Expr::CallIndirect { .. }
        | Expr::StructInit { .. }
        | Expr::AnonStructInit { .. }
        | Expr::EnumVariant { .. }
        | Expr::CpuFlagCarry
        | Expr::CpuFlagZero
        | Expr::CpuFlagOverflow
        | Expr::CpuFlagNegative
        | Expr::Match { .. } => Ok(None),
    }
}

/// A:X += a constant, carry propagated into the high byte.
fn emit_add_const_to_ax(emitter: &mut Emitter, delta: u16) {
    if delta == 0 {
        return;
    }
    emitter.emit_inst("CLC", "");
    emitter.emit_inst("ADC", &format!("#${:02X}", delta & 0xFF));
    emitter.emit_inst("PHA", "");
    emitter.emit_inst("TXA", "");
    emitter.emit_inst("ADC", &format!("#${:02X}", delta >> 8));
    emitter.emit_inst("TAX", "");
    emitter.emit_inst("PLA", "");
    emitter.invalidate_registers();
}

/// A:X += a byte parked in zero page, carry propagated into the high byte.
fn emit_add_zp_to_ax(emitter: &mut Emitter, at: u8) {
    emitter.emit_inst("CLC", "");
    emitter.emit_inst("ADC", &format!("${:02X}", at));
    emitter.emit_inst("PHA", "");
    emitter.emit_inst("TXA", "");
    emitter.emit_inst("ADC", "#$00");
    emitter.emit_inst("TAX", "");
    emitter.emit_inst("PLA", "");
    emitter.invalidate_registers();
}

/// Resolve a chain of *local* (inline, non-pointer) struct accesses to a static
/// `(base_address, struct_type_name)`. Handles `Variable` and nested `Field`
/// objects (and constant array indices). Returns `None` when any link is not
/// statically addressable — a by-reference struct parameter, a runtime array
/// index, or a non-struct — so callers can fall back to another path.
pub(crate) fn resolve_static_struct_lvalue(
    expr: &Spanned<crate::ast::Expr>,
    info: &ProgramInfo,
) -> Option<(StaticBase, String)> {
    use crate::ast::Expr;
    use crate::sema::types::Type;

    match &expr.node {
        Expr::Variable(_) => {
            let (base, ty) = resolve_static_addr(expr, info)?;
            match ty {
                Type::Named(n) => Some((base, n)),
                _ => None,
            }
        }
        Expr::Field { object, field } => {
            let (base, struct_name) = resolve_static_struct_lvalue(object, info)?;
            let sdef = info.type_registry.get_struct(&struct_name)?;
            let finfo = sdef.get_field(&field.node)?;
            match &finfo.ty {
                Type::Named(inner) => Some((base.plus(finfo.offset as u16), inner.clone())),
                _ => None, // scalar field: not a further struct
            }
        }
        Expr::Index { object, index } => {
            // Array element as struct: only a constant index is statically
            // addressable here (runtime indices need pointer arithmetic).
            let (base, elem_struct) = array_of_struct_base(object, info)?;
            let idx = const_index(index, info)?;
            let sdef = info.type_registry.get_struct(&elem_struct)?;
            Some((base.plus((idx * sdef.total_size) as u16), elem_struct))
        }
        Expr::Paren(inner) => resolve_static_struct_lvalue(inner, info),

        // Not a struct place with a constant address. Enumerated for the same
        // reason as in `resolve_static_addr`: a form added to the language
        // should have to be classified rather than fall through as "no".
        Expr::Unary { .. }
        | Expr::U16Low(_)
        | Expr::U16High(_)
        | Expr::SliceLen(_)
        | Expr::BitOp { .. }
        | Expr::Literal(_)
        | Expr::Binary { .. }
        | Expr::Cast { .. }
        | Expr::Slice { .. }
        | Expr::Call { .. }
        | Expr::CallIndirect { .. }
        | Expr::StructInit { .. }
        | Expr::AnonStructInit { .. }
        | Expr::EnumVariant { .. }
        | Expr::CpuFlagCarry
        | Expr::CpuFlagZero
        | Expr::CpuFlagOverflow
        | Expr::CpuFlagNegative
        | Expr::Match { .. } => None,
    }
}

/// Emit the *address* of a struct-valued place into A:X — the pointer
/// convention — and return the struct's name.
///
/// Three shapes, because a struct's bytes are reachable three ways: inline
/// storage whose address the assembler knows, a by-reference parameter whose
/// slot holds a pointer to the caller's storage, and an array element whose
/// offset exists only at run time.
///
/// The three answers are distinct, which is the point of the signature:
///
/// * `Ok(Some(name))` — here is the address.
/// * `Ok(None)` — `expr` is not a place. A literal or a call chooses its own
///   storage and hands back a pointer already, which is the return-by-value
///   path's business, not this one's.
/// * `Err(..)` — `expr` *is* a struct-typed place and no case produced an
///   address. That is a gap in this function, and it says so rather than
///   returning `None`, which every caller reads as "use the ordinary path" —
///   and the ordinary path copies one byte.
pub(crate) fn emit_struct_place_address(
    expr: &Spanned<Expr>,
    emitter: &mut Emitter,
    info: &ProgramInfo,
    string_collector: &mut StringCollector,
) -> Result<Option<String>, CodegenError> {
    use crate::sema::table::SymbolLocation;
    use crate::sema::types::Type;

    // Inline storage: a local, a static, a nested field, or a constant index.
    if let Some((base, struct_name)) = resolve_static_struct_lvalue(expr, info) {
        base.emit_as_pointer(emitter);
        emitter.invalidate_registers();
        return Ok(Some(struct_name));
    }

    // A by-reference struct parameter, or a `&Struct`: the slot holds a
    // pointer to storage that belongs to someone else, so the address is a
    // load rather than a constant.
    if let Expr::Variable(name) = &expr.node
        && let Some(sym) = info
            .resolved_symbols
            .get(&expr.span)
            .or_else(|| info.table.lookup(name))
        && let SymbolLocation::ZeroPage(slot) = sym.location
    {
        let pointee = match &sym.ty {
            Type::Named(n) => Some(n.clone()),
            Type::Pointer(inner) => match &**inner {
                Type::Named(n) => Some(n.clone()),
                _ => None,
            },
            _ => None,
        };
        if let Some(struct_name) = pointee
            && info.type_registry.get_struct(&struct_name).is_some()
        {
            emitter.emit_inst("LDA", &format!("${:02X}", slot));
            emitter.emit_inst("LDX", &format!("${:02X}", slot + 1));
            emitter.invalidate_registers();
            return Ok(Some(struct_name));
        }
    }

    // An array element at a runtime index: base + index * element size. The
    // scaled index is one byte — element offsets are addressed as `abs,Y`
    // everywhere else, so the array is within 256 bytes either way — which
    // leaves the carry into the high byte as the only 16-bit part.
    if let Expr::Index { object, index } = &expr.node
        && let Some((base, elem_struct)) = array_of_struct_base(object, info)
    {
        let elem_size = info
            .type_registry
            .get_struct(&elem_struct)
            .ok_or_else(|| {
                CodegenError::UnsupportedOperation(format!("struct '{elem_struct}' not found"))
            })?
            .total_size;
        generate_expr(index, emitter, info, string_collector)?;
        emit_scale_index(emitter, elem_size);
        emitter.emit_inst("CLC", "");
        match &base {
            StaticBase::Addr(a) => {
                emitter.emit_inst("ADC", &format!("#${:02X}", a & 0xFF));
                emitter.emit_inst("PHA", "");
                emitter.emit_inst("LDA", &format!("#${:02X}", a >> 8));
            }
            StaticBase::Label(..) => {
                let label = base.operand(0);
                emitter.emit_inst("ADC", &format!("#<{}", label));
                emitter.emit_inst("PHA", "");
                emitter.emit_inst("LDA", &format!("#>{}", label));
            }
        }
        emitter.emit_inst("ADC", "#$00");
        emitter.emit_inst("TAX", "");
        emitter.emit_inst("PLA", "");
        emitter.invalidate_registers();
        return Ok(Some(elem_struct));
    }

    // Nothing above claimed it. If the expression *names storage* and that
    // storage is a struct, then it has an address and no case here knows how
    // to produce one — a gap, not a fact about the form. Saying `None` sends
    // the caller down its ordinary path, which for every caller is the scalar
    // one, and one byte of a struct gets copied.
    //
    // A `Value` reaching here is the honest `None`: a literal or a call builds
    // its own storage and hands back a pointer, which is a different
    // convention and a different caller's business.
    if denotes(&expr.node) == Denotes::Place
        && let Some(Type::Named(name)) = info.resolved_types.get(&expr.span)
        && info.type_registry.get_struct(name).is_some()
    {
        return Err(CodegenError::Internal(format!(
            "no way to take the address of this `{name}` place: it names storage, so an \
             address exists, but no case in `emit_struct_place_address` produces one"
        )));
    }

    Ok(None)
}

/// Scale A (a runtime array index) by a constant element size, leaving the byte
/// offset in A. Uses shifts for power-of-two sizes and unrolled adds otherwise.
fn emit_scale_index(emitter: &mut Emitter, size: usize) {
    if size <= 1 {
        return;
    }
    if size.is_power_of_two() {
        for _ in 0..size.trailing_zeros() {
            emitter.emit_inst("ASL", "A");
        }
    } else {
        // acc = index * size via repeated addition of the saved index.
        let tmp = emitter.memory_layout.loop_end_temp(); // $22
        emitter.emit_inst("STA", &format!("${:02X}", tmp));
        for _ in 1..size {
            emitter.emit_inst("CLC", "");
            emitter.emit_inst("ADC", &format!("${:02X}", tmp));
        }
    }
}

/// Read or write `array[index].field` for a local array-of-struct with a
/// *runtime* index, using absolute,Y indexing (element offset = index × size).
/// `value` = None reads the field into A (low)/Y (high for u16); `Some(v)`
/// evaluates and stores it.
pub(crate) fn emit_array_struct_field_indexed(
    array: &Spanned<Expr>,
    index: &Spanned<Expr>,
    field: &Spanned<String>,
    value: Option<&Spanned<Expr>>,
    emitter: &mut Emitter,
    info: &ProgramInfo,
    string_collector: &mut StringCollector,
) -> Result<bool, CodegenError> {
    let Some((base, elem_struct)) = array_of_struct_base(array, info) else {
        return Ok(false); // not a local array-of-struct; caller falls back
    };
    let sdef = info.type_registry.get_struct(&elem_struct).ok_or_else(|| {
        CodegenError::UnsupportedOperation(format!("struct '{elem_struct}' not found"))
    })?;
    let elem_size = sdef.total_size;
    let finfo = sdef.get_field(&field.node).cloned().ok_or_else(|| {
        CodegenError::UnsupportedOperation(format!(
            "field '{}' not found in struct '{}'",
            field.node, elem_struct
        ))
    })?;
    let field = base.plus(finfo.offset as u16);
    let is_multibyte = is_two_byte_value(&finfo.ty);

    match value {
        None => {
            emitter.emit_comment("Array-of-struct indexed field read");
            generate_expr(index, emitter, info, string_collector)?;
            emit_scale_index(emitter, elem_size);
            emitter.emit_inst("TAY", "");
            emitter.emit_inst("LDA", &format!("{},Y", field.operand_abs(0)));
            if is_multibyte {
                emitter.emit_inst("PHA", "");
                emitter.emit_inst("LDA", &format!("{},Y", field.operand_abs(1)));
                let hi = if high_byte_in_x(&finfo.ty) {
                    "TAX"
                } else {
                    "TAY"
                };
                emitter.emit_inst(hi, ""); // high byte to its register
                emitter.emit_inst("PLA", ""); // A = low byte
            }
            emitter.mark_a_unknown();
        }
        Some(val) => {
            // Sema rejects assigning through a `const`, so reaching here with a
            // label base would mean a store quietly aimed at ROM.
            if base.is_read_only() {
                return Err(CodegenError::UnsupportedOperation(
                    "cannot write to constant data".to_string(),
                ));
            }
            emitter.emit_comment("Array-of-struct indexed field write");
            // Evaluate the value and park it in the arg pool *allocated*, so a
            // call in the index expression — whose own argument staging comes
            // from the same pool — is handed different bytes. The old comment
            // asserted "the index expression below does not touch" $F4/$F5
            // without reserving them; `ps[ident(1)].x = 42` stored 1.
            let park = emitter.temp_alloc.alloc_arg(2).ok_or_else(|| {
                CodegenError::Internal(
                    "temporary storage exhausted in array-of-struct field write".to_string(),
                )
            })?;
            generate_expr(val, emitter, info, string_collector)?;
            emitter.emit_inst("STA", &format!("${:02X}", park));
            if is_multibyte {
                let hi = if high_byte_in_x(&finfo.ty) {
                    "STX"
                } else {
                    "STY"
                };
                emitter.emit_inst(hi, &format!("${:02X}", park + 1));
            }
            // The value now sits in A but was just stashed; drop register
            // tracking so the index load below isn't wrongly elided.
            emitter.mark_a_unknown();
            generate_expr(index, emitter, info, string_collector)?;
            emit_scale_index(emitter, elem_size);
            emitter.emit_inst("TAY", "");
            emitter.emit_inst("LDA", &format!("${:02X}", park));
            emitter.emit_inst("STA", &format!("{},Y", field.operand_abs(0)));
            if is_multibyte {
                emitter.emit_inst("LDA", &format!("${:02X}", park + 1));
                emitter.emit_inst("STA", &format!("{},Y", field.operand_abs(1)));
            }
            emitter.temp_alloc.free_arg(park, 2);
            emitter.mark_a_unknown();
        }
    }
    Ok(true)
}

/// Resolve any statically-addressable lvalue to its `(address, type)`: a local
/// (inline, non-pointer) variable, a nested struct field, or a constant array
/// index. Returns None for by-reference parameters, runtime indices, or
/// non-static forms. Generalizes resolve_static_struct_lvalue to non-struct
/// field types (e.g. an array-of-struct field, needed for `a.b[i].c`).
pub(crate) fn resolve_static_addr(
    expr: &Spanned<crate::ast::Expr>,
    info: &ProgramInfo,
) -> Option<(StaticBase, crate::sema::types::Type)> {
    use crate::ast::Expr;
    use crate::sema::table::{SymbolKind, SymbolLocation};
    use crate::sema::types::Type;

    match &expr.node {
        Expr::Variable(name) => {
            let sym = info
                .resolved_symbols
                .get(&expr.span)
                .or_else(|| info.table.lookup(name))?;
            if sym.is_param {
                return None; // pointer, not inline storage
            }
            // A const *array* is laid out in the DATA section under its own
            // name. Any other const is folded into the instruction stream and
            // has no address to speak of.
            if sym.kind == SymbolKind::Constant {
                return match &sym.ty {
                    Type::Array(..) => Some((StaticBase::Label(name.clone(), 0), sym.ty.clone())),
                    _ => None,
                };
            }
            let addr = match sym.location {
                SymbolLocation::ZeroPage(a) => a as u16,
                SymbolLocation::Absolute(a) => a,
                _ => return None,
            };
            Some((StaticBase::Addr(addr), sym.ty.clone()))
        }
        Expr::Field { object, field } => {
            let (base, ty) = resolve_static_addr(object, info)?;
            let Type::Named(sname) = ty else { return None };
            let sdef = info.type_registry.get_struct(&sname)?;
            let finfo = sdef.get_field(&field.node)?;
            Some((base.plus(finfo.offset as u16), finfo.ty.clone()))
        }
        Expr::Index { object, index } => {
            let (base, ty) = resolve_static_addr(object, info)?;
            let Type::Array(elem, _) = ty else {
                return None;
            };
            let idx = const_index(index, info)?;
            let esize = type_byte_size(&elem, info);
            Some((base.plus((idx * esize) as u16), (*elem).clone()))
        }
        // A parenthesised place is the place inside it.
        Expr::Paren(inner) => resolve_static_addr(inner, info),

        // No *constant* address, for one of two reasons, and enumerated rather
        // than left to a catch-all so a new form has to be classified here.
        //
        // `Deref` and the accessors below name storage, but through a pointer
        // or at an offset this function is not asked about — the `.low` store
        // path resolves the *object*, not the accessor node. Everything after
        // them produces a value and has no storage at all.
        Expr::Unary { .. }
        | Expr::U16Low(_)
        | Expr::U16High(_)
        | Expr::SliceLen(_)
        | Expr::BitOp { .. }
        | Expr::Literal(_)
        | Expr::Binary { .. }
        | Expr::Cast { .. }
        | Expr::Slice { .. }
        | Expr::Call { .. }
        | Expr::CallIndirect { .. }
        | Expr::StructInit { .. }
        | Expr::AnonStructInit { .. }
        | Expr::EnumVariant { .. }
        | Expr::CpuFlagCarry
        | Expr::CpuFlagZero
        | Expr::CpuFlagOverflow
        | Expr::CpuFlagNegative
        | Expr::Match { .. } => None,
    }
}

/// If `expr` is a statically-addressable array-of-struct lvalue (a variable or
/// a nested field), return its base address and the element struct's name.
fn array_of_struct_base(
    expr: &Spanned<crate::ast::Expr>,
    info: &ProgramInfo,
) -> Option<(StaticBase, String)> {
    use crate::sema::types::Type;

    let (addr, ty) = resolve_static_addr(expr, info)?;
    match ty {
        Type::Array(elem, _) => match *elem {
            Type::Named(n) if info.type_registry.get_struct(&n).is_some() => Some((addr, n)),
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
    base: StaticBase,
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
    let is_multibyte = is_two_byte_value(&finfo.ty);
    let at = base.plus(finfo.offset as u16);
    emitter.emit_inst("LDA", &at.operand(0));
    if is_multibyte {
        let hi = if high_byte_in_x(&finfo.ty) {
            "LDX"
        } else {
            "LDY"
        };
        emitter.emit_inst(hi, &at.operand(1));
    }
    emitter.mark_a_unknown();
    Ok(())
}

/// If `expr` is a field/index chain whose root is a pointer variable, return
/// that variable's name.
///
/// `p.field` auto-derefs because a pointer's slot holds an address exactly as a
/// struct parameter's does. `p.a.b` does not: `resolve_static_struct_lvalue`
/// computes a compile-time address, and there is no compile-time address behind
/// a pointer. Naming the pointer makes the fix obvious rather than leaving the
/// generic "only supported on variables" message to be puzzled over.
fn pointer_rooted_chain_base(
    expr: &Spanned<crate::ast::Expr>,
    info: &ProgramInfo,
) -> Option<String> {
    use crate::ast::Expr;
    let mut cur = expr;
    loop {
        match &cur.node {
            Expr::Field { object, .. } | Expr::Index { object, .. } => cur = object,
            Expr::Paren(inner) => cur = inner,
            Expr::Variable(name) => {
                let is_ptr = matches!(
                    info.resolved_types.get(&cur.span),
                    Some(crate::sema::types::Type::Pointer(_))
                );
                return if is_ptr { Some(name.clone()) } else { None };
            }
            _ => return None,
        }
    }
}

pub(super) fn generate_field_access(
    object: &Spanned<crate::ast::Expr>,
    field: &Spanned<String>,
    emitter: &mut Emitter,
    info: &ProgramInfo,
    string_collector: &mut StringCollector,
) -> Result<(), CodegenError> {
    use crate::ast::Expr;

    // Nested field access (a.b.c) or array-of-struct element (arr[i].f): the
    // object is not a plain variable. Resolve a static address for local chains.
    if !matches!(&object.node, Expr::Variable(_)) {
        if let Some((base, struct_name)) = resolve_static_struct_lvalue(object, info) {
            emitter.emit_comment(&format!("Nested field access: .{}", field.node));
            return emit_static_field_load(base, &struct_name, field, emitter, info);
        }
        // arr[i].field with a runtime index: absolute,Y indexed access.
        if let Expr::Index {
            object: array,
            index,
        } = &object.node
            && emit_array_struct_field_indexed(
                array,
                index,
                field,
                None,
                emitter,
                info,
                string_collector,
            )?
        {
            return Ok(());
        }
        // Anything else whose address can be computed at run time: a struct
        // that a call just handed back — `mk(6).f1` — an element of a nested
        // array, a chain through a pointer. The address goes into A:X, the
        // field is read through it.
        if let Some(ty) = emit_aggregate_base(object, emitter, info, string_collector)? {
            let sname = match ty {
                crate::sema::types::Type::Named(n) => Some(n),
                crate::sema::types::Type::Pointer(inner) => match *inner {
                    crate::sema::types::Type::Named(n) => Some(n),
                    _ => None,
                },
                _ => None,
            };
            if let Some(sname) = sname
                && let Some(sdef) = info.type_registry.get_struct(&sname)
                && let Some(finfo) = sdef.get_field(&field.node)
            {
                let ptr = emitter.memory_layout.deref_ptr();
                emitter.emit_inst("STA", &format!("${:02X}", ptr));
                emitter.emit_inst("STX", &format!("${:02X}", ptr + 1));
                emit_deref_load(
                    emitter,
                    ptr,
                    finfo.offset as u8,
                    is_two_byte_value(&finfo.ty),
                    high_byte_in_x(&finfo.ty),
                );
                return Ok(());
            }
        }
        if let Some(base) = pointer_rooted_chain_base(object, info) {
            return Err(CodegenError::UnsupportedOperation(format!(
                "cannot follow a field chain through the pointer '{}'; bind an intermediate, \
                 as `let inner: &T = &{}.<field>;`",
                base, base
            )));
        }
        return Err(CodegenError::UnsupportedOperation(
            "a field can be read from a name, a place, or a call's result — not from this \
             expression, which names no storage"
                .to_string(),
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
            // range test no longer works). A `&Struct` is in exactly the same
            // shape as a struct parameter — a 2-byte address in the slot — so
            // `p.field` takes the same path, which is why it auto-derefs.
            let sym_is_param =
                sym.is_param || matches!(sym.ty, crate::sema::types::Type::Pointer(_));

            emitter.emit_comment(&format!("Field access: {}.{}", var_name, field.node));

            // Get the struct type name from the symbol's type, looking through
            // one level of pointer.
            let struct_name = match &sym.ty {
                crate::sema::types::Type::Named(name) => name,
                crate::sema::types::Type::Pointer(inner) => match &**inner {
                    crate::sema::types::Type::Named(name) => name,
                    _ => {
                        return Err(CodegenError::UnsupportedOperation(format!(
                            "variable '{}' is not a pointer to a struct",
                            var_name
                        )));
                    }
                },
                _ => {
                    return Err(CodegenError::UnsupportedOperation(format!(
                        "variable '{}' is not a struct type",
                        var_name
                    )));
                }
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
            // A function-pointer field holds a 2-byte code address, so it must be
            // loaded as a pair like u16 — reading only the low byte would call
            // through a half-formed vector.
            let is_multibyte = is_two_byte_value(&field_info.ty);

            if is_parameter {
                // The struct pointer lives directly in this parameter's frame slot;
                // frame coloring guarantees nested calls cannot clobber it.
                emit_deref_load(
                    emitter,
                    base_addr as u8,
                    field_info.offset as u8,
                    is_multibyte,
                    high_byte_in_x(&field_info.ty),
                );
            } else {
                // Local struct - direct access
                let field_addr = base_addr + field_info.offset as u16;
                let hi = if high_byte_in_x(&field_info.ty) {
                    "LDX"
                } else {
                    "LDY"
                };
                if field_addr < 0x100 {
                    emitter.emit_inst("LDA", &format!("${:02X}", field_addr));
                    if is_multibyte {
                        emitter.emit_inst(hi, &format!("${:02X}", field_addr + 1));
                    }
                } else {
                    emitter.emit_inst("LDA", &format!("${:04X}", field_addr));
                    if is_multibyte {
                        emitter.emit_inst(hi, &format!("${:04X}", field_addr + 1));
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
        // Constant payload: collect it into DATA (dedup'd), no inline JMP.
        generate_enum_variant_inline(
            enum_name,
            variant,
            data,
            emitter,
            variant_info,
            string_collector,
        )
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

/// Generate an enum variant with a constant payload.
///
/// The tag byte and the constant field bytes are collected into the DATA
/// section (deduplicated) rather than emitted inline in the instruction stream
/// behind a `JMP` — which saves the 3-byte jump and keeps the payload out of
/// the code path. The construction leaves the payload's address in A:X.
fn generate_enum_variant_inline(
    _enum_name: &Spanned<String>,
    variant: &Spanned<String>,
    data: &crate::ast::VariantData,
    emitter: &mut Emitter,
    variant_info: &crate::sema::type_defs::VariantInfo,
    string_collector: &mut crate::codegen::StringCollector,
) -> Result<(), CodegenError> {
    // Build the payload: discriminant tag followed by the field bytes.
    let mut bytes = vec![variant_info.tag];

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
                push_constant_value(&value_expr.node, field_type.size(), &mut bytes)?;
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
                    push_constant_value(&value_expr.node, field_info.ty.size(), &mut bytes)?;
                } else {
                    // Field not provided - initialize to zero
                    bytes.resize(bytes.len() + field_info.ty.size(), 0);
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

    let label = string_collector.add_enum_blob(bytes);

    // Load the address of the payload into A (low byte) and X (high byte).
    emitter.emit_inst("LDA", &format!("#<{}", label));
    emitter.emit_inst("LDX", &format!("#>{}", label));

    Ok(())
}

/// Append a constant value's little-endian bytes to `out`.
fn push_constant_value(
    expr: &crate::ast::Expr,
    size: usize,
    out: &mut Vec<u8>,
) -> Result<(), CodegenError> {
    if let crate::ast::Expr::Literal(lit) = expr {
        match lit {
            crate::ast::Literal::Integer(val) => {
                if size == 1 {
                    out.push(*val as u8);
                } else if size == 2 {
                    out.push((*val & 0xFF) as u8);
                    out.push(((*val >> 8) & 0xFF) as u8);
                } else {
                    return Err(CodegenError::UnsupportedOperation(format!(
                        "field type with size {} not yet supported",
                        size
                    )));
                }
            }
            crate::ast::Literal::Bool(b) => {
                out.push(if *b { 1 } else { 0 });
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

    // We don't free the temp storage here: the caller needs the pointer we just
    // loaded, so the block must stay reserved for the rest of the expression.
    // `generate_function` resets the pool at each function boundary, which
    // reclaims it — do not add a per-construction free.

    Ok(())
}

#[cfg(test)]
mod pointer_size_tests {
    use crate::ast::PrimitiveType;
    use crate::sema::types::Type;

    /// `type_byte_size` reaches `Type::Pointer` through its `other => other.size()`
    /// fallback rather than an arm of its own, which is easy to break by adding
    /// a more specific arm above it. Two bytes is the whole calling convention:
    /// get it wrong and arguments are marshalled at the wrong width.
    #[test]
    fn a_pointer_is_two_bytes_here_too() {
        assert_eq!(
            Type::Pointer(Box::new(Type::Primitive(PrimitiveType::U8))).size(),
            2
        );
        assert_eq!(
            Type::Pointer(Box::new(Type::Named("Device".into()))).size(),
            2
        );
    }
}
