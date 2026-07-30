//! Peephole optimizer for 6502 assembly code
//!
//! This module implements pattern-based peephole optimization to improve
//! the quality of generated assembly code by eliminating redundant instructions,
//! dead code, and other inefficiencies.

use std::collections::HashSet;
use std::fmt;

/// Names of memory-mapped I/O locations (`addr` declarations) whose accesses
/// have side effects and must never be optimized away.
///
/// Reads and writes are tracked separately so the guard mirrors the declared
/// access mode (the language's `R` / `W` / `RW` syntax): a read-only register
/// only ever appears in loads, a write-only register only in stores, and a
/// read-write register in both. A load that reads such a register can trigger a
/// hardware side effect (e.g. clearing a status latch) and can return a
/// different value than a preceding write, so redundant-load and
/// load-after-store folding is unsafe on it; a store can likewise latch data or
/// strobe a device, so redundant-store and dead-store folding is unsafe. Plain
/// RAM variables are never listed here and optimize normally.
#[derive(Default, Debug, Clone)]
pub struct VolatileSymbols {
    /// Symbols readable as volatile I/O (access mode `R` or `RW`).
    pub reads: HashSet<String>,
    /// Symbols writable as volatile I/O (access mode `W` or `RW`).
    pub writes: HashSet<String>,
}

impl VolatileSymbols {
    /// True if `operand` names a volatile-readable location.
    fn is_volatile_read(&self, operand: &Option<String>) -> bool {
        operand.as_deref().is_some_and(|op| self.reads.contains(op))
    }

    /// True if `operand` names a volatile-writable location.
    fn is_volatile_write(&self, operand: &Option<String>) -> bool {
        operand
            .as_deref()
            .is_some_and(|op| self.writes.contains(op))
    }
}

/// A parsed assembly instruction
#[derive(Debug, Clone, PartialEq)]
pub enum Line {
    /// An instruction with mnemonic and operand
    Instruction {
        mnemonic: String,
        operand: Option<String>,
        comment: Option<String>,
    },
    /// A label definition
    Label(String),
    /// A comment line
    Comment(String),
    /// A directive (.BYTE, .ORG, etc.)
    Directive { name: String, args: String },
    /// Empty line
    Empty,
}

impl fmt::Display for Line {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Line::Instruction {
                mnemonic,
                operand,
                comment,
            } => {
                write!(f, "    {}", mnemonic)?;
                if let Some(op) = operand {
                    write!(f, " {}", op)?;
                }
                if let Some(cmt) = comment {
                    write!(f, " {}", cmt)?;
                }
                Ok(())
            }
            Line::Label(name) => write!(f, "{}:", name),
            Line::Comment(text) => write!(f, "{}", text),
            Line::Directive { name, args } => write!(f, "{} {}", name, args),
            Line::Empty => Ok(()),
        }
    }
}

/// Parse assembly output into structured lines
pub fn parse_assembly(asm: &str) -> Vec<Line> {
    asm.lines()
        .map(|line| {
            let trimmed = line.trim();

            if trimmed.is_empty() {
                return Line::Empty;
            }

            // Comment line
            if trimmed.starts_with(';') {
                return Line::Comment(line.to_string());
            }

            // Label (a single token ending with a colon). Leading whitespace is
            // allowed: some raw-emitted stdlib labels are indented, and parsing
            // them as instructions let eliminate_unreachable_after_terminator
            // delete the whole block following an unconditional JMP (e.g. the
            // div16/mul16/mod16 bodies after their divide-by-zero guard).
            if trimmed.ends_with(':') && !trimmed[..trimmed.len() - 1].contains(char::is_whitespace)
            {
                return Line::Label(trimmed.trim_end_matches(':').to_string());
            }

            // Directive (starts with .)
            if trimmed.starts_with('.') {
                let parts: Vec<&str> = trimmed.splitn(2, ' ').collect();
                return Line::Directive {
                    name: parts[0].to_string(),
                    args: parts.get(1).unwrap_or(&"").to_string(),
                };
            }

            // Instruction (has leading whitespace)
            if line.starts_with(' ') || line.starts_with('\t') {
                // Split into mnemonic, operand, and optional comment
                let mut parts = trimmed.splitn(2, ' ');
                let mnemonic = parts.next().unwrap_or("").to_string();

                let rest = parts.next().unwrap_or("");
                let (operand, comment) = if let Some(comment_pos) = rest.find(';') {
                    (
                        Some(rest[..comment_pos].trim().to_string()),
                        Some(rest[comment_pos..].to_string()),
                    )
                } else if rest.is_empty() {
                    (None, None)
                } else {
                    (Some(rest.to_string()), None)
                };

                return Line::Instruction {
                    mnemonic,
                    operand,
                    comment,
                };
            }

            // Default: treat as comment
            Line::Comment(line.to_string())
        })
        .collect()
}

/// Apply peephole optimizations to parsed assembly.
///
/// `volatile` names the memory-mapped I/O locations whose loads/stores must be
/// preserved verbatim; pass `&VolatileSymbols::default()` when there are none.
pub fn optimize(lines: &[Line], volatile: &VolatileSymbols) -> Vec<Line> {
    let mut result = lines.to_vec();
    let mut changed = true;

    // Keep applying optimizations until no more changes
    while changed {
        changed = false;

        // Apply each optimization pass
        let before_len = result.len();
        result = eliminate_redundant_loads(&result, volatile);
        result = eliminate_redundant_stores(&result, volatile);
        result = eliminate_load_after_store(&result, volatile);
        result = eliminate_dead_stores(&result, volatile);
        result = eliminate_nop_operations(&result);
        result = eliminate_redundant_transfers(&result);
        result = eliminate_unreachable_after_terminator(&result);
        result = eliminate_jmp_to_next(&result);
        result = collapse_boolean_compares(&result);
        result = fold_literal_operand(&result);
        result = eliminate_nop_carry_pairs(&result);
        result = eliminate_redundant_cmp_zero(&result);
        result = eliminate_redundant_ldy_zero(&result);
        // DISABLED: eliminate_branch_over_jump breaks while loops with large bodies
        // by inverting branches that exceed the 127-byte limit
        // result = eliminate_branch_over_jump(&result);
        result = eliminate_redundant_ldx_zero(&result);
        // DISABLED: eliminate_clc_adc_zero / eliminate_sec_sbc_zero remove
        // `CLC; ADC #$00` / `SEC; SBC #$00`, which preserve A but DO change the
        // N/Z/C/V flags. Codegen uses `SEC; SBC #imm` (incl. #$00) for signed
        // comparisons where those flags are consumed by a following branch, so
        // eliminating the pair silently miscompiles. The compiler never emits
        // these pairs as genuine value no-ops, so dropping the passes loses
        // nothing. (Their unit tests still exercise the functions directly.)
        // result = eliminate_clc_adc_zero(&result);
        // result = eliminate_sec_sbc_zero(&result);
        result = eliminate_redundant_flag_ops(&result);
        result = eliminate_redundant_address_loads(&result);
        result = apply_strength_reduction(&result);
        result = optimize_tail_calls(&result);

        if result.len() != before_len {
            changed = true;
        }
    }

    result
}

/// Eliminate redundant consecutive loads: LDA $40; LDA $40 → LDA $40
fn eliminate_redundant_loads(lines: &[Line], volatile: &VolatileSymbols) -> Vec<Line> {
    let mut result = Vec::new();
    let mut i = 0;

    while i < lines.len() {
        if i + 1 < lines.len()
            && let (
                Line::Instruction {
                    mnemonic: m1,
                    operand: op1,
                    ..
                },
                Line::Instruction {
                    mnemonic: m2,
                    operand: op2,
                    ..
                },
            ) = (&lines[i], &lines[i + 1])
        {
            // Check for same load instruction with same operand. A load of a
            // volatile I/O register is never redundant: the second read may
            // return fresh hardware state or trigger a side effect.
            if (m1 == "LDA" || m1 == "LDX" || m1 == "LDY")
                && m1 == m2
                && op1 == op2
                && !volatile.is_volatile_read(op1)
            {
                // Keep only the first load
                result.push(lines[i].clone());
                i += 2; // Skip the redundant load
                continue;
            }
        }

        result.push(lines[i].clone());
        i += 1;
    }

    result
}

/// Eliminate redundant consecutive stores: STA $40; STA $40 → STA $40
fn eliminate_redundant_stores(lines: &[Line], volatile: &VolatileSymbols) -> Vec<Line> {
    let mut result = Vec::new();
    let mut i = 0;

    while i < lines.len() {
        if i + 1 < lines.len()
            && let (
                Line::Instruction {
                    mnemonic: m1,
                    operand: op1,
                    ..
                },
                Line::Instruction {
                    mnemonic: m2,
                    operand: op2,
                    ..
                },
            ) = (&lines[i], &lines[i + 1])
        {
            // Check for same store instruction with same operand. A store to a
            // volatile I/O register is never redundant: each write may latch
            // data or strobe the device.
            if (m1 == "STA" || m1 == "STX" || m1 == "STY")
                && m1 == m2
                && op1 == op2
                && !volatile.is_volatile_write(op1)
            {
                // Keep only the first store
                result.push(lines[i].clone());
                i += 2; // Skip the redundant store
                continue;
            }
        }

        result.push(lines[i].clone());
        i += 1;
    }

    result
}

