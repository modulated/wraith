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
    Directive {
        name: String,
        args: String,
    },
    /// Empty line
    Empty,
}

impl fmt::Display for Line {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Line::Instruction { mnemonic, operand, comment } => {
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
        if i + 1 < lines.len() {
            if let (
                Line::Instruction { mnemonic: m1, operand: op1, .. },
                Line::Instruction { mnemonic: m2, operand: op2, .. },
            ) = (&lines[i], &lines[i + 1])
            {
                // Check for same load instruction with same operand
                if (m1 == "LDA" || m1 == "LDX" || m1 == "LDY")
                    && m1 == m2
                    && op1 == op2
                {
                    // Keep only the first load
                    result.push(lines[i].clone());
                    i += 2; // Skip the redundant load
                    continue;
                }
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
        if i + 1 < lines.len() {
            if let (
                Line::Instruction { mnemonic: m1, operand: op1, .. },
                Line::Instruction { mnemonic: m2, operand: op2, .. },
            ) = (&lines[i], &lines[i + 1])
            {
                // Check for same store instruction with same operand
                if (m1 == "STA" || m1 == "STX" || m1 == "STY")
                    && m1 == m2
                    && op1 == op2
                {
                    // Keep only the first store
                    result.push(lines[i].clone());
                    i += 2; // Skip the redundant store
                    continue;
                }
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
        if i + 1 < lines.len() {
            if let (
                Line::Instruction { mnemonic: m1, operand: op1, .. },
                Line::Instruction { mnemonic: m2, operand: op2, .. },
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
        if i + 2 < lines.len() {
            if let (
                Line::Instruction { mnemonic: m1, operand: op1, .. },
                Line::Instruction { mnemonic: m2, .. },
                Line::Instruction { mnemonic: m3, operand: op3, .. },
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
            if let Line::Instruction { mnemonic, operand, .. } = line {
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
        if i + 1 < lines.len() {
            if let (
                Line::Instruction { mnemonic: m1, operand: None, .. },
                Line::Instruction { mnemonic: m2, operand: None, .. },
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
        }

        result.push(lines[i].clone());
        i += 1;
    }

    result
}

/// Convert optimized lines back to assembly string
pub fn lines_to_string(lines: &[Line]) -> String {
    lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n")
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
}
