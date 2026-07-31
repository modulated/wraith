//! Statement Code Generation
//!
//! Compiles statements into assembly instructions.

use crate::ast::{Spanned, Stmt};
use crate::codegen::expr::generate_expr;
use crate::codegen::{CodegenError, Emitter, StringCollector};
use crate::sema::ProgramInfo;
use rustc_hash::FxHashMap as HashMap;

/// Strategy for generating match statement code
#[derive(Debug)]
enum MatchStrategy {
    /// Use sequential CMP/BEQ comparisons (for small matches)
    Sequential,
    /// Use jump table for efficient dispatch (for enum matches with 3+ arms)
    JumpTable {
        /// Maximum tag value in the enum
        max_tag: u8,
        /// Index of the wildcard arm, if any
        wildcard_arm_index: Option<usize>,
    },
}

/// Determine the best strategy for generating a match statement
///
/// Uses jump tables for enum matches with 3+ arms to avoid BEQ branch distance limits.
/// Sequential comparisons are used for small matches (1-2 arms) or non-enum matches.
fn determine_match_strategy(arms: &[crate::ast::MatchArm], info: &ProgramInfo) -> MatchStrategy {
    use crate::ast::Pattern;

    // Collect enum variant tags from the patterns
    let mut enum_tags: Vec<u8> = Vec::new();
    let mut wildcard_arm_index: Option<usize> = None;
    // The widest tag range of any enum named in the patterns. The dispatch
    // table is indexed by the *runtime* tag, so it must span the enum's whole
    // range, not just the tags that happen to have arms: a variant without an
    // arm whose tag exceeds every armed tag would otherwise read past the end
    // of the table into whatever bytes follow it.
    let mut enum_def_max_tag: u8 = 0;

    for (i, arm) in arms.iter().enumerate() {
        match &arm.pattern.node {
            Pattern::EnumVariant {
                enum_name, variant, ..
            } => {
                // Look up the enum definition and get the tag
                if let Some(enum_def) = info.type_registry.get_enum(&enum_name.node)
                    && let Some(variant_info) = enum_def.get_variant(&variant.node)
                {
                    enum_tags.push(variant_info.tag);
                    if let Some(def_max) = enum_def.variants.iter().map(|v| v.tag).max() {
                        enum_def_max_tag = enum_def_max_tag.max(def_max);
                    }
                }
            }
            Pattern::Wildcard | Pattern::Variable(_) => {
                wildcard_arm_index = Some(i);
            }
            _ => {}
        }
    }

    // Use jump table for enum matches with 3+ arms
    // This avoids the BEQ branch distance limitation for large match bodies
    if !enum_tags.is_empty() && arms.len() >= 3 {
        let max_tag = enum_def_max_tag;

        // Dispatch doubles the tag into an 8-bit index (`ASL; TAX`), so a tag
        // above 127 wraps and selects the wrong entry. The table is also dense,
        // one word per tag from 0 upward, so sparse explicit discriminants
        // would spend hundreds of ROM bytes to describe a handful of arms.
        // Either way the sequential comparison chain is the better answer;
        // enums with explicit values are usually few-armed anyway.
        let index_would_overflow = max_tag > 127;
        let table_entries = max_tag as usize + 1;
        let too_sparse = table_entries > 4 * enum_tags.len().max(1);

        if index_would_overflow || too_sparse {
            return MatchStrategy::Sequential;
        }

        MatchStrategy::JumpTable {
            max_tag,
            wildcard_arm_index,
        }
    } else {
        MatchStrategy::Sequential
    }
}

/// Check if a statement unconditionally terminates control flow
///
/// Used for dead code elimination - if a match arm ends with return/break/continue,
/// we don't need to emit a JMP to the match end label.
fn stmt_terminates(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::Return(_) | Stmt::Break | Stmt::Continue => true,
        Stmt::Block(stmts) => stmts
            .last()
            .map(|s| stmt_terminates(&s.node))
            .unwrap_or(false),
        Stmt::If {
            then_branch,
            else_branch: Some(else_branch),
            ..
        } => stmt_terminates(&then_branch.node) && stmt_terminates(&else_branch.node),
        Stmt::Match { arms, .. } => {
            // A match terminates if all arms terminate
            !arms.is_empty() && arms.iter().all(|arm| stmt_terminates(&arm.body.node))
        }
        _ => false,
    }
}

