//! Assignment statement codegen: index, slice, field, and pointer
//! stores, plus the materialization of aggregates into their local slots.

use super::*;

pub(super) fn generate_index_assignment(
    object: &Spanned<crate::ast::Expr>,
    index: &Spanned<crate::ast::Expr>,
    value: &Spanned<crate::ast::Expr>,
    emitter: &mut Emitter,
    info: &ProgramInfo,
    string_collector: &mut StringCollector,
) -> Result<(), CodegenError> {
    use crate::ast::Expr;
    use crate::sema::table::SymbolLocation;
    use crate::sema::types::Type;

    emitter.emit_comment("Array element assignment");

    // Step 1: Get the element type for the array
    let object_type = info.resolved_types.get(&object.span).ok_or_else(|| {
        CodegenError::UnsupportedOperation("Type information not found".to_string())
    })?;

    // A pointer's slot holds a base address just as a local array's does, so
    // `p[i] = v` is the same indirect store scaled by the element width. The
    // pointer carries no length, so there is nothing to bounds-check.
    //
    // A `str<N>` buffer's slot likewise holds a pointer, but to `[len][bytes]`:
    // `s[i]` writes one byte at offset i+1, past the length prefix. It has u8
    // elements, so it rides the same single-byte indirect store path with the
    // index nudged by one (handled after the index is in Y). Sema has already
    // rejected writing through a plain (possibly ROM) `str`.
    let is_string = matches!(object_type, Type::String);
    let element_type: Option<&Type> = match object_type {
        Type::Array(elem_ty, ..) | Type::Pointer(elem_ty) => Some(elem_ty),
        Type::String => None,
        _ => {
            return Err(CodegenError::UnsupportedOperation(
                "Can only index arrays, pointers, and string buffers".to_string(),
            ));
        }
    };

    // A two-byte element (u16/i16/b16, a function pointer, or a `&T`) is scaled
    // by the element width and stored as a low/high pair. Function pointers are
    // what installing a driver into a table (`handlers[i] = drv`) writes, so
    // omitting them here stored only the low byte at an unscaled offset.
    let is_multibyte = element_type.is_some_and(crate::codegen::expr::is_two_byte_value);

    // A runtime index into a fixed-size array whose scaled offset would exceed
    // the 8-bit index register silently wraps (`ASL` drops its carry; `base,Y`
    // reaches only base+255) — the same store-side hole the read path guards.
    if let Type::Array(elem_ty, len) = object_type {
        crate::codegen::expr::check_runtime_index_range(
            crate::codegen::expr::type_byte_size(elem_ty, info),
            *len,
            index,
            info,
        )?;
    }

    // Step 2: Evaluate the value expression
    emitter.emit_comment("Evaluate value to assign");
    generate_expr(value, emitter, info, string_collector)?;

    // Step 3: Save the value while the index is evaluated. It cannot live in the
    // shared $20 temp: evaluating a compound index like `a[i + 2]` uses $20 for
    // its own operand, overwriting the value, and the store below would then
    // write the index expression's operand instead. Take a dedicated slot —
    // and if the pool is exhausted, fail loudly: the old $20/$21 fallback was
    // the very temp the comment above explains cannot hold the value.
    emitter.emit_comment("Save value to temp");
    let save = emitter.temp_alloc.alloc_high(2).ok_or_else(|| {
        CodegenError::Internal("temporary storage exhausted in index assignment".to_string())
    })?;
    let (save_lo, save_hi) = (save, save + 1);
    emitter.emit_inst("STA", &format!("${:02X}", save_lo));
    if is_multibyte {
        emitter.emit_inst("STY", &format!("${:02X}", save_hi));
    }
    // The value expression and these raw stores bypass register tracking, so a
    // belief left from before (e.g. `a = ZeroPage(i)` after `let i = 3`) would
    // wrongly elide the index load below, indexing by the *value* instead.
    emitter.invalidate_registers();

    // Step 4: Evaluate index expression
    emitter.emit_comment("Evaluate index");
    generate_expr(index, emitter, info, string_collector)?;

    // Step 5: Transfer index to Y register
    emitter.emit_inst("TAY", "");

    // A string buffer's data starts one byte past the length prefix, so s[i]
    // lands at offset i+1. u8 elements only, so there is no index scaling to
    // interact with this nudge.
    if is_string {
        emitter.emit_comment("Skip str length prefix: index += 1");
        emitter.emit_inst("INY", "");
    }

    // Step 6: Get array base address
    // For now, only support simple variable arrays
    if let Expr::Variable(array_name) = &object.node {
        let sym = info
            .resolved_symbols
            .get(&object.span)
            .or_else(|| info.table.lookup(array_name))
            .ok_or_else(|| CodegenError::SymbolNotFound(array_name.clone()))?;

        // A mutable `static` array lives inline at its own label in RAM, so it is
        // stored with absolute-indexed addressing (`STA NAME,Y`) rather than the
        // indirect path used for local arrays, whose slot holds a pointer.
        if sym.containing_function.is_none()
            && matches!(sym.location, SymbolLocation::Absolute(_))
            && matches!(sym.ty, Type::Array(..))
        {
            if is_multibyte {
                emitter.emit_comment("Scale index for u16 array (multiply by 2)");
                emitter.emit_inst("TYA", "");
                emitter.emit_inst("ASL", "A");
                emitter.emit_inst("TAY", "");
            }
            emitter.emit_inst("LDA", &format!("${:02X}", save_lo));
            emitter.emit_inst("STA", &format!("{},Y", array_name));
            if is_multibyte {
                emitter.emit_inst("INY", "");
                emitter.emit_inst("LDA", &format!("${:02X}", save_hi));
                emitter.emit_inst("STA", &format!("{},Y", array_name));
            }
            emitter.invalidate_registers();
            emitter.temp_alloc.free_high(save, 2);
            return Ok(());
        }

        // A local array's slot holds a pointer, whether the slot lives in the
        // zero page proper or at a low absolute address — both reach it with the
        // same `(zp),Y` indirect store, so derive the one-byte pointer address
        // and share the store body.
        let ptr = match sym.location {
            SymbolLocation::ZeroPage(addr) => addr,
            SymbolLocation::Absolute(addr) if addr < 256 => addr as u8,
            _ => {
                return Err(CodegenError::UnsupportedOperation(format!(
                    "'{}' must be in zero page for indexed assignment",
                    array_name
                )));
            }
        };
        if !is_multibyte {
            // Restore the saved value and store it at array[index].
            emitter.emit_inst("LDA", &format!("${:02X}", save_lo));
            emitter.emit_inst("STA", &format!("(${:02X}),Y", ptr));
        } else {
            // Scale the index by the 2-byte element width, then store low/high.
            emitter.emit_comment("Scale index for u16 array (multiply by 2)");
            emitter.emit_inst("TYA", "");
            emitter.emit_inst("ASL", "A");
            emitter.emit_inst("TAY", "");

            emitter.emit_inst("LDA", &format!("${:02X}", save_lo));
            emitter.emit_inst("STA", &format!("(${:02X}),Y", ptr));

            emitter.emit_inst("INY", "");
            emitter.emit_inst("LDA", &format!("${:02X}", save_hi));
            emitter.emit_inst("STA", &format!("(${:02X}),Y", ptr));
        }
    } else {
        return Err(CodegenError::UnsupportedOperation(
            "Can only assign to array variables, not expressions".to_string(),
        ));
    }

    emitter.temp_alloc.free_high(save, 2);

    // This path mutates A/X/Y through raw instructions the register tracker does
    // not follow, so drop all cached register beliefs. Otherwise a following read
    // of a variable the tracker thinks is still in A would be wrongly elided.
    emitter.invalidate_registers();

    Ok(())
}