/// Eliminate load immediately after store to same location: STA $40; LDA $40 → STA $40
fn eliminate_load_after_store(lines: &[Line], volatile: &VolatileSymbols) -> Vec<Line> {
    let mut result = Vec::new();
    let mut i = 0;

    while i < lines.len() {
        if i + 1 < lines.len()
            && let (
                Line::Instruction {
                    mnemonic: m1,
                    operand: op1,
                    ..
                },
                Line::Instruction {
                    mnemonic: m2,
                    operand: op2,
                    ..
                },
            ) = (&lines[i], &lines[i + 1])
        {
            // The load-back can only be dropped when the location is guaranteed
            // to still hold what was just stored. For a volatile I/O register
            // (a read-write mapping) the read may return different hardware
            // state, so preserve it whenever the location is a volatile read.
            let volatile_read = volatile.is_volatile_read(op1);
            // STA $40; LDA $40 → STA $40 (A already contains the value)
            if m1 == "STA" && m2 == "LDA" && op1 == op2 && !volatile_read {
                result.push(lines[i].clone());
                i += 2; // Skip the load
                continue;
            }
            // STX $40; LDX $40 → STX $40
            if m1 == "STX" && m2 == "LDX" && op1 == op2 && !volatile_read {
                result.push(lines[i].clone());
                i += 2;
                continue;
            }
            // STY $40; LDY $40 → STY $40
            if m1 == "STY" && m2 == "LDY" && op1 == op2 && !volatile_read {
                result.push(lines[i].clone());
                i += 2;
                continue;
            }
        }

        result.push(lines[i].clone());
        i += 1;
    }

    result
}

/// Eliminate dead stores: STA $40; LDA #$05; STA $40 → LDA #$05; STA $40
fn eliminate_dead_stores(lines: &[Line], volatile: &VolatileSymbols) -> Vec<Line> {
    let mut result = Vec::new();
    let mut i = 0;

    while i < lines.len() {
        if i + 2 < lines.len()
            && let (
                Line::Instruction {
                    mnemonic: m1,
                    operand: op1,
                    ..
                },
                Line::Instruction { mnemonic: m2, .. },
                Line::Instruction {
                    mnemonic: m3,
                    operand: op3,
                    ..
                },
            ) = (&lines[i], &lines[i + 1], &lines[i + 2])
        {
            // STA $40; LDA #$05; STA $40 → LDA #$05; STA $40
            // First store is dead because second store overwrites it. A store to
            // a volatile I/O register is never dead: the first write has its own
            // side effect and must be kept.
            if m1 == "STA"
                && m2 == "LDA"
                && m3 == "STA"
                && op1 == op3
                && !volatile.is_volatile_write(op1)
            {
                // Skip the first store
                result.push(lines[i + 1].clone());
                result.push(lines[i + 2].clone());
                i += 3;
                continue;
            }
        }

        result.push(lines[i].clone());
        i += 1;
    }

    result
}

/// Eliminate no-op operations: ORA #$00, AND #$FF, etc.
///
/// These leave A unchanged but DO write N and Z from A, so removing one is
/// sound only when nothing downstream reads those flags before they are
/// overwritten — decided by flags liveness, like the CMP #$00 pass.
fn eliminate_nop_operations(lines: &[Line]) -> Vec<Line> {
    let live_out = compute_flag_liveness(lines);
    lines
        .iter()
        .enumerate()
        .filter(|(i, line)| {
            if let Line::Instruction {
                mnemonic, operand, ..
            } = line
            {
                let value_nop = matches!(
                    (mnemonic.as_str(), operand.as_deref()),
                    ("ORA", Some("#$00")) | ("AND", Some("#$FF")) | ("EOR", Some("#$00"))
                );
                if value_nop && live_out[*i] & (FLAG_N | FLAG_Z) == 0 {
                    return false;
                }
                // ADC #$00 with carry clear is a no-op (but we can't always know carry state)
                // CLC; ADC #$00 can be eliminated as a pair
            }
            true
        })
        .map(|(_, line)| line.clone())
        .collect()
}

/// Eliminate redundant register transfers: TAX; TXA → (nothing, unless A is modified between)
///
/// Both transfers write N and Z from the transferred value, so removing the
/// pair is sound only when those flags are dead afterwards.
fn eliminate_redundant_transfers(lines: &[Line]) -> Vec<Line> {
    let live_out = compute_flag_liveness(lines);
    let mut result = Vec::new();
    let mut i = 0;

    while i < lines.len() {
        if i + 1 < lines.len()
            && let (
                Line::Instruction {
                    mnemonic: m1,
                    operand: None,
                    ..
                },
                Line::Instruction {
                    mnemonic: m2,
                    operand: None,
                    ..
                },
            ) = (&lines[i], &lines[i + 1])
            && live_out[i + 1] & (FLAG_N | FLAG_Z) == 0
        {
            // TAX; TXA → nothing (if no X usage between)
            if m1 == "TAX" && m2 == "TXA" {
                i += 2; // Skip both
                continue;
            }
            // TAY; TYA → nothing
            if m1 == "TAY" && m2 == "TYA" {
                i += 2;
                continue;
            }
            // TXA; TAX → nothing
            if m1 == "TXA" && m2 == "TAX" {
                i += 2;
                continue;
            }
            // TYA; TAY → nothing
            if m1 == "TYA" && m2 == "TAY" {
                i += 2;
                continue;
            }
        }

        result.push(lines[i].clone());
        i += 1;
    }

    result
}

/// Eliminate `CLC; ADC #$00` / `SEC; SBC #$00` pairs.
///
/// With the carry forced, both are value no-ops — but they DO write flags,
/// so the pair is removable only when no flag is live afterwards. (The old
/// CLC/ADC pass was disabled for removing these unconditionally: codegen
/// used to lean on the flags. Liveness makes it safe.)
fn eliminate_nop_carry_pairs(lines: &[Line]) -> Vec<Line> {
    let live_out = compute_flag_liveness(lines);
    let mut result = Vec::new();
    let mut i = 0;

    while i < lines.len() {
        if i + 1 < lines.len()
            && let (
                Line::Instruction {
                    mnemonic: m1,
                    operand: None,
                    ..
                },
                Line::Instruction {
                    mnemonic: m2,
                    operand: Some(op),
                    ..
                },
            ) = (&lines[i], &lines[i + 1])
        {
            let pair = matches!(
                (m1.as_str(), m2.as_str(), op.as_str()),
                ("CLC", "ADC", "#$00") | ("SEC", "SBC", "#$00")
            );
            if pair && live_out[i + 1] & FLAG_ALL == 0 {
                i += 2;
                continue;
            }
        }

        result.push(lines[i].clone());
        i += 1;
    }

    result
}

/// Fold a literal right operand that round-trips through `$20`:
///
/// ```text
///     LDA #$03
///     STA $20
///     LDA x        (and an optional CLC/SEC)
///     OP $20
/// ```
///
/// becomes `LDA x; OP #$03` — the store, the zp read, and 6 cycles gone.
/// OP must have an immediate form (ADC/SBC/AND/ORA/EOR/CMP).
///
/// The store is only dead if nothing reads the staged `$20` afterwards, so
/// scan forward: the next touch of `$20` must be a write, with no label
/// (control could arrive from elsewhere) or call in between.
fn fold_literal_operand(lines: &[Line]) -> Vec<Line> {
    const IMM_OPS: [&str; 6] = ["ADC", "SBC", "AND", "ORA", "EOR", "CMP"];

    // Significant-line indices, as in collapse_boolean_compares.
    let significant: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, l)| !matches!(l, Line::Comment(_) | Line::Empty))
        .map(|(i, _)| i)
        .collect();

    // Would the staged $20 be read before it is next written? Only a read of
    // $20 (a match's scrutinee, a ($20),Y indirection) or a call (the callee
    // may stage its own values there) makes the store live. Reaching a
    // write, a function end, or the end of the stream means it's dead.
    let staged_value_is_dead = |from_sig_pos: usize| {
        for &idx in significant.iter().skip(from_sig_pos) {
            match &lines[idx] {
                Line::Directive { .. } => return true,
                Line::Instruction {
                    mnemonic, operand, ..
                } => {
                    match mnemonic.as_str() {
                        "JSR" => return false,
                        "RTS" | "RTI" | "BRK" => return true,
                        _ => {}
                    }
                    if let Some(op) = operand {
                        if op == "$20" {
                            // First touch: a plain write kills the staged
                            // value (safe); anything else reads it (unsafe).
                            return matches!(
                                mnemonic.as_str(),
                                "STA" | "STX" | "STY" | "INC" | "DEC"
                            );
                        }
                        if op.contains("$20") {
                            return false; // ($20),Y and friends read it
                        }
                    }
                }
                _ => {}
            }
        }
        true
    };

    let mut result = Vec::new();
    let mut i = 0;

    // The instruction at significant position p, if it is one.
    let inst = |p: usize| -> Option<(String, Option<String>)> {
        significant.get(p).and_then(|&idx| match &lines[idx] {
            Line::Instruction {
                mnemonic, operand, ..
            } => Some((mnemonic.clone(), operand.clone())),
            _ => None,
        })
    };

    while i < lines.len() {
        let sig_pos = significant.iter().position(|&idx| idx == i);
        let applied = sig_pos.and_then(|p| {
            let (m0, imm) = inst(p)?;
            let (m1, st) = inst(p + 1)?;
            let (m2, src) = inst(p + 2)?;

            // Either [LDA #imm, STA $20, LDA src, OP $20] or the same with a
            // CLC/SEC before the op.
            let (m3, _) = inst(p + 3)?;
            let (mid, op_mnemonic, op_operand, op_pos): (
                Option<String>,
                String,
                Option<String>,
                usize,
            ) = if IMM_OPS.contains(&m3.as_str()) {
                let (_, op) = inst(p + 3)?;
                (None, m3, op, p + 3)
            } else if matches!(m3.as_str(), "CLC" | "SEC") {
                let (m4, op4) = inst(p + 4)?;
                if IMM_OPS.contains(&m4.as_str()) {
                    (Some(m3), m4, op4, p + 4)
                } else {
                    return None;
                }
            } else {
                return None;
            };

            if m0 == "LDA"
                && imm.as_deref().is_some_and(|s| s.starts_with("#$"))
                && m1 == "STA"
                && st.as_deref() == Some("$20")
                && m2 == "LDA"
                && src.as_deref() != Some("$20")
                && op_operand.as_deref() == Some("$20")
                && staged_value_is_dead(op_pos + 1)
            {
                result.push(Line::Instruction {
                    mnemonic: "LDA".to_string(),
                    operand: src,
                    comment: None,
                });
                if let Some(flag_op) = mid {
                    result.push(Line::Instruction {
                        mnemonic: flag_op,
                        operand: None,
                        comment: None,
                    });
                }
                result.push(Line::Instruction {
                    mnemonic: op_mnemonic,
                    operand: imm,
                    comment: None,
                });
                i = significant[op_pos] + 1;
                return Some(());
            }
            None
        });

        if applied.is_some() {
            continue;
        }

        result.push(lines[i].clone());
        i += 1;
    }

    result
}

