//! Flat 6502 assembler for the compiler's absolute (`.ORG`-placed) output.
//!
//! Wraith emits fully-placed absolute assembly: `.ORG` seeks to an absolute
//! address, code and data are laid down at their final addresses, and the
//! interrupt vector table is written directly at `$FFFA`-`$FFFF`. That is a
//! flat-image model, not the relocatable-segment model a ca65/ld65 linker
//! config assumes, so this in-tree assembler is what turns compiler output into
//! a runnable image.
//!
//! Dialect: `.ORG`/`.BYTE`/`.WORD`/`.RES`, `NAME = VALUE` symbol constants,
//! `label:` definitions (optionally followed by an instruction on the same
//! line, as inline `asm { }` emits), `;` comments, the standard 6502 addressing
//! modes, `#<label`/`#>label` low/high bytes, and `label+N` offsets.
//!
//! The full 64 KB image is addressed so that byte `i` is memory address `i`;
//! callers wanting a ROM slice can take, e.g., `image[0x8000..=0xFFFF]`.

use std::collections::HashMap;

/// Size of the assembled image: the whole 16-bit address space.
pub const IMAGE_SIZE: usize = 65536;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Imp,
    Acc,
    Imm,
    Zp,
    ZpX,
    ZpY,
    Abs,
    AbsX,
    AbsY,
    IndX,
    IndY,
    Ind,
    /// Absolute indexed indirect `(abs,X)` — 65C02, `JMP` only.
    AbsIndX,
    Rel,
    /// Zero-page + relative `$zp,label` — 65C02 `BBRn`/`BBSn` (3 bytes).
    ZpRel,
}

