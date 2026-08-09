//! `match` statement codegen: strategy selection, sequential and jump-table
//! dispatch, and enum-payload binding extraction.

use super::*;

/// Strategy for generating match statement code
#[derive(Debug)]
enum MatchStrategy {
    /// Use sequential CMP/BEQ comparisons (for small matches)
    Sequential,
    /// Use jump table for efficient dispatch (for enum matches with 3+ arms)
    JumpTable {
        /// Maximum tag value in the enum
        max_tag: u8,
        /// Index of the wildcard arm, if any
        wildcard_arm_index: Option<usize>,
    },
}

/// Determine the best strategy for generating a match statement
///
/// Uses jump tables for enum matches with 3+ arms to avoid BEQ branch distance limits.
/// Sequential comparisons are used for small matches (1-2 arms) or non-enum matches.
fn determine_match_strategy(arms: &[crate::ast::MatchArm], info: &ProgramInfo) -> MatchStrategy {
    use crate::ast::Pattern;

    // Collect enum variant tags from the patterns
    let mut enum_tags: Vec<u8> = Vec::new();
    let mut wildcard_arm_index: Option<usize> = None;
    // The widest tag range of any enum named in the patterns. The dispatch
    // table is indexed by the *runtime* tag, so it must span the enum's whole
    // range, not just the tags that happen to have arms: a variant without an
    // arm whose tag exceeds every armed tag would otherwise read past the end
    // of the table into whatever bytes follow it.
    let mut enum_def_max_tag: u8 = 0;

    for (i, arm) in arms.iter().enumerate() {
        match &arm.pattern.node {
            Pattern::EnumVariant {
                enum_name, variant, ..
            } => {
                // Look up the enum definition and get the tag
                if let Some(enum_def) = info.type_registry.get_enum(&enum_name.node)
                    && let Some(variant_info) = enum_def.get_variant(&variant.node)
                {
                    enum_tags.push(variant_info.tag);
                    if let Some(def_max) = enum_def.variants.iter().map(|v| v.tag).max() {
                        enum_def_max_tag = enum_def_max_tag.max(def_max);
                    }
                }
            }
            Pattern::Wildcard | Pattern::Variable(_) => {
                wildcard_arm_index = Some(i);
            }
            _ => {}
        }
    }

    // Use jump table for enum matches with 3+ arms
    // This avoids the BEQ branch distance limitation for large match bodies
    if !enum_tags.is_empty() && arms.len() >= 3 {
        let max_tag = enum_def_max_tag;

        // Dispatch doubles the tag into an 8-bit index (`ASL; TAX`), so a tag
        // above 127 wraps and selects the wrong entry. The table is also dense,
        // one word per tag from 0 upward, so sparse explicit discriminants
        // would spend hundreds of ROM bytes to describe a handful of arms.
        // Either way the sequential comparison chain is the better answer;
        // enums with explicit values are usually few-armed anyway.
        let index_would_overflow = max_tag > 127;
        let table_entries = max_tag as usize + 1;
        let too_sparse = table_entries > 4 * enum_tags.len().max(1);

        if index_would_overflow || too_sparse {
            return MatchStrategy::Sequential;
        }

        MatchStrategy::JumpTable {
            max_tag,
            wildcard_arm_index,
        }
    } else {
        MatchStrategy::Sequential
    }
}

pub(super) fn generate_match(
    expr: &Spanned<crate::ast::Expr>,
    arms: &[crate::ast::MatchArm],
    emitter: &mut Emitter,
    info: &ProgramInfo,
    string_collector: &mut StringCollector,
) -> Result<(), CodegenError> {
    // Determine the best strategy for this match
    let strategy = determine_match_strategy(arms, info);

    match strategy {
        MatchStrategy::Sequential => {
            generate_match_sequential(expr, arms, emitter, info, string_collector)
        }
        MatchStrategy::JumpTable {
            max_tag,
            wildcard_arm_index,
        } => generate_match_jump_table(
            expr,
            arms,
            emitter,
            info,
            string_collector,
            max_tag,
            wildcard_arm_index,
        ),
    }
}