/// Collapse a materialized boolean that a condition immediately re-tests:
///
/// ```text
///     Bcc true_N          (an 8-bit comparison's tail)
///     LDA #$00
///     JMP end_M
/// true_N:
///     LDA #$01
/// end_M:
///     CMP #$00
///     BNE then_X          (the if/while dispatch)
/// ```
///
/// becomes `Bcc then_X` — the boolean never exists. This is the hottest
/// pattern in the language (`if (x > 3)`), worth 6 instructions per site.
///
/// Safe only when the labels are used nowhere else (so the removed region
/// has no other entrants) and no flags are live after the BNE: the rewrite
/// leaves the original compare's flags, not `CMP #$00`'s, on the fall-through
/// path.
fn collapse_boolean_compares(lines: &[Line]) -> Vec<Line> {
    // How many times `label` appears as an operand or a definition.
    let uses = |lines: &[Line], label: &str| {
        lines
            .iter()
            .filter(|l| match l {
                Line::Label(name) => name == label,
                Line::Instruction { operand, .. } => operand.as_deref() == Some(label),
                _ => false,
            })
            .count()
    };

    let live_out = compute_flag_liveness(lines);
    let mut result = Vec::new();
    let mut i = 0;

    // Match over the significant (non-comment, non-empty) lines: codegen
    // interleaves comments freely, and they don't affect the semantics.
    let significant: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, l)| !matches!(l, Line::Comment(_) | Line::Empty))
        .map(|(i, _)| i)
        .collect();

    while i < lines.len() {
        // Significant-line window starting at i.
        let win: Option<[usize; 8]> = significant
            .iter()
            .position(|&idx| idx == i)
            .and_then(|p| significant.get(p..p + 8).map(|w| w.try_into().unwrap()));

        if let Some(w) = win
            && let (
                Line::Instruction {
                    mnemonic: br,
                    operand: Some(true_target),
                    ..
                },
                Line::Instruction {
                    mnemonic: m1,
                    operand: Some(a1),
                    ..
                },
                Line::Instruction {
                    mnemonic: m2,
                    operand: Some(end_target),
                    ..
                },
                Line::Label(true_label),
                Line::Instruction {
                    mnemonic: m4,
                    operand: Some(a4),
                    ..
                },
                Line::Label(end_label),
                Line::Instruction {
                    mnemonic: m6,
                    operand: Some(a6),
                    ..
                },
                Line::Instruction {
                    mnemonic: m7,
                    operand: Some(then_target),
                    ..
                },
            ) = (
                &lines[w[0]],
                &lines[w[1]],
                &lines[w[2]],
                &lines[w[3]],
                &lines[w[4]],
                &lines[w[5]],
                &lines[w[6]],
                &lines[w[7]],
            )
        {
            let is_cond_branch = matches!(
                br.as_str(),
                "BCC" | "BCS" | "BEQ" | "BNE" | "BMI" | "BPL" | "BVC" | "BVS"
            );
            if is_cond_branch
                && true_target == true_label
                && m1 == "LDA"
                && a1 == "#$00"
                && m2 == "JMP"
                && end_target == end_label
                && m4 == "LDA"
                && a4 == "#$01"
                && m6 == "CMP"
                && a6 == "#$00"
                && m7 == "BNE"
                && uses(lines, true_label) == 2
                && uses(lines, end_label) == 2
                && live_out[w[7]] == 0
            {
                result.push(Line::Instruction {
                    mnemonic: br.clone(),
                    operand: Some(then_target.clone()),
                    comment: None,
                });
                // Skip everything up to and including the BNE; comments in
                // the middle go with it.
                i = w[7] + 1;
                continue;
            }
        }

        result.push(lines[i].clone());
        i += 1;
    }

    result
}

/// Remove `JMP L` where L is the next real line — the jump is a no-op.
/// (`if` without `else` emits one of these at the end of the then-body.)
/// Labels and comments between the JMP and its target don't change
/// fall-through; a directive or instruction would, so stop there.
fn eliminate_jmp_to_next(lines: &[Line]) -> Vec<Line> {
    let mut result = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if let Line::Instruction {
            mnemonic,
            operand: Some(op),
            ..
        } = line
            && mnemonic == "JMP"
        {
            let target_is_next = lines[i + 1..]
                .iter()
                .find(|l| !matches!(l, Line::Comment(_) | Line::Empty))
                .is_some_and(|next| matches!(next, Line::Label(name) if name == op));
            if target_is_next {
                continue;
            }
        }
        result.push(line.clone());
    }
    result
}

/// Eliminate unreachable code after unconditional control flow terminators
///
/// Removes instructions that follow RTS, JMP, or BRK since they can never be executed.
/// Stops at labels since they may be jump targets from elsewhere.
/// Preserves comments, directives, and empty lines (only removes unreachable instructions).
fn eliminate_unreachable_after_terminator(lines: &[Line]) -> Vec<Line> {
    let mut result = Vec::new();
    let mut skip_until_label = false;

    for line in lines {
        match line {
            // Labels are always kept and reset the skip flag
            Line::Label(_) => {
                skip_until_label = false;
                result.push(line.clone());
            }
            // Check for control flow terminators
            Line::Instruction {
                mnemonic, operand, ..
            } => {
                if skip_until_label {
                    // Skip this instruction - it's unreachable
                    continue;
                }

                result.push(line.clone());

                // Start skipping after unconditional control flow
                if mnemonic == "RTS" || mnemonic == "RTI" || mnemonic == "BRK" {
                    skip_until_label = true;
                } else if mnemonic == "JMP" {
                    // JMP is unconditional (unlike branches)
                    // But JMP ($xxxx) indirect might not terminate if it's a computed jump
                    // For safety, only treat direct JMP as terminator
                    if let Some(op) = operand
                        && !op.starts_with('(')
                    {
                        skip_until_label = true;
                    }
                }
            }
            // Always keep comments, directives, and empty lines
            // These provide structure and documentation, not executable code
            _ => {
                result.push(line.clone());
            }
        }
    }

    result
}

// ============================================================================
// Flags liveness analysis
// ============================================================================
//
// Several peephole rules want to remove or rewrite instructions whose only
// side effect beyond a register/memory value is on the CPU status flags.
// Whether that is sound depends on whether anything downstream *reads* those
// flags before they are overwritten. Hand-reasoning about this per-rule has
// produced repeated miscompiles (the CMP #$00 carry bug here; the disabled
// CLC/ADC #$00 and SEC/SBC #$00 passes). This backward dataflow answers the
// question once, properly, over the branch graph.

/// Flag bit masks for the liveness sets.
const FLAG_N: u8 = 0b0001;
const FLAG_Z: u8 = 0b0010;
const FLAG_C: u8 = 0b0100;
const FLAG_V: u8 = 0b1000;
const FLAG_ALL: u8 = 0b1111;

