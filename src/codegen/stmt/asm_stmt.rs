//! Inline-assembly statement codegen: variable substitution and per-expansion
//! label uniquification.

use super::*;

/// Substitute {variable} patterns in inline assembly with actual addresses
pub(super) fn substitute_asm_vars(
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
pub(super) fn uniquify_asm_labels(line: &str, suffix: usize) -> String {
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