/// Opcode for a (mnemonic, addressing-mode) pair.
///
/// Covers the NMOS 6502 set plus the WDC 65C02 additions the compiler may emit
/// when targeting `--cpu 65c02`: `STZ`, `BRA`, `PHX`/`PLX`/`PHY`/`PLY`,
/// accumulator `INC A`/`DEC A`, `TSB`/`TRB`, and the Rockwell `SMB`/`RMB` bit
/// ops. The assembler does not gate these on a target — codegen decides whether
/// to emit them; the assembler just needs to encode whatever it is handed.
fn opcode(mnem: &str, mode: Mode) -> Option<u8> {
    use Mode::*;
    // 65C02 Rockwell RMB0-7 / SMB0-7: zero-page only, bit number in the
    // mnemonic. RMBn = 0x07 | n<<4, SMBn = 0x87 | n<<4.
    if let Some(bit) = mnem.strip_prefix("RMB").and_then(|d| d.parse::<u8>().ok()) {
        return if bit <= 7 && mode == Zp {
            Some(0x07 | (bit << 4))
        } else {
            None
        };
    }
    if let Some(bit) = mnem.strip_prefix("SMB").and_then(|d| d.parse::<u8>().ok()) {
        return if bit <= 7 && mode == Zp {
            Some(0x87 | (bit << 4))
        } else {
            None
        };
    }
    // 65C02 Rockwell BBR0-7 / BBS0-7: zero-page byte plus a relative branch,
    // bit number in the mnemonic. BBRn = 0x0F | n<<4, BBSn = 0x8F | n<<4.
    if let Some(bit) = mnem.strip_prefix("BBR").and_then(|d| d.parse::<u8>().ok()) {
        return if bit <= 7 && mode == ZpRel {
            Some(0x0F | (bit << 4))
        } else {
            None
        };
    }
    if let Some(bit) = mnem.strip_prefix("BBS").and_then(|d| d.parse::<u8>().ok()) {
        return if bit <= 7 && mode == ZpRel {
            Some(0x8F | (bit << 4))
        } else {
            None
        };
    }
    let m = |want: Mode, op: u8| if mode == want { Some(op) } else { None };
    match mnem {
        "ADC" => m(Imm, 0x69)
            .or(m(Zp, 0x65))
            .or(m(ZpX, 0x75))
            .or(m(Abs, 0x6D))
            .or(m(AbsX, 0x7D))
            .or(m(AbsY, 0x79))
            .or(m(IndX, 0x61))
            .or(m(IndY, 0x71)),
        "AND" => m(Imm, 0x29)
            .or(m(Zp, 0x25))
            .or(m(ZpX, 0x35))
            .or(m(Abs, 0x2D))
            .or(m(AbsX, 0x3D))
            .or(m(AbsY, 0x39))
            .or(m(IndX, 0x21))
            .or(m(IndY, 0x31)),
        "ASL" => m(Acc, 0x0A)
            .or(m(Zp, 0x06))
            .or(m(ZpX, 0x16))
            .or(m(Abs, 0x0E))
            .or(m(AbsX, 0x1E)),
        "BCC" => m(Rel, 0x90),
        "BCS" => m(Rel, 0xB0),
        "BEQ" => m(Rel, 0xF0),
        "BIT" => m(Zp, 0x24).or(m(Abs, 0x2C)),
        "BMI" => m(Rel, 0x30),
        "BNE" => m(Rel, 0xD0),
        "BPL" => m(Rel, 0x10),
        "BRA" => m(Rel, 0x80), // 65C02
        "BRK" => m(Imp, 0x00),
        "BVC" => m(Rel, 0x50),
        "BVS" => m(Rel, 0x70),
        "CLC" => m(Imp, 0x18),
        "CLD" => m(Imp, 0xD8),
        "CLI" => m(Imp, 0x58),
        "CLV" => m(Imp, 0xB8),
        "CMP" => m(Imm, 0xC9)
            .or(m(Zp, 0xC5))
            .or(m(ZpX, 0xD5))
            .or(m(Abs, 0xCD))
            .or(m(AbsX, 0xDD))
            .or(m(AbsY, 0xD9))
            .or(m(IndX, 0xC1))
            .or(m(IndY, 0xD1)),
        "CPX" => m(Imm, 0xE0).or(m(Zp, 0xE4)).or(m(Abs, 0xEC)),
        "CPY" => m(Imm, 0xC0).or(m(Zp, 0xC4)).or(m(Abs, 0xCC)),
        "DEC" => m(Acc, 0x3A) // 65C02 accumulator form
            .or(m(Zp, 0xC6))
            .or(m(ZpX, 0xD6))
            .or(m(Abs, 0xCE))
            .or(m(AbsX, 0xDE)),
        "DEX" => m(Imp, 0xCA),
        "DEY" => m(Imp, 0x88),
        "EOR" => m(Imm, 0x49)
            .or(m(Zp, 0x45))
            .or(m(ZpX, 0x55))
            .or(m(Abs, 0x4D))
            .or(m(AbsX, 0x5D))
            .or(m(AbsY, 0x59))
            .or(m(IndX, 0x41))
            .or(m(IndY, 0x51)),
        "INC" => m(Acc, 0x1A) // 65C02 accumulator form
            .or(m(Zp, 0xE6))
            .or(m(ZpX, 0xF6))
            .or(m(Abs, 0xEE))
            .or(m(AbsX, 0xFE)),
        "INX" => m(Imp, 0xE8),
        "INY" => m(Imp, 0xC8),
        "JMP" => m(Abs, 0x4C).or(m(Ind, 0x6C)).or(m(AbsIndX, 0x7C)), // (abs,X) is 65C02
        "JSR" => m(Abs, 0x20),
        "LDA" => m(Imm, 0xA9)
            .or(m(Zp, 0xA5))
            .or(m(ZpX, 0xB5))
            .or(m(Abs, 0xAD))
            .or(m(AbsX, 0xBD))
            .or(m(AbsY, 0xB9))
            .or(m(IndX, 0xA1))
            .or(m(IndY, 0xB1)),
        "LDX" => m(Imm, 0xA2)
            .or(m(Zp, 0xA6))
            .or(m(ZpY, 0xB6))
            .or(m(Abs, 0xAE))
            .or(m(AbsY, 0xBE)),
        "LDY" => m(Imm, 0xA0)
            .or(m(Zp, 0xA4))
            .or(m(ZpX, 0xB4))
            .or(m(Abs, 0xAC))
            .or(m(AbsX, 0xBC)),
        "LSR" => m(Acc, 0x4A)
            .or(m(Zp, 0x46))
            .or(m(ZpX, 0x56))
            .or(m(Abs, 0x4E))
            .or(m(AbsX, 0x5E)),
        "NOP" => m(Imp, 0xEA),
        "ORA" => m(Imm, 0x09)
            .or(m(Zp, 0x05))
            .or(m(ZpX, 0x15))
            .or(m(Abs, 0x0D))
            .or(m(AbsX, 0x1D))
            .or(m(AbsY, 0x19))
            .or(m(IndX, 0x01))
            .or(m(IndY, 0x11)),
        "PHA" => m(Imp, 0x48),
        "PHP" => m(Imp, 0x08),
        "PHX" => m(Imp, 0xDA), // 65C02
        "PHY" => m(Imp, 0x5A), // 65C02
        "PLA" => m(Imp, 0x68),
        "PLP" => m(Imp, 0x28),
        "PLX" => m(Imp, 0xFA), // 65C02
        "PLY" => m(Imp, 0x7A), // 65C02
        "ROL" => m(Acc, 0x2A)
            .or(m(Zp, 0x26))
            .or(m(ZpX, 0x36))
            .or(m(Abs, 0x2E))
            .or(m(AbsX, 0x3E)),
        "ROR" => m(Acc, 0x6A)
            .or(m(Zp, 0x66))
            .or(m(ZpX, 0x76))
            .or(m(Abs, 0x6E))
            .or(m(AbsX, 0x7E)),
        "RTI" => m(Imp, 0x40),
        "RTS" => m(Imp, 0x60),
        "SBC" => m(Imm, 0xE9)
            .or(m(Zp, 0xE5))
            .or(m(ZpX, 0xF5))
            .or(m(Abs, 0xED))
            .or(m(AbsX, 0xFD))
            .or(m(AbsY, 0xF9))
            .or(m(IndX, 0xE1))
            .or(m(IndY, 0xF1)),
        "SEC" => m(Imp, 0x38),
        "SED" => m(Imp, 0xF8),
        "SEI" => m(Imp, 0x78),
        "STA" => m(Zp, 0x85)
            .or(m(ZpX, 0x95))
            .or(m(Abs, 0x8D))
            .or(m(AbsX, 0x9D))
            .or(m(AbsY, 0x99))
            .or(m(IndX, 0x81))
            .or(m(IndY, 0x91)),
        "STX" => m(Zp, 0x86).or(m(ZpY, 0x96)).or(m(Abs, 0x8E)),
        "STY" => m(Zp, 0x84).or(m(ZpX, 0x94)).or(m(Abs, 0x8C)),
        "STZ" => m(Zp, 0x64)
            .or(m(ZpX, 0x74))
            .or(m(Abs, 0x9C))
            .or(m(AbsX, 0x9E)), // 65C02
        "TAX" => m(Imp, 0xAA),
        "TAY" => m(Imp, 0xA8),
        "TRB" => m(Zp, 0x14).or(m(Abs, 0x1C)), // 65C02
        "TSB" => m(Zp, 0x04).or(m(Abs, 0x0C)), // 65C02
        "TSX" => m(Imp, 0xBA),
        "TXA" => m(Imp, 0x8A),
        "TXS" => m(Imp, 0x9A),
        "TYA" => m(Imp, 0x98),
        _ => None,
    }
}

