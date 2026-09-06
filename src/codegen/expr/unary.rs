//! Unary operation code generation
//!
//! This module handles all unary operators:
//! - Negation (`-x`): Two's complement negation
//! - Bitwise NOT (`~x`): Bitwise complement
//! - Logical NOT (`!x`): Boolean negation (converts to 0 or 1)

use super::aggregate::StaticBase;
use crate::ast::{Spanned, UnaryOp};
use crate::codegen::{CodegenError, Emitter, StringCollector};
use crate::sema::ProgramInfo;

// Import generate_expr from parent module for recursive calls
use super::generate_expr;

/// Generate code for unary operations
///
/// Handles all unary operators:
/// - `-x`: Negation (two's complement: `~x + 1`)
/// - `~x`: Bitwise NOT (XOR with $FF)
/// - `!x`: Logical NOT (converts to boolean 0/1 and inverts)
pub(super) fn generate_unary(
    op: UnaryOp,
    operand: &Spanned<crate::ast::Expr>,
    emitter: &mut Emitter,
    info: &ProgramInfo,
    string_collector: &mut StringCollector,
) -> Result<(), CodegenError> {
    // Address-of and dereference have to intercept before the operand is
    // evaluated: `&x` wants the operand's *location*, not its value.
    match op {
        UnaryOp::AddrOf => return generate_addr_of(operand, emitter, info, string_collector),
        UnaryOp::Deref => return generate_deref(operand, emitter, info, string_collector),
        _ => {}
    }

    // Evaluate operand first
    generate_expr(operand, emitter, info, string_collector)?;

    // Determine operand width so 16-bit values negate/complement both bytes.
    let is_u16 = matches!(
        info.resolved_types.get(&operand.span),
        Some(crate::sema::types::Type::Primitive(
            crate::ast::PrimitiveType::U16
                | crate::ast::PrimitiveType::I16
                | crate::ast::PrimitiveType::B16
                | crate::ast::PrimitiveType::Q8_8
        ))
    );

    // Apply unary operation to A (low) / Y (high for u16)
    match op {
        UnaryOp::AddrOf | UnaryOp::Deref => unreachable!("handled above"),
        UnaryOp::Neg => {
            if is_u16 {
                // 16-bit two's complement: ~value + 1 across both bytes, carrying
                // from low into high. Low in A, high in Y; $22 holds the low result.
                let tmp = emitter.memory_layout.loop_end_temp(); // $22
                emitter.emit_inst("EOR", "#$FF"); // ~low
                emitter.emit_inst("CLC", "");
                emitter.emit_inst("ADC", "#$01"); // ~low + 1 (carry out)
                emitter.emit_inst("STA", &format!("${:02X}", tmp));
                emitter.emit_inst("TYA", ""); // high
                emitter.emit_inst("EOR", "#$FF"); // ~high
                emitter.emit_inst("ADC", "#$00"); // + carry from low
                emitter.emit_inst("TAY", ""); // Y = high result
                emitter.emit_inst("LDA", &format!("${:02X}", tmp)); // A = low result
            } else {
                // 8-bit two's complement: ~A + 1
                emitter.emit_inst("EOR", "#$FF"); // Bitwise NOT
                emitter.emit_inst("CLC", "");
                emitter.emit_inst("ADC", "#$01"); // Add 1
            }
        }
        UnaryOp::BitNot => {
            if is_u16 {
                // Complement both bytes; stash the low result while doing the high.
                let tmp = emitter.memory_layout.loop_end_temp(); // $22
                emitter.emit_inst("EOR", "#$FF"); // ~low
                emitter.emit_inst("STA", &format!("${:02X}", tmp));
                emitter.emit_inst("TYA", ""); // high
                emitter.emit_inst("EOR", "#$FF"); // ~high
                emitter.emit_inst("TAY", ""); // Y = ~high
                emitter.emit_inst("LDA", &format!("${:02X}", tmp)); // A = ~low
            } else {
                // Bitwise NOT
                emitter.emit_inst("EOR", "#$FF");
            }
        }
        UnaryOp::Not => {
            // Logical NOT: convert to boolean (0 or 1) and invert
            let true_label = emitter.next_label("nt");
            let end_label = emitter.next_label("nx");

            emitter.emit_inst("CMP", "#$00");
            emitter.emit_inst("BEQ", &true_label); // If zero, result is true (1)

            // False case (input was non-zero)
            emitter.emit_inst("LDA", "#$00");
            emitter.emit_inst("JMP", &end_label);

            // True case (input was zero)
            emitter.emit_label(&true_label);
            emitter.emit_inst("LDA", "#$01");

            emitter.emit_label(&end_label);
        }
    }

    // Unary ops rewrite A (and Y for u16) via raw instructions the tracker does
    // not follow; drop cached register beliefs.
    emitter.mark_a_unknown();
    Ok(())
}

