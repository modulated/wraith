//! Execution test harness: compile a Wraith program, assemble its output to a
//! flat 64 KB image, run it on a real 6502 emulator, and read back memory.
//!
//! This is what lets tests assert on *runtime behavior* (does `0 <= 0` actually
//! evaluate to 1?) instead of just on generated assembly strings. It is fully
//! self-contained: a small flat assembler for the compiler's exact output
//! dialect (`.ORG`/`.BYTE`/`.WORD`/`.RES`, standard addressing modes) plus the
//! `mos6502` emulator crate - no external assembler/linker required.
//!
//! Shared test infrastructure: this module is compiled into several test
//! binaries but only exercised by some, so unused-item warnings are expected.
#![allow(dead_code)]

use mos6502::cpu::CPU;
use mos6502::instruction::Nmos6502;
use mos6502::memory::Bus;
use std::collections::HashMap;

use super::compile_success;

// ============================================================================
// Flat 6502 assembler
// ============================================================================

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
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
    Rel,
}

/// Official NMOS 6502 opcode for a (mnemonic, addressing-mode) pair.
fn opcode(mnem: &str, mode: Mode) -> Option<u8> {
    use Mode::*;
    let m = |want: Mode, op: u8| if mode == want { Some(op) } else { None };
    // Each arm lists the modes the instruction supports; return the opcode when
    // the requested mode matches.
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
        "DEC" => m(Zp, 0xC6)
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
        "INC" => m(Zp, 0xE6)
            .or(m(ZpX, 0xF6))
            .or(m(Abs, 0xEE))
            .or(m(AbsX, 0xFE)),
        "INX" => m(Imp, 0xE8),
        "INY" => m(Imp, 0xC8),
        "JMP" => m(Abs, 0x4C).or(m(Ind, 0x6C)),
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
        "PLA" => m(Imp, 0x68),
        "PLP" => m(Imp, 0x28),
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
        "TAX" => m(Imp, 0xAA),
        "TAY" => m(Imp, 0xA8),
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
        Mode::Abs | Mode::AbsX | Mode::AbsY | Mode::Ind => 3,
    }
}

/// A parsed operand: the addressing mode plus how to compute its value.
struct ParsedOperand {
    mode: Mode,
    /// Value source, resolved in pass 2 (labels need the symbol table).
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
}

/// Parse a numeric term like `$4E`, `$0200`, `42`, or `$4E+1` -> value.
fn parse_num(s: &str) -> Option<u16> {
    // handle "+N" tail
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
        "BCC" | "BCS" | "BEQ" | "BNE" | "BMI" | "BPL" | "BVC" | "BVS"
    )
}