fn instr_len(mode: Mode) -> u16 {
    match mode {
        Mode::Imp | Mode::Acc => 1,
        Mode::Imm | Mode::Zp | Mode::ZpX | Mode::ZpY | Mode::IndX | Mode::IndY | Mode::Rel => 2,
        Mode::Abs | Mode::AbsX | Mode::AbsY | Mode::Ind | Mode::AbsIndX | Mode::ZpRel => 3,
    }
}

/// A parsed operand: the addressing mode plus how to compute its value.
struct ParsedOperand {
    mode: Mode,
    value: ValueExpr,
}

#[derive(Clone)]
enum ValueExpr {
    None,
    Num(u16),
    /// label name + constant offset
    Label(String, i32),
    /// low byte of (label + offset)
    LabelLow(String, i32),
    /// high byte of (label + offset)
    LabelHigh(String, i32),
    /// zero-page byte plus a branch target label — 65C02 `BBRn`/`BBSn`.
    ZpRel {
        zp: u8,
        label: String,
        off: i32,
    },
}

/// Parse a numeric term like `$4E`, `$0200`, `42`, or `$4E+1` -> value.
fn parse_num(s: &str) -> Option<u16> {
    let (base, off) = match s.split_once('+') {
        Some((b, o)) => (b, o.trim().parse::<i32>().ok()?),
        None => (s, 0),
    };
    let base = base.trim();
    let v = if let Some(hex) = base.strip_prefix('$') {
        u16::from_str_radix(hex, 16).ok()?
    } else {
        base.parse::<u16>().ok()?
    };
    Some((v as i32 + off) as u16)
}

/// Split `base` and integer offset from `label` or `label+1`.
fn split_label(s: &str) -> (String, i32) {
    match s.split_once('+') {
        Some((b, o)) => (b.trim().to_string(), o.trim().parse::<i32>().unwrap_or(0)),
        None => (s.trim().to_string(), 0),
    }
}

fn is_branch(mnem: &str) -> bool {
    matches!(
        mnem,
        "BCC" | "BCS" | "BEQ" | "BNE" | "BMI" | "BPL" | "BVC" | "BVS" | "BRA"
    )
}

/// A 65C02 Rockwell bit-test-branch: `BBRn`/`BBSn`, taking `$zp,label`.
fn is_bit_branch(mnem: &str) -> bool {
    (mnem
        .strip_prefix("BBR")
        .or_else(|| mnem.strip_prefix("BBS")))
    .and_then(|d| d.parse::<u8>().ok())
    .is_some_and(|bit| bit <= 7)
}