/// Flags an instruction writes (defines).
fn flags_written(mnemonic: &str) -> u8 {
    match mnemonic {
        "LDA" | "LDX" | "LDY" | "TAX" | "TAY" | "TXA" | "TYA" | "TSX" | "INX" | "INY" | "DEX"
        | "DEY" | "INC" | "DEC" | "AND" | "ORA" | "EOR" | "PLA" => FLAG_N | FLAG_Z,
        "ASL" | "LSR" | "ROL" | "ROR" | "CMP" | "CPX" | "CPY" => FLAG_N | FLAG_Z | FLAG_C,
        "ADC" | "SBC" => FLAG_ALL,
        "CLC" | "SEC" => FLAG_C,
        "CLV" => FLAG_V,
        "BIT" => FLAG_N | FLAG_Z | FLAG_V,
        "PLP" | "RTI" => FLAG_ALL,
        _ => 0,
    }
}

/// Flags an instruction reads (uses). Conservative: anything that leaves the
/// analyzed code (JSR, RTS, BRK) is treated as reading everything.
fn flags_read(mnemonic: &str) -> u8 {
    match mnemonic {
        "BCC" | "BCS" => FLAG_C,
        "BEQ" | "BNE" => FLAG_Z,
        "BMI" | "BPL" => FLAG_N,
        "BVC" | "BVS" => FLAG_V,
        "ADC" | "SBC" | "ROL" | "ROR" => FLAG_C,
        "PHP" | "JSR" | "RTS" | "BRK" => FLAG_ALL,
        _ => 0,
    }
}

fn is_branch_mnemonic(m: &str) -> bool {
    matches!(
        m,
        "BCC" | "BCS" | "BEQ" | "BNE" | "BMI" | "BPL" | "BVC" | "BVS"
    )
}

/// Compute, for every line, the set of flags that may be read after that line
/// executes (before being overwritten) - i.e. the live-out flag set.
///
/// Backward fixpoint over the assembly's control flow: fallthrough edges,
/// branch/JMP label edges. Unknown control flow (indirect JMP, missing label)
/// is treated as all-flags-live. Non-instruction lines pass liveness through.
fn compute_flag_liveness(lines: &[Line]) -> Vec<u8> {
    use std::collections::HashMap;

    // Label name -> line index.
    let mut label_at: HashMap<&str, usize> = HashMap::new();
    for (i, line) in lines.iter().enumerate() {
        if let Line::Label(name) = line {
            label_at.insert(name.as_str(), i);
        }
    }

    let n = lines.len();
    let mut live_in: Vec<u8> = vec![0; n + 1]; // live_in[n] = end of stream
    let mut live_out: Vec<u8> = vec![0; n];

    // Iterate to fixpoint (flag sets only grow, 4 bits each => fast).
    let mut changed = true;
    while changed {
        changed = false;
        for i in (0..n).rev() {
            let (new_out, new_in) = match &lines[i] {
                Line::Instruction {
                    mnemonic, operand, ..
                } => {
                    let mut out = 0u8;
                    if mnemonic == "JMP" {
                        // Indirect JMP or unknown target: assume all live.
                        match operand.as_deref().and_then(|op| label_at.get(op)) {
                            Some(&t) => out |= live_in[t],
                            None => out = FLAG_ALL,
                        }
                    } else if mnemonic == "RTS" || mnemonic == "RTI" || mnemonic == "BRK" {
                        // No successor inside this stream; reads handled below.
                    } else {
                        out |= live_in[i + 1]; // fallthrough
                        if is_branch_mnemonic(mnemonic) {
                            match operand.as_deref().and_then(|op| label_at.get(op)) {
                                Some(&t) => out |= live_in[t],
                                None => out = FLAG_ALL,
                            }
                        }
                    }
                    let inn = flags_read(mnemonic) | (out & !flags_written(mnemonic));
                    (out, inn)
                }
                // Labels, comments, directives, empty lines: pass through.
                _ => (live_in[i + 1], live_in[i + 1]),
            };
            if new_out != live_out[i] || new_in != live_in[i] {
                live_out[i] = new_out;
                live_in[i] = new_in;
                changed = true;
            }
        }
    }

    live_out
}

/// Eliminate redundant CMP #$00 after LDA/AND/ORA/EOR.
///
/// Those instructions already set N and Z exactly as CMP #$00 would (both
/// reflect the value in A). CMP #$00 additionally sets the carry (A >= 0
/// always holds), so the CMP is removable precisely when the carry is dead
/// afterwards - decided by flags liveness over the branch graph, not by
/// pattern-matching the next instruction.
fn eliminate_redundant_cmp_zero(lines: &[Line]) -> Vec<Line> {
    let live_out = compute_flag_liveness(lines);
    let mut result = Vec::new();
    let mut i = 0;

    while i < lines.len() {
        if i + 1 < lines.len()
            && let (
                Line::Instruction { mnemonic: m1, .. },
                Line::Instruction {
                    mnemonic: m2,
                    operand: op2,
                    ..
                },
            ) = (&lines[i], &lines[i + 1])
            && (m1 == "LDA" || m1 == "AND" || m1 == "ORA" || m1 == "EOR")
            && m2 == "CMP"
            && op2.as_deref() == Some("#$00")
            && live_out[i + 1] & FLAG_C == 0
        {
            result.push(lines[i].clone());
            i += 2; // Skip the CMP
            continue;
        }

        result.push(lines[i].clone());
        i += 1;
    }

    result
}

/// Eliminate redundant LDY #$00 when Y is already known to be 0
///
/// Tracks Y register value through the instruction stream and removes
/// redundant loads of 0 into Y.
/// Whether control leaving here means X and Y come back unknown.
///
/// A `JSR` runs a whole function, which is free to use both index registers —
/// and every function this compiler emits does. `RTS`/`RTI`/`BRK`/`JMP` end
/// the straight-line run for the same reason a label starts a new one.
fn clobbers_index_registers(mnemonic: &str) -> bool {
    matches!(mnemonic, "JSR" | "RTS" | "RTI" | "BRK" | "JMP")
}

fn eliminate_redundant_ldy_zero(lines: &[Line]) -> Vec<Line> {
    let mut result = Vec::new();
    let mut y_is_zero = false;

    for line in lines {
        match line {
            Line::Instruction {
                mnemonic, operand, ..
            } => {
                // Check if this is LDY #$00 when Y is already 0
                if mnemonic == "LDY" && operand.as_deref() == Some("#$00") && y_is_zero {
                    // Skip this redundant instruction
                    continue;
                }

                result.push(line.clone());

                // Track Y register state
                if mnemonic == "LDY" {
                    y_is_zero = operand.as_deref() == Some("#$00");
                } else if mnemonic == "INY" || mnemonic == "DEY" {
                    // Y is modified, no longer known to be 0
                    y_is_zero = false;
                } else if mnemonic == "TAY" {
                    // Y = A, unknown value
                    y_is_zero = false;
                } else if mnemonic == "PLY" {
                    // Y pulled from stack, unknown
                    y_is_zero = false;
                } else if clobbers_index_registers(mnemonic) {
                    // A called function is free to use Y, and every function
                    // this compiler emits does — for field offsets, indexing
                    // and the `(zp),Y` stores. Assuming Y survives a JSR
                    // dropped the `LDY #$00` in front of a store through a
                    // pointer, which then wrote at the callee's leftover
                    // offset instead of at the pointer itself.
                    y_is_zero = false;
                }
            }
            Line::Label(_) => {
                // At labels, we don't know Y's value (could jump here from anywhere)
                y_is_zero = false;
                result.push(line.clone());
            }
            _ => {
                result.push(line.clone());
            }
        }
    }

    result
}

// Invert a branch condition
//
// Returns the inverted branch mnemonic, or None if not a conditional branch.
// fn invert_branch(mnemonic: &str) -> Option<&'static str> {
//     match mnemonic {
//         "BEQ" => Some("BNE"),
//         "BNE" => Some("BEQ"),
//         "BCS" => Some("BCC"),
//         "BCC" => Some("BCS"),
//         "BMI" => Some("BPL"),
//         "BPL" => Some("BMI"),
//         "BVS" => Some("BVC"),
//         "BVC" => Some("BVS"),
//         _ => None,
//     }
// }

// Eliminate branch over jump by inverting the branch condition
//
// Pattern:
//     BEQ skip_label
//     JMP target_label
// skip_label:
//
// Becomes:
//     BNE target_label
// skip_label:
//
// Saves 3 bytes (the JMP instruction).
// fn eliminate_branch_over_jump(lines: &[Line]) -> Vec<Line> {
//     let mut result = Vec::new();
//     let mut i = 0;

//     while i < lines.len() {
//         // Check for pattern: Bxx skip; JMP target; skip:
//         if i + 2 < lines.len()
//             && let (
//                 Line::Instruction {
//                     mnemonic: branch_m,
//                     operand: Some(skip_label),
//                     comment: branch_comment,
//                 },
//                 Line::Instruction {
//                     mnemonic: jmp_m,
//                     operand: Some(target_label),
//                     ..
//                 },
//                 Line::Label(label),
//             ) = (&lines[i], &lines[i + 1], &lines[i + 2])
//         {
//             // Check if this is a conditional branch followed by JMP, and the label matches
//             if let Some(inverted) = invert_branch(branch_m)
//                 && jmp_m == "JMP"
//                 && skip_label == label
//             {
//                 // Replace with inverted branch to target
//                 result.push(Line::Instruction {
//                     mnemonic: inverted.to_string(),
//                     operand: Some(target_label.clone()),
//                     comment: branch_comment.clone(),
//                 });
//                 // Keep the label (might be used elsewhere)
//                 result.push(lines[i + 2].clone());
//                 i += 3;
//                 continue;
//             }
//         }

