//! Peephole optimizer for 6502 assembly code
//!
//! This module implements pattern-based peephole optimization to improve
//! the quality of generated assembly code by eliminating redundant instructions,
//! dead code, and other inefficiencies.

use std::fmt;

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

            // Label (ends with colon, no leading whitespace in original)
            if !line.starts_with(' ') && trimmed.ends_with(':') {
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

/// Apply peephole optimizations to parsed assembly
pub fn optimize(lines: &[Line]) -> Vec<Line> {
    let mut result = lines.to_vec();
    let mut changed = true;

    // Keep applying optimizations until no more changes
    while changed {
        changed = false;

        // Apply each optimization pass
        let before_len = result.len();
        result = eliminate_redundant_loads(&result);
        result = eliminate_redundant_stores(&result);
        result = eliminate_load_after_store(&result);
        result = eliminate_dead_stores(&result);
        result = eliminate_nop_operations(&result);
        result = eliminate_redundant_transfers(&result);
        result = eliminate_unreachable_after_terminator(&result);
        result = eliminate_redundant_cmp_zero(&result);
        result = eliminate_redundant_ldy_zero(&result);
        // DISABLED: eliminate_branch_over_jump breaks while loops with large bodies
        // by inverting branches that exceed the 127-byte limit
        // result = eliminate_branch_over_jump(&result);
        result = eliminate_redundant_ldx_zero(&result);
        result = eliminate_clc_adc_zero(&result);
        result = eliminate_sec_sbc_zero(&result);
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
fn eliminate_redundant_loads(lines: &[Line]) -> Vec<Line> {
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
            // Check for same load instruction with same operand
            if (m1 == "LDA" || m1 == "LDX" || m1 == "LDY") && m1 == m2 && op1 == op2 {
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
fn eliminate_redundant_stores(lines: &[Line]) -> Vec<Line> {
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
            // Check for same store instruction with same operand
            if (m1 == "STA" || m1 == "STX" || m1 == "STY") && m1 == m2 && op1 == op2 {
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
fn eliminate_load_after_store(lines: &[Line]) -> Vec<Line> {
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
            // STA $40; LDA $40 → STA $40 (A already contains the value)
            if m1 == "STA" && m2 == "LDA" && op1 == op2 {
                result.push(lines[i].clone());
                i += 2; // Skip the load
                continue;
            }
            // STX $40; LDX $40 → STX $40
            if m1 == "STX" && m2 == "LDX" && op1 == op2 {
                result.push(lines[i].clone());
                i += 2;
                continue;
            }
            // STY $40; LDY $40 → STY $40
            if m1 == "STY" && m2 == "LDY" && op1 == op2 {
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
fn eliminate_dead_stores(lines: &[Line]) -> Vec<Line> {
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
            // First store is dead because second store overwrites it
            if m1 == "STA" && m2 == "LDA" && m3 == "STA" && op1 == op3 {
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
fn eliminate_nop_operations(lines: &[Line]) -> Vec<Line> {
    lines
        .iter()
        .filter(|line| {
            if let Line::Instruction {
                mnemonic, operand, ..
            } = line
            {
                // ORA #$00 is a no-op
                if mnemonic == "ORA" && operand.as_deref() == Some("#$00") {
                    return false;
                }
                // AND #$FF is a no-op
                if mnemonic == "AND" && operand.as_deref() == Some("#$FF") {
                    return false;
                }
                // EOR #$00 is a no-op
                if mnemonic == "EOR" && operand.as_deref() == Some("#$00") {
                    return false;
                }
                // ADC #$00 with carry clear is a no-op (but we can't always know carry state)
                // CLC; ADC #$00 can be eliminated as a pair
            }
            true
        })
        .cloned()
        .collect()
}

/// Eliminate redundant register transfers: TAX; TXA → (nothing, unless A is modified between)
fn eliminate_redundant_transfers(lines: &[Line]) -> Vec<Line> {
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

/// Eliminate redundant CMP #$00 after LDA
///
/// LDA sets the Z flag based on the loaded value, so CMP #$00 is redundant
/// when we only care about the zero flag for BEQ/BNE.
fn eliminate_redundant_cmp_zero(lines: &[Line]) -> Vec<Line> {
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
        {
            // LDA followed by CMP #$00 - the CMP is redundant
            // LDA already sets Z flag if value is 0
            if m1 == "LDA" && m2 == "CMP" && op2.as_deref() == Some("#$00") {
                result.push(lines[i].clone());
                i += 2; // Skip the CMP
                continue;
            }
            // Also handle AND, ORA, EOR which set Z flag
            if (m1 == "AND" || m1 == "ORA" || m1 == "EOR")
                && m2 == "CMP"
                && op2.as_deref() == Some("#$00")
            {
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

/// Eliminate redundant LDY #$00 when Y is already known to be 0
///
/// Tracks Y register value through the instruction stream and removes
/// redundant loads of 0 into Y.
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
                }
                // Note: JSR/RTS don't necessarily change Y on 6502
                // but we reset at labels to be safe
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

/// Invert a branch condition
///
/// Returns the inverted branch mnemonic, or None if not a conditional branch.
fn invert_branch(mnemonic: &str) -> Option<&'static str> {
    match mnemonic {
        "BEQ" => Some("BNE"),
        "BNE" => Some("BEQ"),
        "BCS" => Some("BCC"),
        "BCC" => Some("BCS"),
        "BMI" => Some("BPL"),
        "BPL" => Some("BMI"),
        "BVS" => Some("BVC"),
        "BVC" => Some("BVS"),
        _ => None,
    }
}

/// Eliminate branch over jump by inverting the branch condition
///
/// Pattern:
///     BEQ skip_label
///     JMP target_label
/// skip_label:
///
/// Becomes:
///     BNE target_label
/// skip_label:
///
/// Saves 3 bytes (the JMP instruction).
fn eliminate_branch_over_jump(lines: &[Line]) -> Vec<Line> {
    let mut result = Vec::new();
    let mut i = 0;

    while i < lines.len() {
        // Check for pattern: Bxx skip; JMP target; skip:
        if i + 2 < lines.len()
            && let (
                Line::Instruction {
                    mnemonic: branch_m,
                    operand: Some(skip_label),
                    comment: branch_comment,
                },
                Line::Instruction {
                    mnemonic: jmp_m,
                    operand: Some(target_label),
                    ..
                },
                Line::Label(label),
            ) = (&lines[i], &lines[i + 1], &lines[i + 2])
        {
            // Check if this is a conditional branch followed by JMP, and the label matches
            if let Some(inverted) = invert_branch(branch_m)
                && jmp_m == "JMP"
                && skip_label == label
            {
                // Replace with inverted branch to target
                result.push(Line::Instruction {
                    mnemonic: inverted.to_string(),
                    operand: Some(target_label.clone()),
                    comment: branch_comment.clone(),
                });
                // Keep the label (might be used elsewhere)
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
        let optimized = eliminate_redundant_loads(&lines);
        assert_eq!(optimized.len(), 1);
    }

    #[test]
    fn test_load_after_store_elimination() {
        let asm = "    STA $40\n    LDA $40\n";
        let lines = parse_assembly(asm);
        let optimized = eliminate_load_after_store(&lines);
        assert_eq!(optimized.len(), 1);
    }

    #[test]
    fn test_dead_store_elimination() {
        let asm = "    STA $40\n    LDA #$05\n    STA $40\n";
        let lines = parse_assembly(asm);
        let optimized = eliminate_dead_stores(&lines);
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
        let asm = "    LDA $40\n    CMP #$00\n    BEQ label\n";
        let lines = parse_assembly(asm);
        let optimized = eliminate_redundant_cmp_zero(&lines);
        // CMP #$00 should be removed, LDA sets Z flag
        assert_eq!(optimized.len(), 2);
        assert!(matches!(&optimized[0], Line::Instruction { mnemonic, .. } if mnemonic == "LDA"));
        assert!(matches!(&optimized[1], Line::Instruction { mnemonic, .. } if mnemonic == "BEQ"));
    }

    #[test]
    fn test_redundant_cmp_zero_after_and() {
        let asm = "    AND #$0F\n    CMP #$00\n    BNE label\n";
        let lines = parse_assembly(asm);
        let optimized = eliminate_redundant_cmp_zero(&lines);
        // CMP #$00 should be removed, AND sets Z flag
        assert_eq!(optimized.len(), 2);
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

    // Branch inversion tests

    #[test]
    fn test_branch_inversion_beq_jmp() {
        let asm = "    BEQ skip\n    JMP target\nskip:\n    LDA #$00\n";
        let lines = parse_assembly(asm);
        let optimized = eliminate_branch_over_jump(&lines);
        // BEQ skip; JMP target; skip: → BNE target; skip:
        assert_eq!(optimized.len(), 3);
        assert!(
            matches!(&optimized[0], Line::Instruction { mnemonic, operand, .. }
            if mnemonic == "BNE" && operand.as_deref() == Some("target"))
        );
        assert!(matches!(&optimized[1], Line::Label(l) if l == "skip"));
    }

    #[test]
    fn test_branch_inversion_bne_jmp() {
        let asm = "    BNE skip\n    JMP target\nskip:\n";
        let lines = parse_assembly(asm);
        let optimized = eliminate_branch_over_jump(&lines);
        // BNE skip; JMP target; skip: → BEQ target; skip:
        assert_eq!(optimized.len(), 2);
        assert!(matches!(&optimized[0], Line::Instruction { mnemonic, .. } if mnemonic == "BEQ"));
    }

    #[test]
    fn test_branch_inversion_bcs_jmp() {
        let asm = "    BCS skip\n    JMP target\nskip:\n";
        let lines = parse_assembly(asm);
        let optimized = eliminate_branch_over_jump(&lines);
        assert!(matches!(&optimized[0], Line::Instruction { mnemonic, .. } if mnemonic == "BCC"));
    }

    #[test]
    fn test_branch_inversion_preserves_nonmatching() {
        // Label doesn't match branch target - should not optimize
        let asm = "    BEQ other\n    JMP target\nskip:\n";
        let lines = parse_assembly(asm);
        let optimized = eliminate_branch_over_jump(&lines);
        // Should keep all 3 lines unchanged
        assert_eq!(optimized.len(), 3);
        assert!(matches!(&optimized[0], Line::Instruction { mnemonic, .. } if mnemonic == "BEQ"));
        assert!(matches!(&optimized[1], Line::Instruction { mnemonic, .. } if mnemonic == "JMP"));
    }

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