/// Parse a mnemonic's operand string into an addressing mode + value.
fn parse_operand(mnem: &str, operand: Option<&str>) -> ParsedOperand {
    let po = |mode, value| ParsedOperand { mode, value };
    // A bare shift/rotate (no operand) is accumulator mode in 6502 assembly
    // (ca65 accepts `LSR` as `LSR A`); everything else with no operand is implied.
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
    if op == "A" {
        return po(Mode::Acc, ValueExpr::None);
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
        let v = parse_num(rest).unwrap_or_else(|| panic!("bad immediate operand: {}", op));
        return po(Mode::Imm, ValueExpr::Num(v));
    }

    // Indirect forms
    if let Some(inner) = op.strip_prefix('(') {
        if let Some(zp) = inner.strip_suffix("),Y") {
            let v = parse_num(zp).unwrap_or_else(|| panic!("bad (zp),Y operand: {}", op));
            return po(Mode::IndY, ValueExpr::Num(v));
        }
        if let Some(zp) = inner.strip_suffix(",X)") {
            let v = parse_num(zp).unwrap_or_else(|| panic!("bad (zp,X) operand: {}", op));
            return po(Mode::IndX, ValueExpr::Num(v));
        }
        if let Some(addr) = inner.strip_suffix(')') {
            let v = parse_num(addr).unwrap_or_else(|| panic!("bad (abs) operand: {}", op));
            return po(Mode::Ind, ValueExpr::Num(v));
        }
        panic!("unrecognized indirect operand: {}", op);
    }

    // Indexed: ",X" / ",Y"
    if let Some(base) = op.strip_suffix(",X") {
        return index_operand(base, true, mnem);
    }
    if let Some(base) = op.strip_suffix(",Y") {
        return index_operand(base, false, mnem);
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

    // Bare label in a non-jump context -> absolute (rare; e.g. LDA table).
    let (l, off) = split_label(op);
    po(Mode::Abs, ValueExpr::Label(l, off))
}

fn index_operand(base: &str, x: bool, mnem: &str) -> ParsedOperand {
    let base = base.trim();
    if let Some(v) = parse_num(base) {
        // A `$XXXX` operand (4+ hex digits) forces absolute indexing even for
        // low addresses — matching how real assemblers select abs,Y when the
        // mnemonic has no zp,Y form (e.g. LDA $0040,Y).
        let force_abs = base.strip_prefix('$').is_some_and(|h| h.len() >= 4);
        // On the 6502 only LDX/STX have a zero-page,Y form; every other mnemonic
        // (LDA, STA, CMP, ...) must use absolute,Y for a low address. Real
        // assemblers promote `LDA $40,Y` to `LDA $0040,Y` automatically — mirror
        // that so stdlib routines using the `{param},Y` idiom assemble.
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
    // label(+off),X/Y -> absolute indexed
    let (l, off) = split_label(base);
    ParsedOperand {
        mode: if x { Mode::AbsX } else { Mode::AbsY },
        value: ValueExpr::Label(l, off),
    }
}

/// Strip a trailing `;` comment and surrounding whitespace.
fn clean_line(line: &str) -> &str {
    let no_comment = line.split(';').next().unwrap_or("");
    no_comment.trim()
}

/// Split an optional leading `label:` off an instruction line, returning the
/// label (if any) and the remaining instruction text. Handles both a bare
/// `foo:` label and an inline-asm `foo: JMP foo` (label and instruction on one
/// line, as emitted from `asm { }` blocks). A `NAME = VALUE` symbol definition
/// has no colon and is left untouched.
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

/// Assemble the compiler's asm text into a flat 64 KB image.
pub fn assemble(asm: &str) -> [u8; 65536] {
    // ---- Pass 1: collect label addresses ----
    let mut labels: HashMap<String, u16> = HashMap::new();
    let mut addr: u16 = 0;
    for raw in asm.lines() {
        let line = clean_line(raw);
        if line.is_empty() {
            continue;
        }
        // Symbol definition: `NAME = VALUE` (ca65 syntax used for addr constants).
        if let Some((name, val)) = line.split_once('=') {
            let value = parse_num(val.trim()).expect("bad symbol definition value");
            labels.insert(name.trim().to_string(), value);
            continue;
        }
        // A leading `label:` (possibly followed by an instruction on the same
        // line, as inline asm emits) records the label at the current address.
        let (label, line) = split_leading_label(line);
        if let Some(l) = label {
            labels.insert(l.to_string(), addr);
        }
        if line.is_empty() {
            continue;
        }
        let mut parts = line.splitn(2, char::is_whitespace);
        let head = parts.next().unwrap();
        let rest = parts.next().map(str::trim);
        match head {
            ".ORG" => {
                addr = parse_num(rest.expect(".ORG needs an address")).expect("bad .ORG address");
            }
            ".BYTE" => {
                addr = addr.wrapping_add(rest.unwrap().split(',').count() as u16);
            }
            ".WORD" => {
                addr = addr.wrapping_add(2);
            }
            ".RES" => {
                let n: u16 = rest.unwrap().trim().parse().expect("bad .RES count");
                addr = addr.wrapping_add(n);
            }
            mnem => {
                let parsed = parse_operand(mnem, rest);
                addr = addr.wrapping_add(instr_len(parsed.mode));
            }
        }
    }

    // ---- Pass 2: emit bytes ----
    let resolve = |v: &ValueExpr| -> u16 {
        match v {
            ValueExpr::None => 0,
            ValueExpr::Num(n) => *n,
            ValueExpr::Label(l, off) => {
                let base = *labels
                    .get(l)
                    .unwrap_or_else(|| panic!("undefined label: {}", l));
                (base as i32 + off) as u16
            }
            ValueExpr::LabelLow(l, off) => {
                let base = *labels
                    .get(l)
                    .unwrap_or_else(|| panic!("undefined label: {}", l));
                ((base as i32 + off) as u16) & 0xFF
            }
            ValueExpr::LabelHigh(l, off) => {
                let base = *labels
                    .get(l)
                    .unwrap_or_else(|| panic!("undefined label: {}", l));
                (((base as i32 + off) as u16) >> 8) & 0xFF
            }
        }
    };

    let mut mem = [0u8; 65536];
    let mut addr: u16 = 0;
    let emit = |mem: &mut [u8; 65536], a: &mut u16, byte: u8| {
        mem[*a as usize] = byte;
        *a = a.wrapping_add(1);
    };

    for raw in asm.lines() {
        let line = clean_line(raw);
        if line.is_empty() || line.contains('=') {
            // blank or `NAME = VALUE` (recorded in pass 1)
            continue;
        }
        // Drop any leading `label:` (recorded in pass 1); emit the instruction
        // that may follow it on the same line.
        let (_label, line) = split_leading_label(line);
        if line.is_empty() {
            continue;
        }
        let mut parts = line.splitn(2, char::is_whitespace);
        let head = parts.next().unwrap();
        let rest = parts.next().map(str::trim);
        match head {
            ".ORG" => addr = parse_num(rest.unwrap()).unwrap(),
            ".BYTE" => {
                for tok in rest.unwrap().split(',') {
                    let v = parse_num(tok.trim()).expect("bad .BYTE value");
                    emit(&mut mem, &mut addr, v as u8);
                }
            }
            ".WORD" => {
                let tok = rest.unwrap().trim();
                let v = match parse_num(tok) {
                    Some(n) => n,
                    None => {
                        let (l, off) = split_label(tok);
                        resolve(&ValueExpr::Label(l, off))
                    }
                };
                emit(&mut mem, &mut addr, (v & 0xFF) as u8);
                emit(&mut mem, &mut addr, (v >> 8) as u8);
            }
            ".RES" => {
                let n: u16 = rest.unwrap().trim().parse().unwrap();
                for _ in 0..n {
                    emit(&mut mem, &mut addr, 0);
                }
            }
            mnem => {
                let parsed = parse_operand(mnem, rest);
                let op = opcode(mnem, parsed.mode)
                    .unwrap_or_else(|| panic!("no opcode for {} in this addressing mode", mnem));
                let instr_addr = addr;
                emit(&mut mem, &mut addr, op);
                match parsed.mode {
                    Mode::Imp | Mode::Acc => {}
                    Mode::Rel => {
                        let target = resolve(&parsed.value) as i32;
                        // relative to the address AFTER the 2-byte branch
                        let next = instr_addr as i32 + 2;
                        let delta = target - next;
                        assert!(
                            (-128..=127).contains(&delta),
                            "branch out of range: {} -> {} (delta {})",
                            instr_addr,
                            target,
                            delta
                        );
                        emit(&mut mem, &mut addr, (delta as i8) as u8);
                    }
                    Mode::Imm | Mode::Zp | Mode::ZpX | Mode::ZpY | Mode::IndX | Mode::IndY => {
                        emit(&mut mem, &mut addr, resolve(&parsed.value) as u8);
                    }
                    Mode::Abs | Mode::AbsX | Mode::AbsY | Mode::Ind => {
                        let v = resolve(&parsed.value);
                        emit(&mut mem, &mut addr, (v & 0xFF) as u8);
                        emit(&mut mem, &mut addr, (v >> 8) as u8);
                    }
                }
            }
        }
    }

    mem
}

// ============================================================================
// Execution harness
// ============================================================================

/// A flat 64 KB RAM bus with software-controllable IRQ/NMI lines. The crate's
/// default `Memory` bus never asserts an interrupt, so tests that exercise
/// `#[irq]`/`#[nmi]` handlers use this to pulse the lines under their control.
pub struct TestBus {
    ram: Vec<u8>,
    irq: bool,
    nmi: bool,
    /// Memory-mapped devices. Addresses they claim bypass RAM entirely, so a
    /// read can have side effects (consuming a FIFO byte, clearing a flag).
    pub devices: super::devices::Devices,
}

impl TestBus {
    fn new() -> Self {
        Self {
            ram: vec![0u8; 65536],
            irq: false,
            nmi: false,
            devices: super::devices::Devices::default(),
        }
    }
}

impl Bus for TestBus {
    fn get_byte(&mut self, address: u16) -> u8 {
        match self.devices.read(address) {
            Some(v) => v,
            None => self.ram[address as usize],
        }
    }
    fn set_byte(&mut self, address: u16, value: u8) {
        if !self.devices.write(address, value) {
            self.ram[address as usize] = value;
        }
    }
    fn irq_pending(&mut self) -> bool {
        // Either the harness pulsed the line, or a device is asserting it.
        self.irq || self.devices.irq_asserted()
    }
    fn nmi_pending(&mut self) -> bool {
        self.nmi
    }
}

/// Result of running a compiled Wraith program on the emulator.
pub struct Exec {
    cpu: CPU<TestBus, Nmos6502>,
    /// Address of the terminating `loop {}` (JMP-to-self) the program settled
    /// into. Interrupt pulses run the handler and return control here.
    idle_pc: u16,
    pub steps: usize,
    pub halted: bool,
}

impl Exec {
    /// The attached devices, for feeding input and inspecting captured output.
    pub fn devices(&mut self) -> &mut super::devices::Devices {
        &mut self.cpu.memory.devices
    }

    /// Bytes the program transmitted through the UART, as a string.
    pub fn uart_output(&mut self) -> String {
        let tx = &self
            .cpu
            .memory
            .devices
            .uart
            .as_ref()
            .expect("no UART attached")
            .tx;
        String::from_utf8_lossy(tx).to_string()
    }

    /// Queue bytes for the program to receive on the UART.
    pub fn uart_feed(&mut self, bytes: &[u8]) {
        self.cpu
            .memory
            .devices
            .uart
            .as_mut()
            .expect("no UART attached")
            .feed(bytes);
    }

    /// Run up to `max_steps` more instructions, stopping early if the program
    /// settles back into a JMP-to-self idle loop. Used after feeding a device so
    /// the driver can observe the new state.
    pub fn resume(&mut self, max_steps: usize) {
        for _ in 0..max_steps {
            let pc_before = self.cpu.registers.program_counter;
            if !self.cpu.single_step() {
                break;
            }
            if self.cpu.registers.program_counter == pc_before {
                self.idle_pc = pc_before;
                break;
            }
        }
    }

    /// Read a byte of memory after execution.
    pub fn mem(&mut self, addr: u16) -> u8 {
        self.cpu.memory.get_byte(addr)
    }

    /// Read a little-endian 16-bit word after execution.
    pub fn mem16(&mut self, addr: u16) -> u16 {
        u16::from_le_bytes([self.mem(addr), self.mem(addr.wrapping_add(1))])
    }

    pub fn a(&self) -> u8 {
        self.cpu.registers.accumulator
    }
    pub fn x(&self) -> u8 {
        self.cpu.registers.index_x
    }
    pub fn y(&self) -> u8 {
        self.cpu.registers.index_y
    }

    /// Assert the IRQ line for exactly one servicing: with the program paused in
    /// its idle loop, take the interrupt, run the handler, and resume at idle.
    /// The program must have enabled IRQs (e.g. `asm { "CLI" }`) for this to fire.
    pub fn pulse_irq(&mut self) {
        self.pulse(true);
    }

    /// Assert the NMI line for one edge-triggered servicing.
    pub fn pulse_nmi(&mut self) {
        self.pulse(false);
    }

    /// Hold the IRQ line asserted for a while and report whether it stayed
    /// masked (the CPU never left the idle loop). Used to verify the I-flag
    /// blocks IRQs. Deasserts the line before returning.
    pub fn irq_stays_masked(&mut self) -> bool {
        let idle = self.idle_pc;
        self.cpu.memory.irq = true;
        let mut masked = true;
        for _ in 0..2000 {
            self.cpu.single_step();
            if self.cpu.registers.program_counter != idle {
                masked = false;
                break;
            }
        }
        self.cpu.memory.irq = false;
        masked
    }

    fn pulse(&mut self, irq: bool) {
        let idle = self.idle_pc;
        let mut budget = 200_000usize;

        // Assert the line and step until the interrupt is taken (PC leaves the
        // idle loop). NMI is non-maskable; IRQ needs the I-flag clear.
        if irq {
            self.cpu.memory.irq = true;
        } else {
            self.cpu.memory.nmi = true;
        }
        while self.cpu.registers.program_counter == idle && budget > 0 {
            self.cpu.single_step();
            budget -= 1;
        }
        assert!(
            self.cpu.registers.program_counter != idle,
            "interrupt was never serviced (IRQ needs `CLI`, or no handler is installed)"
        );

        // Deassert so a level-triggered IRQ fires exactly once, and an NMI edge
        // can re-arm for the next pulse.
        if irq {
            self.cpu.memory.irq = false;
        } else {
            self.cpu.memory.nmi = false;
        }

        // Run the handler to completion (RTI returns to the idle loop).
        while self.cpu.registers.program_counter != idle && budget > 0 {
            self.cpu.single_step();
            budget -= 1;
        }
        assert!(
            budget > 0,
            "interrupt handler did not return to the idle loop"
        );
    }
}

/// Compile, assemble, and run a Wraith program to completion.
///
/// Execution starts at the reset vector ($FFFC) and runs until the program
/// reaches its terminating `loop {}` (a `JMP` to its own address, detected as
/// the program counter no longer advancing) or a step budget is exhausted.
/// Panics if the program does not halt within the budget.
pub fn run(source: &str) -> Exec {
    run_with_devices(source, super::devices::Devices::default())
}

/// Compile and run a program against a machine with the given memory-mapped
/// devices attached. Device reads/writes have real side effects (FIFOs drain,
/// flags clear, timers count), so drivers can be exercised end to end.
///
/// Unlike [`run`], execution stops as soon as the program reaches its idle
/// `loop {}` *or* the step budget runs out — a driver that spins waiting on a
/// device the test has not fed would otherwise never halt. Inspect
/// [`Exec::halted`] when that distinction matters.
pub fn run_with_devices(source: &str, devices: super::devices::Devices) -> Exec {
    let asm = compile_success(source);
    let image = assemble(&asm);

    let mut bus = TestBus::new();
    bus.devices = devices;
    // Load the full 64 KB image directly: the default Bus::set_bytes truncates
    // its length to u16 (65536 -> 0), so it would copy nothing.
    bus.ram.copy_from_slice(&image);
    let mut cpu = CPU::new(bus, Nmos6502);
    cpu.reset();

    const BUDGET: usize = 20_000_000;
    let mut steps = 0usize;
    let mut halted = false;
    let mut idle_pc = 0u16;
    while steps < BUDGET {
        let pc_before = cpu.registers.program_counter;
        let executed = cpu.single_step();
        steps += 1;
        if !executed {
            // Couldn't decode an instruction (bad opcode) or a wait state.
            halted = true;
            idle_pc = cpu.registers.program_counter;
            break;
        }
        if cpu.registers.program_counter == pc_before {
            // JMP-to-self: the program's terminating `loop {}`.
            halted = true;
            idle_pc = pc_before;
            break;
        }
    }
    assert!(
        halted || cpu.memory.devices.uart.is_some() || cpu.memory.devices.via.is_some(),
        "program did not halt within {} steps (possible infinite loop or missing `loop {{}}`)",
        BUDGET
    );

    Exec {
        cpu,
        idle_pc,
        steps,
        halted,
    }
}