/// A conditional branch to an arm body. The compare section and the bodies
/// are emitted as two blocks, so the target can sit arbitrarily far away —
/// past every other arm's body — and a plain conditional branch overflows its
/// ±127-byte range once a match has enough (or large enough) arms. Emit the
/// inverse branch over a JMP instead: the hop is always 3 bytes.
pub(super) fn emit_far_arm_branch(
    emitter: &mut Emitter,
    cond: &str,
    arm_label: &str,
    skip_label: &str,
) {
    let inv = match cond {
        "BEQ" => "BNE",
        "BNE" => "BEQ",
        "BCC" => "BCS",
        "BCS" => "BCC",
        "BMI" => "BPL",
        "BPL" => "BMI",
        "BVC" => "BVS",
        "BVS" => "BVC",
        other => unreachable!("not an invertible branch: {}", other),
    };
    emitter.emit_inst(inv, skip_label);
    emitter.emit_inst("JMP", arm_label);
    emitter.emit_label(skip_label);
}

/// Generate match statement using sequential CMP/BEQ comparisons
///
/// Used for small matches (1-2 arms) or non-enum patterns.
/// Arm-ward branches go through emit_far_arm_branch so arm bodies may be any
/// size and any number.
fn generate_match_sequential(
    expr: &Spanned<crate::ast::Expr>,
    arms: &[crate::ast::MatchArm],
    emitter: &mut Emitter,
    info: &ProgramInfo,
    string_collector: &mut StringCollector,
) -> Result<(), CodegenError> {
    use crate::ast::Pattern;
    use crate::sema::table::SymbolLocation;
    use crate::sema::types::Type;

    let match_id = emitter.next_match_id();

    emitter.emit_comment("Match statement (sequential)");

    // Check if we're matching on an enum by looking at the first pattern
    let is_enum_match = arms
        .iter()
        .any(|arm| matches!(arm.pattern.node, Pattern::EnumVariant { .. }));

    // Evaluate the matched expression into accumulator
    generate_expr(expr, emitter, info, string_collector)?;

    // Whether the matched value is 16-bit (low in A/$20, high in Y/$21).
    let scrutinee_is_u16 = matches!(
        info.resolved_types.get(&expr.span),
        Some(
            Type::Primitive(crate::ast::PrimitiveType::U16)
                | Type::Primitive(crate::ast::PrimitiveType::I16)
                | Type::Primitive(crate::ast::PrimitiveType::B16)
        )
    );

    // Whether the matched value is signed (i8/i16) — range patterns must then
    // use signed comparisons instead of unsigned BCC/BCS.
    let scrutinee_is_signed = info
        .resolved_types
        .get(&expr.span)
        .is_some_and(|t| t.is_signed());

    // Use pointer ops area for indirect addressing to avoid conflict with temp storage
    let ptr_base = emitter.memory_layout.pointer_ops_start; // $30 by default

    if is_enum_match {
        // For enum matching, expression returns a pointer in A:X
        // Store pointer at pointer ops area (not $20 which is used by temp storage)
        emitter.emit_inst("STA", &format!("${:02X}", ptr_base));
        emitter.emit_inst("STX", &format!("${:02X}", ptr_base + 1));

        // Load the discriminant tag from the enum (first byte)
        emitter.emit_inst("LDY", "#$00");
        emitter.emit_inst("LDA", &format!("(${:02X}),Y", ptr_base));
        emitter.emit_inst("STA", &format!("${:02X}", ptr_base + 2)); // Store tag
    } else {
        // For simple value matching, store the low byte at $20 (and, for u16,
        // the high byte at $21 so literal patterns and variable bindings see
        // the full value rather than a truncated low byte).
        emitter.emit_inst("STA", "$20");
        if scrutinee_is_u16 {
            emitter.emit_inst("STY", "$21");
        }
    }

    // Generate code for each arm
    let mut has_wildcard = false;
    for (i, arm) in arms.iter().enumerate() {
        match &arm.pattern.node {
            Pattern::Literal(lit_expr) => {
                // Compare with literal value
                if let crate::ast::Expr::Literal(crate::ast::Literal::Integer(val)) = &lit_expr.node
                {
                    let arm_label = format!("match_{}_arm_{}", match_id, i);
                    if scrutinee_is_u16 {
                        // Compare both bytes; only branch to the arm if both match.
                        let skip = format!("match_{}_arm_{}_skip", match_id, i);
                        emitter.emit_inst("LDA", "$20");
                        emitter.emit_inst("CMP", &format!("#${:02X}", (*val as u16) & 0xFF));
                        emitter.emit_inst("BNE", &skip);
                        emitter.emit_inst("LDA", "$21");
                        emitter.emit_inst("CMP", &format!("#${:02X}", ((*val as u16) >> 8) & 0xFF));
                        emit_far_arm_branch(
                            emitter,
                            "BEQ",
                            &arm_label,
                            &format!("match_{}_arm_{}_hifar", match_id, i),
                        );
                        emitter.emit_label(&skip);
                    } else {
                        emitter.emit_inst("LDA", "$20");
                        emitter.emit_inst("CMP", &format!("#${:02X}", (*val as u16) & 0xFF));
                        emit_far_arm_branch(
                            emitter,
                            "BEQ",
                            &arm_label,
                            &format!("match_{}_arm_{}_far", match_id, i),
                        );
                    }
                }
            }
            Pattern::Range {
                start,
                end,
                inclusive,
            } => {
                // Range check: value >= start && value <= end (or < end+1 for inclusive)
                if let (
                    crate::ast::Expr::Literal(crate::ast::Literal::Integer(start_val)),
                    crate::ast::Expr::Literal(crate::ast::Literal::Integer(end_val)),
                ) = (&start.node, &end.node)
                {
                    let arm_label = format!("match_{}_arm_{}", match_id, i);
                    let skip_label = format!("match_{}_arm_{}_end", match_id, i);
                    let upper_bound = if *inclusive { end_val + 1 } else { *end_val };

                    if scrutinee_is_signed {
                        // Signed range: (value < start) skips; (value < end+1)
                        // matches. Signed "less than" is (N eor V) after a
                        // subtraction, folded into N via EOR #$80 on overflow.
                        // Comparing against 0 is just a sign-bit test (BMI) — and
                        // it avoids emitting `SEC; SBC #$00`, which the peephole
                        // eliminates as a value no-op even though its flags matter.
                        let emit_signed_lt =
                            |emitter: &mut Emitter,
                             bound: i64,
                             target: &str,
                             tag: &str,
                             far: bool| {
                                let nov = format!("match_{}_arm_{}_{}", match_id, i, tag);
                                crate::codegen::expr::compare::emit_signed_lt_flag(
                                    emitter, bound, &nov,
                                );
                                if far {
                                    emit_far_arm_branch(
                                        emitter,
                                        "BMI",
                                        target,
                                        &format!("match_{}_arm_{}_{}far", match_id, i, tag),
                                    );
                                } else {
                                    emitter.emit_inst("BMI", target);
                                }
                            };
                        emit_signed_lt(emitter, *start_val, &skip_label, "v1", false); // < start -> skip
                        emit_signed_lt(emitter, upper_bound, &arm_label, "v2", true); // <= end -> match
                    } else {
                        emitter.emit_inst("LDA", "$20");
                        // Check if value < start, skip this arm
                        emitter.emit_inst("CMP", &format!("#${:02X}", start_val));
                        emitter.emit_inst("BCC", &skip_label);
                        // Check if value <= end (or < end+1)
                        emitter.emit_inst("CMP", &format!("#${:02X}", upper_bound));
                        emit_far_arm_branch(
                            emitter,
                            "BCC",
                            &arm_label,
                            &format!("match_{}_arm_{}_far", match_id, i),
                        );
                    }

                    emitter.emit_label(&skip_label);
                }
            }
            Pattern::Wildcard => {
                // Wildcard catches everything - no comparison needed
                has_wildcard = true;
                emitter.emit_inst("JMP", &format!("match_{}_arm_{}", match_id, i));
            }
            Pattern::Variable(name) => {
                // Variable pattern binds the whole matched value - a catch-all
                // like wildcard, but the scrutinee must be copied into the
                // binding's storage so the arm body can read it.
                has_wildcard = true;
                // Sema records the binding under the arm's pattern span (the
                // arm-body scope is gone by codegen, so a name lookup fails).
                let loc = info
                    .resolved_symbols
                    .get(&arm.pattern.span)
                    .map(|sym| sym.location.clone())
                    .ok_or_else(|| CodegenError::SymbolNotFound(name.clone()))?;
                match loc {
                    SymbolLocation::ZeroPage(addr) => {
                        emitter.emit_inst("LDA", "$20");
                        emitter.emit_inst("STA", &format!("${:02X}", addr));
                        if scrutinee_is_u16 {
                            emitter.emit_inst("LDA", "$21");
                            emitter.emit_inst("STA", &format!("${:02X}", addr + 1));
                        }
                    }
                    SymbolLocation::Absolute(addr) => {
                        emitter.emit_inst("LDA", "$20");
                        emitter.emit_inst("STA", &format!("${:04X}", addr));
                        if scrutinee_is_u16 {
                            emitter.emit_inst("LDA", "$21");
                            emitter.emit_inst("STA", &format!("${:04X}", addr + 1));
                        }
                    }
                    _ => {
                        return Err(CodegenError::UnsupportedOperation(format!(
                            "match binding '{}' has unsupported storage location",
                            name
                        )));
                    }
                }
                emitter.emit_inst("JMP", &format!("match_{}_arm_{}", match_id, i));
            }
            Pattern::EnumVariant {
                enum_name,
                variant,
                bindings,
            } => {
                // Look up the enum definition
                let enum_def = info
                    .type_registry
                    .get_enum(&enum_name.node)
                    .ok_or_else(|| {
                        CodegenError::UnsupportedOperation(format!(
                            "enum '{}' not found in type registry",
                            enum_name.node
                        ))
                    })?;

                // Find the variant
                let variant_info = enum_def.get_variant(&variant.node).ok_or_else(|| {
                    CodegenError::UnsupportedOperation(format!(
                        "variant '{}' not found in enum '{}'",
                        variant.node, enum_name.node
                    ))
                })?;

                // Compare the tag with the expected variant tag
                emitter.emit_inst("LDA", &format!("${:02X}", ptr_base + 2)); // Load stored tag
                emitter.emit_inst("CMP", &format!("#${:02X}", variant_info.tag));
                emit_far_arm_branch(
                    emitter,
                    "BEQ",
                    &format!("match_{}_arm_{}", match_id, i),
                    &format!("match_{}_arm_{}_far", match_id, i),
                );

                // If bindings are present, we'll extract them in the arm body
                // For now, we just check the tag - bindings will be handled later
                if !bindings.is_empty() {
                    emitter.emit_comment(&format!("Variant has {} binding(s)", bindings.len()));
                }
            }
        }
    }

    // If no pattern matched and no wildcard, this is an error (should be caught in semantic analysis)
    if !has_wildcard {
        emitter.emit_comment("No pattern matched - should not reach here");
    }

    // Generate arm bodies
    generate_match_arm_bodies(arms, emitter, info, string_collector, match_id)?;

    emitter.emit_label(&format!("match_{}_end", match_id));

    Ok(())
}