/// Emit `&operand` — the operand's address, in A (low) : X (high).
///
/// A:X rather than A:Y because that is what every other pointer-like value in
/// this compiler already uses: arrays, strings, enums and struct-init results.
fn generate_addr_of(
    operand: &Spanned<crate::ast::Expr>,
    emitter: &mut Emitter,
    info: &ProgramInfo,
    string_collector: &mut StringCollector,
) -> Result<(), CodegenError> {
    use crate::ast::Expr;
    use crate::sema::table::SymbolLocation;
    use crate::sema::types::Type;

    // Unwrap parentheses and `&*p`, both of which sema already accepted.
    let operand = match &operand.node {
        Expr::Paren(inner) => &**inner,
        _ => operand,
    };

    // A chain — `&arr[i]`, `&s.field`, `&x.f[0]`, `&m[i][j]` — is the base of
    // whatever it names plus the offsets along the way. One routine computes
    // that, and folds it to two immediate loads wherever the whole chain is
    // constant.
    if matches!(&operand.node, Expr::Index { .. } | Expr::Field { .. }) {
        return match crate::codegen::expr::emit_aggregate_base(
            operand,
            emitter,
            info,
            string_collector,
        )? {
            Some(_) => Ok(()),
            None => Err(CodegenError::UnsupportedOperation(
                "cannot take the address of this expression: it names no storage".to_string(),
            )),
        };
    }

    let sym = info.resolved_symbols.get(&operand.span).ok_or_else(|| {
        CodegenError::Internal("address-of operand has no resolved symbol".to_string())
    })?;

    // An array variable's slot already *holds* the pointer to its data, so
    // taking its address means loading the slot, not the slot's address.
    let is_indirect_slot = matches!(sym.ty, Type::Array(..));

    match sym.location {
        SymbolLocation::ZeroPage(addr) if is_indirect_slot => {
            emitter.emit_comment("Address of array data (the slot holds the pointer)");
            emitter.emit_inst("LDA", &format!("${:02X}", addr));
            emitter.emit_inst("LDX", &format!("${:02X}", addr + 1));
        }
        SymbolLocation::ZeroPage(addr) => {
            // Every local lives in zero page, so the high byte is always $00.
            emitter.emit_comment(&format!("Address of local '{}'", sym.name));
            emitter.emit_inst("LDA", &format!("#${:02X}", addr));
            emitter.emit_inst("LDX", "#$00");
        }
        SymbolLocation::Absolute(_) | SymbolLocation::None => {
            emitter.emit_comment(&format!("Address of '{}'", sym.name));
            static_base_of(sym).emit_as_pointer(emitter);
        }
        _ => {
            return Err(CodegenError::UnsupportedOperation(format!(
                "cannot take the address of '{}'",
                sym.name
            )));
        }
    }
    emitter.reg_state.modify_a();
    Ok(())
}

/// Emit `*operand` — read through a pointer.
fn generate_deref(
    operand: &Spanned<crate::ast::Expr>,
    emitter: &mut Emitter,
    info: &ProgramInfo,
    string_collector: &mut StringCollector,
) -> Result<(), CodegenError> {
    use crate::ast::Expr;
    use crate::sema::table::SymbolLocation;
    use crate::sema::types::Type;

    // What comes back through the pointer, and in which register pair. Asked
    // of the shared predicates rather than by re-listing the two-byte types
    // here: that list left out `&T` and a function pointer, so `*pp` on a
    // `&&u8` loaded *one* byte and the binding then stored whatever X held as
    // the address's high half — `q` pointed at $0000 instead of $0400, and
    // both the read and the write through it landed in zero page. The store
    // side had already been fixed the same way; this is the other half of it.
    let pointee = match info.resolved_types.get(&operand.span) {
        Some(Type::Pointer(inner)) => Some(inner.as_ref()),
        _ => None,
    };
    let pointee_is_multibyte = pointee.is_some_and(crate::codegen::expr::is_two_byte_value);
    let high_in_x = pointee.is_some_and(crate::codegen::expr::high_byte_in_x);

    // Fast path: the pointer is a zero-page variable, so `(zp),Y` can read
    // through it directly.
    if let Expr::Variable(_) = &operand.node
        && let Some(sym) = info.resolved_symbols.get(&operand.span)
        && let SymbolLocation::ZeroPage(addr) = sym.location
    {
        emitter.emit_comment("Dereference pointer");
        crate::codegen::expr::aggregate::emit_deref_load(
            emitter,
            addr,
            0,
            pointee_is_multibyte,
            high_in_x,
        );
        return Ok(());
    }

    // General path: evaluate the pointer into A:X and stage it in zero page,
    // because `(zp),Y` needs the pointer itself to be there.
    generate_expr(operand, emitter, info, string_collector)?;
    let ptr = emitter.memory_layout.deref_ptr();
    emitter.emit_comment("Dereference pointer (staged)");
    emitter.emit_inst("STA", &format!("${:02X}", ptr));
    emitter.emit_inst("STX", &format!("${:02X}", ptr + 1));
    crate::codegen::expr::aggregate::emit_deref_load(
        emitter,
        ptr,
        0,
        pointee_is_multibyte,
        high_in_x,
    );
    Ok(())
}

/// Where a non-local symbol's storage is named from.
///
/// A mutable `static` has a real BSS address. An immutable `const` is ROM data
/// at an assembler label, and sema leaves it at `Absolute(0)` because the
/// address is the linker's to choose — so reading that placeholder as a number
/// is what made `&A[1]` come out as `$0001`.
fn static_base_of(sym: &crate::sema::table::SymbolInfo) -> StaticBase {
    use crate::sema::table::{SymbolKind, SymbolLocation};
    match (&sym.location, &sym.kind) {
        (SymbolLocation::Absolute(a), k) if *k != SymbolKind::Constant => StaticBase::Addr(*a),
        _ => StaticBase::Label(sym.name.clone(), 0),
    }
}