/// Materialize a slice descriptor `arr[start..end]` into the 4-byte frame slot
/// at `dest`: `dest[0..1] = base` (arr's data pointer + start*elem_size),
/// `dest[2..3] = len` (element count). Bounds must be compile-time constants for
/// now; the array must be a zero-page local (its slot holds the data pointer).
#[allow(clippy::too_many_arguments)]
pub(super) fn generate_slice_materialize(
    dest: u8,
    elem: &crate::sema::types::Type,
    object: &Spanned<crate::ast::Expr>,
    start: &Spanned<crate::ast::Expr>,
    end: &Spanned<crate::ast::Expr>,
    inclusive: bool,
    emitter: &mut Emitter,
    info: &ProgramInfo,
    string_collector: &mut StringCollector,
) -> Result<(), CodegenError> {
    use crate::ast::Expr;
    use crate::sema::const_eval::eval_const_expr_with_env;
    use crate::sema::table::SymbolLocation;

    let elem_size = elem.size().max(1);

    // Resolve the sliced array variable's zero-page slot (holds the data pointer).
    let arr_name = if let Expr::Variable(name) = &object.node {
        name
    } else {
        return Err(CodegenError::UnsupportedOperation(
            "slice source must be an array variable".to_string(),
        ));
    };
    let arr_sym = info
        .resolved_symbols
        .get(&object.span)
        .or_else(|| info.table.lookup(arr_name))
        .ok_or_else(|| CodegenError::SymbolNotFound(arr_name.clone()))?;
    let arr_addr = match arr_sym.location {
        SymbolLocation::ZeroPage(a) => a,
        _ => {
            return Err(CodegenError::UnsupportedOperation(format!(
                "slice source array '{}' must be a zero-page local",
                arr_name
            )));
        }
    };

    let env = HashMap::default();
    let const_s = eval_const_expr_with_env(start, &env)
        .ok()
        .and_then(|v| v.as_integer());
    let const_e = eval_const_expr_with_env(end, &env)
        .ok()
        .and_then(|v| v.as_integer());

    if elem_size > 2 {
        return Err(CodegenError::UnsupportedOperation(
            "slices of elements larger than 2 bytes are not yet supported".to_string(),
        ));
    }

    if let (Some(s), Some(e)) = (const_s, const_e) {
        // Fast path: both bounds are compile-time constants.
        let s = s as usize;
        let actual_end = if inclusive {
            e as usize + 1
        } else {
            e as usize
        };
        let len = actual_end.saturating_sub(s);
        let byte_offset = s * elem_size;

        emitter.emit_comment(&format!(
            "Slice materialize: base = {}+{}, len = {}",
            arr_name, byte_offset, len
        ));
        emitter.emit_inst("LDA", &format!("${:02X}", arr_addr));
        emitter.emit_inst("CLC", "");
        emitter.emit_inst("ADC", &format!("#${:02X}", (byte_offset & 0xFF) as u8));
        emitter.emit_inst("STA", &format!("${:02X}", dest));
        emitter.emit_inst("LDA", &format!("${:02X}", arr_addr + 1));
        emitter.emit_inst(
            "ADC",
            &format!("#${:02X}", ((byte_offset >> 8) & 0xFF) as u8),
        );
        emitter.emit_inst("STA", &format!("${:02X}", dest + 1));
        emitter.emit_inst("LDA", &format!("#${:02X}", (len & 0xFF) as u8));
        emitter.emit_inst("STA", &format!("${:02X}", dest + 2));
        emitter.emit_inst("LDA", &format!("#${:02X}", ((len >> 8) & 0xFF) as u8));
        emitter.emit_inst("STA", &format!("${:02X}", dest + 3));
        emitter.invalidate_registers();
        return Ok(());
    }

    // Runtime path: bounds are computed at run time. Only u8-typed runtime
    // bounds are supported (the arithmetic below is 8-bit); u16 runtime bounds
    // would need 16-bit handling. Constant u16 bounds took the fast path above.
    let bound_is_u16 = |sp: &crate::ast::Span| {
        matches!(
            info.resolved_types.get(sp),
            Some(crate::sema::types::Type::Primitive(
                crate::ast::PrimitiveType::U16 | crate::ast::PrimitiveType::I16
            ))
        )
    };
    if bound_is_u16(&start.span) || bound_is_u16(&end.span) {
        return Err(CodegenError::UnsupportedOperation(
            "runtime 16-bit slice bounds are not yet supported (use constant bounds)".to_string(),
        ));
    }

    emitter.emit_comment(&format!(
        "Slice materialize (runtime bounds) from {}",
        arr_name
    ));

    // Evaluate each bound and spill it to the software stack immediately, so
    // a complex bound (a binary op, a call) cannot clobber the one parked
    // before it — the old code staged `end` at $21 across `start`'s
    // evaluation, and $20/$21 are exactly the temps binary ops write. Once
    // both are reloaded the arithmetic below is straight-line and the
    // hardcoded $20-$23 are safe.
    generate_expr(end, emitter, info, string_collector)?;
    emitter.spill_scalar(1);
    generate_expr(start, emitter, info, string_collector)?;
    emitter.spill_scalar(1);
    emitter.reload_scalar(1); // A = start (pushed last)
    emitter.emit_inst("STA", "$20");
    emitter.reload_scalar(1); // A = end
    emitter.emit_inst("STA", "$21");

    // len = (end - start) [+ 1 for an inclusive range], as a 16-bit value. The
    // +1 can carry into the high byte (e.g. `0..=255` is 256 elements).
    let len_addend = if inclusive { 1 } else { 0 };
    emitter.emit_inst("LDA", "$21");
    emitter.emit_inst("SEC", "");
    emitter.emit_inst("SBC", "$20");
    emitter.emit_inst("CLC", "");
    emitter.emit_inst("ADC", &format!("#${:02X}", len_addend));
    emitter.emit_inst("STA", &format!("${:02X}", dest + 2));
    emitter.emit_inst("LDA", "#$00");
    emitter.emit_inst("ADC", "#$00");
    emitter.emit_inst("STA", &format!("${:02X}", dest + 3));

    // byte offset = start * elem_size, as a 16-bit value in $22/$23.
    emitter.emit_inst("LDA", "$20");
    if elem_size == 2 {
        emitter.emit_inst("ASL", "A"); // start * 2 (low byte, carry = bit 8)
        emitter.emit_inst("STA", "$22");
        emitter.emit_inst("LDA", "#$00");
        emitter.emit_inst("ADC", "#$00"); // capture the carry into the high byte
        emitter.emit_inst("STA", "$23");
    } else {
        emitter.emit_inst("STA", "$22");
        emitter.emit_inst("LDA", "#$00");
        emitter.emit_inst("STA", "$23");
    }

    // base = arr pointer + byte offset (16-bit add).
    emitter.emit_inst("LDA", &format!("${:02X}", arr_addr));
    emitter.emit_inst("CLC", "");
    emitter.emit_inst("ADC", "$22");
    emitter.emit_inst("STA", &format!("${:02X}", dest));
    emitter.emit_inst("LDA", &format!("${:02X}", arr_addr + 1));
    emitter.emit_inst("ADC", "$23");
    emitter.emit_inst("STA", &format!("${:02X}", dest + 1));

    emitter.invalidate_registers();
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(super) fn generate_slice_assignment(
    object: &Spanned<crate::ast::Expr>,
    start: &Spanned<crate::ast::Expr>,
    end: &Spanned<crate::ast::Expr>,
    inclusive: bool,
    value: &Spanned<crate::ast::Expr>,
    emitter: &mut Emitter,
    info: &ProgramInfo,
    string_collector: &mut StringCollector,
) -> Result<(), CodegenError> {
    use crate::ast::{Expr, Literal};
    use crate::sema::const_eval::eval_const_expr_with_env;
    use crate::sema::table::SymbolLocation;

    emitter.emit_comment("Slice assignment");

    // Get the array variable info
    let array_name = if let Expr::Variable(name) = &object.node {
        name
    } else {
        return Err(CodegenError::UnsupportedOperation(
            "Slice assignment only supported on array variables".to_string(),
        ));
    };

    let sym = info
        .resolved_symbols
        .get(&object.span)
        .or_else(|| info.table.lookup(array_name))
        .ok_or_else(|| CodegenError::SymbolNotFound(array_name.clone()))?;

    let addr = match sym.location {
        SymbolLocation::ZeroPage(a) => a,
        SymbolLocation::Absolute(a) if a < 256 => a as u8,
        _ => {
            return Err(CodegenError::UnsupportedOperation(format!(
                "Array '{}' must be in zero page for slice assignment",
                array_name
            )));
        }
    };

    // Try to evaluate slice bounds as constants
    let const_env = HashMap::default();
    let start_val = eval_const_expr_with_env(start, &const_env)
        .ok()
        .and_then(|v| v.as_integer())
        .map(|v| v as usize);
    let end_val = eval_const_expr_with_env(end, &const_env)
        .ok()
        .and_then(|v| v.as_integer())
        .map(|v| v as usize);

    // Get values from RHS (must be an array literal for now)
    let values = match &value.node {
        Expr::Literal(Literal::Array(elems)) => elems,
        _ => {
            return Err(CodegenError::UnsupportedOperation(
                "Slice assignment requires an array literal on the right-hand side".to_string(),
            ));
        }
    };

    // If bounds are constant, we can unroll the assignment
    if let (Some(s), Some(e)) = (start_val, end_val) {
        let actual_end = if inclusive { e + 1 } else { e };
        let slice_len = actual_end - s;

        // Verify slice length matches value array length
        if values.len() != slice_len {
            return Err(CodegenError::UnsupportedOperation(format!(
                "Slice length ({}) does not match value array length ({})",
                slice_len,
                values.len()
            )));
        }

        emitter.emit_comment(&format!(
            "Unrolled slice assignment [{}..{}]",
            s, actual_end
        ));

        // Determine element width: a two-byte element indexes by a scaled byte
        // offset (element index * 2), matching generate_index_assignment.
        // Truncating to one byte and using the raw element index silently
        // corrupts u16 (and pointer) arrays.
        use crate::sema::types::Type;
        let element_is_multibyte = matches!(
            info.resolved_types.get(&object.span),
            Some(Type::Array(elem, _)) if crate::codegen::expr::is_two_byte_value(elem)
        );
        let elem_size = if element_is_multibyte { 2usize } else { 1usize };

        // Unroll: generate individual stores for each element
        for (i, val_expr) in values.iter().enumerate() {
            let byte_offset = (s + i) * elem_size;
            if byte_offset + elem_size - 1 > 0xFF {
                return Err(CodegenError::UnsupportedOperation(format!(
                    "slice assignment byte offset {} exceeds zero-page indirect range",
                    byte_offset
                )));
            }

            // Generate the value expression (A = low byte, Y = high byte for u16)
            generate_expr(val_expr, emitter, info, string_collector)?;

            if element_is_multibyte {
                // Y holds the high byte but we need Y for the store index, so
                // stash both bytes first, then store low then high.
                emitter.emit_inst("STA", "$20");
                emitter.emit_inst("STY", "$21");
                emitter.emit_inst("LDY", &format!("#${:02X}", byte_offset));
                emitter.emit_inst("LDA", "$20");
                emitter.emit_inst("STA", &format!("(${:02X}),Y", addr));
                emitter.emit_inst("INY", "");
                emitter.emit_inst("LDA", "$21");
                emitter.emit_inst("STA", &format!("(${:02X}),Y", addr));
            } else {
                // Store to array[target_index] using indirect indexed addressing
                emitter.emit_inst("LDY", &format!("#${:02X}", byte_offset));
                emitter.emit_inst("STA", &format!("(${:02X}),Y", addr));
            }
        }

        // Raw A/Y stores above bypass register tracking; drop cached beliefs.
        emitter.invalidate_registers();
    } else {
        // Dynamic bounds - not supported yet
        return Err(CodegenError::UnsupportedOperation(
            "Slice assignment with non-constant bounds is not yet supported".to_string(),
        ));
    }

    Ok(())
}

pub(super) fn generate_field_assignment(
    object: &Spanned<crate::ast::Expr>,
    field: &Spanned<String>,
    value: &Spanned<crate::ast::Expr>,
    emitter: &mut Emitter,
    info: &ProgramInfo,
    string_collector: &mut StringCollector,
) -> Result<(), CodegenError> {
    use crate::ast::Expr;
    use crate::sema::table::SymbolLocation;
    use crate::sema::types::Type;

    // arr[i].field = x with a runtime index: absolute,Y indexed store.
    if let Expr::Index {
        object: array,
        index,
    } = &object.node
        && crate::codegen::expr::resolve_static_struct_lvalue(object, info).is_none()
        && crate::codegen::expr::emit_array_struct_field_indexed(
            array,
            index,
            field,
            Some(value),
            emitter,
            info,
            string_collector,
        )?
    {
        return Ok(());
    }

    // Nested (a.b.c = x) or array-of-struct (arr[const].f = x) target: resolve a
    // static address for the local struct chain, then store the value there.
    if !matches!(&object.node, Expr::Variable(_))
        && let Some((base, struct_name)) =
            crate::codegen::expr::resolve_static_struct_lvalue(object, info)
    {
        let field_info = info
            .type_registry
            .get_struct(&struct_name)
            .and_then(|s| s.get_field(&field.node).cloned())
            .ok_or_else(|| {
                CodegenError::UnsupportedOperation(format!(
                    "field '{}' not found in struct '{}'",
                    field.node, struct_name
                ))
            })?;
        // Sema rejects assigning through a `const`, so a read-only base here
        // would mean a store quietly aimed at ROM.
        if base.is_read_only() {
            return Err(CodegenError::UnsupportedOperation(
                "cannot write to constant data".to_string(),
            ));
        }
        emitter.emit_comment(&format!("Nested field assignment: .{}", field.node));
        generate_expr(value, emitter, info, string_collector)?;
        // Function-pointer fields are 2-byte code addresses stored as a pair like
        // u16 — a device vtable depends on it; `store_value_pair` handles the width.
        let at = base.plus(field_info.offset as u16);
        store_value_pair(emitter, &field_info.ty, &at.operand(0), &at.operand(1));
        return Ok(());
    }

    // Get the base object (must be a variable for now)
    if let Expr::Variable(var_name) = &object.node {
        // Look up the variable
        let sym = info
            .resolved_symbols
            .get(&object.span)
            .or_else(|| info.table.lookup(var_name))
            .ok_or_else(|| CodegenError::SymbolNotFound(var_name.clone()))?;

        // Get the base address of the struct. A `&Struct` holds a 2-byte
        // address in its slot exactly as a struct parameter does, so it takes
        // the same indirect store path — that is what makes `p.field = v`
        // auto-deref.
        let sym_is_param = sym.is_param || matches!(sym.ty, Type::Pointer(_));
        let base_addr = match sym.location {
            SymbolLocation::ZeroPage(addr) => addr as u16,
            SymbolLocation::Absolute(addr) => addr,
            _ => {
                return Err(CodegenError::UnsupportedOperation(format!(
                    "Cannot assign to field of variable with location: {:?}",
                    sym.location
                )));
            }
        };

        // Get the struct type name from the symbol's type, looking through one
        // level of pointer.
        let struct_name = match &sym.ty {
            Type::Named(name) => name,
            Type::Pointer(inner) => match &**inner {
                Type::Named(name) => name,
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

        // Check if field is multi-byte
        // Function-pointer fields hold a 2-byte code address, so they are stored
        // as a pair like u16 — a device vtable depends on it.
        let is_multibyte = crate::codegen::expr::is_two_byte_value(&field_info.ty);

        emitter.emit_comment(&format!("Field assignment: {}.{}", var_name, field.node));

        // Check if this is a parameter (pass-by-reference) via the explicit flag.
        let is_parameter = sym_is_param;

        // Generate value expression (result in A, or A/Y for u16)
        generate_expr(value, emitter, info, string_collector)?;

        if is_parameter {
            // The struct pointer lives directly in this parameter's frame slot;
            // frame coloring guarantees nested calls cannot clobber it.
            let ptr_addr = base_addr as u8;

            // Use indirect indexed addressing: STA ($ptr),Y
            // Need to save A first since we'll need Y for the offset
            let offset = field_info.offset;

            // Save value to temp
            store_value_pair(emitter, &field_info.ty, "$20", "$21");

            // Set Y to field offset and store via indirect
            emitter.emit_inst("LDY", &format!("#${:02X}", offset));
            emitter.emit_inst("LDA", "$20"); // Restore value
            emitter.emit_inst("STA", &format!("(${:02X}),Y", ptr_addr));

            if is_multibyte {
                // Store high byte at next offset
                emitter.emit_inst("INY", "");
                emitter.emit_inst("LDA", "$21");
                emitter.emit_inst("STA", &format!("(${:02X}),Y", ptr_addr));
            }
        } else {
            // Local struct - direct access
            let field_addr = base_addr + field_info.offset as u16;

            if field_addr < 0x100 {
                store_value_pair(
                    emitter,
                    &field_info.ty,
                    &format!("${:02X}", field_addr),
                    &format!("${:02X}", field_addr + 1),
                );
            } else {
                store_value_pair(
                    emitter,
                    &field_info.ty,
                    &format!("${:04X}", field_addr),
                    &format!("${:04X}", field_addr + 1),
                );
            }
        }

        // Both paths above rewrite A/Y through raw instructions (temp saves in the
        // parameter path, plain stores in the local path) that don't feed register
        // tracking, so drop all cached beliefs — a following load could otherwise be
        // wrongly elided. Mirrors generate_index_assignment.
        emitter.invalidate_registers();

        Ok(())
    } else {
        Err(CodegenError::UnsupportedOperation(
            "Field assignment only supported on variables (not expressions)".to_string(),
        ))
    }
}

/// Which register holds the high byte of a value about to be stored into a
/// struct field. A `&T` arrives in A:X like every other pointer-like value;
/// a u16 or a function pointer arrives in A:Y.
fn store_high(ty: &crate::sema::types::Type) -> &'static str {
    if crate::codegen::expr::high_byte_in_x(ty) {
        "STX"
    } else {
        "STY"
    }
}

/// Store a value in A (low byte) to `lo`, and — for a two-byte `ty` — its high
/// byte (from X or Y per `high_byte_in_x`) to `hi`. `lo`/`hi` are already-formed
/// operands, so the same store shape serves zero-page, absolute, and indirect
/// destinations. The one place that decides a value's store width and high-byte
/// register.
fn store_value_pair(emitter: &mut Emitter, ty: &crate::sema::types::Type, lo: &str, hi: &str) {
    emitter.emit_inst("STA", lo);
    if crate::codegen::expr::is_two_byte_value(ty) {
        emitter.emit_inst(store_high(ty), hi);
    }
}

/// Emit `*p = value`.
///
/// The pointer is staged first and the value parked in the high pool, mirroring
/// `generate_index_assignment`: evaluating the value can clobber whatever is in
/// zero page, so the two cannot simply be produced in order.
pub(super) fn generate_deref_assignment(
    ptr_expr: &Spanned<crate::ast::Expr>,
    value: &Spanned<crate::ast::Expr>,
    emitter: &mut Emitter,
    info: &ProgramInfo,
    string_collector: &mut StringCollector,
) -> Result<(), CodegenError> {
    use crate::sema::table::SymbolLocation;
    use crate::sema::types::Type;

    // The pointee's ABI: a two-byte pointee (u16/i16/b16, but also a `&T` or a
    // function pointer — a `&&u8` or `&fn(..)`) stores both bytes. Its high byte
    // arrives in X for a pointer/function value and in Y for a 16-bit scalar, so
    // ask the shared predicates rather than re-listing the variants here — the
    // list that omitted pointers is exactly what dropped the high byte of
    // `*pp = q`.
    let pointee = match info.resolved_types.get(&ptr_expr.span) {
        Some(Type::Pointer(inner)) => Some(inner.as_ref()),
        _ => None,
    };
    let is_multibyte = pointee.is_some_and(crate::codegen::expr::is_two_byte_value);
    let high_in_x = pointee.is_some_and(crate::codegen::expr::high_byte_in_x);
    let width: u8 = if is_multibyte { 2 } else { 1 };

    // Park the value while the pointer is set up. Not $20/$21 — those are
    // hardcoded by several other paths — but the high pool, as the indexed
    // store does.
    let save = emitter
        .temp_alloc
        .alloc_high(width)
        .ok_or_else(|| CodegenError::Internal("no temp space for a pointer store".to_string()))?;

    emitter.emit_comment("Store through pointer");
    generate_expr(value, emitter, info, string_collector)?;
    emitter.emit_inst("STA", &format!("${:02X}", save));
    if is_multibyte {
        // High byte: X for a pointer/function value, Y for a 16-bit scalar.
        let reg = if high_in_x { "STX" } else { "STY" };
        emitter.emit_inst(reg, &format!("${:02X}", save + 1));
    }

    // Resolve the pointer into a zero-page pair we can use with `(zp),Y`.
    let ptr = if let crate::ast::Expr::Variable(_) = &ptr_expr.node
        && let Some(sym) = info.resolved_symbols.get(&ptr_expr.span)
        && let SymbolLocation::ZeroPage(addr) = sym.location
    {
        addr
    } else {
        generate_expr(ptr_expr, emitter, info, string_collector)?;
        let staged = emitter.memory_layout.deref_ptr();
        emitter.emit_inst("STA", &format!("${:02X}", staged));
        emitter.emit_inst("STX", &format!("${:02X}", staged + 1));
        staged
    };

    for k in 0..width {
        emitter.emit_inst("LDY", &format!("#${:02X}", k));
        emitter.emit_inst("LDA", &format!("${:02X}", save + k));
        emitter.emit_inst("STA", &format!("(${:02X}),Y", ptr));
    }

    emitter.temp_alloc.free_high(save, width);
    // An indirect store can land anywhere; no cached belief about zero page
    // survives it.
    emitter.invalidate_registers();
    Ok(())
}

/// Fill a local array's RAM block from its initializer.
///
/// The data used to be emitted once, as bytes in the code stream. Now it lives
/// in RAM and has to be written at run time, on every call — an unavoidable cost
/// of the array being writable at all. A uniform fill becomes a loop when it is
/// worth one; explicit elements are stored individually.
pub(super) fn generate_local_array_init(
    addr: u16,
    size: u16,
    elem_size: usize,
    init: &Spanned<crate::ast::Expr>,
    emitter: &mut Emitter,
    info: &ProgramInfo,
) -> Result<(), CodegenError> {
    use crate::ast::{Expr, Literal};
    use crate::sema::const_eval::eval_const_expr;

    /// A constant initializer element, or None if it is not compile-time known.
    fn const_of(e: &Spanned<Expr>) -> Option<i64> {
        match &e.node {
            Expr::Literal(Literal::Integer(v)) => Some(*v),
            Expr::Literal(Literal::Bool(b)) => Some(i64::from(*b)),
            _ => eval_const_expr(e).ok().and_then(|c| c.as_integer()),
        }
    }

    /// Store `value` as `elem_size` little-endian bytes at `addr + offset`.
    fn store_elem(emitter: &mut Emitter, addr: u16, offset: u16, value: i64, elem_size: usize) {
        for b in 0..elem_size {
            let byte = ((value >> (8 * b)) & 0xFF) as u8;
            emitter.emit_inst("LDA", &format!("#${:02X}", byte));
            emitter.emit_inst("STA", &format!("${:04X}", addr + offset + b as u16));
        }
    }

    /// Store a function's 2-byte code address at `addr + offset`. The address is
    /// a label the assembler resolves, so it is written as `#<f` / `#>f` rather
    /// than a known constant — this is how a *local* function-pointer table
    /// (`let handlers = [d0, d1]`) is initialized.
    fn store_fn_elem(emitter: &mut Emitter, addr: u16, offset: u16, label: &str) {
        emitter.emit_inst("LDA", &format!("#<{label}"));
        emitter.emit_inst("STA", &format!("${:04X}", addr + offset));
        emitter.emit_inst("LDA", &format!("#>{label}"));
        emitter.emit_inst("STA", &format!("${:04X}", addr + offset + 1));
    }

    // The function name an element refers to, if it is a bare function pointer.
    let fn_label_of = |e: &Spanned<Expr>| -> Option<String> {
        use crate::sema::table::SymbolKind;
        use crate::sema::types::Type;
        if let Expr::Variable(n) = &e.node {
            let sym = info
                .resolved_symbols
                .get(&e.span)
                .or_else(|| info.table.lookup(n))?;
            if sym.kind == SymbolKind::Function || matches!(sym.ty, Type::Function(..)) {
                return Some(n.clone());
            }
        }
        None
    };

    match &init.node {
        // `[v; n]` — every element the same. A byte-wide zero (or any uniform
        // byte) over a block worth looping for becomes a loop; otherwise the
        // stores are cheaper than the loop overhead.
        Expr::Literal(Literal::ArrayFill { value, count }) => {
            // A `[f; n]` fill of one function name — every slot the same driver.
            if let Some(label) = fn_label_of(value) {
                for i in 0..*count as u16 {
                    store_fn_elem(emitter, addr, i * elem_size as u16, &label);
                }
                emitter.invalidate_registers();
                return Ok(());
            }
            let v = const_of(value).ok_or_else(|| {
                CodegenError::UnsupportedOperation(
                    "array fill value must be a constant expression or a function name".to_string(),
                )
            })?;
            let uniform_byte = (0..elem_size).all(|b| ((v >> (8 * b)) & 0xFF) == (v & 0xFF));
            if uniform_byte && size > 8 {
                emitter.emit_inst("LDA", &format!("#${:02X}", (v & 0xFF) as u8));
                // Absolute,X reaches 256 bytes, so a larger block needs one loop
                // per chunk. X counts down and the branch fires on the wrap from
                // $00 to $FF, which is what covers index 0.
                let mut done = 0u16;
                while done < size {
                    let chunk = (size - done).min(256);
                    let loop_label = emitter.next_label("ai");
                    emitter.emit_inst("LDX", &format!("#${:02X}", (chunk - 1) as u8));
                    emitter.emit_label(&loop_label);
                    emitter.emit_inst("STA", &format!("${:04X},X", addr + done));
                    emitter.emit_inst("DEX", "");
                    emitter.emit_inst("CPX", "#$FF");
                    emitter.emit_inst("BNE", &loop_label);
                    done += chunk;
                }
                emitter.invalidate_registers();
            } else {
                for i in 0..*count as u16 {
                    store_elem(emitter, addr, i * elem_size as u16, v, elem_size);
                }
            }
        }
        // `[a, b, c]` — element by element.
        Expr::Literal(Literal::Array(elements)) => {
            for (i, e) in elements.iter().enumerate() {
                let offset = i as u16 * elem_size as u16;
                if let Some(v) = const_of(e) {
                    store_elem(emitter, addr, offset, v, elem_size);
                } else if let Some(label) = fn_label_of(e) {
                    store_fn_elem(emitter, addr, offset, &label);
                } else {
                    return Err(CodegenError::UnsupportedOperation(
                        "array elements must be constant expressions or function names".to_string(),
                    ));
                }
            }
        }
        _ => {
            let _ = info;
            return Err(CodegenError::UnsupportedOperation(
                "a local array must be initialized with an array literal".to_string(),
            ));
        }
    }
    emitter.invalidate_registers();
    Ok(())
}

pub(super) fn generate_var_decl(
    name: &Spanned<String>,
    init: &Spanned<crate::ast::Expr>,
    emitter: &mut Emitter,
    info: &ProgramInfo,
    string_collector: &mut StringCollector,
) -> Result<(), CodegenError> {
    // Look up variable info first
    if let Some(sym) = info.resolved_symbols.get(&name.span) {
        use crate::sema::table::SymbolKind;
        use crate::sema::types::Type;

        // A `str<N>` buffer: fill its RAM block from the literal, then
        // point the frame slot at the block. Downstream, the slot reads
        // as a 2-byte pointer to `[len][bytes]` — identical to a str
        // literal binding — so every str read path works unchanged. The
        // difference is the data lives in RAM and can be edited.
        if let Some(buf) = info.string_buffers.get(&name.span)
            && let crate::sema::table::SymbolLocation::ZeroPage(slot) = sym.location
        {
            let bytes: Vec<u8> = match &init.node {
                crate::ast::Expr::Literal(crate::ast::Literal::String(s)) => s.as_bytes().to_vec(),
                // sema requires a string-literal initializer for a buffer
                _ => Vec::new(),
            };
            let len = bytes.len() as u8;
            emitter.emit_comment(&format!(
                "str<{}> buffer {} @ ${:04X} ({}/{} bytes, RAM)",
                buf.size - 1,
                name.node,
                buf.addr,
                len,
                buf.size - 1
            ));
            // Length prefix, then each content byte as an immediate store.
            emitter.emit_inst("LDA", &format!("#${:02X}", len));
            emitter.emit_inst("STA", &format!("${:04X}", buf.addr));
            for (i, b) in bytes.iter().enumerate() {
                emitter.emit_inst("LDA", &format!("#${:02X}", b));
                emitter.emit_inst("STA", &format!("${:04X}", buf.addr as usize + 1 + i));
            }
            // Point the frame slot at the block.
            emitter.emit_inst("LDA", &format!("#${:02X}", buf.addr & 0xFF));
            emitter.emit_inst("STA", &format!("${:02X}", slot));
            emitter.emit_inst("LDA", &format!("#${:02X}", buf.addr >> 8));
            emitter.emit_inst("STA", &format!("${:02X}", slot + 1));
            emitter.invalidate_registers();
            return Ok(());
        }

        // Check if this is a struct variable initialized with a struct literal
        // Use runtime initialization for struct literals only (not enum variants)
        if let Type::Named(struct_name) = &sym.ty {
            // Only use runtime init if the init expression is a struct literal
            let is_struct_literal = matches!(
                &init.node,
                crate::ast::Expr::StructInit { .. } | crate::ast::Expr::AnonStructInit { .. }
            );

            // Also verify this is actually a struct type (not an enum)
            let is_struct_type = info.type_registry.get_struct(struct_name).is_some();

            if is_struct_literal
                && is_struct_type
                && let crate::sema::table::SymbolLocation::ZeroPage(addr) = sym.location
            {
                // Get fields from the init expression
                let fields = match &init.node {
                    crate::ast::Expr::StructInit { fields, .. } => fields,
                    crate::ast::Expr::AnonStructInit { fields } => fields,
                    _ => unreachable!(),
                };

                // Use runtime struct initialization directly to ZP address
                crate::codegen::expr::generate_struct_init_runtime(
                    struct_name,
                    fields,
                    addr as u16,
                    emitter,
                    info,
                    string_collector,
                )?;
                return Ok(());
            }

            // Struct-by-value initialization from a call, e.g.
            // `let p: Point = make();`. A struct-returning function
            // leaves a pointer to the struct bytes in A:X; copy the
            // whole struct into this local's inline storage (frame
            // coloring keeps the returned pointer valid until the next
            // call, and the copy is the first thing after the call).
            if is_struct_type
                && matches!(&init.node, crate::ast::Expr::Call { .. })
                && let crate::sema::table::SymbolLocation::ZeroPage(dest) = sym.location
                && let Some(sdef) = info.type_registry.get_struct(struct_name)
            {
                let total = sdef.total_size as u8;
                emitter.emit_comment(&format!(
                    "Struct return-by-value: copy {} bytes into ${:02X}",
                    total, dest
                ));
                generate_expr(init, emitter, info, string_collector)?;
                emit_return_by_value_copy(emitter, dest, total);
                emitter.invalidate_registers();
                return Ok(());
            }
        }

        // Local array: the data lives in RAM (see `LocalArray`), not
        // inline in the code stream where it used to be emitted and
        // where writes would have gone to ROM on a real board. Fill the
        // block from the initializer, then put its address in the
        // frame slot — every downstream path still reads the slot as a
        // 2-byte pointer, so indexing is unchanged.
        if let Some(arr) = info.local_arrays.get(&name.span)
            && let crate::sema::table::SymbolLocation::ZeroPage(slot) = sym.location
        {
            let elem_size = match &sym.ty {
                Type::Array(elem, _) => crate::codegen::expr::type_byte_size(elem, info).max(1),
                _ => 1,
            };
            emitter.emit_comment(&format!(
                "local array {} @ ${:04X} ({} bytes, RAM)",
                name.node, arr.addr, arr.size
            ));
            generate_local_array_init(arr.addr, arr.size, elem_size, init, emitter, info)?;
            // Point the frame slot at the block.
            emitter.emit_inst("LDA", &format!("#${:02X}", arr.addr & 0xFF));
            emitter.emit_inst("STA", &format!("${:02X}", slot));
            emitter.emit_inst("LDA", &format!("#${:02X}", arr.addr >> 8));
            emitter.emit_inst("STA", &format!("${:02X}", slot + 1));
            emitter.invalidate_registers();
            return Ok(());
        }

        // Slice value: `let s: &[T] = arr[start..end];`. Materialize the
        // 4-byte fat-pointer descriptor into s's frame slot:
        //   slot[0..1] = base = arr's data pointer + start*elem_size
        //   slot[2..3] = len  = (end - start) in elements
        if let Type::Slice(elem) = &sym.ty
            && let crate::ast::Expr::Slice {
                object,
                start,
                end,
                inclusive,
            } = &init.node
            && let crate::sema::table::SymbolLocation::ZeroPage(dest) = sym.location
        {
            generate_slice_materialize(
                dest,
                elem,
                object,
                start,
                end,
                *inclusive,
                emitter,
                info,
                string_collector,
            )?;
            return Ok(());
        }

        // Slice returned from a call: the callee left a pointer to its
        // 4-byte descriptor in A:X; copy the descriptor into this slot.
        if let Type::Slice(_) = &sym.ty
            && matches!(&init.node, crate::ast::Expr::Call { .. })
            && let crate::sema::table::SymbolLocation::ZeroPage(dest) = sym.location
        {
            emitter.emit_comment("Slice return-by-value: copy 4-byte descriptor");
            generate_expr(init, emitter, info, string_collector)?;
            emit_return_by_value_copy(emitter, dest, 4);
            emitter.invalidate_registers();
            return Ok(());
        }

        // Array of structs: stored inline. Runtime-initialize each element
        // struct literal directly at addr + i*element_size.
        if let Type::Array(elem, _n) = &sym.ty
            && let Type::Named(elem_struct) = &**elem
            && let Some(sdef) = info.type_registry.get_struct(elem_struct)
            && let crate::ast::Expr::Literal(crate::ast::Literal::Array(elements)) = &init.node
            && let crate::sema::table::SymbolLocation::ZeroPage(base) = sym.location
        {
            let elem_size = sdef.total_size as u8;
            emitter.emit_comment(&format!(
                "Array of {} {}: {} elements inline at ${:02X}",
                elem_struct,
                "structs",
                elements.len(),
                base
            ));
            for (i, elem_expr) in elements.iter().enumerate() {
                let elem_addr = base + (i as u8) * elem_size;
                let fields = match &elem_expr.node {
                    crate::ast::Expr::StructInit { fields, .. }
                    | crate::ast::Expr::AnonStructInit { fields } => fields,
                    _ => {
                        return Err(CodegenError::UnsupportedOperation(
                            "array-of-struct elements must be struct literals".to_string(),
                        ));
                    }
                };
                crate::codegen::expr::generate_struct_init_runtime(
                    elem_struct,
                    fields,
                    elem_addr as u16,
                    emitter,
                    info,
                    string_collector,
                )?;
            }
            return Ok(());
        }

        // Check for shorthand array syntax: [value] expanding to [value, value, ...]
        // If init is a single-element array and target is a larger array, synthesize an ArrayFill
        let modified_init;
        let init_expr = if let Type::Array(_, target_size) = &sym.ty {
            if let crate::ast::Expr::Literal(crate::ast::Literal::Array(elements)) = &init.node {
                if elements.len() == 1 && *target_size > 1 {
                    // Shorthand syntax detected! Convert to ArrayFill
                    emitter
                        .emit_comment(&format!("Expanding [value] to [{} elements]", target_size));
                    modified_init = crate::ast::Spanned {
                        node: crate::ast::Expr::Literal(crate::ast::Literal::ArrayFill {
                            value: Box::new(elements[0].clone()),
                            count: *target_size,
                        }),
                        span: init.span,
                    };
                    &modified_init
                } else {
                    init
                }
            } else {
                init
            }
        } else {
            init
        };

        // Generate initialization expression (result in A, and X if u16)
        generate_expr(init_expr, emitter, info, string_collector)?;

        // Check if we need to zero-extend (u8 -> u16)
        // Get the init expression type from resolved_types
        let init_type = info.resolved_types.get(&init.span);
        let target_type = &sym.ty;

        let needs_zero_extend = if let Some(init_ty) = init_type {
            matches!(init_ty, Type::Primitive(crate::ast::PrimitiveType::U8))
                && matches!(
                    target_type,
                    Type::Primitive(crate::ast::PrimitiveType::U16)
                        | Type::Primitive(crate::ast::PrimitiveType::I16)
                        | Type::Primitive(crate::ast::PrimitiveType::B16)
                )
        } else {
            false
        };

        // If we need to zero-extend, set Y=0 for the high byte
        if needs_zero_extend {
            emitter.emit_inst("LDY", "#$00");
        }

        // Check if this is a multi-byte type (arrays, u16, i16, b16, enums)
        // Enums store a 2-byte pointer like arrays
        let is_enum = if let Type::Named(type_name) = &sym.ty {
            info.type_registry.get_enum(type_name).is_some()
        } else {
            false
        };

        let is_multibyte = matches!(
            sym.ty,
            Type::Array(_, _)
                | Type::String
                | Type::Pointer(_)
                | Type::Function(_, _)
                | Type::Primitive(crate::ast::PrimitiveType::U16)
                | Type::Primitive(crate::ast::PrimitiveType::I16)
                | Type::Primitive(crate::ast::PrimitiveType::B16)
        ) || is_enum;

        // Arrays, enums, strings and pointers carry an address in
        // A (low) and X (high); other 16-bit values use A (low) and
        // Y (high). Storing only A leaves the high byte as whatever was
        // in the slot, which reads as a pointer into an arbitrary page.
        let is_array_or_enum =
            matches!(sym.ty, Type::Array(_, _) | Type::String | Type::Pointer(_)) || is_enum;

        // An enum value binds by *copy*: the constructed bytes live in
        // shared codegen scratch until here (and a returned enum points
        // into the callee's reused region), so without the copy two
        // live enums alias and any later temp use destroys the payload.
        // The variable's slot points at its own per-declaration block
        // from here on.
        let enum_block = if is_enum {
            info.enum_blocks.get(&name.span).cloned()
        } else {
            None
        };
        if let Some(block) = &enum_block {
            emit_enum_copy_to_block(block, emitter)?;
        }

        match sym.location {
            crate::sema::table::SymbolLocation::FrameOffset(_) => {
                return Err(CodegenError::Internal(
                    "unresolved FrameOffset reached codegen (frame finalization skipped)"
                        .to_string(),
                ));
            }
            crate::sema::table::SymbolLocation::Absolute(addr) => {
                // Check if this is an address declaration - use symbolic name
                if sym.kind == SymbolKind::Address {
                    emitter.emit_sta_symbol(&name.node);
                } else {
                    emitter.emit_sta_abs(addr);
                    // For multi-byte types, also store high byte
                    if is_multibyte {
                        let hi_inst = if is_array_or_enum { "STX" } else { "STY" };
                        emitter.emit_inst(hi_inst, &format!("${:04X}", addr + 1));
                    }
                }
            }
            crate::sema::table::SymbolLocation::ZeroPage(addr) => {
                emitter.emit_sta_zp(addr);
                // For multi-byte types, also store high byte
                if is_multibyte {
                    let hi_inst = if is_array_or_enum { "STX" } else { "STY" };
                    emitter.emit_inst(hi_inst, &format!("${:02X}", addr + 1));
                }
            }
            crate::sema::table::SymbolLocation::None => {
                return Err(CodegenError::UnsupportedOperation(format!(
                    "VarDecl '{}' has no storage location",
                    name.node
                )));
            }
        }
    } else {
        return Err(CodegenError::SymbolNotFound(name.node.clone()));
    }
    Ok(())
}

pub(super) fn generate_assign(
    target: &Spanned<crate::ast::Expr>,
    value: &Spanned<crate::ast::Expr>,
    emitter: &mut Emitter,
    info: &ProgramInfo,
    string_collector: &mut StringCollector,
) -> Result<(), CodegenError> {
    // Slice reassignment: `s = arr[a..b];` materializes a new descriptor
    // into the slice variable's slot (same as the `let` form).
    if let crate::ast::Expr::Variable(target_name) = &target.node
        && let crate::ast::Expr::Slice {
            object,
            start,
            end,
            inclusive,
        } = &value.node
        && let Some(sym) = info
            .resolved_symbols
            .get(&target.span)
            .or_else(|| info.table.lookup(target_name))
        && let crate::sema::types::Type::Slice(elem) = &sym.ty
        && let crate::sema::table::SymbolLocation::ZeroPage(dest) = sym.location
    {
        generate_slice_materialize(
            dest,
            elem,
            object,
            start,
            end,
            *inclusive,
            emitter,
            info,
            string_collector,
        )?;
        return Ok(());
    }

    // Optimization: detect x = x + 1 and x = x - 1 patterns
    // Use INC/DEC instead of LDA/ADC/STA or LDA/SBC/STA
    if let crate::ast::Expr::Variable(target_name) = &target.node
        && let crate::ast::Expr::Binary { left, op, right } = &value.node
    {
        // Check if left side is the same variable as target
        if let crate::ast::Expr::Variable(left_name) = &left.node
            && left_name == target_name
        {
            // Check if right side is literal 1
            if let crate::ast::Expr::Literal(crate::ast::Literal::Integer(n)) = &right.node
                && *n == 1
            {
                // Look up variable location
                let sym = info
                    .resolved_symbols
                    .get(&target.span)
                    .or_else(|| info.table.lookup(target_name));

                // INC/DEC operate on a single byte and do not touch the
                // carry, so they cannot implement ±1 on a multi-byte
                // value: `a = a + 1` on a u16 holding $00FF must carry
                // into the high byte, which INC alone never does.
                let is_single_byte = sym.is_some_and(|s| {
                    matches!(
                        s.ty,
                        crate::sema::types::Type::Primitive(
                            crate::ast::PrimitiveType::U8
                                | crate::ast::PrimitiveType::I8
                                | crate::ast::PrimitiveType::B8
                                | crate::ast::PrimitiveType::Bool
                        )
                    )
                });

                if let Some(sym) = sym
                    && is_single_byte
                {
                    match (op, &sym.location) {
                        (
                            crate::ast::BinaryOp::Add,
                            crate::sema::table::SymbolLocation::ZeroPage(addr),
                        ) => {
                            // x = x + 1 -> INC $addr
                            emitter.emit_inst("INC", &format!("${:02X}", *addr));
                            emitter.reg_state.invalidate_zero_page(*addr);
                            return Ok(());
                        }
                        (
                            crate::ast::BinaryOp::Add,
                            crate::sema::table::SymbolLocation::Absolute(addr),
                        ) => {
                            // x = x + 1 -> INC $addr
                            emitter.emit_inst("INC", &format!("${:04X}", *addr));
                            emitter.reg_state.invalidate_memory(*addr);
                            return Ok(());
                        }
                        (
                            crate::ast::BinaryOp::Sub,
                            crate::sema::table::SymbolLocation::ZeroPage(addr),
                        ) => {
                            // x = x - 1 -> DEC $addr
                            emitter.emit_inst("DEC", &format!("${:02X}", *addr));
                            emitter.reg_state.invalidate_zero_page(*addr);
                            return Ok(());
                        }
                        (
                            crate::ast::BinaryOp::Sub,
                            crate::sema::table::SymbolLocation::Absolute(addr),
                        ) => {
                            // x = x - 1 -> DEC $addr
                            emitter.emit_inst("DEC", &format!("${:04X}", *addr));
                            emitter.reg_state.invalidate_memory(*addr);
                            return Ok(());
                        }
                        _ => {
                            // Not an INC/DEC pattern, fall through to normal codegen
                        }
                    }
                }
            }
        }
    }

    // `*p = v` — write through a pointer. Handled before the value is
    // evaluated below, because the pointer has to be staged first and
    // the generic path assumes the target is a name.
    if let crate::ast::Expr::Unary {
        op: crate::ast::UnaryOp::Deref,
        operand,
    } = &target.node
    {
        return generate_deref_assignment(operand, value, emitter, info, string_collector);
    }

    // 1. Generate code for value (result in A)
    generate_expr(value, emitter, info, string_collector)?;

    // 2. Store A into target
    // We need a helper to generate store instructions based on target
    match &target.node {
        crate::ast::Expr::Variable(name) => {
            // Look up by span in resolved_symbols first (for local vars)
            let sym = info
                .resolved_symbols
                .get(&target.span)
                .or_else(|| info.table.lookup(name)); // Fallback to global table

            if let Some(sym) = sym {
                use crate::sema::table::SymbolKind;
                use crate::sema::types::Type;

                // Struct-by-value assignment from a call (`p = make();`) or
                // from a computed struct literal (`p = P { x: a + 1 };`). The
                // value expression (already generated above) left a pointer to
                // the struct bytes in A:X; copy the whole struct into the
                // target's inline storage rather than storing just the low byte
                // of the pointer.
                if crate::codegen::expr::yields_struct_pointer(value, info)
                    && let Type::Named(sname) = &sym.ty
                    && let Some(sdef) = info.type_registry.get_struct(sname)
                    && let crate::sema::table::SymbolLocation::ZeroPage(dest) = sym.location
                {
                    let total = sdef.total_size as u8;
                    emitter.emit_comment(&format!(
                        "Struct return-by-value assign: copy {} bytes into ${:02X}",
                        total, dest
                    ));
                    emit_return_by_value_copy(emitter, dest, total);
                    emitter.invalidate_registers();
                    return Ok(());
                }

                // Check if this is an enum type
                let is_enum = if let Type::Named(type_name) = &sym.ty {
                    info.type_registry.get_enum(type_name).is_some()
                } else {
                    false
                };

                // Check if this is a multi-byte type (u16/i16/b16, arrays,
                // enums, function pointers)
                let is_multibyte = matches!(
                    sym.ty,
                    Type::Array(_, _)
                        | Type::Pointer(_)
                        | Type::Function(_, _)
                        | Type::Primitive(crate::ast::PrimitiveType::U16)
                        | Type::Primitive(crate::ast::PrimitiveType::I16)
                        | Type::Primitive(crate::ast::PrimitiveType::B16)
                ) || is_enum;

                // Arrays, enums and pointers carry an address in
                // A (low) and X (high); other 16-bit values use A:Y.
                let is_array_or_enum =
                    matches!(sym.ty, Type::Array(_, _) | Type::Pointer(_)) || is_enum;

                // Same copy-on-bind as the VarDecl path: an enum
                // reassignment must land in the variable's own block,
                // found through the declaration's span. A target
                // without a block (a parameter) keeps the old
                // pointer-store behavior.
                if is_enum
                    && let Some(block) = sym
                        .decl_span
                        .and_then(|sp| info.enum_blocks.get(&sp))
                        .cloned()
                {
                    emit_enum_copy_to_block(&block, emitter)?;
                }

                match sym.location {
                    crate::sema::table::SymbolLocation::FrameOffset(_) => {
                        return Err(CodegenError::Internal(
                            "unresolved FrameOffset reached codegen (frame finalization skipped)"
                                .to_string(),
                        ));
                    }
                    crate::sema::table::SymbolLocation::Absolute(addr) => {
                        // Check if this is an address declaration - use symbolic name
                        if sym.kind == SymbolKind::Address {
                            emitter.emit_sta_symbol(name);
                        } else {
                            emitter.emit_sta_abs(addr);
                            // For multi-byte types, also store high byte
                            if is_multibyte {
                                let hi_inst = if is_array_or_enum { "STX" } else { "STY" };
                                emitter.emit_inst(hi_inst, &format!("${:04X}", addr + 1));
                                // Raw STX/STY overwrites the high byte without
                                // updating tracking; forget any register cached
                                // to it so a later load isn't wrongly elided.
                                emitter.invalidate_abs(addr + 1);
                            }
                        }
                    }
                    crate::sema::table::SymbolLocation::ZeroPage(addr) => {
                        emitter.emit_sta_zp(addr);
                        // For multi-byte types, also store high byte
                        if is_multibyte {
                            let hi_inst = if is_array_or_enum { "STX" } else { "STY" };
                            emitter.emit_inst(hi_inst, &format!("${:02X}", addr + 1));
                            // Raw STX/STY overwrites the high byte without
                            // updating tracking; forget any register cached
                            // to it so a later load isn't wrongly elided.
                            emitter.invalidate_zp(addr + 1);
                        }
                    }
                    crate::sema::table::SymbolLocation::None => {
                        return Err(CodegenError::UnsupportedOperation(format!(
                            "Variable '{}' has no storage location",
                            name
                        )));
                    }
                }
            } else {
                return Err(CodegenError::SymbolNotFound(name.clone()));
            }
        }
        crate::ast::Expr::Index { object, index } => {
            generate_index_assignment(object, index, value, emitter, info, string_collector)?;
        }
        crate::ast::Expr::Field { object, field } => {
            generate_field_assignment(object, field, value, emitter, info, string_collector)?;
        }
        // A `.len`/`.low`/`.high` target that sema re-resolved as a
        // struct field access stores like a plain field.
        crate::ast::Expr::SliceLen(object) if info.accessor_fields.contains(&target.span) => {
            let field = crate::ast::Spanned::new("len".to_string(), target.span);
            generate_field_assignment(object, &field, value, emitter, info, string_collector)?;
        }
        crate::ast::Expr::U16Low(object) if info.accessor_fields.contains(&target.span) => {
            let field = crate::ast::Spanned::new("low".to_string(), target.span);
            generate_field_assignment(object, &field, value, emitter, info, string_collector)?;
        }
        crate::ast::Expr::U16High(object) if info.accessor_fields.contains(&target.span) => {
            let field = crate::ast::Spanned::new("high".to_string(), target.span);
            generate_field_assignment(object, &field, value, emitter, info, string_collector)?;
        }
        crate::ast::Expr::Slice {
            object,
            start,
            end,
            inclusive,
        } => {
            generate_slice_assignment(
                object,
                start,
                end,
                *inclusive,
                value,
                emitter,
                info,
                string_collector,
            )?;
        }
        _ => {
            return Err(CodegenError::UnsupportedOperation(
                "Only variable, index, field, and slice assignment supported".to_string(),
            ));
        }
    }
    Ok(())
}