pub fn generate_stmt(
    stmt: &Spanned<Stmt>,
    emitter: &mut Emitter,
    info: &ProgramInfo,
    string_collector: &mut StringCollector,
) -> Result<(), CodegenError> {
    // Dead code elimination: skip unreachable statements
    if info.unreachable_stmts.contains(&stmt.span) {
        emitter.emit_comment("Unreachable code eliminated");
        return Ok(());
    }

    match &stmt.node {
        Stmt::Block(stmts) => {
            for s in stmts {
                generate_stmt(s, emitter, info, string_collector)?;
            }
            Ok(())
        }
        Stmt::VarDecl {
            name,
            ty: _,
            init,
            mutable: _,
        } => {
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
                        crate::ast::Expr::Literal(crate::ast::Literal::String(s)) => {
                            s.as_bytes().to_vec()
                        }
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
                        crate::ast::Expr::StructInit { .. }
                            | crate::ast::Expr::AnonStructInit { .. }
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
                            addr,
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
                        // A = pointer low, X = pointer high -> $20/$21 vector.
                        emitter.emit_inst("STA", "$20");
                        emitter.emit_inst("STX", "$21");
                        for i in 0..total {
                            emitter.emit_inst("LDY", &format!("#${:02X}", i));
                            emitter.emit_inst("LDA", "($20),Y");
                            emitter.emit_inst("STA", &format!("${:02X}", dest + i));
                        }
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
                        Type::Array(elem, _) => {
                            crate::codegen::expr::type_byte_size(elem, info).max(1)
                        }
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
                    emitter.emit_inst("STA", "$20");
                    emitter.emit_inst("STX", "$21");
                    for k in 0..4u8 {
                        emitter.emit_inst("LDY", &format!("#${:02X}", k));
                        emitter.emit_inst("LDA", "($20),Y");
                        emitter.emit_inst("STA", &format!("${:02X}", dest + k));
                    }
                    emitter.invalidate_registers();
                    return Ok(());
                }

                // Array of structs: stored inline. Runtime-initialize each element
                // struct literal directly at addr + i*element_size.
                if let Type::Array(elem, _n) = &sym.ty
                    && let Type::Named(elem_struct) = &**elem
                    && let Some(sdef) = info.type_registry.get_struct(elem_struct)
                    && let crate::ast::Expr::Literal(crate::ast::Literal::Array(elements)) =
                        &init.node
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
                            elem_addr,
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
                    if let crate::ast::Expr::Literal(crate::ast::Literal::Array(elements)) =
                        &init.node
                    {
                        if elements.len() == 1 && *target_size > 1 {
                            // Shorthand syntax detected! Convert to ArrayFill
                            emitter.emit_comment(&format!(
                                "Expanding [value] to [{} elements]",
                                target_size
                            ));
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
                    matches!(sym.ty, Type::Array(_, _) | Type::String | Type::Pointer(_))
                        || is_enum;

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
        Stmt::Return(expr) => {
            if let Some(e) = expr {
                // Check if this is a tail recursive call
                // Pattern: return func(...) where func is the current function
                let is_tail_recursive = if let crate::ast::Expr::Call { function, .. } = &e.node {
                    // Check if calling the same function we're currently in
                    emitter
                        .current_function()
                        .map(|current_fn| current_fn == function.node.as_str())
                        .unwrap_or(false)
                } else {
                    false
                };

                if is_tail_recursive {
                    // Tail recursive call optimization: convert to loop
                    // Generate the call expression which will:
                    // 1. Evaluate arguments
                    // 2. Store them to parameter locations
                    // 3. Call the function with JSR
                    // But we'll intercept this and generate different code

                    // For now, extract the function call and generate optimized code
                    if let crate::ast::Expr::Call { function, args } = &e.node {
                        emitter.emit_comment(&format!(
                            "Tail recursive call to {} - optimized to loop",
                            function.node
                        ));

                        // Evaluate arguments and store to parameter locations
                        // This is similar to what generate_call does, but without JSR
                        crate::codegen::expr::generate_tail_recursive_update(
                            function,
                            args,
                            emitter,
                            info,
                            string_collector,
                        )?;

                        // Jump back to function start instead of JSR
                        if let Some(loop_label) = emitter.tail_call_loop_label() {
                            emitter.emit_inst("JMP", &loop_label);
                        } else {
                            // Fallback: this shouldn't happen if tail call detection worked
                            return Err(CodegenError::UnsupportedOperation(
                                "Tail recursive call without loop label".to_string(),
                            ));
                        }
                    }
                } else {
                    // Slice return-by-value: a slice variable returns a pointer
                    // to its 4-byte descriptor in A:X (the descriptor lives in the
                    // callee frame, valid until the caller copies it out right
                    // after the call). Mirrors struct return-by-value.
                    let slice_var_return = matches!(
                        info.resolved_types.get(&e.span),
                        Some(crate::sema::types::Type::Slice(_))
                    )
                    .then(|| {
                        if let crate::ast::Expr::Variable(vname) = &e.node
                            && let Some(sym) = info
                                .resolved_symbols
                                .get(&e.span)
                                .or_else(|| info.table.lookup(vname))
                            && let crate::sema::table::SymbolLocation::ZeroPage(addr) = sym.location
                        {
                            Some(addr)
                        } else {
                            None
                        }
                    })
                    .flatten();

                    if let Some(addr) = slice_var_return {
                        emitter.emit_comment("Return slice descriptor pointer (A:X)");
                        emitter.emit_inst("LDA", &format!("#${:02X}", addr));
                        emitter.emit_inst("LDX", "#$00");
                        emitter.mark_a_unknown();
                    } else {
                        // Normal return with value
                        generate_expr(e, emitter, info, string_collector)?;
                    }

                    // A 16-bit return convention is A (low) / Y (high). If the
                    // returned expression is 8-bit (e.g. `return 255;` from a
                    // `-> u16` function, where the literal fits u8), Y still
                    // holds junk from earlier code - extend it explicitly.
                    {
                        use crate::ast::PrimitiveType;
                        use crate::sema::types::Type;
                        let ret_ty = emitter
                            .current_function()
                            .and_then(|name| info.table.lookup(name))
                            .and_then(|sym| match &sym.ty {
                                Type::Function(_, ret) => Some((**ret).clone()),
                                _ => None,
                            });
                        let expr_is_8bit = info.resolved_types.get(&e.span).is_some_and(|t| {
                            matches!(
                                t,
                                Type::Primitive(
                                    PrimitiveType::U8
                                        | PrimitiveType::I8
                                        | PrimitiveType::B8
                                        | PrimitiveType::Bool
                                )
                            )
                        });
                        match ret_ty {
                            Some(Type::Primitive(PrimitiveType::U16 | PrimitiveType::B16))
                                if expr_is_8bit =>
                            {
                                emitter.emit_inst("LDY", "#$00"); // zero-extend
                            }
                            Some(Type::Primitive(PrimitiveType::I16)) if expr_is_8bit => {
                                // Sign-extend A into Y without destroying A.
                                let pos_label = emitter.next_label("sx");
                                emitter.emit_inst("LDY", "#$00");
                                emitter.emit_inst("CMP", "#$80");
                                emitter.emit_inst("BCC", &pos_label);
                                emitter.emit_inst("DEY", ""); // Y = $FF
                                emitter.emit_label(&pos_label);
                            }
                            _ => {}
                        }
                    }

                    // Only emit RTS if we're not in an inline context
                    if !emitter.is_inlining() {
                        emitter.emit_inst("RTS", "");
                    } else if let Some(end) = emitter.inline_end_label() {
                        // An inline expansion has no frame to return from; an
                        // early return jumps past the rest of the body so the
                        // value just computed survives.
                        let end = end.to_string();
                        emitter.emit_inst("JMP", &end);
                    }
                }
            } else {
                // Return with no value
                if !emitter.is_inlining() {
                    emitter.emit_inst("RTS", "");
                } else if let Some(end) = emitter.inline_end_label() {
                    let end = end.to_string();
                    emitter.emit_inst("JMP", &end);
                }
            }
            Ok(())
        }
        Stmt::Assign { target, value } => {
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

                        // Struct-by-value assignment from a call, e.g.
                        // `p = make();`. The value expression (already generated
                        // above) left a pointer to the struct bytes in A:X; copy
                        // the whole struct into the target's inline storage
                        // rather than storing just the low byte of the pointer.
                        if matches!(&value.node, crate::ast::Expr::Call { .. })
                            && let Type::Named(sname) = &sym.ty
                            && let Some(sdef) = info.type_registry.get_struct(sname)
                            && let crate::sema::table::SymbolLocation::ZeroPage(dest) = sym.location
                        {
                            let total = sdef.total_size as u8;
                            emitter.emit_comment(&format!(
                                "Struct return-by-value assign: copy {} bytes into ${:02X}",
                                total, dest
                            ));
                            emitter.emit_inst("STA", "$20");
                            emitter.emit_inst("STX", "$21");
                            for i in 0..total {
                                emitter.emit_inst("LDY", &format!("#${:02X}", i));
                                emitter.emit_inst("LDA", "($20),Y");
                                emitter.emit_inst("STA", &format!("${:02X}", dest + i));
                            }
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
                    generate_index_assignment(
                        object,
                        index,
                        value,
                        emitter,
                        info,
                        string_collector,
                    )?;
                }
                crate::ast::Expr::Field { object, field } => {
                    generate_field_assignment(
                        object,
                        field,
                        value,
                        emitter,
                        info,
                        string_collector,
                    )?;
                }
                // A `.len`/`.low`/`.high` target that sema re-resolved as a
                // struct field access stores like a plain field.
                crate::ast::Expr::SliceLen(object)
                    if info.accessor_fields.contains(&target.span) =>
                {
                    let field = crate::ast::Spanned::new("len".to_string(), target.span);
                    generate_field_assignment(
                        object,
                        &field,
                        value,
                        emitter,
                        info,
                        string_collector,
                    )?;
                }
                crate::ast::Expr::U16Low(object) if info.accessor_fields.contains(&target.span) => {
                    let field = crate::ast::Spanned::new("low".to_string(), target.span);
                    generate_field_assignment(
                        object,
                        &field,
                        value,
                        emitter,
                        info,
                        string_collector,
                    )?;
                }
                crate::ast::Expr::U16High(object)
                    if info.accessor_fields.contains(&target.span) =>
                {
                    let field = crate::ast::Spanned::new("high".to_string(), target.span);
                    generate_field_assignment(
                        object,
                        &field,
                        value,
                        emitter,
                        info,
                        string_collector,
                    )?;
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
        Stmt::Expr(expr) => {
            generate_expr(expr, emitter, info, string_collector)?;
            Ok(())
        }
        Stmt::If {
            condition,
            then_branch,
            else_branch,
        } => {
            let then_label = emitter.next_label("then");
            let else_label = emitter.next_label("else");
            let end_label = emitter.next_label("end");

            // Condition
            generate_expr(condition, emitter, info, string_collector)?;

            // For large if statements, we need to avoid forward branches
            // that might exceed 127 bytes. Use this structure:
            //   condition
            //   BNE then      ; branch if true (short forward jump)
            //   JMP else      ; jump if false
            // then:
            //   then_body
            //   JMP end
            // else:
            //   else_body
            // end:

            if !emitter.is_minimal() {
                emitter.emit_comment("Branch to then if condition is true");
            }
            emitter.emit_inst("CMP", "#$00");
            emitter.emit_inst("BNE", &then_label);
            emitter.emit_inst("JMP", &else_label);

            // Then
            emitter.emit_label(&then_label);
            generate_stmt(then_branch, emitter, info, string_collector)?;
            emitter.emit_inst("JMP", &end_label);

            // Else
            emitter.emit_label(&else_label);
            if let Some(else_b) = else_branch {
                generate_stmt(else_b, emitter, info, string_collector)?;
            }

            // End
            emitter.emit_label(&end_label);
            // Invalidate register state after control flow merge
            // (we don't know which branch was taken)
            emitter.reg_state.invalidate_all();
            Ok(())
        }
        Stmt::While { condition, body } => {
            let body_label = emitter.next_label("wb");
            let check_label = emitter.next_label("wc");
            let end_label = emitter.next_label("we");

            // Structure that avoids long branches:
            // check:
            //   ...condition...
            //   BNE body      ; branch if true (2 bytes forward to body label)
            //   JMP end       ; exit if false (3 bytes)
            // body:
            //   ...body...    ; can be any size
            //   JMP check     ; 3 bytes back
            // end:
            //
            // The BNE only needs to jump 3 bytes forward (past the JMP),
            // so it's always within the 127-byte limit regardless of body size.

            // Condition check
            emitter.emit_label(&check_label);
            // check_label is a back-edge target: register state from before
            // the loop (or from the previous iteration) is stale here.
            emitter.reg_state.invalidate_all();
            generate_expr(condition, emitter, info, string_collector)?;

            if !emitter.is_minimal() {
                emitter.emit_comment("Continue to body if condition is true");
            }
            emitter.emit_inst("CMP", "#$00");
            // BNE jumps only 3 bytes forward (size of JMP instruction)
            // This is always within the 127-byte branch limit
            emitter.emit_inst("BNE", &body_label);
            emitter.emit_inst("JMP", &end_label);

            emitter.emit_label(&body_label);

            // Push loop context for break/continue
            emitter.push_loop(check_label.clone(), end_label.clone());

            // Body
            generate_stmt(body, emitter, info, string_collector)?;

            // Pop loop context
            emitter.pop_loop();

            // Jump back to condition check
            emitter.emit_inst("JMP", &check_label);

            emitter.emit_label(&end_label);
            // Invalidate register state after loop end
            emitter.reg_state.invalidate_all();
            Ok(())
        }
        Stmt::Loop { body } => {
            let loop_label = emitter.next_label("lp");
            let end_label = emitter.next_label("lx");

            emitter.emit_label(&loop_label);
            // loop_label is a back-edge target: register state from before
            // the loop (or from the previous iteration) is stale here.
            emitter.reg_state.invalidate_all();

            // Push loop context for break/continue
            emitter.push_loop(loop_label.clone(), end_label.clone());

            generate_stmt(body, emitter, info, string_collector)?;

            // Pop loop context
            emitter.pop_loop();

            emitter.emit_inst("JMP", &loop_label);
            emitter.emit_label(&end_label);
            Ok(())
        }
        Stmt::For {
            var_name,
            var_type: _,
            range,
            body,
        } => {
            // Check if loop can be unrolled (constant bounds, small count)
            let start_const = info.folded_constants.get(&range.start.span);
            let end_const = info.folded_constants.get(&range.end.span);

            // Threshold for unrolling: 8 iterations or fewer
            const UNROLL_THRESHOLD: i64 = 8;

            if let (
                Some(crate::sema::const_eval::ConstValue::Integer(start)),
                Some(crate::sema::const_eval::ConstValue::Integer(end)),
            ) = (start_const, end_const)
            {
                // Calculate iteration count
                let count = if range.inclusive {
                    end - start + 1
                } else {
                    end - start
                };

                if count > 0 && count <= UNROLL_THRESHOLD {
                    // LOOP UNROLLING: Generate inline code for small constant loops
                    emitter.emit_comment(&format!(
                        "Loop unrolled: {} iteration{}",
                        count,
                        if count == 1 { "" } else { "s" }
                    ));

                    // Resolve the loop variable's actual frame slot (registered at
                    // its declaration span during analysis) rather than assuming the
                    // first variable address.
                    let loop_var_addr = match info
                        .resolved_symbols
                        .get(&var_name.span)
                        .map(|s| &s.location)
                    {
                        Some(crate::sema::table::SymbolLocation::ZeroPage(addr)) => *addr,
                        _ => {
                            return Err(CodegenError::Internal(format!(
                                "unrolled loop variable '{}' has no zero-page frame slot",
                                var_name.node
                            )));
                        }
                    };

                    // A 16-bit loop variable needs its high byte written too,
                    // or body reads of the counter see a garbage high byte.
                    let loop_var_is_16bit =
                        info.resolved_symbols.get(&var_name.span).is_some_and(|s| {
                            matches!(
                                s.ty,
                                crate::sema::types::Type::Primitive(
                                    crate::ast::PrimitiveType::U16
                                        | crate::ast::PrimitiveType::I16
                                        | crate::ast::PrimitiveType::B16
                                )
                            )
                        });

                    // Create end label for break statements
                    let end_label = emitter.next_label("ux");

                    // Generate body for each iteration with loop variable set
                    for i in 0..count {
                        let iter_val = start + i;

                        // Set loop variable to current iteration value
                        emitter.emit_comment(&format!("{} = {}", var_name.node, iter_val));
                        emitter.emit_inst("LDA", &format!("#${:02X}", iter_val as u8));
                        emitter.emit_inst("STA", &format!("${:02X}", loop_var_addr));
                        if loop_var_is_16bit {
                            emitter.emit_inst("LDA", &format!("#${:02X}", (iter_val >> 8) as u8));
                            emitter.emit_inst(
                                "STA",
                                &format!("${:02X}", loop_var_addr.wrapping_add(1)),
                            );
                        }

                        // Create iteration label for continue statements
                        let iter_label = emitter.next_label("ui");

                        // Push loop context so break/continue work
                        emitter.push_loop(iter_label.clone(), end_label.clone());

                        // Execute body
                        emitter.reg_state.invalidate_all();
                        generate_stmt(body, emitter, info, string_collector)?;

                        // Pop loop context
                        emitter.pop_loop();

                        // Emit iteration label for continue
                        emitter.emit_label(&iter_label);
                    }

                    // Emit end label for break
                    emitter.emit_label(&end_label);

                    return Ok(());
                }
            }

            // NORMAL LOOP: Generate standard loop code
            generate_normal_loop(var_name, range, body, emitter, info, string_collector)
        }
        Stmt::ForEach {
            var_name,
            var_type: _,
            iterable,
            body,
            index_var,
        } => {
            // ForEach loop: for item in iterable { ... } or for (index, item) in iterable { ... }
            // Supports arrays and strings
            // Strategy:
            // 1. Evaluate iterable expression to get pointer
            // 2. Use X register as loop counter (0..length)
            // 3. Load iterable[X] into the loop variable
            // 4. Store X in index variable if present
            // 5. Execute body
            // 6. Increment X and loop

            emitter.emit_comment("ForEach loop");

            // Generate the iterable expression (can be array or string variable)
            let (iterable_info, is_string) = match &iterable.node {
                crate::ast::Expr::Variable(name) => {
                    // Look up the variable to get its pointer location and type
                    let sym = info
                        .resolved_symbols
                        .get(&iterable.span)
                        .or_else(|| info.table.lookup(name))
                        .ok_or_else(|| CodegenError::SymbolNotFound(name.clone()))?;

                    // Check if it's an array or string
                    let is_str = matches!(sym.ty, crate::sema::types::Type::String);

                    // Get the location where the pointer is stored
                    let ptr_loc = match sym.location {
                        crate::sema::table::SymbolLocation::ZeroPage(addr) => addr,
                        crate::sema::table::SymbolLocation::Absolute(addr) if addr < 256 => {
                            addr as u8
                        }
                        _ => {
                            return Err(CodegenError::UnsupportedOperation(
                                "ForEach requires pointer in zero page".to_string(),
                            ));
                        }
                    };

                    ((ptr_loc, sym.ty.clone()), is_str)
                }
                _ => {
                    return Err(CodegenError::UnsupportedOperation(
                        "ForEach only supports variables currently".to_string(),
                    ));
                }
            };

            let (iterable_base, iterable_ty) = iterable_info;
            let is_slice = matches!(&iterable_ty, crate::sema::types::Type::Slice(_));

            let loop_label = emitter.next_label("fe");
            let continue_label = emitter.next_label("fc");
            let end_label = emitter.next_label("fz");

            // Slices iterate with a 16-bit counter (they can exceed 255 elements),
            // so they use a dedicated loop that re-reads the descriptor each
            // iteration and advances an element pointer. Handled separately here.
            if is_slice {
                generate_foreach_slice(
                    iterable,
                    iterable_base,
                    &iterable_ty,
                    var_name,
                    index_var.as_ref(),
                    body,
                    &loop_label,
                    &continue_label,
                    &end_label,
                    emitter,
                    info,
                    string_collector,
                )?;
                return Ok(());
            }

            // The loop counter lives in a hidden zero-page slot, not just in X,
            // so the body may clobber X freely (a u8 multiply uses it as a bit
            // counter, a nested loop as its own counter) without corrupting the
            // iteration. X is reloaded from the slot at each loop head and the
            // slot is advanced with INC at continue.
            let counter = match info
                .loop_bound_slots
                .get(&iterable.span)
                .map(|s| &s.location)
            {
                Some(crate::sema::table::SymbolLocation::ZeroPage(a)) => *a,
                _ => {
                    return Err(CodegenError::Internal(
                        "string/array foreach: counter slot was not allocated".to_string(),
                    ));
                }
            };
            emitter.emit_inst("LDA", "#$00");
            emitter.emit_inst("STA", &format!("${:02X}", counter));

            // For arrays the size is a compile-time constant. Strings and
            // slices have a runtime length, staged (with the string pointer)
            // at the loop *head* below.
            let array_size = if is_string || is_slice {
                None
            } else {
                match &iterable_ty {
                    crate::sema::types::Type::Array(_, sz) => Some(*sz),
                    _ => {
                        return Err(CodegenError::UnsupportedOperation(
                            "ForEach requires array, slice, or string type".to_string(),
                        ));
                    }
                }
            };

            // Loop start
            emitter.emit_label(&loop_label);

            // Reload the counter into X. The loop head is a branch target (entry
            // and back-edge), and the body may have left anything in X, so no
            // prior belief holds and X must come from the slot.
            emitter.invalidate_registers();
            emitter.emit_inst("LDX", &format!("${:02X}", counter));

            // String/slice staging ($F0-$F2) lives only from here to the
            // element read — never across the body, which is arbitrary code
            // and may use those bytes itself (an earlier version staged once
            // before the loop, and a body's index assignment silently
            // destroyed the string pointer). It is therefore re-staged on
            // every iteration, which costs a handful of cycles per element.
            if is_string {
                // Re-stage the string pointer and read the length prefix.
                emitter.emit_comment("String iteration - load length");
                emitter.emit_inst("LDA", &format!("${:02X}", iterable_base));
                emitter.emit_inst("STA", "$F0");
                emitter.emit_inst("LDA", &format!("${:02X}", iterable_base + 1));
                emitter.emit_inst("STA", "$F1");
                emitter.emit_inst("LDY", "#$00");
                emitter.emit_inst("LDA", "($F0),Y");
                emitter.emit_inst("STA", "$F2");
                emitter.emit_inst("CPX", "$F2");
            } else if is_slice {
                // Length is the low byte of the descriptor at base+2
                // (iteration is bounded to 255 elements).
                emitter.emit_inst("LDA", &format!("${:02X}", iterable_base + 2));
                emitter.emit_inst("STA", "$F2");
                emitter.emit_inst("CPX", "$F2");
            } else if let Some(size) = array_size {
                // Compare X against known array size
                emitter.emit_inst("CPX", &format!("#${:02X}", size));
            }
            emitter.emit_inst("BCS", &end_label); // Branch if X >= size

            // Push loop context for break/continue. `continue` must land on the
            // increment (continue_label), NOT the loop head — otherwise the index
            // in X is never advanced and the loop spins forever.
            emitter.push_loop(continue_label.clone(), end_label.clone());

            // Store index in index variable if present
            if let Some(idx_var) = index_var
                && let Some(idx_sym) = info.resolved_symbols.get(&idx_var.span)
            {
                match idx_sym.location {
                    crate::sema::table::SymbolLocation::ZeroPage(addr) => {
                        emitter.emit_comment(&format!("Store index in {}", idx_var.node));
                        emitter.emit_inst("STX", &format!("${:02X}", addr));
                    }
                    _ => {
                        return Err(CodegenError::UnsupportedOperation(
                            "ForEach index variable must be in zero page".to_string(),
                        ));
                    }
                }
            }

            // Whether the array/slice element type is 16-bit (u16/i16/b16).
            // Strings always iterate u8 characters.
            let elem_multibyte = {
                let elem = match &iterable_ty {
                    crate::sema::types::Type::Array(elem, _)
                    | crate::sema::types::Type::Slice(elem) => Some(&**elem),
                    _ => None,
                };
                matches!(
                    elem,
                    Some(crate::sema::types::Type::Primitive(
                        crate::ast::PrimitiveType::U16
                            | crate::ast::PrimitiveType::I16
                            | crate::ast::PrimitiveType::B16
                    ))
                )
            };

            // Resolve the loop variable's storage up front.
            let loopvar_loc = info
                .resolved_symbols
                .get(&var_name.span)
                .map(|s| s.location.clone())
                .ok_or_else(|| CodegenError::SymbolNotFound(var_name.node.clone()))?;

            // Index into the iterable: Y = X, scaled ×2 for u16 elements.
            emitter.emit_inst("TXA", "");
            if elem_multibyte {
                emitter.emit_inst("ASL", "A");
            }
            emitter.emit_inst("TAY", "");

            // Emit "load byte at (base),Y then store to <dest>" for the low byte,
            // and (for u16 elements) INY + load/store the high byte.
            let store_lo = |emitter: &mut Emitter, load: &dyn Fn(&mut Emitter)| match loopvar_loc {
                crate::sema::table::SymbolLocation::ZeroPage(addr) => {
                    load(emitter);
                    emitter.emit_inst("STA", &format!("${:02X}", addr));
                    if elem_multibyte {
                        emitter.emit_inst("INY", "");
                        load(emitter);
                        emitter.emit_inst("STA", &format!("${:02X}", addr + 1));
                    }
                    Ok(())
                }
                crate::sema::table::SymbolLocation::Absolute(addr) => {
                    load(emitter);
                    emitter.emit_sta_abs(addr);
                    if elem_multibyte {
                        emitter.emit_inst("INY", "");
                        load(emitter);
                        emitter.emit_sta_abs(addr + 1);
                    }
                    Ok(())
                }
                _ => Err(CodegenError::UnsupportedOperation(
                    "ForEach loop variable must have concrete location".to_string(),
                )),
            };

            if is_string {
                // Strings: skip the length byte, u8 elements only.
                emitter.emit_inst("INY", "");
                emitter.emit_inst("LDA", "($F0),Y");
                match loopvar_loc {
                    crate::sema::table::SymbolLocation::ZeroPage(addr) => emitter.emit_sta_zp(addr),
                    crate::sema::table::SymbolLocation::Absolute(addr) => {
                        emitter.emit_sta_abs(addr)
                    }
                    _ => {
                        return Err(CodegenError::UnsupportedOperation(
                            "ForEach loop variable must have concrete location".to_string(),
                        ));
                    }
                }
            } else {
                let base = iterable_base;
                store_lo(emitter, &move |e: &mut Emitter| {
                    e.emit_inst("LDA", &format!("(${:02X}),Y", base));
                })?;
            }

            // The body is arbitrary code; drop all register beliefs before it.
            // The counter of record is the memory slot, reloaded into X at the
            // next loop head, so the body clobbering X is harmless.
            emitter.reg_state.invalidate_all();

            // Execute loop body
            generate_stmt(body, emitter, info, string_collector)?;

            // Pop loop context
            emitter.pop_loop();

            // Continue target: advance the counter in memory, then re-test at
            // the loop head (which reloads X from it). Advancing the slot rather
            // than X is what lets a body that clobbers X iterate correctly.
            emitter.emit_label(&continue_label);
            emitter.emit_inst("INC", &format!("${:02X}", counter));

            emitter.emit_inst("JMP", &loop_label);
            emitter.emit_label(&end_label);

            Ok(())
        }
        Stmt::Break => {
            if let Some(loop_ctx) = emitter.current_loop() {
                let break_label = loop_ctx.break_label.clone();
                emitter.emit_inst("JMP", &break_label);
                Ok(())
            } else {
                // This should be caught by semantic analysis
                Err(CodegenError::UnsupportedOperation(
                    "break statement outside of loop".to_string(),
                ))
            }
        }
        Stmt::Continue => {
            if let Some(loop_ctx) = emitter.current_loop() {
                let continue_label = loop_ctx.continue_label.clone();
                emitter.emit_inst("JMP", &continue_label);
                Ok(())
            } else {
                // This should be caught by semantic analysis
                Err(CodegenError::UnsupportedOperation(
                    "continue statement outside of loop".to_string(),
                ))
            }
        }
        Stmt::Asm { lines } => {
            // Inline assembly - emit lines directly with variable substitution
            let current_fn = emitter.current_function().map(|s| s.to_string());
            for line in lines {
                // Substitute {var} patterns with actual addresses
                let substituted =
                    substitute_asm_vars(&line.instruction, info, current_fn.as_deref())?;

                // If we're inside an inline function expansion, uniquify labels
                let final_line = if let Some(suffix) = emitter.inline_label_suffix() {
                    uniquify_asm_labels(&substituted, suffix)
                } else {
                    substituted
                };

                let parts: Vec<&str> = final_line.split_whitespace().collect();
                if parts.is_empty() {
                    continue;
                }

                let mnemonic = parts[0];
                let operand = if parts.len() > 1 {
                    parts[1..].join(" ")
                } else {
                    String::new()
                };

                emitter.emit_inst(mnemonic, &operand);
            }
            // Invalidate register state after inline assembly
            // (we don't know what the assembly does to registers)
            emitter.reg_state.invalidate_all();
            Ok(())
        }
        Stmt::Match { expr, arms } => generate_match(expr, arms, emitter, info, string_collector),
    }
}

fn generate_index_assignment(
    object: &Spanned<crate::ast::Expr>,
    index: &Spanned<crate::ast::Expr>,
    value: &Spanned<crate::ast::Expr>,
    emitter: &mut Emitter,
    info: &ProgramInfo,
    string_collector: &mut StringCollector,
) -> Result<(), CodegenError> {
    use crate::ast::Expr;
    use crate::ast::PrimitiveType;
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

    let is_multibyte = element_type.is_some_and(|t| {
        matches!(
            t,
            Type::Primitive(PrimitiveType::U16 | PrimitiveType::I16 | PrimitiveType::B16)
        )
    });

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
fn generate_slice_materialize(
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

/// Iterate a slice with a 16-bit counter (slices can exceed 255 elements). The
/// slice descriptor (`slot[0..1]` = base, `slot[2..3]` = length) is re-read each
/// iteration, and an element pointer is recomputed into $F0/$F1, so the loop is
/// correct even for lengths past 255 and across page boundaries. The counter
/// lives in a hidden frame slot (allocated by sema) so it survives the body.
#[allow(clippy::too_many_arguments)]
fn generate_foreach_slice(
    iterable: &Spanned<crate::ast::Expr>,
    slot: u8,
    iterable_ty: &crate::sema::types::Type,
    var_name: &Spanned<String>,
    index_var: Option<&Spanned<String>>,
    body: &Spanned<crate::ast::Stmt>,
    loop_label: &str,
    continue_label: &str,
    end_label: &str,
    emitter: &mut Emitter,
    info: &ProgramInfo,
    string_collector: &mut StringCollector,
) -> Result<(), CodegenError> {
    use crate::ast::PrimitiveType;
    use crate::sema::table::SymbolLocation;
    use crate::sema::types::Type;

    let elem_multibyte = matches!(
        iterable_ty,
        Type::Slice(elem) if matches!(
            &**elem,
            Type::Primitive(PrimitiveType::U16 | PrimitiveType::I16 | PrimitiveType::B16)
        )
    );

    // Hidden 16-bit counter slot (frame-allocated by sema, keyed on the iterable).
    let counter = match info
        .loop_bound_slots
        .get(&iterable.span)
        .map(|s| &s.location)
    {
        Some(SymbolLocation::ZeroPage(a)) => *a,
        _ => {
            return Err(CodegenError::Internal(
                "slice foreach: 16-bit counter slot was not allocated".to_string(),
            ));
        }
    };

    // Loop-variable storage.
    let loopvar_addr = match info
        .resolved_symbols
        .get(&var_name.span)
        .map(|s| s.location.clone())
        .ok_or_else(|| CodegenError::SymbolNotFound(var_name.node.clone()))?
    {
        SymbolLocation::ZeroPage(a) => a,
        SymbolLocation::Absolute(a) if a < 256 => a as u8,
        _ => {
            return Err(CodegenError::UnsupportedOperation(
                "ForEach loop variable must be in zero page".to_string(),
            ));
        }
    };

    // Optional index variable (u8: the low byte of the counter).
    let index_addr = index_var
        .and_then(|iv| info.resolved_symbols.get(&iv.span))
        .and_then(|s| match s.location {
            SymbolLocation::ZeroPage(a) => Some(a),
            _ => None,
        });

    emitter.emit_comment("Slice foreach (16-bit counter)");
    emitter.emit_inst("LDA", "#$00");
    emitter.emit_inst("STA", &format!("${:02X}", counter));
    emitter.emit_inst("STA", &format!("${:02X}", counter + 1));

    emitter.emit_label(loop_label);
    // The loop head is a branch target (entry and the back-edge), so no register
    // belief from a previous iteration is valid here.
    emitter.invalidate_registers();

    // Exit when counter >= length (unsigned 16-bit compare).
    let body_label = emitter.next_label("fsb");
    emitter.emit_inst("LDA", &format!("${:02X}", counter + 1));
    emitter.emit_inst("CMP", &format!("${:02X}", slot + 3));
    emitter.emit_inst("BCC", &body_label); // counter.hi < len.hi
    emitter.emit_inst("BNE", end_label); // counter.hi > len.hi -> done
    emitter.emit_inst("LDA", &format!("${:02X}", counter));
    emitter.emit_inst("CMP", &format!("${:02X}", slot + 2));
    emitter.emit_inst("BCS", end_label); // counter.lo >= len.lo -> done
    emitter.emit_label(&body_label);

    emitter.push_loop(continue_label.to_string(), end_label.to_string());

    if let Some(ia) = index_addr {
        emitter.emit_inst("LDA", &format!("${:02X}", counter));
        emitter.emit_inst("STA", &format!("${:02X}", ia));
    }

    // Element pointer = base + counter*elem_size, into $F0/$F1 (byte offset in
    // $22/$23). Recomputed every iteration, so it survives a body with calls.
    if elem_multibyte {
        emitter.emit_inst("LDA", &format!("${:02X}", counter));
        emitter.emit_inst("ASL", "A");
        emitter.emit_inst("STA", "$22");
        emitter.emit_inst("LDA", &format!("${:02X}", counter + 1));
        emitter.emit_inst("ROL", "A");
        emitter.emit_inst("STA", "$23");
    } else {
        emitter.emit_inst("LDA", &format!("${:02X}", counter));
        emitter.emit_inst("STA", "$22");
        emitter.emit_inst("LDA", &format!("${:02X}", counter + 1));
        emitter.emit_inst("STA", "$23");
    }
    emitter.emit_inst("LDA", &format!("${:02X}", slot));
    emitter.emit_inst("CLC", "");
    emitter.emit_inst("ADC", "$22");
    emitter.emit_inst("STA", "$F0");
    emitter.emit_inst("LDA", &format!("${:02X}", slot + 1));
    emitter.emit_inst("ADC", "$23");
    emitter.emit_inst("STA", "$F1");

    // Load the element into the loop variable.
    emitter.emit_inst("LDY", "#$00");
    emitter.emit_inst("LDA", "($F0),Y");
    emitter.emit_inst("STA", &format!("${:02X}", loopvar_addr));
    if elem_multibyte {
        emitter.emit_inst("INY", "");
        emitter.emit_inst("LDA", "($F0),Y");
        emitter.emit_inst("STA", &format!("${:02X}", loopvar_addr + 1));
    }
    emitter.invalidate_registers();

    generate_stmt(body, emitter, info, string_collector)?;
    emitter.pop_loop();

    // continue: counter += 1 (16-bit), then re-test.
    emitter.emit_label(continue_label);
    emitter.emit_inst("INC", &format!("${:02X}", counter));
    let no_carry = emitter.next_label("fsc");
    emitter.emit_inst("BNE", &no_carry);
    emitter.emit_inst("INC", &format!("${:02X}", counter + 1));
    emitter.emit_label(&no_carry);
    emitter.emit_inst("JMP", loop_label);
    emitter.emit_label(end_label);
    // The exit is reached from the compare branches with A holding the counter,
    // not any tracked value, so drop all beliefs before following code.
    emitter.invalidate_registers();

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn generate_slice_assignment(
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

        // Determine element width: u16/i16/b16 arrays store two bytes per
        // element and index by a byte offset (element index * 2), matching
        // generate_index_assignment. Truncating to one byte and using the raw
        // element index (as the original code did) silently corrupts u16 arrays.
        use crate::ast::PrimitiveType;
        use crate::sema::types::Type;
        let element_is_multibyte = matches!(
            info.resolved_types.get(&object.span),
            Some(Type::Array(elem, _)) if matches!(
                &**elem,
                Type::Primitive(PrimitiveType::U16)
                    | Type::Primitive(PrimitiveType::I16)
                    | Type::Primitive(PrimitiveType::B16)
            )
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

/// Copy an enum value's bytes from the pointer in A:X into the variable's own
/// per-declaration block, then load the block's address into A:X for the slot
/// store that follows. Without this the slot would point at shared codegen
/// scratch (construction) or a dead frame (call result), and two live enums
/// would alias.
fn emit_enum_copy_to_block(
    block: &crate::sema::LocalArray,
    emitter: &mut Emitter,
) -> Result<(), CodegenError> {
    if block.size == 0 || block.size > 255 {
        return Err(CodegenError::Internal(format!(
            "enum data block of {} bytes cannot be copied by the byte loop",
            block.size
        )));
    }
    let ptr = emitter.memory_layout.deref_ptr();
    emitter.emit_comment("Copy enum value into its own storage");
    emitter.emit_inst("STA", &format!("${:02X}", ptr));
    emitter.emit_inst("STX", &format!("${:02X}", ptr + 1));
    let copy_label = emitter.next_label("encp");
    emitter.emit_inst("LDY", "#$00");
    emitter.emit_label(&copy_label);
    emitter.emit_inst("LDA", &format!("(${:02X}),Y", ptr));
    emitter.emit_inst("STA", &format!("${:04X},Y", block.addr));
    emitter.emit_inst("INY", "");
    emitter.emit_inst("CPY", &format!("#${:02X}", block.size as u8));
    emitter.emit_inst("BNE", &copy_label);
    // The slot now points at the block, whose address is a compile-time constant.
    emitter.emit_inst("LDA", &format!("#${:02X}", block.addr & 0xFF));
    emitter.emit_inst("LDX", &format!("#${:02X}", (block.addr >> 8) & 0xFF));
    emitter.invalidate_registers();
    Ok(())
}

fn generate_field_assignment(
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
        let is_multibyte = crate::codegen::expr::field_is_two_bytes(&field_info.ty);
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
        let is_multibyte = crate::codegen::expr::field_is_two_bytes(&field_info.ty);

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
    if crate::codegen::expr::field_high_byte_in_x(ty) {
        "STX"
    } else {
        "STY"
    }
}

fn generate_match(
    expr: &Spanned<crate::ast::Expr>,
    arms: &[crate::ast::MatchArm],
    emitter: &mut Emitter,
    info: &ProgramInfo,
    string_collector: &mut StringCollector,
) -> Result<(), CodegenError> {
    // Determine the best strategy for this match
    let strategy = determine_match_strategy(arms, info);

    match strategy {
        MatchStrategy::Sequential => {
            generate_match_sequential(expr, arms, emitter, info, string_collector)
        }
        MatchStrategy::JumpTable {
            max_tag,
            wildcard_arm_index,
        } => generate_match_jump_table(
            expr,
            arms,
            emitter,
            info,
            string_collector,
            max_tag,
            wildcard_arm_index,
        ),
    }
}

/// A conditional branch to an arm body. The compare section and the bodies
/// are emitted as two blocks, so the target can sit arbitrarily far away —
/// past every other arm's body — and a plain conditional branch overflows its
/// ±127-byte range once a match has enough (or large enough) arms. Emit the
/// inverse branch over a JMP instead: the hop is always 3 bytes.
fn emit_far_arm_branch(emitter: &mut Emitter, cond: &str, arm_label: &str, skip_label: &str) {
    let inv = match cond {
        "BEQ" => "BNE",
        "BCC" => "BCS",
        "BMI" => "BPL",
        other => unreachable!("not an invertible branch: {}", other),
    };
    emitter.emit_inst(inv, skip_label);
    emitter.emit_inst("JMP", arm_label);
    emitter.emit_label(skip_label);
}

/// Generate match statement using sequential CMP/BEQ comparisons
///
/// Used for small matches (1-2 arms) or non-enum patterns.
/// Arm-ward branches go through emit_far_arm_branch so arm bodies may be any
/// size and any number.
fn generate_match_sequential(
    expr: &Spanned<crate::ast::Expr>,
    arms: &[crate::ast::MatchArm],
    emitter: &mut Emitter,
    info: &ProgramInfo,
    string_collector: &mut StringCollector,
) -> Result<(), CodegenError> {
    use crate::ast::Pattern;
    use crate::sema::table::SymbolLocation;
    use crate::sema::types::Type;

    let match_id = emitter.next_match_id();

    emitter.emit_comment("Match statement (sequential)");

    // Check if we're matching on an enum by looking at the first pattern
    let is_enum_match = arms
        .iter()
        .any(|arm| matches!(arm.pattern.node, Pattern::EnumVariant { .. }));

    // Evaluate the matched expression into accumulator
    generate_expr(expr, emitter, info, string_collector)?;

    // Whether the matched value is 16-bit (low in A/$20, high in Y/$21).
    let scrutinee_is_u16 = matches!(
        info.resolved_types.get(&expr.span),
        Some(
            Type::Primitive(crate::ast::PrimitiveType::U16)
                | Type::Primitive(crate::ast::PrimitiveType::I16)
                | Type::Primitive(crate::ast::PrimitiveType::B16)
        )
    );

    // Whether the matched value is signed (i8/i16) — range patterns must then
    // use signed comparisons instead of unsigned BCC/BCS.
    let scrutinee_is_signed = info
        .resolved_types
        .get(&expr.span)
        .is_some_and(|t| t.is_signed());

    // Use pointer ops area for indirect addressing to avoid conflict with temp storage
    let ptr_base = emitter.memory_layout.pointer_ops_start; // $30 by default

    if is_enum_match {
        // For enum matching, expression returns a pointer in A:X
        // Store pointer at pointer ops area (not $20 which is used by temp storage)
        emitter.emit_inst("STA", &format!("${:02X}", ptr_base));
        emitter.emit_inst("STX", &format!("${:02X}", ptr_base + 1));

        // Load the discriminant tag from the enum (first byte)
        emitter.emit_inst("LDY", "#$00");
        emitter.emit_inst("LDA", &format!("(${:02X}),Y", ptr_base));
        emitter.emit_inst("STA", &format!("${:02X}", ptr_base + 2)); // Store tag
    } else {
        // For simple value matching, store the low byte at $20 (and, for u16,
        // the high byte at $21 so literal patterns and variable bindings see
        // the full value rather than a truncated low byte).
        emitter.emit_inst("STA", "$20");
        if scrutinee_is_u16 {
            emitter.emit_inst("STY", "$21");
        }
    }

    // Generate code for each arm
    let mut has_wildcard = false;
    for (i, arm) in arms.iter().enumerate() {
        match &arm.pattern.node {
            Pattern::Literal(lit_expr) => {
                // Compare with literal value
                if let crate::ast::Expr::Literal(crate::ast::Literal::Integer(val)) = &lit_expr.node
                {
                    let arm_label = format!("match_{}_arm_{}", match_id, i);
                    if scrutinee_is_u16 {
                        // Compare both bytes; only branch to the arm if both match.
                        let skip = format!("match_{}_arm_{}_skip", match_id, i);
                        emitter.emit_inst("LDA", "$20");
                        emitter.emit_inst("CMP", &format!("#${:02X}", (*val as u16) & 0xFF));
                        emitter.emit_inst("BNE", &skip);
                        emitter.emit_inst("LDA", "$21");
                        emitter.emit_inst("CMP", &format!("#${:02X}", ((*val as u16) >> 8) & 0xFF));
                        emit_far_arm_branch(
                            emitter,
                            "BEQ",
                            &arm_label,
                            &format!("match_{}_arm_{}_hifar", match_id, i),
                        );
                        emitter.emit_label(&skip);
                    } else {
                        emitter.emit_inst("LDA", "$20");
                        emitter.emit_inst("CMP", &format!("#${:02X}", (*val as u16) & 0xFF));
                        emit_far_arm_branch(
                            emitter,
                            "BEQ",
                            &arm_label,
                            &format!("match_{}_arm_{}_far", match_id, i),
                        );
                    }
                }
            }
            Pattern::Range {
                start,
                end,
                inclusive,
            } => {
                // Range check: value >= start && value <= end (or < end+1 for inclusive)
                if let (
                    crate::ast::Expr::Literal(crate::ast::Literal::Integer(start_val)),
                    crate::ast::Expr::Literal(crate::ast::Literal::Integer(end_val)),
                ) = (&start.node, &end.node)
                {
                    let arm_label = format!("match_{}_arm_{}", match_id, i);
                    let skip_label = format!("match_{}_arm_{}_end", match_id, i);
                    let upper_bound = if *inclusive { end_val + 1 } else { *end_val };

                    if scrutinee_is_signed {
                        // Signed range: (value < start) skips; (value < end+1)
                        // matches. Signed "less than" is (N eor V) after a
                        // subtraction, folded into N via EOR #$80 on overflow.
                        // Comparing against 0 is just a sign-bit test (BMI) — and
                        // it avoids emitting `SEC; SBC #$00`, which the peephole
                        // eliminates as a value no-op even though its flags matter.
                        let emit_signed_lt =
                            |emitter: &mut Emitter,
                             bound: i64,
                             target: &str,
                             tag: &str,
                             far: bool| {
                                emitter.emit_inst("LDA", "$20");
                                let branch = |emitter: &mut Emitter, target: &str, tag: &str| {
                                    if far {
                                        emit_far_arm_branch(
                                            emitter,
                                            "BMI",
                                            target,
                                            &format!("match_{}_arm_{}_{}far", match_id, i, tag),
                                        );
                                    } else {
                                        emitter.emit_inst("BMI", target);
                                    }
                                };
                                if bound == 0 {
                                    branch(emitter, target, tag); // value < 0 == sign set
                                } else {
                                    let nov = format!("match_{}_arm_{}_{}", match_id, i, tag);
                                    emitter.emit_inst("SEC", "");
                                    emitter.emit_inst("SBC", &format!("#${:02X}", bound as u8));
                                    emitter.emit_inst("BVC", &nov);
                                    emitter.emit_inst("EOR", "#$80");
                                    emitter.emit_label(&nov);
                                    branch(emitter, target, tag);
                                }
                            };
                        emit_signed_lt(emitter, *start_val, &skip_label, "v1", false); // < start -> skip
                        emit_signed_lt(emitter, upper_bound, &arm_label, "v2", true); // <= end -> match
                    } else {
                        emitter.emit_inst("LDA", "$20");
                        // Check if value < start, skip this arm
                        emitter.emit_inst("CMP", &format!("#${:02X}", start_val));
                        emitter.emit_inst("BCC", &skip_label);
                        // Check if value <= end (or < end+1)
                        emitter.emit_inst("CMP", &format!("#${:02X}", upper_bound));
                        emit_far_arm_branch(
                            emitter,
                            "BCC",
                            &arm_label,
                            &format!("match_{}_arm_{}_far", match_id, i),
                        );
                    }

                    emitter.emit_label(&skip_label);
                }
            }
            Pattern::Wildcard => {
                // Wildcard catches everything - no comparison needed
                has_wildcard = true;
                emitter.emit_inst("JMP", &format!("match_{}_arm_{}", match_id, i));
            }
            Pattern::Variable(name) => {
                // Variable pattern binds the whole matched value - a catch-all
                // like wildcard, but the scrutinee must be copied into the
                // binding's storage so the arm body can read it.
                has_wildcard = true;
                // Sema records the binding under the arm's pattern span (the
                // arm-body scope is gone by codegen, so a name lookup fails).
                let loc = info
                    .resolved_symbols
                    .get(&arm.pattern.span)
                    .map(|sym| sym.location.clone())
                    .ok_or_else(|| CodegenError::SymbolNotFound(name.clone()))?;
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
                emitter.emit_inst("JMP", &format!("match_{}_arm_{}", match_id, i));
            }
            Pattern::EnumVariant {
                enum_name,
                variant,
                bindings,
            } => {
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

                // Compare the tag with the expected variant tag
                emitter.emit_inst("LDA", &format!("${:02X}", ptr_base + 2)); // Load stored tag
                emitter.emit_inst("CMP", &format!("#${:02X}", variant_info.tag));
                emit_far_arm_branch(
                    emitter,
                    "BEQ",
                    &format!("match_{}_arm_{}", match_id, i),
                    &format!("match_{}_arm_{}_far", match_id, i),
                );

                // If bindings are present, we'll extract them in the arm body
                // For now, we just check the tag - bindings will be handled later
                if !bindings.is_empty() {
                    emitter.emit_comment(&format!("Variant has {} binding(s)", bindings.len()));
                }
            }
        }
    }

    // If no pattern matched and no wildcard, this is an error (should be caught in semantic analysis)
    if !has_wildcard {
        emitter.emit_comment("No pattern matched - should not reach here");
    }

    // Generate arm bodies
    generate_match_arm_bodies(arms, emitter, info, string_collector, match_id)?;

    emitter.emit_label(&format!("match_{}_end", match_id));

    Ok(())
}

/// Generate match statement using jump table dispatch
///
/// Used for enum matches with 3+ arms to avoid BEQ branch distance limitations.
/// The jump table allows arm bodies to be arbitrarily large.
fn generate_match_jump_table(
    expr: &Spanned<crate::ast::Expr>,
    arms: &[crate::ast::MatchArm],
    emitter: &mut Emitter,
    info: &ProgramInfo,
    string_collector: &mut StringCollector,
    max_tag: u8,
    wildcard_arm_index: Option<usize>,
) -> Result<(), CodegenError> {
    let match_id = emitter.next_match_id();
    let jump_ptr = emitter.memory_layout.jump_ptr();

    emitter.emit_comment("Match statement (jump table)");

    // Evaluate the matched expression into accumulator
    // For enum matching, expression returns a pointer in A:X
    generate_expr(expr, emitter, info, string_collector)?;

    // Use pointer ops area for indirect addressing
    let ptr_base = emitter.memory_layout.pointer_ops_start;

    // Store pointer at pointer ops area (not $20 which conflicts with temp storage)
    emitter.emit_inst("STA", &format!("${:02X}", ptr_base));
    emitter.emit_inst("STX", &format!("${:02X}", ptr_base + 1));

    // Load the discriminant tag from the enum (first byte)
    emitter.emit_inst("LDY", "#$00");
    emitter.emit_inst("LDA", &format!("(${:02X}),Y", ptr_base));
    emitter.emit_inst("STA", &format!("${:02X}", ptr_base + 2)); // Store tag for binding extraction

    // Jump table dispatch:
    // 1. Double the tag (addresses are 2 bytes)
    // 2. Use as index into jump table
    // 3. Load address and JMP indirect
    emitter.emit_inst("ASL", ""); // tag * 2
    emitter.emit_inst("TAX", ""); // Transfer to X for indexing
    emitter.emit_inst("LDA", &format!("match_{}_jt,X", match_id));
    emitter.emit_inst("STA", &format!("${:02X}", jump_ptr));
    emitter.emit_inst("LDA", &format!("match_{}_jt+1,X", match_id));
    emitter.emit_inst("STA", &format!("${:02X}", jump_ptr + 1));
    emitter.emit_inst("JMP", &format!("(${:02X})", jump_ptr));

    // Emit jump table
    emit_jump_table(emitter, arms, info, match_id, max_tag, wildcard_arm_index)?;

    // Generate arm bodies
    generate_match_arm_bodies(arms, emitter, info, string_collector, match_id)?;

    // Panic handler for non-exhaustive matches (if no wildcard)
    if wildcard_arm_index.is_none() {
        emitter.emit_label(&format!("match_{}_panic", match_id));
        emitter.emit_comment("Unreachable - non-exhaustive match");
        emitter.emit_inst("BRK", "");
    }

    emitter.emit_label(&format!("match_{}_end", match_id));

    Ok(())
}

/// Emit the jump table for a match statement
///
/// The table contains .WORD entries for each tag value from 0 to max_tag.
/// Missing tags are filled with the wildcard arm label (or panic label if no wildcard).
fn emit_jump_table(
    emitter: &mut Emitter,
    arms: &[crate::ast::MatchArm],
    info: &ProgramInfo,
    match_id: u32,
    max_tag: u8,
    wildcard_arm_index: Option<usize>,
) -> Result<(), CodegenError> {
    use crate::ast::Pattern;

    // Build mapping from tag -> arm index
    let mut tag_to_arm: Vec<Option<usize>> = vec![None; (max_tag + 1) as usize];

    for (arm_index, arm) in arms.iter().enumerate() {
        if let Pattern::EnumVariant {
            enum_name, variant, ..
        } = &arm.pattern.node
            && let Some(enum_def) = info.type_registry.get_enum(&enum_name.node)
            && let Some(variant_info) = enum_def.get_variant(&variant.node)
            && (variant_info.tag as usize) < tag_to_arm.len()
        {
            tag_to_arm[variant_info.tag as usize] = Some(arm_index);
        }
    }

    // Emit jump table label
    emitter.emit_label(&format!("match_{}_jt", match_id));

    // Emit .WORD entries for each tag
    for tag in 0..=max_tag {
        let arm_label = if let Some(arm_index) = tag_to_arm[tag as usize] {
            format!("match_{}_arm_{}", match_id, arm_index)
        } else if let Some(wildcard_index) = wildcard_arm_index {
            format!("match_{}_arm_{}", match_id, wildcard_index)
        } else {
            format!("match_{}_panic", match_id)
        };
        emitter.emit_word_label(&arm_label);
    }

    Ok(())
}

/// Generate arm bodies for a match statement
///
/// Shared between sequential and jump table strategies.
/// Copy an enum variant's payload fields into their pattern bindings' storage.
///
/// `ptr_base`/`ptr_base+1` hold the enum pointer (low/high); field data begins
/// one byte past the tag. Every byte of each field is copied so multi-byte
/// (u16/i16/b16) payloads keep their high byte. Shared by the match-statement
/// and match-expression code paths so both extract bindings identically.
pub(crate) fn extract_enum_bindings(
    enum_name: &Spanned<String>,
    variant: &Spanned<String>,
    bindings: &[crate::ast::PatternBinding],
    ptr_base: u8,
    emitter: &mut Emitter,
    info: &ProgramInfo,
) -> Result<(), CodegenError> {
    if bindings.is_empty() {
        return Ok(());
    }

    // Look up the enum definition to get field information.
    let enum_def = info
        .type_registry
        .get_enum(&enum_name.node)
        .ok_or_else(|| {
            CodegenError::UnsupportedOperation(format!(
                "enum '{}' not found in type registry",
                enum_name.node
            ))
        })?;

    let variant_info = enum_def.get_variant(&variant.node).ok_or_else(|| {
        CodegenError::UnsupportedOperation(format!(
            "variant '{}' not found in enum '{}'",
            variant.node, enum_name.node
        ))
    })?;

    // Copy `field_size` bytes from enum data at `offset` into the binding's slot.
    let copy_field = |offset: u8,
                      field_size: u8,
                      binding: &crate::ast::PatternBinding,
                      emitter: &mut Emitter|
     -> Result<(), CodegenError> {
        let loc = info
            .resolved_symbols
            .get(&binding.name.span)
            .map(|sym| sym.location.clone())
            .ok_or_else(|| CodegenError::SymbolNotFound(binding.name.node.clone()))?;
        for byte in 0..field_size {
            emitter.emit_inst("LDY", &format!("#${:02X}", offset + byte));
            emitter.emit_inst("LDA", &format!("(${:02X}),Y", ptr_base));
            match loc {
                crate::sema::table::SymbolLocation::ZeroPage(addr) => {
                    emitter.emit_sta_zp(addr + byte);
                }
                crate::sema::table::SymbolLocation::Absolute(addr) => {
                    emitter.emit_sta_abs(addr + byte as u16);
                }
                _ => {
                    return Err(CodegenError::UnsupportedOperation(format!(
                        "Binding '{}' has unsupported location",
                        binding.name.node
                    )));
                }
            }
        }
        Ok(())
    };

    match &variant_info.data {
        crate::sema::type_defs::VariantData::Tuple(field_types) => {
            // Tuple variant: extract each field by position.
            if bindings.len() != field_types.len() {
                return Err(CodegenError::UnsupportedOperation(format!(
                    "Pattern binding count mismatch: expected {}, got {}",
                    field_types.len(),
                    bindings.len()
                )));
            }
            let mut offset = 1u8; // Start after the tag byte
            for (binding, field_type) in bindings.iter().zip(field_types.iter()) {
                let field_size = field_type.size().max(1) as u8;
                copy_field(offset, field_size, binding, emitter)?;
                offset += field_size;
            }
        }
        crate::sema::type_defs::VariantData::Struct(struct_fields) => {
            // Struct variant: each binding name selects a field; offsets are
            // relative to the variant data, one byte past the tag.
            for binding in bindings.iter() {
                let field = struct_fields
                    .iter()
                    .find(|f| f.name == binding.name.node)
                    .ok_or_else(|| {
                        CodegenError::UnsupportedOperation(format!(
                            "field '{}' not found in struct variant",
                            binding.name.node
                        ))
                    })?;
                let field_size = field.ty.size().max(1) as u8;
                let base_offset = 1 + field.offset as u8; // skip tag
                copy_field(base_offset, field_size, binding, emitter)?;
            }
        }
        crate::sema::type_defs::VariantData::Unit => {
            // Unit variant shouldn't have bindings.
            return Err(CodegenError::UnsupportedOperation(
                "Unit variant should not have bindings".to_string(),
            ));
        }
    }

    Ok(())
}

fn generate_match_arm_bodies(
    arms: &[crate::ast::MatchArm],
    emitter: &mut Emitter,
    info: &ProgramInfo,
    string_collector: &mut StringCollector,
    match_id: u32,
) -> Result<(), CodegenError> {
    use crate::ast::Pattern;

    for (i, arm) in arms.iter().enumerate() {
        emitter.emit_label(&format!("match_{}_arm_{}", match_id, i));

        // Extract bindings for enum variant patterns
        if let Pattern::EnumVariant {
            enum_name,
            variant,
            bindings,
        } = &arm.pattern.node
        {
            let ptr_base = emitter.memory_layout.pointer_ops_start;
            extract_enum_bindings(enum_name, variant, bindings, ptr_base, emitter, info)?;
        }

        generate_stmt(&arm.body, emitter, info, string_collector)?;

        // Only emit JMP if the arm body doesn't already terminate control flow
        // (e.g., return, break, continue) - this eliminates dead code
        if !stmt_terminates(&arm.body.node) {
            emitter.emit_inst("JMP", &format!("match_{}_end", match_id));
        }
    }

    Ok(())
}

/// Emit `*p = value`.
///
/// The pointer is staged first and the value parked in the high pool, mirroring
/// `generate_index_assignment`: evaluating the value can clobber whatever is in
/// zero page, so the two cannot simply be produced in order.
fn generate_deref_assignment(
    ptr_expr: &Spanned<crate::ast::Expr>,
    value: &Spanned<crate::ast::Expr>,
    emitter: &mut Emitter,
    info: &ProgramInfo,
    string_collector: &mut StringCollector,
) -> Result<(), CodegenError> {
    use crate::ast::PrimitiveType;
    use crate::sema::table::SymbolLocation;
    use crate::sema::types::Type;

    let is_multibyte = matches!(
        info.resolved_types.get(&ptr_expr.span),
        Some(Type::Pointer(inner))
            if matches!(
                **inner,
                Type::Primitive(PrimitiveType::U16)
                    | Type::Primitive(PrimitiveType::I16)
                    | Type::Primitive(PrimitiveType::B16)
            )
    );
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
        // A 16-bit value arrives as A:Y.
        emitter.emit_inst("STY", &format!("${:02X}", save + 1));
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
fn generate_local_array_init(
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

    match &init.node {
        // `[v; n]` — every element the same. A byte-wide zero (or any uniform
        // byte) over a block worth looping for becomes a loop; otherwise the
        // stores are cheaper than the loop overhead.
        Expr::Literal(Literal::ArrayFill { value, count }) => {
            let v = const_of(value).ok_or_else(|| {
                CodegenError::UnsupportedOperation(
                    "array fill value must be a constant expression".to_string(),
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
                let v = const_of(e).ok_or_else(|| {
                    CodegenError::UnsupportedOperation(
                        "array elements must be constant expressions".to_string(),
                    )
                })?;
                store_elem(emitter, addr, i as u16 * elem_size as u16, v, elem_size);
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

/// Substitute {variable} patterns in inline assembly with actual addresses
fn substitute_asm_vars(
    instruction: &str,
    info: &ProgramInfo,
    current_function: Option<&str>,
) -> Result<String, CodegenError> {
    let mut result = instruction.to_string();

    // Find all {var} patterns
    while let Some(start) = result.find('{') {
        if let Some(end) = result[start..].find('}') {
            let end = start + end;
            let var_name = &result[start + 1..end];

            // Look up the variable in resolved_symbols (by name)
            // We search through resolved_symbols because the symbol table's scopes
            // have been exited after semantic analysis
            // Priority: 1) Local variables in current function, 2) Global symbols
            let symbol = info
                .resolved_symbols
                .values()
                .find(|s| {
                    s.name == var_name
                        && (s.containing_function.as_deref() == current_function
                            || s.containing_function.is_none())
                })
                // Prefer local over global if both exist with same name
                .or_else(|| {
                    info.resolved_symbols
                        .values()
                        .find(|s| s.name == var_name && s.containing_function.is_none())
                })
                .ok_or_else(|| CodegenError::SymbolNotFound(var_name.to_string()))?;

            // Convert the location to an address string
            let address = match symbol.location {
                crate::sema::table::SymbolLocation::ZeroPage(addr) => format!("${:02X}", addr),
                crate::sema::table::SymbolLocation::Absolute(addr) => format!("${:04X}", addr),
                crate::sema::table::SymbolLocation::None => {
                    return Err(CodegenError::SymbolNotFound(format!(
                        "{} has no memory location",
                        var_name
                    )));
                }
                crate::sema::table::SymbolLocation::FrameOffset(_) => {
                    return Err(CodegenError::Internal(
                        "unresolved FrameOffset reached codegen (frame finalization skipped)"
                            .to_string(),
                    ));
                }
            };

            // Replace {var} with the address
            result.replace_range(start..=end, &address);
        } else {
            // Unmatched {, just break
            break;
        }
    }

    Ok(result)
}

/// Uniquify assembly labels by appending a suffix
/// This is needed when inlining functions to avoid duplicate label errors
fn uniquify_asm_labels(line: &str, suffix: usize) -> String {
    let trimmed = line.trim();

    // Check if this is a label definition (ends with :)
    if let Some(label_name) = trimmed.strip_suffix(':') {
        // Label definition: append suffix before the colon
        return format!("{}_{}:", label_name, suffix);
    }

    // Check if line contains a label reference
    // Label references are typically in the operand part of an instruction
    let parts: Vec<&str> = trimmed.split_whitespace().collect();
    if parts.len() < 2 {
        // No operand, return as-is
        return line.to_string();
    }

    let mnemonic = parts[0];
    let operand = parts[1..].join(" ");

    // Special case: BBS/BBR instructions have format "BBS0 $20,label"
    // where the label is after a comma
    if (mnemonic.starts_with("BBS") || mnemonic.starts_with("BBR"))
        && let Some(comma_pos) = operand.find(',')
    {
        let addr_part = &operand[..comma_pos];
        let label_part = operand[comma_pos + 1..].trim();
        return format!("{} {},{}_{}", mnemonic, addr_part, label_part, suffix);
    }

    // Check if operand looks like a label reference
    // Labels are alphanumeric/underscore, not registers ($, #, A, X, Y) or numbers
    let is_label_ref = !operand.starts_with('$')  // Not hex address
                    && !operand.starts_with('#')  // Not immediate
                    && operand != "A"              // Not accumulator
                    && !operand.starts_with("A,") // Not indexed
                    && !operand.starts_with("X")  // Not X register
                    && !operand.starts_with("Y")  // Not Y register
                    && operand.chars().next().is_some_and(|c| c.is_alphabetic() || c == '_');

    if is_label_ref {
        // Split operand by comma (for "label,X" style addressing)
        let op_parts: Vec<&str> = operand.split(',').collect();
        let label_part = op_parts[0];
        let rest = if op_parts.len() > 1 {
            format!(",{}", op_parts[1..].join(","))
        } else {
            String::new()
        };

        format!("{} {}_{}{}", mnemonic, label_part, suffix, rest)
    } else {
        line.to_string()
    }
}

/// Generate a normal (non-unrolled) for loop
fn generate_normal_loop(
    var_name: &Spanned<String>,
    range: &crate::ast::Range,
    body: &Spanned<Stmt>,
    emitter: &mut Emitter,
    info: &ProgramInfo,
    string_collector: &mut StringCollector,
) -> Result<(), CodegenError> {
    use crate::ast::PrimitiveType;
    use crate::sema::types::Type;

    // The X-register machinery below is 8-bit: the counter lives in X and only
    // one byte of the range end is ever compared. A 16-bit counter must take
    // the memory-pair path instead, or `for i: u16 in 0..30000` silently runs
    // 0x30 = 48 iterations.
    let counter_ty = info
        .resolved_symbols
        .get(&var_name.span)
        .or_else(|| info.resolved_symbols.get(&body.span))
        .or_else(|| info.table.lookup(&var_name.node))
        .map(|s| &s.ty);

    match counter_ty {
        Some(Type::Primitive(PrimitiveType::U16)) => {
            return generate_normal_loop_u16(
                var_name,
                range,
                body,
                emitter,
                info,
                string_collector,
            );
        }
        Some(Type::Primitive(PrimitiveType::I16 | PrimitiveType::B16)) => {
            // Backstop for sema's rejection: the 16-bit comparison is unsigned
            // (wrong for i16) and the increment is binary (wrong for BCD b16).
            return Err(CodegenError::UnsupportedOperation(format!(
                "for loop counter '{}' is not u16; 16-bit loop counters are currently u16-only",
                var_name.node
            )));
        }
        _ => {}
    }

    // Resolve the loop variable's storage once; every strategy needs it.
    let var_operand = info
        .resolved_symbols
        .get(&var_name.span)
        .or_else(|| info.resolved_symbols.get(&body.span))
        .or_else(|| info.table.lookup(&var_name.node))
        .and_then(|sym| match sym.location {
            crate::sema::table::SymbolLocation::ZeroPage(addr) => Some(format!("${:02X}", addr)),
            crate::sema::table::SymbolLocation::Absolute(addr) => Some(format!("${:04X}", addr)),
            _ => None,
        });

    // Strategy 1: count-down. When the body never mentions the counter, only
    // the iteration count matters, so the loop can decrement its own frame
    // slot toward zero: DEC var / BNE is 8 cycles of overhead per iteration
    // versus ~14 for the counting-up shape, immune to X clobbering, and free
    // of any comparison. Shadowing in the body can only make the reference
    // walk over-report (skipping the optimization), never under-report.
    if let Some(var) = &var_operand
        && !stmt_references_name(&body.node, &var_name.node)
    {
        return generate_countdown_loop(var.clone(), range, body, emitter, info, string_collector);
    }

    // Strategy 2: bottom-test counting loop. The exit test lives at the
    // bottom, so each iteration runs one compare and one (usually short)
    // backward branch; a single entry guard handles empty ranges. The counter
    // is memory-backed: X is reloaded before the increment only when the body
    // could have disturbed it.

    // Initialize loop counter with range start
    generate_expr(&range.start, emitter, info, string_collector)?;
    emitter.emit_inst("TAX", ""); // Counter in X register

    // Range end operand: constant ends compare as an immediate; non-constant
    // ends are evaluated once into the loop's hidden frame slot, which nested
    // loops, scratch-clobbering expressions, and calls in the body cannot
    // touch (unlike the old shared zero-page scratch byte).
    let end_operand = match info.folded_constants.get(&range.end.span) {
        Some(crate::sema::const_eval::ConstValue::Integer(n)) => {
            format!("#${:02X}", *n as u8)
        }
        _ => {
            let slot_addr = loop_bound_slot(var_name, range, info)?;
            generate_expr(&range.end, emitter, info, string_collector)?;
            emitter.emit_inst("STA", &format!("${:02X}", slot_addr));
            format!("${:02X}", slot_addr)
        }
    };

    // Store X (loop counter) to the loop variable location
    if let Some(var) = &var_operand {
        emitter.emit_inst("STX", var);
    }

    let body_label = emitter.next_label("fb");
    let incr_label = emitter.next_label("fi");
    let end_label = emitter.next_label("fx");

    // Entry guard (once): enter only when the range is non-empty.
    emitter.emit_inst("CPX", &end_operand);
    if range.inclusive {
        emitter.emit_inst("BEQ", &body_label); // start == end: one iteration
        emitter.emit_inst("BCC", &body_label); // start < end
    } else {
        emitter.emit_inst("BCC", &body_label); // start < end
    }
    emitter.emit_inst("JMP", &end_label);

    emitter.emit_label(&body_label);

    // `continue` jumps to the increment (not the body start), so it cannot
    // skip the increment.
    emitter.push_loop(incr_label.clone(), end_label.clone());

    // Execute body
    emitter.reg_state.invalidate_all(); // Back-edge target: registers are stale
    let body_output_pos = emitter.output_len();
    let body_bytes_start = emitter.byte_count();
    generate_stmt(body, emitter, info, string_collector)?;

    // Pop loop context
    emitter.pop_loop();

    emitter.emit_label(&incr_label);

    // Reload the counter only if the body could have clobbered X or written
    // the loop variable (nested loops, shift helpers, calls, assignments).
    // Also decide whether the emitted body's byte count can be trusted for
    // the short-branch choice (inline data directives are under-counted).
    let (need_reload, short_ok) = {
        let body_asm = emitter.output_since(body_output_pos);
        let reload = if let Some(var) = &var_operand {
            body_disturbs_counter(body_asm, var)
        } else {
            false
        };
        (reload, !body_defeats_size_estimate(body_asm))
    };
    if need_reload && let Some(var) = &var_operand {
        emitter.emit_inst("LDX", var);
    }

    if range.inclusive {
        // Exit when the counter is at (or, if the body mutated it, past) the
        // endpoint. A bare equality test would miss a body assignment above
        // the endpoint and loop forever; >= also prevents the `..=0xFF` wrap.
        emitter.emit_inst("CPX", &end_operand);
        emitter.emit_inst("BCS", &end_label);
        emitter.emit_inst("INX", "");
        if let Some(var) = &var_operand {
            emitter.emit_inst("STX", var);
        }
        // Unconditional back edge: C is clear here (the BCS above fell
        // through; INX/STX preserve C), so BCC is always taken when in range.
        if short_ok && emitter.byte_count().wrapping_sub(body_bytes_start) <= SHORT_BRANCH_LIMIT {
            emitter.emit_inst("BCC", &body_label);
        } else {
            emitter.emit_inst("JMP", &body_label);
        }
    } else {
        emitter.emit_inst("INX", "");
        if let Some(var) = &var_operand {
            emitter.emit_inst("STX", var);
        }
        // Bottom test: loop while X < end.
        emitter.emit_inst("CPX", &end_operand);
        if short_ok && emitter.byte_count().wrapping_sub(body_bytes_start) <= SHORT_BRANCH_LIMIT {
            emitter.emit_inst("BCC", &body_label);
        } else {
            emitter.emit_inst("BCS", &end_label); // short hop over the JMP
            emitter.emit_inst("JMP", &body_label);
        }
    }
    emitter.emit_label(&end_label);
    emitter.reg_state.invalidate_all();

    Ok(())
}

/// Maximum body size (bytes) for which a bottom-test loop uses a direct
/// backward conditional branch. The 6502 relative branch reaches -128 bytes;
/// the margin covers the increment/compare sequence between the body and the
/// branch plus the branch itself.
const SHORT_BRANCH_LIMIT: u16 = 100;

/// Generate a count-down loop for an 8-bit counter the body never reads.
///
/// The loop variable's frame slot is reused as a pure iteration counter:
///
/// ```text
///     LDA #count      ; or runtime: end - start (guarded, see below)
///     STA var
/// fb:
///     ...body...
/// fi:                 ; continue target
///     DEC var
///     BNE fb          ; 8 cycles/iteration total overhead
/// fx:
/// ```
///
/// A count of 256 (`0..=255`) is encoded as $00: DEC wraps it to $FF first,
/// so the BNE-loop still runs exactly 256 times. Constant empty ranges emit
/// no code at all; runtime bounds get an entry guard that skips the loop when
/// `end <= start` (exclusive) or `end < start` (inclusive).
fn generate_countdown_loop(
    var: String,
    range: &crate::ast::Range,
    body: &Spanned<Stmt>,
    emitter: &mut Emitter,
    info: &ProgramInfo,
    string_collector: &mut StringCollector,
) -> Result<(), CodegenError> {
    let folded = |span: &crate::ast::Span| match info.folded_constants.get(span) {
        Some(crate::sema::const_eval::ConstValue::Integer(n)) => Some(*n),
        _ => None,
    };

    let body_label = emitter.next_label("fb");
    let incr_label = emitter.next_label("fi");
    let end_label = emitter.next_label("fx");

    match (folded(&range.start.span), folded(&range.end.span)) {
        (Some(s), Some(e)) => {
            let count = if range.inclusive { e - s + 1 } else { e - s };
            if count <= 0 {
                // Statically empty range: the loop vanishes entirely.
                return Ok(());
            }
            emitter.emit_inst("LDA", &format!("#${:02X}", (count as u16 & 0xFF) as u8));
            emitter.emit_inst("STA", &var);
        }
        _ => {
            // Runtime bounds: count = end - start, entered only when positive
            // (the subtraction would otherwise wrap into a huge count).
            generate_expr(&range.start, emitter, info, string_collector)?;
            emitter.emit_inst("STA", &var);
            generate_expr(&range.end, emitter, info, string_collector)?;
            let enter_label = emitter.next_label("fg");
            emitter.emit_inst("CMP", &var);
            if range.inclusive {
                emitter.emit_inst("BCS", &enter_label); // end >= start
            } else {
                let skip_label = emitter.next_label("fs");
                emitter.emit_inst("BEQ", &skip_label); // end == start: empty
                emitter.emit_inst("BCS", &enter_label); // end > start
                emitter.emit_label(&skip_label);
            }
            emitter.emit_inst("JMP", &end_label);
            emitter.emit_label(&enter_label);
            // Carry is set on every path here, so SBC computes end - start.
            emitter.emit_inst("SBC", &var);
            emitter.emit_inst("STA", &var);
            if range.inclusive {
                emitter.emit_inst("INC", &var); // count + 1; 256 wraps to $00
            }
        }
    }

    emitter.emit_label(&body_label);
    emitter.push_loop(incr_label.clone(), end_label.clone());
    emitter.reg_state.invalidate_all(); // Back-edge target: registers are stale
    let body_output_pos = emitter.output_len();
    let body_bytes_start = emitter.byte_count();
    generate_stmt(body, emitter, info, string_collector)?;
    emitter.pop_loop();

    emitter.emit_label(&incr_label);
    emitter.emit_inst("DEC", &var);
    let short_ok = !body_defeats_size_estimate(emitter.output_since(body_output_pos));
    if short_ok && emitter.byte_count().wrapping_sub(body_bytes_start) <= SHORT_BRANCH_LIMIT {
        emitter.emit_inst("BNE", &body_label);
    } else {
        emitter.emit_inst("BEQ", &end_label); // short hop over the JMP
        emitter.emit_inst("JMP", &body_label);
    }
    emitter.emit_label(&end_label);
    emitter.reg_state.invalidate_all();

    Ok(())
}

/// Generate a normal (non-unrolled) for loop with a 16-bit counter.
///
/// Unlike the 8-bit path, the counter cannot live in X; it lives in the loop
/// variable's own memory pair (low at `addr`, high at `addr + 1`) and both
/// bytes participate in the comparison and increment. The loop is
/// bottom-tested: one entry guard handles empty ranges, then each iteration
/// pays a single compare and (usually short) backward branch:
///
/// ```text
///     LDA var_lo       ; entry guard (once): enter while var < end
///     CMP end_lo       ;   (var - end borrows => C clear)
///     LDA var_hi
///     SBC end_hi
///     BCC fb
///     JMP fx
/// fb:
///     ...body...
/// fi:                  ; continue target (never skips the increment)
///     INC var_lo
///     BNE fs
///     INC var_hi
/// fs:
///     LDA var_lo       ; bottom test: loop while var < end
///     CMP end_lo
///     LDA var_hi
///     SBC end_hi
///     BCC fb           ; direct when the body is small; JMP trampoline else
/// fx:
/// ```
///
/// Constant range ends are compared as immediates so there is nothing in
/// memory for the body to clobber; non-constant ends are evaluated once into
/// the loop's hidden frame slot pair (allocated by sema and colored with the
/// call graph), so nested loops, scratch-clobbering expressions, and calls in
/// the body cannot corrupt a live bound.
///
/// Inclusive ranges instead test `var == end` before the increment (their
/// only per-iteration test, with an unconditional back edge), so `..=0xFFFF`
/// cannot wrap the counter to zero and loop forever.
fn generate_normal_loop_u16(
    var_name: &Spanned<String>,
    range: &crate::ast::Range,
    body: &Spanned<Stmt>,
    emitter: &mut Emitter,
    info: &ProgramInfo,
    string_collector: &mut StringCollector,
) -> Result<(), CodegenError> {
    use crate::ast::PrimitiveType;
    use crate::sema::table::SymbolLocation;
    use crate::sema::types::Type;

    let sym = info
        .resolved_symbols
        .get(&var_name.span)
        .or_else(|| info.resolved_symbols.get(&body.span))
        .or_else(|| info.table.lookup(&var_name.node))
        .ok_or_else(|| CodegenError::SymbolNotFound(var_name.node.clone()))?;

    let (var_lo, var_hi) = match sym.location {
        SymbolLocation::ZeroPage(addr) => (
            format!("${:02X}", addr),
            format!("${:02X}", addr.wrapping_add(1)),
        ),
        SymbolLocation::Absolute(addr) => (format!("${:04X}", addr), format!("${:04X}", addr + 1)),
        _ => {
            return Err(CodegenError::Internal(format!(
                "16-bit loop variable '{}' has no direct memory location",
                var_name.node
            )));
        }
    };

    // True when the expression at `span` already produces a 16-bit value
    // (low in A, high in Y); an 8-bit bound needs Y zero-extended by hand.
    let expr_is_16bit = |span| {
        info.resolved_types.get(&span).is_some_and(|ty| {
            matches!(
                ty,
                Type::Primitive(PrimitiveType::U16 | PrimitiveType::I16 | PrimitiveType::B16)
            )
        })
    };

    // Initialize the counter from the range start.
    generate_expr(&range.start, emitter, info, string_collector)?;
    if !expr_is_16bit(range.start.span) {
        emitter.emit_inst("LDY", "#$00"); // zero-extend 8-bit start
    }
    emitter.emit_inst("STA", &var_lo);
    emitter.emit_inst("STY", &var_hi);

    // Range end operands for the per-iteration comparison.
    let folded_end = match info.folded_constants.get(&range.end.span) {
        Some(crate::sema::const_eval::ConstValue::Integer(n)) => Some(*n as u16),
        _ => None,
    };
    let (end_lo, end_hi) = if let Some(v) = folded_end {
        (format!("#${:02X}", v & 0xFF), format!("#${:02X}", v >> 8))
    } else {
        let slot_addr = loop_bound_slot(var_name, range, info)?;
        generate_expr(&range.end, emitter, info, string_collector)?;
        if !expr_is_16bit(range.end.span) {
            emitter.emit_inst("LDY", "#$00"); // zero-extend 8-bit end
        }
        emitter.emit_inst("STA", &format!("${:02X}", slot_addr));
        emitter.emit_inst("STY", &format!("${:02X}", slot_addr.wrapping_add(1)));
        (
            format!("${:02X}", slot_addr),
            format!("${:02X}", slot_addr.wrapping_add(1)),
        )
    };

    let body_label = emitter.next_label("fb");
    let incr_label = emitter.next_label("fi");
    let end_label = emitter.next_label("fx");

    // Entry guard (once): enter only when the range is non-empty. The
    // per-iteration test lives at the bottom instead, saving the JMP-around
    // every iteration.
    if range.inclusive {
        // Enter while var <= end  <=>  end - var does not borrow (C set).
        emitter.emit_inst("LDA", &end_lo);
        emitter.emit_inst("CMP", &var_lo);
        emitter.emit_inst("LDA", &end_hi);
        emitter.emit_inst("SBC", &var_hi);
        emitter.emit_inst("BCS", &body_label);
    } else {
        // Enter while var < end  <=>  var - end borrows (C clear).
        emitter.emit_inst("LDA", &var_lo);
        emitter.emit_inst("CMP", &end_lo);
        emitter.emit_inst("LDA", &var_hi);
        emitter.emit_inst("SBC", &end_hi);
        emitter.emit_inst("BCC", &body_label);
    }
    emitter.emit_inst("JMP", &end_label);
    emitter.emit_label(&body_label);

    // `continue` jumps to the increment, `break` to the end label.
    emitter.push_loop(incr_label.clone(), end_label.clone());
    emitter.reg_state.invalidate_all(); // Back-edge target: registers are stale
    let body_output_pos = emitter.output_len();
    let body_bytes_start = emitter.byte_count();
    generate_stmt(body, emitter, info, string_collector)?;
    emitter.pop_loop();

    let (short_ok, body_writes_counter) = {
        let body_asm = emitter.output_since(body_output_pos);
        (
            !body_defeats_size_estimate(body_asm),
            body_may_write_addresses(body_asm, &[&var_lo, &var_hi]),
        )
    };

    emitter.emit_label(&incr_label);
    if range.inclusive {
        // Exit when the counter reaches the endpoint. When the body can write
        // the counter, a bare equality test would miss an assignment above
        // the endpoint and loop forever, so a full >= compare is required;
        // otherwise the counter is exactly start + iterations (entry-guarded
        // to var <= end), it hits the endpoint precisely, and the cheaper
        // equality pair suffices - including at `..=0xFFFF`, which it reaches
        // before any wrap. This is the loop's only per-iteration test.
        if body_writes_counter {
            emitter.emit_inst("LDA", &var_lo);
            emitter.emit_inst("CMP", &end_lo);
            emitter.emit_inst("LDA", &var_hi);
            emitter.emit_inst("SBC", &end_hi);
            emitter.emit_inst("BCS", &end_label); // var >= end: done
        } else {
            let go_label = emitter.next_label("fg");
            emitter.emit_inst("LDA", &var_lo);
            emitter.emit_inst("CMP", &end_lo);
            emitter.emit_inst("BNE", &go_label);
            emitter.emit_inst("LDA", &var_hi);
            emitter.emit_inst("CMP", &end_hi);
            emitter.emit_inst("BEQ", &end_label); // var == end: done
            emitter.emit_label(&go_label);
        }
        let skip_label = emitter.next_label("fs");
        emitter.emit_inst("INC", &var_lo);
        emitter.emit_inst("BNE", &skip_label);
        emitter.emit_inst("INC", &var_hi);
        emitter.emit_label(&skip_label);
        if short_ok && emitter.byte_count().wrapping_sub(body_bytes_start) <= SHORT_BRANCH_LIMIT {
            // The back edge is unconditional here. In the >=-shape, C is
            // clear (the BCS fell through and INC preserves C), so BCC is
            // always taken. In the equality shape, Z reflects the last INC's
            // result, which is never zero mid-loop (a zero would mean the
            // counter wrapped past 0xFFFF, impossible before the equality
            // exit fires), so BNE is always taken.
            if body_writes_counter {
                emitter.emit_inst("BCC", &body_label);
            } else {
                emitter.emit_inst("BNE", &body_label);
            }
        } else {
            emitter.emit_inst("JMP", &body_label);
        }
    } else {
        // 16-bit increment, then bottom test: loop while var < end.
        let skip_label = emitter.next_label("fs");
        emitter.emit_inst("INC", &var_lo);
        emitter.emit_inst("BNE", &skip_label);
        emitter.emit_inst("INC", &var_hi);
        emitter.emit_label(&skip_label);
        emitter.emit_inst("LDA", &var_lo);
        emitter.emit_inst("CMP", &end_lo);
        emitter.emit_inst("LDA", &var_hi);
        emitter.emit_inst("SBC", &end_hi);
        if short_ok && emitter.byte_count().wrapping_sub(body_bytes_start) <= SHORT_BRANCH_LIMIT {
            emitter.emit_inst("BCC", &body_label);
        } else {
            emitter.emit_inst("BCS", &end_label); // short hop over the JMP
            emitter.emit_inst("JMP", &body_label);
        }
    }

    emitter.emit_label(&end_label);
    emitter.reg_state.invalidate_all();

    Ok(())
}

/// Resolve the hidden frame slot sema allocated for a for-loop's non-constant
/// range end (low byte at the returned address, high byte - for 16-bit
/// counters - at the next one).
fn loop_bound_slot(
    var_name: &Spanned<String>,
    range: &crate::ast::Range,
    info: &ProgramInfo,
) -> Result<u8, CodegenError> {
    match info
        .loop_bound_slots
        .get(&range.end.span)
        .map(|s| &s.location)
    {
        Some(crate::sema::table::SymbolLocation::ZeroPage(addr)) => Ok(*addr),
        _ => Err(CodegenError::Internal(format!(
            "for loop over '{}' has a non-constant range end but no bound slot was allocated",
            var_name.node
        ))),
    }
}

/// True when the statement tree references the given variable name.
///
/// Used to decide whether a for-loop counter is observable by its body.
/// Shadowing (a nested binding of the same name) makes this return true even
/// though the outer variable is not really referenced - over-reporting only
/// disables an optimization, never miscompiles. Inline assembly is opaque and
/// counts as a reference.
fn stmt_references_name(stmt: &Stmt, name: &str) -> bool {
    use crate::ast::Stmt as S;
    match stmt {
        S::VarDecl {
            name: n,
            init,
            ty: _,
            mutable: _,
        } => n.node == name || expr_references_name(&init.node, name),
        S::Assign { target, value } => {
            expr_references_name(&target.node, name) || expr_references_name(&value.node, name)
        }
        S::Expr(e) => expr_references_name(&e.node, name),
        S::Return(Some(e)) => expr_references_name(&e.node, name),
        S::Return(None) | S::Break | S::Continue => false,
        S::If {
            condition,
            then_branch,
            else_branch,
        } => {
            expr_references_name(&condition.node, name)
                || stmt_references_name(&then_branch.node, name)
                || else_branch
                    .as_ref()
                    .is_some_and(|e| stmt_references_name(&e.node, name))
        }
        S::While { condition, body } => {
            expr_references_name(&condition.node, name) || stmt_references_name(&body.node, name)
        }
        S::Loop { body } => stmt_references_name(&body.node, name),
        S::For {
            var_name,
            range,
            body,
            var_type: _,
        } => {
            var_name.node == name
                || expr_references_name(&range.start.node, name)
                || expr_references_name(&range.end.node, name)
                || stmt_references_name(&body.node, name)
        }
        S::ForEach {
            var_name,
            iterable,
            body,
            index_var,
            var_type: _,
        } => {
            var_name.node == name
                || index_var.as_ref().is_some_and(|v| v.node == name)
                || expr_references_name(&iterable.node, name)
                || stmt_references_name(&body.node, name)
        }
        S::Match { expr, arms } => {
            expr_references_name(&expr.node, name)
                || arms
                    .iter()
                    .any(|arm| stmt_references_name(&arm.body.node, name))
        }
        S::Block(stmts) => stmts.iter().any(|s| stmt_references_name(&s.node, name)),
        S::Asm { .. } => true, // opaque: may reference anything
    }
}

/// Expression half of [`stmt_references_name`].
fn expr_references_name(expr: &crate::ast::Expr, name: &str) -> bool {
    use crate::ast::Expr as E;
    use crate::ast::VariantData;
    match expr {
        E::Variable(n) => n == name,
        E::Literal(_)
        | E::CpuFlagCarry
        | E::CpuFlagZero
        | E::CpuFlagOverflow
        | E::CpuFlagNegative => false,
        E::Binary { left, right, .. } => {
            expr_references_name(&left.node, name) || expr_references_name(&right.node, name)
        }
        E::Unary { operand, .. } => expr_references_name(&operand.node, name),
        E::Cast { expr, .. } => expr_references_name(&expr.node, name),
        E::Field { object, .. } => expr_references_name(&object.node, name),
        E::Index { object, index } => {
            expr_references_name(&object.node, name) || expr_references_name(&index.node, name)
        }
        E::Slice {
            object, start, end, ..
        } => {
            expr_references_name(&object.node, name)
                || expr_references_name(&start.node, name)
                || expr_references_name(&end.node, name)
        }
        E::Call { args, .. } => args.iter().any(|a| expr_references_name(&a.node, name)),
        // An indirect call's target is unknown to the frame-coloring pass, so
        // the callee's frame may overlap this function's - treat it as
        // touching everything (disables the count-down optimization).
        E::CallIndirect { .. } => true,
        E::StructInit { fields, .. } | E::AnonStructInit { fields } => fields
            .iter()
            .any(|f| expr_references_name(&f.value.node, name)),
        E::EnumVariant { data, .. } => match data {
            VariantData::Unit => false,
            VariantData::Tuple(exprs) => exprs.iter().any(|e| expr_references_name(&e.node, name)),
            VariantData::Struct(fields) => fields
                .iter()
                .any(|f| expr_references_name(&f.value.node, name)),
        },
        E::SliceLen(e) | E::U16Low(e) | E::U16High(e) | E::Paren(e) => {
            expr_references_name(&e.node, name)
        }
        E::Match { expr, arms } => {
            expr_references_name(&expr.node, name)
                || arms
                    .iter()
                    .any(|arm| expr_references_name(&arm.body.node, name))
        }
    }
}

/// True when the emitted body assembly may change X or write the loop
/// variable's storage, in which case the counting loop must reload X from
/// memory before incrementing.
///
/// Conservative by construction: only a whitelist of X-preserving mnemonics
/// passes, any write through a pointer (`(zp),Y` / `(zp,X)` operands) counts
/// as touching the variable, and anything unrecognized (JSR, RTS, inline
/// assembly passthrough) counts as a clobber.
fn body_disturbs_counter(body_asm: &str, var_operand: &str) -> bool {
    for raw in body_asm.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with(';') || line.ends_with(':') || line.starts_with('.')
        {
            continue;
        }
        let mut parts = line.split_whitespace();
        let mnem = parts.next().unwrap_or("");
        let operand = parts.next().unwrap_or("");

        let x_preserved = matches!(
            mnem,
            "LDA"
                | "STA"
                | "LDY"
                | "STY"
                | "STX"
                | "CMP"
                | "CPX"
                | "CPY"
                | "ADC"
                | "SBC"
                | "AND"
                | "ORA"
                | "EOR"
                | "ASL"
                | "LSR"
                | "ROL"
                | "ROR"
                | "INC"
                | "DEC"
                | "INY"
                | "DEY"
                | "TAY"
                | "TYA"
                | "TXA"
                | "TXS"
                | "CLC"
                | "SEC"
                | "CLV"
                | "CLD"
                | "SED"
                | "CLI"
                | "SEI"
                | "BIT"
                | "NOP"
                | "PHA"
                | "PLA"
                | "PHP"
                | "PLP"
                | "BCC"
                | "BCS"
                | "BEQ"
                | "BNE"
                | "BMI"
                | "BPL"
                | "BVC"
                | "BVS"
                | "JMP"
        );
        if !x_preserved {
            return true;
        }

        // Writes that could hit the loop variable's storage.
        let writes_mem = matches!(
            mnem,
            "STA" | "STX" | "STY" | "INC" | "DEC" | "ASL" | "LSR" | "ROL" | "ROR"
        );
        if writes_mem && (operand.contains(var_operand) || operand.starts_with('(')) {
            return true;
        }
    }
    false
}

/// True when the emitted body contains lines whose byte size the emitter's
/// instruction-size accounting cannot estimate reliably - assembler
/// directives like `.BYTE`/`.WORD`/`.RES` from inline assembly can span any
/// number of bytes but are counted as a single instruction. Such bodies must
/// use the JMP-trampoline back edge instead of a direct relative branch.
fn body_defeats_size_estimate(body_asm: &str) -> bool {
    body_asm
        .lines()
        .any(|line| line.trim_start().starts_with('.'))
}

/// True when the emitted body assembly may write any of the given memory
/// operands. Conservative: pointer-indirect writes, unrecognized mnemonics,
/// and calls (whose callees could write through pointers) all count as
/// potential writes.
fn body_may_write_addresses(body_asm: &str, addrs: &[&str]) -> bool {
    for raw in body_asm.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with(';') || line.ends_with(':') || line.starts_with('.')
        {
            continue;
        }
        let mut parts = line.split_whitespace();
        let mnem = parts.next().unwrap_or("");
        let operand = parts.next().unwrap_or("");

        let known_non_writing = matches!(
            mnem,
            "LDA"
                | "LDX"
                | "LDY"
                | "CMP"
                | "CPX"
                | "CPY"
                | "ADC"
                | "SBC"
                | "AND"
                | "ORA"
                | "EOR"
                | "BIT"
                | "TAX"
                | "TAY"
                | "TXA"
                | "TYA"
                | "TXS"
                | "TSX"
                | "INX"
                | "INY"
                | "DEX"
                | "DEY"
                | "CLC"
                | "SEC"
                | "CLV"
                | "CLD"
                | "SED"
                | "CLI"
                | "SEI"
                | "NOP"
                | "PHA"
                | "PLA"
                | "PHP"
                | "PLP"
                | "BCC"
                | "BCS"
                | "BEQ"
                | "BNE"
                | "BMI"
                | "BPL"
                | "BVC"
                | "BVS"
                | "JMP"
        );
        let known_writing = matches!(
            mnem,
            "STA" | "STX" | "STY" | "INC" | "DEC" | "ASL" | "LSR" | "ROL" | "ROR"
        );

        if known_writing {
            if operand.starts_with('(') || addrs.iter().any(|a| operand.contains(*a)) {
                return true;
            }
        } else if !known_non_writing {
            // JSR, RTS, inline-asm oddities: assume the worst.
            return true;
        }
    }
    false
}
