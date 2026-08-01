//! Bitfield operations: `x.bit(n)`, `x.set_bit(n)`, `x.clear_bit(n)`,
//! `x.toggle_bit(n)`.
//!
//! The bit index is a compile-time constant (sema enforces range), so every
//! mask is known here. On a 65C02 a zero-page set/clear becomes a single
//! `SMB`/`RMB`; otherwise (NMOS, an absolute target, or toggle) it is the
//! classic `LDA; ORA/AND/EOR #mask; STA` read-modify-write. A read masks the
//! byte and canonicalizes to a 0/1 bool.

use crate::ast::{BitOpKind, Expr, Spanned};
use crate::codegen::{CodegenError, Emitter, StringCollector};
use crate::sema::ProgramInfo;
use crate::sema::table::{SymbolKind, SymbolLocation};

use super::generate_expr;

/// Where the byte holding the target bit lives, and how to address it.
enum ByteLoc {
    Zp(u8),
    Abs(u16),
    /// An `addr` register — addressed by its symbolic name.
    Sym(String),
}

pub(super) fn generate_bitop(
    object: &Spanned<Expr>,
    kind: BitOpKind,
    bit: &Spanned<Expr>,
    emitter: &mut Emitter,
    info: &ProgramInfo,
    string_collector: &mut StringCollector,
) -> Result<(), CodegenError> {
    // Sema validated the index is a constant in range; recover it here.
    let n = crate::sema::const_eval::eval_const_expr(bit)
        .ok()
        .and_then(|v| v.as_integer())
        .ok_or_else(|| CodegenError::Internal("bit index did not fold to a constant".to_string()))?
        as u8;
    let byte_off: u16 = (n / 8) as u16; // 0 = low byte, 1 = high byte (u16)
    let bit_in_byte = n % 8;
    let mask: u8 = 1 << bit_in_byte;

    if kind == BitOpKind::Get {
        // Read the value; select the byte holding bit n, mask it, and
        // canonicalize the result to a 0/1 bool.
        generate_expr(object, emitter, info, string_collector)?;
        if byte_off == 1 {
            // The high byte of a u16 arrives in Y (A:Y convention).
            emitter.emit_inst("TYA", "");
        }
        emitter.emit_comment(&format!("Read bit {}", n));
        emitter.emit_inst("AND", &format!("#${:02X}", mask));
        let done = emitter.next_label("bit");
        emitter.emit_inst("BEQ", &done);
        emitter.emit_inst("LDA", "#$01");
        emitter.emit_label(&done);
        emitter.mark_a_unknown();
        return Ok(());
    }

    // A mutation. Resolve the target byte's location.
    let Expr::Variable(name) = &object.node else {
        return Err(CodegenError::UnsupportedOperation(
            "bit mutation target must be a plain variable".to_string(),
        ));
    };
    let sym = info
        .resolved_symbols
        .get(&object.span)
        .or_else(|| info.table.lookup(name))
        .ok_or_else(|| CodegenError::SymbolNotFound(name.clone()))?;

    let loc = match sym.location {
        SymbolLocation::ZeroPage(addr) => ByteLoc::Zp(addr + byte_off as u8),
        SymbolLocation::Absolute(addr) => {
            if sym.kind == SymbolKind::Address {
                // A byte MMIO register: address it by name (byte_off is 0).
                ByteLoc::Sym(name.clone())
            } else {
                ByteLoc::Abs(addr + byte_off)
            }
        }
        SymbolLocation::FrameOffset(_) => {
            return Err(CodegenError::Internal(
                "unresolved FrameOffset reached bit-op codegen".to_string(),
            ));
        }
        SymbolLocation::None => {
            return Err(CodegenError::UnsupportedOperation(
                "bit mutation target has no storage".to_string(),
            ));
        }
    };

    // 65C02 fast path: a zero-page set/clear is a single instruction that leaves
    // the registers untouched.
    if emitter.target.has_rockwell_bit_ops()
        && let ByteLoc::Zp(addr) = &loc
        && matches!(kind, BitOpKind::Set | BitOpKind::Clear)
    {
        let mnem = if kind == BitOpKind::Set {
            format!("SMB{}", bit_in_byte)
        } else {
            format!("RMB{}", bit_in_byte)
        };
        emitter.emit_comment(&format!(
            "{} bit {} of {}",
            if kind == BitOpKind::Set {
                "Set"
            } else {
                "Clear"
            },
            n,
            name
        ));
        emitter.emit_inst(&mnem, &format!("${:02X}", addr));
        // The byte changed under any register belief about it; drop beliefs.
        emitter.invalidate_registers();
        return Ok(());
    }

    // Read-modify-write fallback (NMOS, absolute/MMIO target, or toggle).
    emitter.emit_comment(&format!(
        "{} bit {} of {}",
        match kind {
            BitOpKind::Set => "Set",
            BitOpKind::Clear => "Clear",
            BitOpKind::Toggle => "Toggle",
            BitOpKind::Get => unreachable!(),
        },
        n,
        name
    ));
    load(&loc, emitter);
    match kind {
        BitOpKind::Set => emitter.emit_inst("ORA", &format!("#${:02X}", mask)),
        BitOpKind::Clear => emitter.emit_inst("AND", &format!("#${:02X}", !mask)),
        BitOpKind::Toggle => emitter.emit_inst("EOR", &format!("#${:02X}", mask)),
        BitOpKind::Get => unreachable!(),
    }
    store(&loc, emitter);
    emitter.invalidate_registers();
    Ok(())
}

fn load(loc: &ByteLoc, emitter: &mut Emitter) {
    match loc {
        ByteLoc::Zp(a) => emitter.emit_lda_zp(*a),
        ByteLoc::Abs(a) => emitter.emit_lda_abs(*a),
        ByteLoc::Sym(s) => emitter.emit_lda_symbol(s),
    }
}

fn store(loc: &ByteLoc, emitter: &mut Emitter) {
    match loc {
        ByteLoc::Zp(a) => emitter.emit_sta_zp(*a),
        ByteLoc::Abs(a) => emitter.emit_sta_abs(*a),
        ByteLoc::Sym(s) => emitter.emit_sta_symbol(s),
    }
}