/// Parse a mnemonic's operand string into an addressing mode + value.
fn parse_operand(mnem: &str, operand: Option<&str>) -> Result<ParsedOperand, String> {
    let po = |mode, value| Ok(ParsedOperand { mode, value });
    // A bare shift/rotate (no operand) is accumulator mode; everything else with
    // no operand is implied.
    let bare_mode = if matches!(mnem, "ASL" | "LSR" | "ROL" | "ROR") {
        Mode::Acc
    } else {
        Mode::Imp
    };
    let Some(op) = operand else {
        return po(bare_mode, ValueExpr::None);
    };
    let op = op.trim();
    if op.is_empty() {
        return po(bare_mode, ValueExpr::None);
    }
    // A bare `A` operand is the accumulator, but only for the mnemonics that
    // actually have an accumulator addressing mode. For every other mnemonic
    // (`LDA A`, `STA A`, `ADC A`, …) an operand of `A` is a label — a user
    // symbol literally named `A` — so it must fall through to the label path
    // below. This is unambiguous because codegen only ever writes `A` as an
    // operand for genuine accumulator ops; memory operands go through
    // load/modify/store or a resolved numeric address, never `INC A`-the-label.
    if op == "A" && matches!(mnem, "ASL" | "LSR" | "ROL" | "ROR" | "INC" | "DEC") {
        return po(Mode::Acc, ValueExpr::None);
    }

    // 65C02 bit-test-branch `BBRn $zp,label` / `BBSn $zp,label`.
    if is_bit_branch(mnem) {
        let (zp_str, label) = op
            .split_once(',')
            .ok_or_else(|| format!("{mnem} expects `$zp,label`, got: {op}"))?;
        let zp = parse_num(zp_str.trim())
            .filter(|v| *v <= 0xFF)
            .ok_or_else(|| format!("bad zero-page operand for {mnem}: {op}"))?
            as u8;
        let (l, off) = split_label(label);
        return po(Mode::ZpRel, ValueExpr::ZpRel { zp, label: l, off });
    }

    // Immediate
    if let Some(rest) = op.strip_prefix('#') {
        if let Some(r) = rest.strip_prefix('<') {
            return match parse_num(r) {
                Some(v) => po(Mode::Imm, ValueExpr::Num(v & 0xFF)),
                None => {
                    let (l, off) = split_label(r);
                    po(Mode::Imm, ValueExpr::LabelLow(l, off))
                }
            };
        }
        if let Some(r) = rest.strip_prefix('>') {
            return match parse_num(r) {
                Some(v) => po(Mode::Imm, ValueExpr::Num((v >> 8) & 0xFF)),
                None => {
                    let (l, off) = split_label(r);
                    po(Mode::Imm, ValueExpr::LabelHigh(l, off))
                }
            };
        }
        let v = parse_num(rest).ok_or_else(|| format!("bad immediate operand: {op}"))?;
        return po(Mode::Imm, ValueExpr::Num(v));
    }

    // Indirect forms
    if let Some(inner) = op.strip_prefix('(') {
        if let Some(zp) = inner.strip_suffix("),Y") {
            let v = parse_num(zp).ok_or_else(|| format!("bad (zp),Y operand: {op}"))?;
            return po(Mode::IndY, ValueExpr::Num(v));
        }
        if let Some(base) = inner.strip_suffix(",X)") {
            // `JMP (abs,X)` is absolute indexed indirect (65C02); it is the only
            // instruction that takes a 16-bit or label operand here. Everything
            // else is the zero-page `(zp,X)` form.
            if mnem == "JMP" {
                return match parse_num(base) {
                    Some(v) => po(Mode::AbsIndX, ValueExpr::Num(v)),
                    None => {
                        let (l, off) = split_label(base);
                        po(Mode::AbsIndX, ValueExpr::Label(l, off))
                    }
                };
            }
            let v = parse_num(base).ok_or_else(|| format!("bad (zp,X) operand: {op}"))?;
            return po(Mode::IndX, ValueExpr::Num(v));
        }
        if let Some(addr) = inner.strip_suffix(')') {
            let v = parse_num(addr).ok_or_else(|| format!("bad (abs) operand: {op}"))?;
            return po(Mode::Ind, ValueExpr::Num(v));
        }
        return Err(format!("unrecognized indirect operand: {op}"));
    }

    // Indexed: ",X" / ",Y"
    if let Some(base) = op.strip_suffix(",X") {
        return Ok(index_operand(base, true, mnem));
    }
    if let Some(base) = op.strip_suffix(",Y") {
        return Ok(index_operand(base, false, mnem));
    }

    // Branches take a relative label target.
    if is_branch(mnem) {
        let (l, off) = split_label(op);
        return po(Mode::Rel, ValueExpr::Label(l, off));
    }

    // JMP/JSR to a bare label -> absolute.
    if (mnem == "JMP" || mnem == "JSR") && parse_num(op).is_none() {
        let (l, off) = split_label(op);
        return po(Mode::Abs, ValueExpr::Label(l, off));
    }

    // Numeric operand: zero page if it fits in a byte, else absolute.
    if let Some(v) = parse_num(op) {
        if v <= 0xFF {
            return po(Mode::Zp, ValueExpr::Num(v));
        }
        return po(Mode::Abs, ValueExpr::Num(v));
    }

    // Bare label in a non-jump context -> absolute (e.g. LDA table).
    let (l, off) = split_label(op);
    po(Mode::Abs, ValueExpr::Label(l, off))
}