//         result.push(lines[i].clone());
//         i += 1;
//     }

//     result
// }

/// Eliminate redundant LDX #$00 when X is already known to be 0
///
/// Tracks X register value through the instruction stream and removes
/// redundant loads of 0 into X.
fn eliminate_redundant_ldx_zero(lines: &[Line]) -> Vec<Line> {
    let mut result = Vec::new();
    let mut x_is_zero = false;

    for line in lines {
        match line {
            Line::Instruction {
                mnemonic, operand, ..
            } => {
                // Check if this is LDX #$00 when X is already 0
                if mnemonic == "LDX" && operand.as_deref() == Some("#$00") && x_is_zero {
                    // Skip this redundant instruction
                    continue;
                }

                result.push(line.clone());

                // Track X register state
                if mnemonic == "LDX" {
                    x_is_zero = operand.as_deref() == Some("#$00");
                } else if mnemonic == "INX" || mnemonic == "DEX" {
                    // X is modified, no longer known to be 0
                    x_is_zero = false;
                } else if mnemonic == "TAX" {
                    // X = A, unknown value
                    x_is_zero = false;
                } else if mnemonic == "TSX" {
                    // X = stack pointer, unknown
                    x_is_zero = false;
                } else if mnemonic == "PLX" {
                    // X pulled from stack, unknown
                    x_is_zero = false;
                } else if clobbers_index_registers(mnemonic) {
                    // Same as Y: a callee owns X, and a pointer return value
                    // arrives in it.
                    x_is_zero = false;
                }
            }
            Line::Label(_) => {
                // At labels, we don't know X's value (could jump here from anywhere)
                x_is_zero = false;
                result.push(line.clone());
            }
            _ => {
                result.push(line.clone());
            }
        }
    }

    result
}

/// Eliminate CLC; ADC #$00 pair (no-op addition)
///
/// When carry is cleared and we add 0, the result is unchanged.
// Disabled in the optimize() pipeline (flag-unsafe); retained for its unit tests.
#[allow(dead_code)]
fn eliminate_clc_adc_zero(lines: &[Line]) -> Vec<Line> {
    let mut result = Vec::new();
    let mut i = 0;

    while i < lines.len() {
        if i + 1 < lines.len()
            && let (
                Line::Instruction {
                    mnemonic: m1,
                    operand: None,
                    ..
                },
                Line::Instruction {
                    mnemonic: m2,
                    operand: Some(op2),
                    ..
                },
            ) = (&lines[i], &lines[i + 1])
        {
            // CLC followed by ADC #$00 is a no-op
            if m1 == "CLC" && m2 == "ADC" && op2 == "#$00" {
                i += 2; // Skip both instructions
                continue;
            }
        }

        result.push(lines[i].clone());
        i += 1;
    }

    result
}

/// Eliminate SEC; SBC #$00 pair (no-op subtraction)
///
/// When carry is set and we subtract 0, the result is unchanged.
// Disabled in the optimize() pipeline (flag-unsafe); retained for its unit tests.
#[allow(dead_code)]
fn eliminate_sec_sbc_zero(lines: &[Line]) -> Vec<Line> {
    let mut result = Vec::new();
    let mut i = 0;

    while i < lines.len() {
        if i + 1 < lines.len()
            && let (
                Line::Instruction {
                    mnemonic: m1,
                    operand: None,
                    ..
                },
                Line::Instruction {
                    mnemonic: m2,
                    operand: Some(op2),
                    ..
                },
            ) = (&lines[i], &lines[i + 1])
        {
            // SEC followed by SBC #$00 is a no-op
            if m1 == "SEC" && m2 == "SBC" && op2 == "#$00" {
                i += 2; // Skip both instructions
                continue;
            }
        }

        result.push(lines[i].clone());
        i += 1;
    }

    result
}

/// Eliminate redundant flag operations
///
/// Patterns:
///   CLC; CLC → CLC (duplicate)
///   SEC; SEC → SEC (duplicate)
///   CLC; SEC → SEC (first is dead)
///   SEC; CLC → CLC (first is dead)
///   CLI; CLI → CLI
///   SEI; SEI → SEI
///   CLI; SEI → SEI
///   SEI; CLI → CLI
///   CLD; CLD → CLD
///   SED; SED → SED
///   CLD; SED → SED
///   SED; CLD → CLD
fn eliminate_redundant_flag_ops(lines: &[Line]) -> Vec<Line> {
    let mut result = Vec::new();
    let mut i = 0;

    let flag_pairs = [
        ("CLC", "SEC"), // Carry flag
        ("CLI", "SEI"), // Interrupt disable flag
        ("CLD", "SED"), // Decimal mode flag
        ("CLV", "CLV"), // Overflow flag (no SEV on 6502, CLV only)
    ];

    while i < lines.len() {
        if i + 1 < lines.len()
            && let (
                Line::Instruction {
                    mnemonic: m1,
                    operand: None,
                    ..
                },
                Line::Instruction {
                    mnemonic: m2,
                    operand: None,
                    ..
                },
            ) = (&lines[i], &lines[i + 1])
        {
            let mut skip_first = false;

            for (clear, set) in &flag_pairs {
                // Duplicate: CLC; CLC or SEC; SEC
                if m1 == *clear && m2 == *clear {
                    skip_first = true;
                    break;
                }
                if m1 == *set && m2 == *set {
                    skip_first = true;
                    break;
                }
                // Dead operation: CLC; SEC or SEC; CLC
                if m1 == *clear && m2 == *set {
                    skip_first = true;
                    break;
                }
                if m1 == *set && m2 == *clear {
                    skip_first = true;
                    break;
                }
            }

            if skip_first {
                // Skip the first instruction, keep the second
                i += 1;
                continue;
            }
        }

        result.push(lines[i].clone());
        i += 1;
    }

    result
}

/// Eliminate redundant address loading patterns
///
/// When loading a 16-bit address into A/X (low/high bytes), track the loaded
/// address and skip redundant loads of the same address components.
///
/// Pattern:
///     LDA #<label
///     LDX #>label
///     ... (code that doesn't modify A/X)
///     LDA #<label    ; redundant if A still has #<label
///     LDX #>label    ; redundant if X still has #>label
fn eliminate_redundant_address_loads(lines: &[Line]) -> Vec<Line> {
    let mut result = Vec::new();
    let mut a_value: Option<String> = None; // Track what's in A
    let mut x_value: Option<String> = None; // Track what's in X

    for line in lines {
        match line {
            Line::Instruction {
                mnemonic,
                operand,
                comment,
            } => {
                // Check for redundant LDA #immediate
                if mnemonic == "LDA"
                    && let Some(op) = operand
                    && op.starts_with("#")
                {
                    if a_value.as_ref() == Some(op) {
                        // A already contains this value, skip the load
                        continue;
                    }
                    // Track the new value in A
                    a_value = Some(op.clone());
                    result.push(line.clone());
                    continue;
                }

                // Check for redundant LDX #immediate
                if mnemonic == "LDX"
                    && let Some(op) = operand
                    && op.starts_with("#")
                {
                    if x_value.as_ref() == Some(op) {
                        // X already contains this value, skip the load
                        continue;
                    }
                    // Track the new value in X
                    x_value = Some(op.clone());
                    result.push(line.clone());
                    continue;
                }

                // Instructions that modify A
                if matches!(
                    mnemonic.as_str(),
                    "LDA"
                        | "TXA"
                        | "TYA"
                        | "PLA"
                        | "ADC"
                        | "SBC"
                        | "AND"
                        | "ORA"
                        | "EOR"
                        | "ASL"
                        | "LSR"
                        | "ROL"
                        | "ROR"
                ) {
                    a_value = None;
                }

                // Instructions that modify X
                if matches!(
                    mnemonic.as_str(),
                    "LDX" | "TAX" | "TSX" | "INX" | "DEX" | "PLX"
                ) {
                    x_value = None;
                }

                // JSR/RTS/JMP invalidate register state (calling convention)
                if matches!(mnemonic.as_str(), "JSR" | "RTS" | "RTI" | "JMP" | "BRK") {
                    a_value = None;
                    x_value = None;
                }

                result.push(Line::Instruction {
                    mnemonic: mnemonic.clone(),
                    operand: operand.clone(),
                    comment: comment.clone(),
                });
            }
            Line::Label(_) => {
                // Labels are potential jump targets, reset tracking
                a_value = None;
                x_value = None;
                result.push(line.clone());
            }
            _ => {
                result.push(line.clone());
            }
        }
    }

    result
}