/// Generate match statement using jump table dispatch
///
/// Used for enum matches with 3+ arms to avoid BEQ branch distance limitations.
/// The jump table allows arm bodies to be arbitrarily large.
fn generate_match_jump_table(
    expr: &Spanned<crate::ast::Expr>,
    arms: &[crate::ast::MatchArm],
    emitter: &mut Emitter,
    info: &ProgramInfo,
    string_collector: &mut StringCollector,
    max_tag: u8,
    wildcard_arm_index: Option<usize>,
) -> Result<(), CodegenError> {
    let match_id = emitter.next_match_id();
    let jump_ptr = emitter.memory_layout.jump_ptr();

    emitter.emit_comment("Match statement (jump table)");

    // Evaluate the matched expression into accumulator
    // For enum matching, expression returns a pointer in A:X
    generate_expr(expr, emitter, info, string_collector)?;

    // Use pointer ops area for indirect addressing
    let ptr_base = emitter.memory_layout.pointer_ops_start;

    // Store pointer at pointer ops area (not $20 which conflicts with temp storage)
    emitter.emit_inst("STA", &format!("${:02X}", ptr_base));
    emitter.emit_inst("STX", &format!("${:02X}", ptr_base + 1));

    // Load the discriminant tag from the enum (first byte)
    emitter.emit_inst("LDY", "#$00");
    emitter.emit_inst("LDA", &format!("(${:02X}),Y", ptr_base));
    emitter.emit_inst("STA", &format!("${:02X}", ptr_base + 2)); // Store tag for binding extraction

    // Jump table dispatch:
    // 1. Double the tag (addresses are 2 bytes)
    // 2. Use as index into jump table
    // 3. Load address and JMP indirect
    emitter.emit_inst("ASL", ""); // tag * 2
    emitter.emit_inst("TAX", ""); // Transfer to X for indexing
    if emitter.target.is_cmos() {
        // The 65C02 reads the target straight from the table with absolute
        // indexed-indirect addressing — no zero-page vector, four fewer
        // instructions, and no scratch pair to collide with anything.
        emitter.emit_inst("JMP", &format!("(match_{}_jt,X)", match_id));
    } else {
        emitter.emit_inst("LDA", &format!("match_{}_jt,X", match_id));
        emitter.emit_inst("STA", &format!("${:02X}", jump_ptr));
        emitter.emit_inst("LDA", &format!("match_{}_jt+1,X", match_id));
        emitter.emit_inst("STA", &format!("${:02X}", jump_ptr + 1));
        emitter.emit_inst("JMP", &format!("(${:02X})", jump_ptr));
    }

    // Emit jump table
    emit_jump_table(emitter, arms, info, match_id, max_tag, wildcard_arm_index)?;

    // Generate arm bodies
    generate_match_arm_bodies(arms, emitter, info, string_collector, match_id)?;

    // Panic handler for non-exhaustive matches (if no wildcard)
    if wildcard_arm_index.is_none() {
        emitter.emit_label(&format!("match_{}_panic", match_id));
        emitter.emit_comment("Unreachable - non-exhaustive match");
        emitter.emit_inst("BRK", "");
    }

    emitter.emit_label(&format!("match_{}_end", match_id));

    Ok(())
}