fn index_operand(base: &str, x: bool, mnem: &str) -> ParsedOperand {
    let base = base.trim();
    if let Some(v) = parse_num(base) {
        // A `$XXXX` operand (4+ hex digits) forces absolute indexing even for
        // low addresses.
        let force_abs = base.strip_prefix('$').is_some_and(|h| h.len() >= 4);
        // Only LDX/STX have a zero-page,Y form; every other mnemonic must use
        // absolute,Y for a low address.
        let zp_y_ok = matches!(mnem, "LDX" | "STX");
        let mode = if v <= 0xFF && !force_abs {
            if x {
                Mode::ZpX
            } else if zp_y_ok {
                Mode::ZpY
            } else {
                Mode::AbsY
            }
        } else if x {
            Mode::AbsX
        } else {
            Mode::AbsY
        };
        return ParsedOperand {
            mode,
            value: ValueExpr::Num(v),
        };
    }
    let (l, off) = split_label(base);
    ParsedOperand {
        mode: if x { Mode::AbsX } else { Mode::AbsY },
        value: ValueExpr::Label(l, off),
    }
}

/// Strip a trailing `;` comment and surrounding whitespace.
fn clean_line(line: &str) -> &str {
    line.split(';').next().unwrap_or("").trim()
}

/// Split an optional leading `label:` off an instruction line, returning the
/// label (if any) and the remaining instruction text. Handles both a bare
/// `foo:` label and an inline-asm `foo: JMP foo`. A `NAME = VALUE` symbol
/// definition (no colon) is left untouched.
fn split_leading_label(line: &str) -> (Option<&str>, &str) {
    if !line.contains('=')
        && let Some(idx) = line.find(':')
    {
        let label = line[..idx].trim();
        let rest = line[idx + 1..].trim();
        if !label.is_empty() && !label.contains(char::is_whitespace) {
            return (Some(label), rest);
        }
    }
    (None, line)
}

/// Resolve a value expression against the symbol table (pass 2).
fn resolve(labels: &HashMap<String, u16>, v: &ValueExpr) -> Result<u16, String> {
    let base = |l: &str| {
        labels
            .get(l)
            .copied()
            .ok_or_else(|| format!("undefined label: {l}"))
    };
    Ok(match v {
        ValueExpr::None => 0,
        ValueExpr::Num(n) => *n,
        ValueExpr::Label(l, off) => (base(l)? as i32 + off) as u16,
        ValueExpr::LabelLow(l, off) => ((base(l)? as i32 + off) as u16) & 0xFF,
        ValueExpr::LabelHigh(l, off) => (((base(l)? as i32 + off) as u16) >> 8) & 0xFF,
        // A bit-test-branch is emitted directly (it needs the instruction
        // address for its relative offset), never resolved to a single value.
        ValueExpr::ZpRel { .. } => {
            return Err("internal: BBR/BBS operand resolved as a scalar".to_string());
        }
    })
}