/// Apply strength reduction optimizations
///
/// Convert expensive operations into cheaper equivalents:
///
/// Pattern 1: Multiply by 2 using addition
///     CLC
///     ADC <same-location>  ; A = A + A (effectively A * 2)
/// Becomes:
///     ASL A                ; Shift left (same result, fewer cycles)
///
/// Pattern 2: Self-addition (doubling)
///     LDA $xx
///     CLC
///     ADC $xx              ; A = A + A
/// Becomes:
///     LDA $xx
///     ASL A
fn apply_strength_reduction(lines: &[Line]) -> Vec<Line> {
    let mut result = Vec::new();
    let mut i = 0;

    while i < lines.len() {
        // Pattern 1: LDA $xx; CLC; ADC $xx → LDA $xx; ASL A
        if i + 2 < lines.len()
            && let (
                Line::Instruction {
                    mnemonic: m1,
                    operand: Some(op1),
                    ..
                },
                Line::Instruction {
                    mnemonic: m2,
                    operand: None,
                    ..
                },
                Line::Instruction {
                    mnemonic: m3,
                    operand: Some(op3),
                    comment: c3,
                },
            ) = (&lines[i], &lines[i + 1], &lines[i + 2])
        {
            // Check for LDA $xx; CLC; ADC $xx pattern (doubling)
            if m1 == "LDA" && m2 == "CLC" && m3 == "ADC" && op1 == op3 && !op1.starts_with("#") {
                // Replace with LDA $xx; ASL A
                result.push(lines[i].clone()); // Keep the LDA
                result.push(Line::Instruction {
                    mnemonic: "ASL".to_string(),
                    operand: Some("A".to_string()),
                    comment: c3.clone(),
                });
                i += 3;
                continue;
            }
        }

        // Pattern 2: CLC; ADC $xx where A already contains value from $xx
        // This requires tracking what's in A, which we do via context
        // For now, check simpler pattern: CLC; ADC A (self-add in accumulator mode)
        // Note: 6502 doesn't have "ADC A" as accumulator mode, but some assemblers support it
        // The more common pattern is covered above

        result.push(lines[i].clone());
        i += 1;
    }

    result
}

/// Optimize tail calls: JSR followed by RTS becomes JMP
///
/// Pattern:
///     JSR subroutine
///     [optional comments]
///     RTS
/// Becomes:
///     JMP subroutine
///
/// This saves cycles and stack space since the subroutine's RTS
/// will return directly to our caller.
fn optimize_tail_calls(lines: &[Line]) -> Vec<Line> {
    let mut result = Vec::new();
    let mut i = 0;

    while i < lines.len() {
        // Check for JSR instruction
        if let Line::Instruction {
            mnemonic,
            operand: Some(target),
            comment,
        } = &lines[i]
            && mnemonic == "JSR"
        {
            // Look ahead for RTS, skipping comments and empty lines
            let mut j = i + 1;
            let mut skipped_lines = Vec::new();

            while j < lines.len() {
                match &lines[j] {
                    Line::Comment(_) | Line::Empty => {
                        skipped_lines.push(lines[j].clone());
                        j += 1;
                    }
                    Line::Instruction {
                        mnemonic: m2,
                        operand: None,
                        ..
                    } if m2 == "RTS" => {
                        // Found JSR; [comments]; RTS pattern - optimize to JMP
                        result.push(Line::Instruction {
                            mnemonic: "JMP".to_string(),
                            operand: Some(target.clone()),
                            comment: comment.clone(),
                        });
                        // Skip the JSR, comments, and RTS
                        i = j + 1;
                        // Break inner loop; the `if i > j` check below will continue outer loop
                        break;
                    }
                    _ => {
                        // Not RTS, can't optimize
                        break;
                    }
                }
            }

            // If we found a match, we've already handled it above
            if i > j {
                continue;
            }
        }

        result.push(lines[i].clone());
        i += 1;
    }

    result
}

