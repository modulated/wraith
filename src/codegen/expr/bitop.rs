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

use super::{StaticBase, generate_expr, resolve_static_addr};

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

    // A mutation. Resolve the target byte's location and a name for comments.
    let loc = resolve_bit_target(object, byte_off, info)?;
    let name = render_target(object);

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

/// Resolve a `x.bit(n)` read to a zero-page byte plus the bit within it, when
/// the target lives in zero page.
///
/// This is the enabling query for the 65C02 `BBRn`/`BBSn` fusion: `if x.bit(n)`
/// can fold into a single bit-test-branch only when the byte is zero-page
/// addressable (the Rockwell ops have no absolute form). Returns `None` for an
/// absolute/MMIO byte, a ROM constant, an indirect target, or a non-constant
/// bit index — the caller then falls back to the mask-and-compare read.
pub(crate) fn bit_test_zp(
    object: &Spanned<Expr>,
    bit: &Spanned<Expr>,
    info: &ProgramInfo,
) -> Option<(u8, u8)> {
    let n = crate::sema::const_eval::eval_const_expr(bit)
        .ok()
        .and_then(|v| v.as_integer())? as u8;
    if n >= 16 {
        return None;
    }
    let byte_off: u16 = (n / 8) as u16;
    let bit_in_byte = n % 8;
    match resolve_bit_target(object, byte_off, info) {
        Ok(ByteLoc::Zp(addr)) => Some((addr, bit_in_byte)),
        _ => None,
    }
}

/// Resolve the byte holding the target bit for a *mutation*.
///
/// A plain variable comes straight from its symbol (and an `addr` register keeps
/// its symbolic name). A field or array element is resolved to a fixed address
/// through the shared static-lvalue walk — the same one `p.field` codegen uses —
/// so `FLAGS.enable.set_bit(3)` and `T[2].mode.clear_bit(1)` land on the right
/// byte and still lower to `SMB`/`RMB` when it sits in zero page.
///
/// A pointer/parameter target (indirect) and a runtime array index are not
/// reachable this way yet; both surface as a clear error rather than a wrong
/// address.
fn resolve_bit_target(
    object: &Spanned<Expr>,
    byte_off: u16,
    info: &ProgramInfo,
) -> Result<ByteLoc, CodegenError> {
    if let Expr::Variable(name) = &object.node {
        let sym = info
            .resolved_symbols
            .get(&object.span)
            .or_else(|| info.table.lookup(name))
            .ok_or_else(|| CodegenError::SymbolNotFound(name.clone()))?;
        return match sym.location {
            SymbolLocation::ZeroPage(addr) => Ok(ByteLoc::Zp(addr + byte_off as u8)),
            SymbolLocation::Absolute(addr) => {
                if sym.kind == SymbolKind::Address {
                    // A byte MMIO register: address it by name (byte_off is 0).
                    Ok(ByteLoc::Sym(name.clone()))
                } else {
                    Ok(ByteLoc::Abs(addr + byte_off))
                }
            }
            SymbolLocation::FrameOffset(_) => Err(CodegenError::Internal(
                "unresolved FrameOffset reached bit-op codegen".to_string(),
            )),
            SymbolLocation::None => Err(CodegenError::UnsupportedOperation(
                "bit mutation target has no storage".to_string(),
            )),
        };
    }

    match resolve_static_addr(object, info) {
        Some((StaticBase::Addr(base), _)) => {
            let byte = base.wrapping_add(byte_off);
            Ok(if byte < 0x100 {
                ByteLoc::Zp(byte as u8)
            } else {
                ByteLoc::Abs(byte)
            })
        }
        Some((StaticBase::Label(..), _)) => Err(CodegenError::UnsupportedOperation(
            "cannot modify a bit of constant (ROM) data".to_string(),
        )),
        None => Err(CodegenError::UnsupportedOperation(
            "a bit mutation target must be a variable, static, or a field/element at a fixed \
             address; a pointer target or a runtime index is not supported yet"
                .to_string(),
        )),
    }
}

/// A readable name for the target, for the emitted comment only.
fn render_target(object: &Spanned<Expr>) -> String {
    match &object.node {
        Expr::Variable(n) => n.clone(),
        Expr::Field { object, field } => format!("{}.{}", render_target(object), field.node),
        Expr::Index { object, .. } => format!("{}[..]", render_target(object)),
        Expr::Paren(inner) => render_target(inner),
        _ => "target".to_string(),
    }
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