/// Emit the jump table for a match statement
///
/// The table contains .WORD entries for each tag value from 0 to max_tag.
/// Missing tags are filled with the wildcard arm label (or panic label if no wildcard).
fn emit_jump_table(
    emitter: &mut Emitter,
    arms: &[crate::ast::MatchArm],
    info: &ProgramInfo,
    match_id: u32,
    max_tag: u8,
    wildcard_arm_index: Option<usize>,
) -> Result<(), CodegenError> {
    use crate::ast::Pattern;

    // Build mapping from tag -> arm index
    let mut tag_to_arm: Vec<Option<usize>> = vec![None; (max_tag + 1) as usize];

    for (arm_index, arm) in arms.iter().enumerate() {
        if let Pattern::EnumVariant {
            enum_name, variant, ..
        } = &arm.pattern.node
            && let Some(enum_def) = info.type_registry.get_enum(&enum_name.node)
            && let Some(variant_info) = enum_def.get_variant(&variant.node)
            && (variant_info.tag as usize) < tag_to_arm.len()
        {
            tag_to_arm[variant_info.tag as usize] = Some(arm_index);
        }
    }

    // Emit jump table label
    emitter.emit_label(&format!("match_{}_jt", match_id));

    // Emit .WORD entries for each tag
    for tag in 0..=max_tag {
        let arm_label = if let Some(arm_index) = tag_to_arm[tag as usize] {
            format!("match_{}_arm_{}", match_id, arm_index)
        } else if let Some(wildcard_index) = wildcard_arm_index {
            format!("match_{}_arm_{}", match_id, wildcard_index)
        } else {
            format!("match_{}_panic", match_id)
        };
        emitter.emit_word_label(&arm_label);
    }

    Ok(())
}