/// Convert optimized lines back to assembly string
pub fn lines_to_string(lines: &[Line]) -> String {
    let mut result = lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");

    // Ensure the file ends with a newline (Unix text file convention)
    if !result.ends_with('\n') {
        result.push('\n');
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_redundant_load_elimination() {
        let asm = "    LDA $40\n    LDA $40\n";
        let lines = parse_assembly(asm);
        let optimized = eliminate_redundant_loads(&lines, &VolatileSymbols::default());
        assert_eq!(optimized.len(), 1);
    }

    #[test]
    fn test_load_after_store_elimination() {
        let asm = "    STA $40\n    LDA $40\n";
        let lines = parse_assembly(asm);
        let optimized = eliminate_load_after_store(&lines, &VolatileSymbols::default());
        assert_eq!(optimized.len(), 1);
    }

    #[test]
    fn test_dead_store_elimination() {
        let asm = "    STA $40\n    LDA #$05\n    STA $40\n";
        let lines = parse_assembly(asm);
        let optimized = eliminate_dead_stores(&lines, &VolatileSymbols::default());
        assert_eq!(optimized.len(), 2);
    }

    #[test]
    fn test_unreachable_after_rts() {
        let asm = "    RTS\n    JMP label\n    LDA #$00\nlabel:\n";
        let lines = parse_assembly(asm);
        let optimized = eliminate_unreachable_after_terminator(&lines);
        // Should keep RTS and label, remove JMP and LDA
        assert_eq!(optimized.len(), 2);
        assert!(matches!(&optimized[0], Line::Instruction { mnemonic, .. } if mnemonic == "RTS"));
        assert!(matches!(&optimized[1], Line::Label(l) if l == "label"));
    }

    #[test]
    fn test_unreachable_after_jmp() {
        let asm = "    JMP somewhere\n    LDA #$00\n    STA $40\nnext:\n";
        let lines = parse_assembly(asm);
        let optimized = eliminate_unreachable_after_terminator(&lines);
        // Should keep JMP and label, remove LDA and STA
        assert_eq!(optimized.len(), 2);
        assert!(matches!(&optimized[0], Line::Instruction { mnemonic, .. } if mnemonic == "JMP"));
        assert!(matches!(&optimized[1], Line::Label(l) if l == "next"));
    }

    #[test]
    fn test_unreachable_preserves_comments() {
        let asm = "    RTS\n; This is a comment\n    LDA #$00\nlabel:\n";
        let lines = parse_assembly(asm);
        let optimized = eliminate_unreachable_after_terminator(&lines);
        // Should keep RTS, comment, and label; remove LDA
        assert_eq!(optimized.len(), 3);
        assert!(matches!(&optimized[1], Line::Comment(_)));
    }

    #[test]
    fn test_unreachable_indirect_jmp_not_terminator() {
        // Indirect JMP like JMP ($30) should NOT be treated as terminator
        // because it could be a computed jump that returns
        let asm = "    JMP ($30)\n    LDA #$00\nlabel:\n";
        let lines = parse_assembly(asm);
        let optimized = eliminate_unreachable_after_terminator(&lines);
        // Should keep all lines since indirect JMP is not a terminator
        assert_eq!(optimized.len(), 3);
    }

    #[test]
    fn test_redundant_cmp_zero_after_lda() {
        let asm = "    LDA $40\n    CMP #$00\n    BEQ label\nlabel:\n";
        let lines = parse_assembly(asm);
        let optimized = eliminate_redundant_cmp_zero(&lines);
        // CMP #$00 should be removed, LDA sets Z flag
        assert_eq!(optimized.len(), 3);
        assert!(matches!(&optimized[0], Line::Instruction { mnemonic, .. } if mnemonic == "LDA"));
        assert!(matches!(&optimized[1], Line::Instruction { mnemonic, .. } if mnemonic == "BEQ"));
    }

    #[test]
    fn test_redundant_cmp_zero_after_and() {
        let asm = "    AND #$0F\n    CMP #$00\n    BNE label\nlabel:\n";
        let lines = parse_assembly(asm);
        let optimized = eliminate_redundant_cmp_zero(&lines);
        // CMP #$00 should be removed, AND sets Z flag
        assert_eq!(optimized.len(), 3);
    }

    #[test]
    fn test_cmp_zero_kept_when_carry_consumed_by_sbc() {
        // Multi-byte compare: the carry from CMP #$00 seeds the SBC borrow.
        let asm = "    LDA $40\n    CMP #$00\n    LDA $41\n    SBC #$01\n    BCC label\nlabel:\n";
        let lines = parse_assembly(asm);
        let optimized = eliminate_redundant_cmp_zero(&lines);
        assert_eq!(optimized.len(), 6, "CMP #$00 must survive: carry is live");
    }

    #[test]
    fn test_cmp_zero_kept_when_carry_live_at_branch_target() {
        // The carry is dead on the fallthrough but read at the branch target;
        // liveness must join over both edges.
        let asm = "    LDA $40\n    CMP #$00\n    BEQ tgt\n    RTS\ntgt:\n    BCS other\nother:\n";
        let lines = parse_assembly(asm);
        let optimized = eliminate_redundant_cmp_zero(&lines);
        let cmp_count = optimized
            .iter()
            .filter(|l| matches!(l, Line::Instruction { mnemonic, .. } if mnemonic == "CMP"))
            .count();
        assert_eq!(cmp_count, 1, "CMP #$00 must survive: carry live at target");
    }

    #[test]
    fn test_cmp_zero_removed_across_non_flag_instruction() {
        // STA doesn't touch flags; the BNE still only needs Z, so liveness
        // removes the CMP even though the branch is not immediately adjacent.
        let asm = "    LDA $40\n    CMP #$00\n    STA $50\n    BNE label\nlabel:\n";
        let lines = parse_assembly(asm);
        let optimized = eliminate_redundant_cmp_zero(&lines);
        assert_eq!(optimized.len(), 4, "CMP #$00 removable across STA");
    }

    #[test]
    fn test_cmp_nonzero_not_eliminated() {
        let asm = "    LDA $40\n    CMP #$05\n    BEQ label\n";
        let lines = parse_assembly(asm);
        let optimized = eliminate_redundant_cmp_zero(&lines);
        // CMP #$05 should NOT be removed
        assert_eq!(optimized.len(), 3);
    }

    #[test]
    fn test_nop_removed_when_flags_are_dead() {
        let asm = "    LDA $40\n    ORA #$00\n    STA $50\n";
        let lines = parse_assembly(asm);
        let optimized = eliminate_nop_operations(&lines);
        assert_eq!(optimized.len(), 2, "ORA #$00 removable when N/Z are dead");
    }

    #[test]
    fn test_nop_kept_when_a_branch_reads_the_flags() {
        // LDX sets N/Z from X; the ORA re-establishes them from A for the BEQ.
        // Removing it makes the branch test X instead of A.
        let asm = "    LDA $40\n    LDX #$00\n    ORA #$00\n    BEQ label\nlabel:\n";
        let lines = parse_assembly(asm);
        let optimized = eliminate_nop_operations(&lines);
        assert_eq!(optimized.len(), 5, "ORA #$00 feeds the BEQ's Z flag");
    }

    #[test]
    fn test_transfer_pair_removed_when_flags_are_dead() {
        let asm = "    LDA $40\n    TAX\n    TXA\n    STA $50\n";
        let lines = parse_assembly(asm);
        let optimized = eliminate_redundant_transfers(&lines);
        assert_eq!(optimized.len(), 2, "TAX;TXA removable when N/Z are dead");
    }

    #[test]
    fn test_transfer_pair_kept_when_a_branch_reads_the_flags() {
        // Removing TAX;TXA leaves the BEQ testing the LDY's flags (Y's value),
        // not A's.
        let asm = "    LDA $40\n    LDY $20\n    TAX\n    TXA\n    BEQ label\nlabel:\n";
        let lines = parse_assembly(asm);
        let optimized = eliminate_redundant_transfers(&lines);
        assert_eq!(optimized.len(), 6, "the pair feeds the BEQ's Z flag");
    }

    #[test]
    fn test_redundant_ldy_zero() {
        let asm = "    LDY #$00\n    LDA ($20),Y\n    LDY #$00\n    LDA ($22),Y\n";
        let lines = parse_assembly(asm);
        let optimized = eliminate_redundant_ldy_zero(&lines);
        // Second LDY #$00 should be removed
        assert_eq!(optimized.len(), 3);
    }

    #[test]
    fn test_ldy_zero_after_iny() {
        let asm = "    LDY #$00\n    INY\n    LDY #$00\n";
        let lines = parse_assembly(asm);
        let optimized = eliminate_redundant_ldy_zero(&lines);
        // After INY, Y is not 0, so second LDY #$00 is needed
        assert_eq!(optimized.len(), 3);
    }

    #[test]
    fn test_ldy_zero_after_label() {
        let asm = "    LDY #$00\nlabel:\n    LDY #$00\n";
        let lines = parse_assembly(asm);
        let optimized = eliminate_redundant_ldy_zero(&lines);
        // After label, Y state is unknown, so second LDY #$00 is needed
        assert_eq!(optimized.len(), 3);
    }

    // // Branch inversion tests

    // #[test]
    // fn test_branch_inversion_beq_jmp() {
    //     let asm = "    BEQ skip\n    JMP target\nskip:\n    LDA #$00\n";
    //     let lines = parse_assembly(asm);
    //     let optimized = eliminate_branch_over_jump(&lines);
    //     // BEQ skip; JMP target; skip: → BNE target; skip:
    //     assert_eq!(optimized.len(), 3);
    //     assert!(
    //         matches!(&optimized[0], Line::Instruction { mnemonic, operand, .. }
    //         if mnemonic == "BNE" && operand.as_deref() == Some("target"))
    //     );
    //     assert!(matches!(&optimized[1], Line::Label(l) if l == "skip"));
    // }

    // #[test]
    // fn test_branch_inversion_bne_jmp() {
    //     let asm = "    BNE skip\n    JMP target\nskip:\n";
    //     let lines = parse_assembly(asm);
    //     let optimized = eliminate_branch_over_jump(&lines);
    //     // BNE skip; JMP target; skip: → BEQ target; skip:
    //     assert_eq!(optimized.len(), 2);
    //     assert!(matches!(&optimized[0], Line::Instruction { mnemonic, .. } if mnemonic == "BEQ"));
    // }

    // #[test]
    // fn test_branch_inversion_bcs_jmp() {
    //     let asm = "    BCS skip\n    JMP target\nskip:\n";
    //     let lines = parse_assembly(asm);
    //     let optimized = eliminate_branch_over_jump(&lines);
    //     assert!(matches!(&optimized[0], Line::Instruction { mnemonic, .. } if mnemonic == "BCC"));
    // }

    // #[test]
    // fn test_branch_inversion_preserves_nonmatching() {
    //     // Label doesn't match branch target - should not optimize
    //     let asm = "    BEQ other\n    JMP target\nskip:\n";
    //     let lines = parse_assembly(asm);
    //     let optimized = eliminate_branch_over_jump(&lines);
    //     // Should keep all 3 lines unchanged
    //     assert_eq!(optimized.len(), 3);
    //     assert!(matches!(&optimized[0], Line::Instruction { mnemonic, .. } if mnemonic == "BEQ"));
    //     assert!(matches!(&optimized[1], Line::Instruction { mnemonic, .. } if mnemonic == "JMP"));
    // }

    // LDX #$00 tracking tests

    #[test]
    fn test_redundant_ldx_zero() {
        let asm = "    LDX #$00\n    STX $40\n    LDX #$00\n";
        let lines = parse_assembly(asm);
        let optimized = eliminate_redundant_ldx_zero(&lines);
        // Second LDX #$00 should be removed
        assert_eq!(optimized.len(), 2);
    }

    #[test]
    fn test_ldx_zero_after_inx() {
        let asm = "    LDX #$00\n    INX\n    LDX #$00\n";
        let lines = parse_assembly(asm);
        let optimized = eliminate_redundant_ldx_zero(&lines);
        // After INX, X is not 0, so second LDX #$00 is needed
        assert_eq!(optimized.len(), 3);
    }

    #[test]
    fn test_ldx_zero_after_tax() {
        let asm = "    LDX #$00\n    TAX\n    LDX #$00\n";
        let lines = parse_assembly(asm);
        let optimized = eliminate_redundant_ldx_zero(&lines);
        // After TAX, X = A (unknown), so second LDX #$00 is needed
        assert_eq!(optimized.len(), 3);
    }

    // CLC; ADC #$00 tests

    #[test]
    fn test_clc_adc_zero_elimination() {
        let asm = "    CLC\n    ADC #$00\n    STA $40\n";
        let lines = parse_assembly(asm);
        let optimized = eliminate_clc_adc_zero(&lines);
        // CLC; ADC #$00 should be removed
        assert_eq!(optimized.len(), 1);
        assert!(matches!(&optimized[0], Line::Instruction { mnemonic, .. } if mnemonic == "STA"));
    }

    #[test]
    fn test_clc_adc_nonzero_preserved() {
        let asm = "    CLC\n    ADC #$01\n";
        let lines = parse_assembly(asm);
        let optimized = eliminate_clc_adc_zero(&lines);
        // CLC; ADC #$01 should NOT be removed
        assert_eq!(optimized.len(), 2);
    }

    // SEC; SBC #$00 tests

    #[test]
    fn test_sec_sbc_zero_elimination() {
        let asm = "    SEC\n    SBC #$00\n    STA $40\n";
        let lines = parse_assembly(asm);
        let optimized = eliminate_sec_sbc_zero(&lines);
        // SEC; SBC #$00 should be removed
        assert_eq!(optimized.len(), 1);
        assert!(matches!(&optimized[0], Line::Instruction { mnemonic, .. } if mnemonic == "STA"));
    }

    #[test]
    fn test_sec_sbc_nonzero_preserved() {
        let asm = "    SEC\n    SBC #$01\n";
        let lines = parse_assembly(asm);
        let optimized = eliminate_sec_sbc_zero(&lines);
        // SEC; SBC #$01 should NOT be removed
        assert_eq!(optimized.len(), 2);
    }

    // Redundant flag operations tests

    #[test]
    fn test_redundant_clc() {
        let asm = "    CLC\n    CLC\n    ADC $40\n";
        let lines = parse_assembly(asm);
        let optimized = eliminate_redundant_flag_ops(&lines);
        // First CLC is redundant
        assert_eq!(optimized.len(), 2);
    }

    #[test]
    fn test_redundant_sec() {
        let asm = "    SEC\n    SEC\n";
        let lines = parse_assembly(asm);
        let optimized = eliminate_redundant_flag_ops(&lines);
        assert_eq!(optimized.len(), 1);
    }

    #[test]
    fn test_clc_sec_elimination() {
        let asm = "    CLC\n    SEC\n    SBC $40\n";
        let lines = parse_assembly(asm);
        let optimized = eliminate_redundant_flag_ops(&lines);
        // CLC is dead before SEC
        assert_eq!(optimized.len(), 2);
        assert!(matches!(&optimized[0], Line::Instruction { mnemonic, .. } if mnemonic == "SEC"));
    }

    #[test]
    fn test_sec_clc_elimination() {
        let asm = "    SEC\n    CLC\n    ADC $40\n";
        let lines = parse_assembly(asm);
        let optimized = eliminate_redundant_flag_ops(&lines);
        // SEC is dead before CLC
        assert_eq!(optimized.len(), 2);
        assert!(matches!(&optimized[0], Line::Instruction { mnemonic, .. } if mnemonic == "CLC"));
    }

    #[test]
    fn test_cli_sei_elimination() {
        let asm = "    CLI\n    SEI\n";
        let lines = parse_assembly(asm);
        let optimized = eliminate_redundant_flag_ops(&lines);
        // CLI is dead before SEI
        assert_eq!(optimized.len(), 1);
        assert!(matches!(&optimized[0], Line::Instruction { mnemonic, .. } if mnemonic == "SEI"));
    }

    // ========================================================================
    // Address Loading Optimization Tests
    // ========================================================================

    #[test]
    fn test_redundant_address_load_a() {
        let asm = "    LDA #$00\n    STA $40\n    LDA #$00\n    STA $41\n";
        let lines = parse_assembly(asm);
        let optimized = eliminate_redundant_address_loads(&lines);
        // Second LDA #$00 should be removed since A still has #$00
        assert_eq!(optimized.len(), 3);
    }

    #[test]
    fn test_redundant_address_load_x() {
        let asm = "    LDX #$10\n    STX $40\n    LDX #$10\n";
        let lines = parse_assembly(asm);
        let optimized = eliminate_redundant_address_loads(&lines);
        // Second LDX #$10 should be removed
        assert_eq!(optimized.len(), 2);
    }

    #[test]
    fn test_address_load_after_modification() {
        let asm = "    LDA #$00\n    ADC #$01\n    LDA #$00\n";
        let lines = parse_assembly(asm);
        let optimized = eliminate_redundant_address_loads(&lines);
        // After ADC, A is modified, so second LDA is needed
        assert_eq!(optimized.len(), 3);
    }

    #[test]
    fn test_address_load_invalidated_by_label() {
        let asm = "    LDA #$00\nlabel:\n    LDA #$00\n";
        let lines = parse_assembly(asm);
        let optimized = eliminate_redundant_address_loads(&lines);
        // After label, A state is unknown, so second LDA is needed
        assert_eq!(optimized.len(), 3);
    }

    #[test]
    fn test_address_load_different_values() {
        let asm = "    LDA #$00\n    LDA #$01\n";
        let lines = parse_assembly(asm);
        let optimized = eliminate_redundant_address_loads(&lines);
        // Different values, both should be kept
        assert_eq!(optimized.len(), 2);
    }

    // ========================================================================
    // Strength Reduction Tests
    // ========================================================================

    #[test]
    fn test_strength_reduction_double() {
        let asm = "    LDA $40\n    CLC\n    ADC $40\n";
        let lines = parse_assembly(asm);
        let optimized = apply_strength_reduction(&lines);
        // LDA $40; CLC; ADC $40 → LDA $40; ASL A
        assert_eq!(optimized.len(), 2);
        assert!(
            matches!(&optimized[0], Line::Instruction { mnemonic, operand, .. }
            if mnemonic == "LDA" && operand.as_deref() == Some("$40"))
        );
        assert!(
            matches!(&optimized[1], Line::Instruction { mnemonic, operand, .. }
            if mnemonic == "ASL" && operand.as_deref() == Some("A"))
        );
    }

    #[test]
    fn test_strength_reduction_immediate_not_applied() {
        // Don't apply to immediate values - this is for self-addition only
        let asm = "    LDA #$05\n    CLC\n    ADC #$05\n";
        let lines = parse_assembly(asm);
        let optimized = apply_strength_reduction(&lines);
        // Immediate addition is different from self-addition, keep original
        assert_eq!(optimized.len(), 3);
    }

    #[test]
    fn test_strength_reduction_different_operands() {
        let asm = "    LDA $40\n    CLC\n    ADC $41\n";
        let lines = parse_assembly(asm);
        let optimized = apply_strength_reduction(&lines);
        // Different operands, not a doubling pattern
        assert_eq!(optimized.len(), 3);
    }

    // ========================================================================
    // Tail Call Optimization Tests
    // ========================================================================

    #[test]
    fn test_tail_call_jsr_rts() {
        let asm = "    JSR subroutine\n    RTS\n";
        let lines = parse_assembly(asm);
        let optimized = optimize_tail_calls(&lines);
        // JSR; RTS → JMP
        assert_eq!(optimized.len(), 1);
        assert!(
            matches!(&optimized[0], Line::Instruction { mnemonic, operand, .. }
            if mnemonic == "JMP" && operand.as_deref() == Some("subroutine"))
        );
    }

    #[test]
    fn test_tail_call_with_code_between() {
        let asm = "    JSR subroutine\n    LDA #$00\n    RTS\n";
        let lines = parse_assembly(asm);
        let optimized = optimize_tail_calls(&lines);
        // Code between JSR and RTS, cannot optimize
        assert_eq!(optimized.len(), 3);
    }

    #[test]
    fn test_tail_call_multiple() {
        let asm = "    JSR func1\n    RTS\n    JSR func2\n    RTS\n";
        let lines = parse_assembly(asm);
        let optimized = optimize_tail_calls(&lines);
        // Both JSR; RTS pairs should be optimized
        assert_eq!(optimized.len(), 2);
        assert!(matches!(&optimized[0], Line::Instruction { mnemonic, .. } if mnemonic == "JMP"));
        assert!(matches!(&optimized[1], Line::Instruction { mnemonic, .. } if mnemonic == "JMP"));
    }

    #[test]
    fn test_tail_call_preserves_jsr_without_rts() {
        let asm = "    JSR func1\n    JSR func2\n    RTS\n";
        let lines = parse_assembly(asm);
        let optimized = optimize_tail_calls(&lines);
        // First JSR cannot be optimized (followed by another JSR)
        // Second JSR; RTS can be optimized
        assert_eq!(optimized.len(), 2);
        assert!(matches!(&optimized[0], Line::Instruction { mnemonic, .. } if mnemonic == "JSR"));
        assert!(matches!(&optimized[1], Line::Instruction { mnemonic, .. } if mnemonic == "JMP"));
    }

    #[test]
    fn test_tail_call_with_comments_between() {
        let asm = "    JSR subroutine\n; Returns: A=result\n    RTS\n";
        let lines = parse_assembly(asm);
        let optimized = optimize_tail_calls(&lines);
        // JSR; comment; RTS → JMP (comment skipped)
        assert_eq!(optimized.len(), 1);
        assert!(
            matches!(&optimized[0], Line::Instruction { mnemonic, operand, .. }
            if mnemonic == "JMP" && operand.as_deref() == Some("subroutine"))
        );
    }
}

#[cfg(test)]
mod jmp_next_tests {
    use super::*;

    #[test]
    fn jmp_to_the_next_line_is_removed() {
        let asm = "    LDA $40\n    JMP end_1\nend_1:\n    RTS\n";
        let lines = parse_assembly(asm);
        let optimized = eliminate_jmp_to_next(&lines);
        assert_eq!(optimized.len(), 3);
    }

    #[test]
    fn jmp_past_an_instruction_stays() {
        let asm = "    JMP end_1\n    LDA $40\nend_1:\n    RTS\n";
        let lines = parse_assembly(asm);
        let optimized = eliminate_jmp_to_next(&lines);
        assert_eq!(optimized.len(), 4);
    }

    #[test]
    fn jmp_past_a_directive_stays() {
        let asm = "    JMP end_1\n.BYTE 1\nend_1:\n    RTS\n";
        let lines = parse_assembly(asm);
        let optimized = eliminate_jmp_to_next(&lines);
        assert_eq!(optimized.len(), 4);
    }
}

#[cfg(test)]
mod fold_literal_tests {
    use super::*;

    #[test]
    fn debug_fold() {
        let asm = "    LDA #$03\n    STA $20\n    LDA $40\n    CLC\n    ADC $20\n    STA $41\n    LDA #$01\n    STA $20\n";
        let lines = parse_assembly(asm);
        let out = fold_literal_operand(&lines);
        for l in &out {
            eprintln!("{}", l);
        }
        assert_eq!(out.len(), 6, "LDA $40; CLC; ADC #$03 + trailing");
    }
}