/// Assemble the compiler's asm text into a flat 64 KB image.
///
/// Returns an error string on malformed input (undefined label, out-of-range
/// branch, unknown mnemonic/addressing mode, missing directive argument).
pub fn assemble(asm: &str) -> Result<[u8; IMAGE_SIZE], String> {
    // ---- Pass 1: collect label addresses ----
    let mut labels: HashMap<String, u16> = HashMap::new();
    // The counter is u32 so that a program ending exactly at $10000 (the
    // vector table's last byte is $FFFF) is legal; only crossing past it —
    // or defining a label there — is an error.
    let mut addr: u32 = 0;
    for raw in asm.lines() {
        let line = clean_line(raw);
        if line.is_empty() {
            continue;
        }
        // Symbol definition: `NAME = VALUE` (used for addr constants).
        if let Some((name, val)) = line.split_once('=') {
            let value =
                parse_num(val.trim()).ok_or_else(|| format!("bad symbol value: {}", val.trim()))?;
            let name = name.trim().to_string();
            if labels.insert(name.clone(), value).is_some() {
                return Err(format!("duplicate symbol '{name}'"));
            }
            continue;
        }
        let (label, line) = split_leading_label(line);
        if let Some(l) = label {
            // A second definition used to replace the first silently, so
            // every reference landed on whichever came last.
            if addr > 0xFFFF {
                return Err(format!("label '{l}' is defined past $FFFF"));
            }
            if labels.insert(l.to_string(), addr as u16).is_some() {
                return Err(format!("duplicate label '{l}'"));
            }
        }
        if line.is_empty() {
            continue;
        }
        let mut parts = line.splitn(2, char::is_whitespace);
        let head = parts.next().unwrap();
        let rest = parts.next().map(str::trim);
        // Labels past a wrap would record addresses near $0000 and emit over
        // the program's own start.
        let bump = |addr: &mut u32, n: u16| -> Result<(), String> {
            *addr += n as u32;
            if *addr > 0x10000 {
                return Err("assembly exceeds the $0000-$FFFF address space".to_string());
            }
            Ok(())
        };
        match head {
            ".ORG" => {
                let a = rest.ok_or(".ORG needs an address")?;
                addr = parse_num(a).ok_or_else(|| format!("bad .ORG address: {a}"))? as u32;
            }
            ".BYTE" => {
                let r = rest.ok_or(".BYTE needs values")?;
                bump(&mut addr, r.split(',').count() as u16)?;
            }
            ".WORD" => bump(&mut addr, 2)?,
            ".RES" => {
                let r = rest.ok_or(".RES needs a count")?;
                let n: u16 = r
                    .trim()
                    .parse()
                    .map_err(|_| format!("bad .RES count: {r}"))?;
                bump(&mut addr, n)?;
            }
            mnem => {
                let parsed = parse_operand(mnem, rest)?;
                bump(&mut addr, instr_len(parsed.mode))?;
            }
        }
    }

    // ---- Pass 2: emit bytes ----
    let mut mem = [0u8; IMAGE_SIZE];
    let mut addr: u32 = 0;
    fn emit(mem: &mut [u8; IMAGE_SIZE], a: &mut u32, byte: u8) -> Result<(), String> {
        // Emitting the byte *at* $FFFF is legal (the vector table lives
        // there); emitting one after it is the wrap that used to silently
        // overwrite the program's start.
        if *a >= IMAGE_SIZE as u32 {
            return Err("assembly exceeds the $0000-$FFFF address space".to_string());
        }
        mem[*a as usize] = byte;
        *a += 1;
        Ok(())
    }

    for raw in asm.lines() {
        let line = clean_line(raw);
        if line.is_empty() || line.contains('=') {
            continue;
        }
        let (_label, line) = split_leading_label(line);
        if line.is_empty() {
            continue;
        }
        let mut parts = line.splitn(2, char::is_whitespace);
        let head = parts.next().unwrap();
        let rest = parts.next().map(str::trim);
        match head {
            ".ORG" => {
                let a = rest.ok_or(".ORG needs an address")?;
                addr = parse_num(a).ok_or_else(|| format!("bad .ORG address: {a}"))? as u32;
            }
            ".BYTE" => {
                for tok in rest.ok_or(".BYTE needs values")?.split(',') {
                    let v = parse_num(tok.trim())
                        .ok_or_else(|| format!("bad .BYTE value: {}", tok.trim()))?;
                    // Same rule as byte-mode instruction operands: no silent
                    // truncation of a value that doesn't fit.
                    if v > 0xFF {
                        return Err(format!(".BYTE value {v:#06X} does not fit in a byte"));
                    }
                    emit(&mut mem, &mut addr, v as u8)?;
                }
            }
            ".WORD" => {
                let tok = rest.ok_or(".WORD needs a value")?.trim();
                let v = match parse_num(tok) {
                    Some(n) => n,
                    None => {
                        let (l, off) = split_label(tok);
                        resolve(&labels, &ValueExpr::Label(l, off))?
                    }
                };
                emit(&mut mem, &mut addr, (v & 0xFF) as u8)?;
                emit(&mut mem, &mut addr, (v >> 8) as u8)?;
            }
            ".RES" => {
                let r = rest.ok_or(".RES needs a count")?;
                let n: u16 = r
                    .trim()
                    .parse()
                    .map_err(|_| format!("bad .RES count: {r}"))?;
                for _ in 0..n {
                    emit(&mut mem, &mut addr, 0)?;
                }
            }
            mnem => {
                let parsed = parse_operand(mnem, rest)?;
                let op = opcode(mnem, parsed.mode)
                    .ok_or_else(|| format!("no opcode for {mnem} in this addressing mode"))?;
                let instr_addr = addr;
                emit(&mut mem, &mut addr, op)?;
                match parsed.mode {
                    Mode::Imp | Mode::Acc => {}
                    Mode::Rel => {
                        let target = resolve(&labels, &parsed.value)? as i32;
                        // relative to the address AFTER the 2-byte branch
                        let next = instr_addr as i32 + 2;
                        let delta = target - next;
                        if !(-128..=127).contains(&delta) {
                            return Err(format!(
                                "branch out of range: {instr_addr:#06X} -> {target:#06X} (delta {delta})"
                            ));
                        }
                        emit(&mut mem, &mut addr, (delta as i8) as u8)?;
                    }
                    Mode::Imm | Mode::Zp | Mode::ZpX | Mode::ZpY | Mode::IndX | Mode::IndY => {
                        let v = resolve(&labels, &parsed.value)?;
                        // A byte-mode operand that doesn't fit is almost
                        // always a bug upstream (a 300-element loop bound, a
                        // hand-written `#$1234`): truncating it assembles the
                        // wrong program without a word.
                        if v > 0xFF {
                            return Err(format!(
                                "{mnem} operand {v:#06X} does not fit in a byte (at {instr_addr:#06X})"
                            ));
                        }
                        emit(&mut mem, &mut addr, v as u8)?;
                    }
                    Mode::Abs | Mode::AbsX | Mode::AbsY | Mode::Ind | Mode::AbsIndX => {
                        let v = resolve(&labels, &parsed.value)?;
                        emit(&mut mem, &mut addr, (v & 0xFF) as u8)?;
                        emit(&mut mem, &mut addr, (v >> 8) as u8)?;
                    }
                    Mode::ZpRel => {
                        // `BBRn $zp,label` / `BBSn $zp,label`: the zero-page byte
                        // then a branch offset relative to the address after the
                        // 3-byte instruction.
                        let ValueExpr::ZpRel { zp, label, off } = &parsed.value else {
                            return Err(format!("{mnem} has a malformed operand"));
                        };
                        emit(&mut mem, &mut addr, *zp)?;
                        let target =
                            resolve(&labels, &ValueExpr::Label(label.clone(), *off))? as i32;
                        let next = instr_addr as i32 + 3;
                        let delta = target - next;
                        if !(-128..=127).contains(&delta) {
                            return Err(format!(
                                "bit-branch out of range: {instr_addr:#06X} -> {target:#06X} (delta {delta})"
                            ));
                        }
                        emit(&mut mem, &mut addr, (delta as i8) as u8)?;
                    }
                }
            }
        }
    }

    Ok(mem)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_duplicate_label_is_an_error_not_a_silent_redefinition() {
        // Every reference used to land on whichever definition came last.
        let err = assemble(".ORG $8000\nfoo:\nfoo:\n    RTS\n").unwrap_err();
        assert!(err.contains("duplicate label 'foo'"), "{err}");
    }

    #[test]
    fn rockwell_bit_set_reset_opcodes() {
        // RMBn = 0x07 | n<<4, SMBn = 0x87 | n<<4, all zero-page (2 bytes each).
        let mem = assemble(".ORG $8000\n    RMB0 $12\n    RMB7 $34\n    SMB0 $12\n    SMB7 $34\n")
            .unwrap();
        assert_eq!(
            &mem[0x8000..0x8008],
            &[0x07, 0x12, 0x77, 0x34, 0x87, 0x12, 0xF7, 0x34]
        );
    }

    #[test]
    fn rockwell_bit_ops_reject_a_non_zero_page_operand() {
        // SMB/RMB have no absolute form.
        assert!(assemble(".ORG $8000\n    SMB0 $1234\n").is_err());
    }

    #[test]
    fn base_65c02_opcodes_encode() {
        let mem = assemble(
            ".ORG $8000\n\
             STZ $12\n\
             STZ $1234\n\
             STZ $12,X\n\
             STZ $1234,X\n\
             PHX\n    PHY\n    PLX\n    PLY\n\
             INC A\n    DEC A\n\
             TSB $12\n    TRB $1234\n",
        )
        .unwrap();
        assert_eq!(
            &mem[0x8000..0x8000 + 21],
            &[
                0x64, 0x12, // STZ $12
                0x9C, 0x34, 0x12, // STZ $1234
                0x74, 0x12, // STZ $12,X
                0x9E, 0x34, 0x12, // STZ $1234,X
                0xDA, 0x5A, 0xFA, 0x7A, // PHX PHY PLX PLY
                0x1A, 0x3A, // INC A / DEC A
                0x04, 0x12, // TSB $12
                0x1C, 0x34, 0x12, // TRB $1234
            ]
        );
    }

    #[test]
    fn a_label_named_a_is_not_mistaken_for_the_accumulator() {
        // `A` as an operand is the accumulator only for the mnemonics that have
        // an accumulator mode. `LDA A` / `STA A` target a user symbol named `A`
        // (absolute); `ASL A` / `INC A` stay accumulator ops.
        let mem = assemble(
            ".ORG $8000\n\
             A = $0400\n\
             LDA A\n\
             STA A\n\
             ASL A\n\
             INC A\n",
        )
        .unwrap();
        assert_eq!(
            &mem[0x8000..0x8000 + 8],
            &[
                0xAD, 0x00, 0x04, // LDA $0400  (label A, absolute)
                0x8D, 0x00, 0x04, // STA $0400  (label A, absolute)
                0x0A, // ASL A     (accumulator)
                0x1A, // INC A     (accumulator, CMOS)
            ]
        );
    }

    #[test]
    fn jmp_absolute_indexed_indirect_is_three_bytes() {
        // 65C02 JMP (abs,X) = 0x7C with a 16-bit operand (the table label).
        let mem = assemble(".ORG $8000\n    JMP (tbl,X)\ntbl:\n    .WORD $9000\n").unwrap();
        assert_eq!(mem[0x8000], 0x7C);
        assert_eq!(mem[0x8001], 0x03); // tbl = $8003
        assert_eq!(mem[0x8002], 0x80);
    }

    #[test]
    fn zero_page_indexed_indirect_stays_two_bytes() {
        // The NMOS `(zp,X)` form is unchanged by adding `JMP (abs,X)`.
        let mem = assemble(".ORG $8000\n    LDA ($40,X)\n").unwrap();
        assert_eq!(&mem[0x8000..0x8002], &[0xA1, 0x40]);
    }

    #[test]
    fn bra_is_a_relative_branch() {
        // BRA $8005 from $8000: opcode 0x80, displacement +3 (target − end).
        let mem =
            assemble(".ORG $8000\n    BRA there\n    NOP\n    NOP\nthere:\n    RTS\n").unwrap();
        assert_eq!(mem[0x8000], 0x80);
        assert_eq!(mem[0x8001], 0x02); // $8005 − $8002 (next-instruction address)
    }

    #[test]
    fn rockwell_bit_branch_opcodes() {
        // BBRn = 0x0F | n<<4, BBSn = 0x8F | n<<4: opcode, zero-page byte, then a
        // branch offset relative to the address after the 3-byte instruction.
        let mem = assemble(
            ".ORG $8000\n\
             BBR0 $12,there\n\
             BBS7 $34,there\n\
             NOP\n\
             there:\n    RTS\n",
        )
        .unwrap();
        // BBR0 at $8000: 0x0F, $12, and `there` = $8007, so delta = 7 − 3 = 4.
        assert_eq!(&mem[0x8000..0x8003], &[0x0F, 0x12, 0x04]);
        // BBS7 at $8003: 0x8F | 7<<4 = 0xFF, $34, delta = $8007 − $8006 = 1.
        assert_eq!(&mem[0x8003..0x8006], &[0xFF, 0x34, 0x01]);
    }

    #[test]
    fn a_bit_branch_can_reach_backwards() {
        // A negative displacement: the target is before the instruction.
        let mem = assemble(
            ".ORG $8000\n\
             back:\n    NOP\n\
             BBR3 $40,back\n",
        )
        .unwrap();
        // back = $8000; BBR3 at $8001: opcode 0x3F, $40, delta = $8000 − $8004 = −4.
        assert_eq!(&mem[0x8001..0x8004], &[0x3F, 0x40, (-4i8) as u8]);
    }

    #[test]
    fn a_bit_branch_rejects_a_non_zero_page_operand() {
        let err = assemble(".ORG $8000\n    BBR0 $1234,there\nthere:\n    RTS\n").unwrap_err();
        assert!(err.contains("zero-page"), "{err}");
    }

    #[test]
    fn a_duplicate_symbol_is_an_error() {
        let err = assemble("X = $0400\nX = $0500\n").unwrap_err();
        assert!(err.contains("duplicate symbol 'X'"), "{err}");
    }

    #[test]
    fn emission_past_the_top_of_memory_is_an_error() {
        // The counter used to wrap to $0000 and overwrite the program's start.
        let err = assemble(".ORG $FFFF\n.BYTE 1, 2\n").unwrap_err();
        assert!(err.contains("exceeds"), "{err}");
    }

    #[test]
    fn a_byte_at_the_very_top_of_memory_is_legal() {
        // The vector table's last byte is $FFFF; ending exactly there must work.
        let mem = assemble(".ORG $FFFF\n.BYTE $EE\n").unwrap();
        assert_eq!(mem[0xFFFF], 0xEE);
    }

    #[test]
    fn a_byte_value_that_does_not_fit_is_an_error() {
        // Used to truncate to 44.
        let err = assemble(".ORG $8000\n.BYTE 300\n").unwrap_err();
        assert!(err.contains("does not fit"), "{err}");
    }
}
