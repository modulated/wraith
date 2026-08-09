//! Loop codegen: `for`/`foreach`/`while`/`loop` lowering, the u8/u16 and
//! countdown loop shapes, and the body-analysis helpers that pick between them.

use super::match_stmt::emit_far_arm_branch;
use super::*;

/// Iterate a slice with a 16-bit counter (slices can exceed 255 elements). The
/// slice descriptor (`slot[0..1]` = base, `slot[2..3]` = length) is re-read each
/// iteration, and an element pointer is recomputed into $F0/$F1, so the loop is
/// correct even for lengths past 255 and across page boundaries. The counter
/// lives in a hidden frame slot (allocated by sema) so it survives the body.
#[allow(clippy::too_many_arguments)]
pub(super) fn generate_foreach_slice(
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

    // Exit when counter >= length (unsigned 16-bit compare). end_label is past
    // the whole body, so the two exit branches route through a nearby JMP: the
    // conditional branches all hop short distances and only the JMP is far,
    // which keeps a large body from overflowing the ±127 branch range.
    let body_label = emitter.next_label("fsb");
    let exit_label = emitter.next_label("fsx");
    emitter.emit_inst("LDA", &format!("${:02X}", counter + 1));
    emitter.emit_inst("CMP", &format!("${:02X}", slot + 3));
    emitter.emit_inst("BCC", &body_label); // counter.hi < len.hi -> body
    emitter.emit_inst("BNE", &exit_label); // counter.hi > len.hi -> exit
    emitter.emit_inst("LDA", &format!("${:02X}", counter));
    emitter.emit_inst("CMP", &format!("${:02X}", slot + 2));
    emitter.emit_inst("BCC", &body_label); // counter.lo < len.lo -> body
    emitter.emit_label(&exit_label);
    emitter.emit_inst("JMP", end_label);
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

/// Generate a normal (non-unrolled) for loop
pub(super) fn generate_normal_loop(
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
pub(super) fn loop_bound_slot(
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
        E::BitOp { object, bit, .. } => {
            expr_references_name(&object.node, name) || expr_references_name(&bit.node, name)
        }
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

pub(super) fn generate_for(
    var_name: &Spanned<String>,
    range: &crate::ast::Range,
    body: &Spanned<Stmt>,
    emitter: &mut Emitter,
    info: &ProgramInfo,
    string_collector: &mut StringCollector,
) -> Result<(), CodegenError> {
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
            let loop_var_is_16bit = info.resolved_symbols.get(&var_name.span).is_some_and(|s| {
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
                    emitter.emit_inst("STA", &format!("${:02X}", loop_var_addr.wrapping_add(1)));
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

pub(super) fn generate_foreach(
    var_name: &Spanned<String>,
    iterable: &Spanned<crate::ast::Expr>,
    body: &Spanned<Stmt>,
    index_var: &Option<Spanned<String>>,
    emitter: &mut Emitter,
    info: &ProgramInfo,
    string_collector: &mut StringCollector,
) -> Result<(), CodegenError> {
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
                crate::sema::table::SymbolLocation::Absolute(addr) if addr < 256 => addr as u8,
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
    // Exit when X >= length. end_label sits past the whole body, so a
    // plain `BCS end_label` overflows its ±127 range once the body is
    // large (an inlined call, say). Route the far jump through a JMP and
    // let the conditional branch hop only over it.
    let fe_body = emitter.next_label("feb");
    emit_far_arm_branch(emitter, "BCS", &end_label, &fe_body);

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
            crate::sema::types::Type::Array(elem, _) | crate::sema::types::Type::Slice(elem) => {
                Some(&**elem)
            }
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
            crate::sema::table::SymbolLocation::Absolute(addr) => emitter.emit_sta_abs(addr),
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
