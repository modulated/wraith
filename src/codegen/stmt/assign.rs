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

        match sym.location {
            SymbolLocation::ZeroPage(addr) => {
                // For u8 arrays: direct indexed addressing
                if !is_multibyte {
                    // Restore value
                    emitter.emit_inst("LDA", &format!("${:02X}", save_lo));
                    // Store to array[index]
                    emitter.emit_inst("STA", &format!("(${:02X}),Y", addr));
                } else {
                    // For u16 arrays: need to scale index by 2
                    emitter.emit_comment("Scale index for u16 array (multiply by 2)");
                    emitter.emit_inst("TYA", ""); // Get index back to A
                    emitter.emit_inst("ASL", "A"); // Multiply by 2
                    emitter.emit_inst("TAY", ""); // Back to Y

                    // Restore and store low byte
                    emitter.emit_inst("LDA", &format!("${:02X}", save_lo));
                    emitter.emit_inst("STA", &format!("(${:02X}),Y", addr));

                    // Store high byte at next position
                    emitter.emit_inst("INY", "");
                    emitter.emit_inst("LDA", &format!("${:02X}", save_hi));
                    emitter.emit_inst("STA", &format!("(${:02X}),Y", addr));
                }
            }
            SymbolLocation::Absolute(addr) if addr < 256 => {
                let addr_u8 = addr as u8;
                // For u8 arrays: direct indexed addressing
                if !is_multibyte {
                    // Restore value
                    emitter.emit_inst("LDA", &format!("${:02X}", save_lo));
                    // Store to array[index]
                    emitter.emit_inst("STA", &format!("(${:02X}),Y", addr_u8));
                } else {
                    // For u16 arrays: need to scale index by 2
                    emitter.emit_comment("Scale index for u16 array (multiply by 2)");
                    emitter.emit_inst("TYA", ""); // Get index back to A
                    emitter.emit_inst("ASL", "A"); // Multiply by 2
                    emitter.emit_inst("TAY", ""); // Back to Y

                    // Restore and store low byte
                    emitter.emit_inst("LDA", &format!("${:02X}", save_lo));
                    emitter.emit_inst("STA", &format!("(${:02X}),Y", addr_u8));

                    // Store high byte at next position
                    emitter.emit_inst("INY", "");
                    emitter.emit_inst("LDA", &format!("${:02X}", save_hi));
                    emitter.emit_inst("STA", &format!("(${:02X}),Y", addr_u8));
                }
            }
            _ => {
                return Err(CodegenError::UnsupportedOperation(format!(
                    "'{}' must be in zero page for indexed assignment",
                    array_name
                )));
            }
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
        // Function-pointer fields are 2-byte code addresses, so they must be
        // stored (and loaded) as a pair like u16 -- a device vtable depends on it.
        let is_multibyte = crate::codegen::expr::is_two_byte_value(&field_info.ty);
        // Sema rejects assigning through a `const`, so a read-only base here
        // would mean a store quietly aimed at ROM.
        if base.is_read_only() {
            return Err(CodegenError::UnsupportedOperation(
                "cannot write to constant data".to_string(),
            ));
        }
        emitter.emit_comment(&format!("Nested field assignment: .{}", field.node));
        generate_expr(value, emitter, info, string_collector)?;
        let at = base.plus(field_info.offset as u16);
        emitter.emit_inst("STA", &at.operand(0));
        if is_multibyte {
            emitter.emit_inst(store_high(&field_info.ty), &at.operand(1));
        }
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
            emitter.emit_inst("STA", "$20"); // Save low byte
            if is_multibyte {
                emitter.emit_inst(store_high(&field_info.ty), "$21"); // Save high byte
            }

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

            let hi = store_high(&field_info.ty);
            if field_addr < 0x100 {
                emitter.emit_inst("STA", &format!("${:02X}", field_addr));
                if is_multibyte {
                    emitter.emit_inst(hi, &format!("${:02X}", field_addr + 1));
                }
            } else {
                emitter.emit_inst("STA", &format!("${:04X}", field_addr));
                if is_multibyte {
                    emitter.emit_inst(hi, &format!("${:04X}", field_addr + 1));
                }
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