/// Generate arm bodies for a match statement
///
/// Shared between sequential and jump table strategies.
/// Copy an enum variant's payload fields into their pattern bindings' storage.
///
/// `ptr_base`/`ptr_base+1` hold the enum pointer (low/high); field data begins
/// one byte past the tag. Every byte of each field is copied so multi-byte
/// (u16/i16/b16) payloads keep their high byte. Shared by the match-statement
/// and match-expression code paths so both extract bindings identically.
pub(crate) fn extract_enum_bindings(
    enum_name: &Spanned<String>,
    variant: &Spanned<String>,
    bindings: &[crate::ast::PatternBinding],
    ptr_base: u8,
    emitter: &mut Emitter,
    info: &ProgramInfo,
) -> Result<(), CodegenError> {
    if bindings.is_empty() {
        return Ok(());
    }

    // Look up the enum definition to get field information.
    let enum_def = info
        .type_registry
        .get_enum(&enum_name.node)
        .ok_or_else(|| {
            CodegenError::UnsupportedOperation(format!(
                "enum '{}' not found in type registry",
                enum_name.node
            ))
        })?;

    let variant_info = enum_def.get_variant(&variant.node).ok_or_else(|| {
        CodegenError::UnsupportedOperation(format!(
            "variant '{}' not found in enum '{}'",
            variant.node, enum_name.node
        ))
    })?;

    // Copy `field_size` bytes from enum data at `offset` into the binding's slot.
    let copy_field = |offset: u8,
                      field_size: u8,
                      binding: &crate::ast::PatternBinding,
                      emitter: &mut Emitter|
     -> Result<(), CodegenError> {
        let loc = info
            .resolved_symbols
            .get(&binding.name.span)
            .map(|sym| sym.location.clone())
            .ok_or_else(|| CodegenError::SymbolNotFound(binding.name.node.clone()))?;
        for byte in 0..field_size {
            emitter.emit_inst("LDY", &format!("#${:02X}", offset + byte));
            emitter.emit_inst("LDA", &format!("(${:02X}),Y", ptr_base));
            match loc {
                crate::sema::table::SymbolLocation::ZeroPage(addr) => {
                    emitter.emit_sta_zp(addr + byte);
                }
                crate::sema::table::SymbolLocation::Absolute(addr) => {
                    emitter.emit_sta_abs(addr + byte as u16);
                }
                _ => {
                    return Err(CodegenError::UnsupportedOperation(format!(
                        "Binding '{}' has unsupported location",
                        binding.name.node
                    )));
                }
            }
        }
        Ok(())
    };

    match &variant_info.data {
        crate::sema::type_defs::VariantData::Tuple(field_types) => {
            // Tuple variant: extract each field by position.
            if bindings.len() != field_types.len() {
                return Err(CodegenError::UnsupportedOperation(format!(
                    "Pattern binding count mismatch: expected {}, got {}",
                    field_types.len(),
                    bindings.len()
                )));
            }
            let mut offset = 1u8; // Start after the tag byte
            for (binding, field_type) in bindings.iter().zip(field_types.iter()) {
                let field_size = field_type.size().max(1) as u8;
                copy_field(offset, field_size, binding, emitter)?;
                offset += field_size;
            }
        }
        crate::sema::type_defs::VariantData::Struct(struct_fields) => {
            // Struct variant: each binding name selects a field; offsets are
            // relative to the variant data, one byte past the tag.
            for binding in bindings.iter() {
                let field = struct_fields
                    .iter()
                    .find(|f| f.name == binding.name.node)
                    .ok_or_else(|| {
                        CodegenError::UnsupportedOperation(format!(
                            "field '{}' not found in struct variant",
                            binding.name.node
                        ))
                    })?;
                let field_size = field.ty.size().max(1) as u8;
                let base_offset = 1 + field.offset as u8; // skip tag
                copy_field(base_offset, field_size, binding, emitter)?;
            }
        }
        crate::sema::type_defs::VariantData::Unit => {
            // Unit variant shouldn't have bindings.
            return Err(CodegenError::UnsupportedOperation(
                "Unit variant should not have bindings".to_string(),
            ));
        }
    }

    Ok(())
}

fn generate_match_arm_bodies(
    arms: &[crate::ast::MatchArm],
    emitter: &mut Emitter,
    info: &ProgramInfo,
    string_collector: &mut StringCollector,
    match_id: u32,
) -> Result<(), CodegenError> {
    use crate::ast::Pattern;

    for (i, arm) in arms.iter().enumerate() {
        emitter.emit_label(&format!("match_{}_arm_{}", match_id, i));

        // Extract bindings for enum variant patterns
        if let Pattern::EnumVariant {
            enum_name,
            variant,
            bindings,
        } = &arm.pattern.node
        {
            let ptr_base = emitter.memory_layout.pointer_ops_start;
            extract_enum_bindings(enum_name, variant, bindings, ptr_base, emitter, info)?;
        }

        generate_stmt(&arm.body, emitter, info, string_collector)?;

        // Only emit JMP if the arm body doesn't already terminate control flow
        // (e.g., return, break, continue) - this eliminates dead code
        if !stmt_terminates(&arm.body.node) {
            emitter.emit_inst("JMP", &format!("match_{}_end", match_id));
        }
    }

    Ok(())
}
