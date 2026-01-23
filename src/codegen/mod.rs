pub mod address_allocator;
pub mod comment_utils;
pub mod emitter;
pub mod expr;
pub mod item;
pub mod memory_layout;
pub mod peephole;
pub mod regstate;
pub mod section_allocator;
pub mod stmt;

use crate::ast::SourceFile;
use crate::sema::ProgramInfo;
use emitter::Emitter;
use item::generate_item;
use section_allocator::SectionAllocator;
use std::collections::HashMap;

/// Controls the verbosity level of generated assembly comments
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CommentVerbosity {
    /// Minimal comments - only function headers and critical info
    Minimal,
    /// Normal comments - function headers, operation types, basic context
    #[default]
    Normal,
    /// Verbose - full register state, detailed explanations, memory layout
    Verbose,
}

#[derive(Debug, Clone)]
pub enum CodegenError {
    Unknown,
    UnsupportedOperation(String),
    SymbolNotFound(String),
    SectionError(String),
    AddressConflict(String),
}

impl std::fmt::Display for CodegenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CodegenError::Unknown => write!(f, "unknown error"),
            CodegenError::UnsupportedOperation(msg) => write!(f, "unsupported operation: {}", msg),
            CodegenError::SymbolNotFound(name) => write!(f, "undefined symbol '{}'", name),
            CodegenError::SectionError(msg) => write!(f, "section error: {}", msg),
            CodegenError::AddressConflict(msg) => write!(f, "{}", msg),
        }
    }
}

impl std::error::Error for CodegenError {}

/// Collects and manages string literals for emission to DATA section
pub struct StringCollector {
    strings: HashMap<String, String>, // content -> label
    next_id: usize,
}

impl Default for StringCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl StringCollector {
    pub fn new() -> Self {
        Self {
            strings: HashMap::new(),
            next_id: 0,
        }
    }

    /// Register a string and get its label (deduplicated automatically)
    pub fn add_string(&mut self, content: String) -> String {
        if let Some(label) = self.strings.get(&content) {
            // Deduplication: return existing label
            label.clone()
        } else {
            let label = format!("str_{}", self.next_id);
            self.next_id += 1;
            self.strings.insert(content, label.clone());
            label
        }
    }

    /// Emit all collected strings to DATA section
    pub fn emit_strings(
        &self,
        emitter: &mut Emitter,
        section_alloc: &mut SectionAllocator,
    ) -> Result<(), CodegenError> {
        if self.strings.is_empty() {
            return Ok(());
        }

        emitter.emit_comment("============================");
        emitter.emit_comment("String Literal Data");
        emitter.emit_comment("============================");

        for (content, label) in &self.strings {
            // Allocate in DATA section
            let data_size = 2 + content.len() as u16; // length prefix + bytes
            let addr = section_alloc
                .allocate("DATA", data_size)
                .map_err(CodegenError::SectionError)?;

            emitter.emit_org(addr);
            emitter.emit_label(label);

            // Emit length as u16 little-endian
            let len = content.len() as u16;
            emitter.emit_raw(&format!(
                "    .BYTE ${:02X}, ${:02X}  ; length = {}",
                len & 0xFF,
                (len >> 8) & 0xFF,
                len
            ));

            // Emit string bytes
            if !content.is_empty() {
                // Escape special characters for display in comment
                let display = content
                    .chars()
                    .map(|c| match c {
                        '\n' => "\\n".to_string(),
                        '\r' => "\\r".to_string(),
                        '\t' => "\\t".to_string(),
                        '\0' => "\\0".to_string(),
                        '\\' => "\\\\".to_string(),
                        '"' => "\\\"".to_string(),
                        c if c.is_ascii_graphic() || c == ' ' => c.to_string(),
                        c => format!("\\x{:02X}", c as u8),
                    })
                    .collect::<String>();
                emitter.emit_comment(&format!("\"{}\"", display));

                // Emit bytes in groups of 16 for readability
                for (i, chunk) in content.as_bytes().chunks(16).enumerate() {
                    let bytes_str = chunk
                        .iter()
                        .map(|b| format!("${:02X}", b))
                        .collect::<Vec<_>>()
                        .join(", ");

                    if i == 0 && chunk.len() < content.len() {
                        emitter.emit_raw(&format!(
                            "    .BYTE {}  ; bytes 0-{}",
                            bytes_str,
                            chunk.len() - 1
                        ));
                    } else if chunk.len() < 16 {
                        let start = i * 16;
                        emitter.emit_raw(&format!(
                            "    .BYTE {}  ; bytes {}-{}",
                            bytes_str,
                            start,
                            start + chunk.len() - 1
                        ));
                    } else {
                        emitter.emit_raw(&format!("    .BYTE {}", bytes_str));
                    }
                }
            }
        }

        Ok(())
    }
}

/// Emit stdlib math functions (mul16, div16) if they were used
fn emit_stdlib_math_functions(
    emitter: &mut Emitter,
    section_alloc: &mut SectionAllocator,
) -> Result<(), CodegenError> {
    if !emitter.needs_mul16 && !emitter.needs_div16 && !emitter.needs_mod16 {
        return Ok(()); // Nothing to emit
    }

    emitter.emit_comment("============================================================");
    emitter.emit_comment("Standard Library Math Functions");
    emitter.emit_comment("Automatically included for u16 multiplication, division, modulo");
    emitter.emit_comment("============================================================");

    if emitter.needs_mul16 {
        let org_addr = section_alloc
            .allocate("CODE", 74)
            .map_err(CodegenError::SectionError)?;
        emitter.emit_org(org_addr);
        emitter.emit_comment("Function: mul16");
        emitter.emit_comment("  Params: a: u16 in $80-$81, b: u16 in $82-$83");
        emitter.emit_comment("  Returns: u16 in A/Y (low/high)");
        emitter.emit_comment(&format!("  Location: ${:04X}", org_addr));
        emitter.emit_label("mul16");

        // Emit mul16 implementation
        // Memory layout: $D0-$D1 multiplicand, $D2-$D3 result,
        //               $D4-$D5 multiplier, $D6 loop counter
        emitter.emit_raw("    LDA #$00");
        emitter.emit_raw("    STA $D2"); // result_low at $D2
        emitter.emit_raw("    STA $D3"); // result_high at $D3
        emitter.emit_raw("    LDA $80");
        emitter.emit_raw("    STA $D0"); // param_a_low at $D0
        emitter.emit_raw("    LDA $81");
        emitter.emit_raw("    STA $D1"); // param_a_high at $D1
        emitter.emit_raw("    LDA $82");
        emitter.emit_raw("    STA $D4"); // param_b_low at $D4
        emitter.emit_raw("    LDA $83");
        emitter.emit_raw("    STA $D5"); // param_b_high at $D5
        emitter.emit_raw("    LDX #$10");
        emitter.emit_raw("    STX $D6"); // loop_counter at $D6
        emitter.emit_raw("    mul16_loop:");
        emitter.emit_raw("    LDA $D4");
        emitter.emit_raw("    LSR A");
        emitter.emit_raw("    BCC mul16_skip_add");
        emitter.emit_raw("    CLC");
        emitter.emit_raw("    LDA $D2");
        emitter.emit_raw("    ADC $D0");
        emitter.emit_raw("    STA $D2");
        emitter.emit_raw("    LDA $D3");
        emitter.emit_raw("    ADC $D1");
        emitter.emit_raw("    STA $D3");
        emitter.emit_raw("    mul16_skip_add:");
        emitter.emit_raw("    LSR $D5");
        emitter.emit_raw("    ROR $D4");
        emitter.emit_raw("    ASL $D0");
        emitter.emit_raw("    ROL $D1");
        emitter.emit_raw("    DEC $D6");
        emitter.emit_raw("    BNE mul16_loop");
        emitter.emit_raw("    LDA $D2");
        emitter.emit_raw("    LDY $D3");
        emitter.emit_raw("    RTS");
    }

    if emitter.needs_div16 {
        let org_addr = section_alloc
            .allocate("CODE", 110)
            .map_err(CodegenError::SectionError)?;
        emitter.emit_org(org_addr);
        emitter.emit_comment("Function: div16");
        emitter.emit_comment("  Params: a: u16 in $80-$81, b: u16 in $82-$83");
        emitter.emit_comment("  Returns: u16 in A/Y (low/high)");
        emitter.emit_comment(&format!("  Location: ${:04X}", org_addr));
        emitter.emit_label("div16");

        // Emit div16 implementation using proper remainder register
        // Memory layout: $D0-$D1 dividend, $D2-$D3 divisor, $D4-$D5 quotient,
        //               $D6-$D7 remainder, $D8 loop counter

        // Zero check - return 0xFFFF for division by zero
        emitter.emit_raw("    LDA $82");
        emitter.emit_raw("    ORA $83");
        emitter.emit_raw("    BNE div16_not_zero");
        emitter.emit_raw("    LDA #$FF");
        emitter.emit_raw("    TAY");
        emitter.emit_raw("    JMP div16_done");

        emitter.emit_raw("    div16_not_zero:");
        // Initialize quotient and remainder to 0
        emitter.emit_raw("    LDA #$00");
        emitter.emit_raw("    STA $D4"); // quotient_low
        emitter.emit_raw("    STA $D5"); // quotient_high
        emitter.emit_raw("    STA $D6"); // remainder_low
        emitter.emit_raw("    STA $D7"); // remainder_high

        // Copy dividend to working storage
        emitter.emit_raw("    LDA $80");
        emitter.emit_raw("    STA $D0"); // dividend_low
        emitter.emit_raw("    LDA $81");
        emitter.emit_raw("    STA $D1"); // dividend_high

        // Copy divisor to working storage
        emitter.emit_raw("    LDA $82");
        emitter.emit_raw("    STA $D2"); // divisor_low
        emitter.emit_raw("    LDA $83");
        emitter.emit_raw("    STA $D3"); // divisor_high

        // Loop counter = 16
        emitter.emit_raw("    LDA #$10");
        emitter.emit_raw("    STA $D8");

        emitter.emit_raw("    div16_loop:");
        // Shift dividend left, high bit goes into remainder
        emitter.emit_raw("    ASL $D0");
        emitter.emit_raw("    ROL $D1");
        emitter.emit_raw("    ROL $D6"); // Carry from dividend -> remainder
        emitter.emit_raw("    ROL $D7");

        // Shift quotient left to make room for next bit
        emitter.emit_raw("    ASL $D4");
        emitter.emit_raw("    ROL $D5");

        // Compare remainder with divisor (16-bit)
        emitter.emit_raw("    LDA $D7"); // remainder_high
        emitter.emit_raw("    CMP $D3"); // divisor_high
        emitter.emit_raw("    BCC div16_skip"); // remainder < divisor
        emitter.emit_raw("    BNE div16_sub"); // remainder > divisor
        // High bytes equal, compare low bytes
        emitter.emit_raw("    LDA $D6"); // remainder_low
        emitter.emit_raw("    CMP $D2"); // divisor_low
        emitter.emit_raw("    BCC div16_skip"); // remainder < divisor

        emitter.emit_raw("    div16_sub:");
        // remainder -= divisor
        emitter.emit_raw("    SEC");
        emitter.emit_raw("    LDA $D6");
        emitter.emit_raw("    SBC $D2");
        emitter.emit_raw("    STA $D6");
        emitter.emit_raw("    LDA $D7");
        emitter.emit_raw("    SBC $D3");
        emitter.emit_raw("    STA $D7");
        // Set quotient bit 0
        emitter.emit_raw("    INC $D4");

        emitter.emit_raw("    div16_skip:");
        emitter.emit_raw("    DEC $D8");
        emitter.emit_raw("    BNE div16_loop");

        // Return quotient in A/Y
        emitter.emit_raw("    LDA $D4");
        emitter.emit_raw("    LDY $D5");

        emitter.emit_raw("    div16_done:");
        emitter.emit_raw("    RTS");
    }

    if emitter.needs_mod16 {
        let org_addr = section_alloc
            .allocate("CODE", 110)
            .map_err(CodegenError::SectionError)?;
        emitter.emit_org(org_addr);
        emitter.emit_comment("Function: mod16");
        emitter.emit_comment("  Params: a: u16 in $80-$81, b: u16 in $82-$83");
        emitter.emit_comment("  Returns: u16 remainder in A/Y (low/high)");
        emitter.emit_comment(&format!("  Location: ${:04X}", org_addr));
        emitter.emit_label("mod16");

        // Emit mod16 implementation - same as div16 but returns remainder
        // Memory layout: $D0-$D1 dividend, $D2-$D3 divisor, $D4-$D5 quotient,
        //               $D6-$D7 remainder, $D8 loop counter

        // Zero check - return 0xFFFF for modulo by zero
        emitter.emit_raw("    LDA $82");
        emitter.emit_raw("    ORA $83");
        emitter.emit_raw("    BNE mod16_not_zero");
        emitter.emit_raw("    LDA #$FF");
        emitter.emit_raw("    TAY");
        emitter.emit_raw("    JMP mod16_done");

        emitter.emit_raw("    mod16_not_zero:");
        // Initialize quotient and remainder to 0
        emitter.emit_raw("    LDA #$00");
        emitter.emit_raw("    STA $D4"); // quotient_low
        emitter.emit_raw("    STA $D5"); // quotient_high
        emitter.emit_raw("    STA $D6"); // remainder_low
        emitter.emit_raw("    STA $D7"); // remainder_high

        // Copy dividend to working storage
        emitter.emit_raw("    LDA $80");
        emitter.emit_raw("    STA $D0"); // dividend_low
        emitter.emit_raw("    LDA $81");
        emitter.emit_raw("    STA $D1"); // dividend_high

        // Copy divisor to working storage
        emitter.emit_raw("    LDA $82");
        emitter.emit_raw("    STA $D2"); // divisor_low
        emitter.emit_raw("    LDA $83");
        emitter.emit_raw("    STA $D3"); // divisor_high

        // Loop counter = 16
        emitter.emit_raw("    LDA #$10");
        emitter.emit_raw("    STA $D8");

        emitter.emit_raw("    mod16_loop:");
        // Shift dividend left, high bit goes into remainder
        emitter.emit_raw("    ASL $D0");
        emitter.emit_raw("    ROL $D1");
        emitter.emit_raw("    ROL $D6"); // Carry from dividend -> remainder
        emitter.emit_raw("    ROL $D7");

        // Shift quotient left to make room for next bit
        emitter.emit_raw("    ASL $D4");
        emitter.emit_raw("    ROL $D5");

        // Compare remainder with divisor (16-bit)
        emitter.emit_raw("    LDA $D7"); // remainder_high
        emitter.emit_raw("    CMP $D3"); // divisor_high
        emitter.emit_raw("    BCC mod16_skip"); // remainder < divisor
        emitter.emit_raw("    BNE mod16_sub"); // remainder > divisor
        // High bytes equal, compare low bytes
        emitter.emit_raw("    LDA $D6"); // remainder_low
        emitter.emit_raw("    CMP $D2"); // divisor_low
        emitter.emit_raw("    BCC mod16_skip"); // remainder < divisor

        emitter.emit_raw("    mod16_sub:");
        // remainder -= divisor
        emitter.emit_raw("    SEC");
        emitter.emit_raw("    LDA $D6");
        emitter.emit_raw("    SBC $D2");
        emitter.emit_raw("    STA $D6");
        emitter.emit_raw("    LDA $D7");
        emitter.emit_raw("    SBC $D3");
        emitter.emit_raw("    STA $D7");
        // Set quotient bit 0
        emitter.emit_raw("    INC $D4");

        emitter.emit_raw("    mod16_skip:");
        emitter.emit_raw("    DEC $D8");
        emitter.emit_raw("    BNE mod16_loop");

        // Return REMAINDER in A/Y (difference from div16)
        emitter.emit_raw("    LDA $D6");
        emitter.emit_raw("    LDY $D7");

        emitter.emit_raw("    mod16_done:");
        emitter.emit_raw("    RTS");
    }

    Ok(())
}

pub fn generate(
    ast: &SourceFile,
    program: &ProgramInfo,
    verbosity: CommentVerbosity,
) -> Result<(String, SectionAllocator), CodegenError> {
    use crate::sema::table::{SymbolKind, SymbolLocation};
    use std::collections::{HashMap, HashSet};

    let mut emitter = Emitter::new(verbosity);
    let mut section_alloc = SectionAllocator::default();
    let mut string_collector = StringCollector::new();

    // Build a map of symbol names to their import source file
    let mut import_sources: HashMap<String, String> = HashMap::new();
    for item in &ast.items {
        if let crate::ast::Item::Import(import) = &item.node {
            for symbol in &import.symbols {
                import_sources.insert(symbol.node.clone(), import.path.node.clone());
            }
        }
    }

    // Emit address labels for all addresses (including imported ones)
    // Use resolved_symbols which contains all symbols that are actually used
    let mut emitted_addresses = HashSet::new();

    // Emit addresses from resolved_symbols (includes both local and imported addresses)
    for symbol in program.resolved_symbols.values() {
        if symbol.kind == SymbolKind::Address
            && let SymbolLocation::Absolute(addr) = symbol.location
            && emitted_addresses.insert(symbol.name.clone())
        {
            // Emit comment if this address was imported
            if let Some(source) = import_sources.get(&symbol.name) {
                emitter.emit_comment(&format!("Imported from {}", source));
            }
            emitter.emit_raw(&format!("{} = ${:04X}", symbol.name, addr));
        }
    }

    // Track which items have been emitted to avoid duplicates
    let mut emitted_items: HashSet<String> = HashSet::new();

    // Emit const arrays to DATA section FIRST
    // This separates read-only data from code
    let has_const_arrays = ast.items.iter().chain(&program.imported_items).any(|item| {
        if let crate::ast::Item::Static(s) = &item.node {
            !s.mutable && matches!(s.ty.node, crate::ast::TypeExpr::Array { .. })
        } else {
            false
        }
    });

    if has_const_arrays {
        emitter.emit_comment("============================================================");
        emitter.emit_comment("Data Section (Const Arrays)");
        emitter.emit_comment("============================================================");

        // Emit .ORG for DATA section (default location $C000)
        // TODO: Make this configurable via wraith.toml
        emitter.emit_data_org(0xC000);
        emitter.emit_raw("");

        // Emit const arrays from imported modules first
        for item in &program.imported_items {
            if let crate::ast::Item::Static(s) = &item.node
                && !s.mutable
                && matches!(s.ty.node, crate::ast::TypeExpr::Array { .. })
            {
                let name = s.name.node.clone();
                if emitted_items.insert(name) {
                    generate_item(
                        item,
                        &mut emitter,
                        program,
                        &mut section_alloc,
                        &mut string_collector,
                    )?;
                }
            }
        }

        // Emit const arrays from main module
        for item in &ast.items {
            if let crate::ast::Item::Static(s) = &item.node
                && !s.mutable
                && matches!(s.ty.node, crate::ast::TypeExpr::Array { .. })
            {
                let name = s.name.node.clone();
                if emitted_items.insert(name) {
                    generate_item(
                        item,
                        &mut emitter,
                        program,
                        &mut section_alloc,
                        &mut string_collector,
                    )?;
                }
            }
        }

        emitter.emit_raw("");
    }

    // Generate code for imported items FIRST
    // This ensures that imported functions are defined before they're called
    // Only emit section header if there are actually imported items to generate
    let has_imported_code = program.imported_items.iter().any(|item| {
        !matches!(
            item.node,
            crate::ast::Item::Import(_)
                | crate::ast::Item::Address(_)
                | crate::ast::Item::Static(_)
        )
    });

    if has_imported_code {
        emitter.emit_comment("============================================================");
        emitter.emit_comment("Code from imported modules");
        emitter.emit_comment("============================================================");
    }

    for item in &program.imported_items {
        // Get the item name to check for duplicates
        let item_name = match &item.node {
            crate::ast::Item::Function(f) => Some(f.name.node.clone()),
            crate::ast::Item::Static(s) => Some(s.name.node.clone()),
            crate::ast::Item::Struct(s) => Some(s.name.node.clone()),
            crate::ast::Item::Enum(e) => Some(e.name.node.clone()),
            crate::ast::Item::Address(a) => Some(a.name.node.clone()),
            crate::ast::Item::Import(_) => None, // Skip imports
        };

        // Skip if we've already emitted this item or if it's an import
        if let Some(name) = item_name {
            if !emitted_items.insert(name.clone()) {
                continue; // Already emitted
            }
        } else {
            continue; // It's an import, skip it
        }

        // Skip address declarations - they were already emitted above
        if matches!(item.node, crate::ast::Item::Address(_)) {
            continue;
        }

        generate_item(
            item,
            &mut emitter,
            program,
            &mut section_alloc,
            &mut string_collector,
        )?;
    }

    // Generate code for main module items
    // Only emit section header if there are actually main module items to generate
    let has_main_code = ast.items.iter().any(|item| {
        !matches!(
            item.node,
            crate::ast::Item::Import(_)
                | crate::ast::Item::Address(_)
                | crate::ast::Item::Static(_)
        )
    });

    if has_main_code {
        emitter.emit_comment("============================================================");
        emitter.emit_comment("Code from main module");
        emitter.emit_comment("============================================================");
    }

    for item in &ast.items {
        // Get the item name to check for duplicates
        let item_name = match &item.node {
            crate::ast::Item::Function(f) => Some(f.name.node.clone()),
            crate::ast::Item::Static(s) => Some(s.name.node.clone()),
            crate::ast::Item::Struct(s) => Some(s.name.node.clone()),
            crate::ast::Item::Enum(e) => Some(e.name.node.clone()),
            crate::ast::Item::Address(a) => Some(a.name.node.clone()),
            crate::ast::Item::Import(_) => None,
        };

        // Skip if we've already emitted this item
        if let Some(name) = &item_name
            && !emitted_items.insert(name.clone())
        {
            continue; // Already emitted
        }

        // Skip address declarations - they were already emitted above
        if matches!(item.node, crate::ast::Item::Address(_)) {
            continue;
        }
        generate_item(
            item,
            &mut emitter,
            program,
            &mut section_alloc,
            &mut string_collector,
        )?;
    }

    // Check for address conflicts after all functions have been generated
    let conflicts = section_alloc.check_conflicts();
    if !conflicts.is_empty() {
        // Format detailed error message showing all conflicts
        let mut error_msg = String::from("address conflict detected\n");

        for (i, (alloc1, alloc2)) in conflicts.iter().enumerate() {
            if i > 0 {
                error_msg.push('\n');
            }

            error_msg.push_str(&format!(
                "  = note: function '{}' at ${:04X}-${:04X} ({})\n",
                alloc1.name, alloc1.start, alloc1.end, alloc1.source
            ));
            error_msg.push_str(&format!(
                "  = note: conflicts with '{}' at ${:04X}-${:04X} ({})\n",
                alloc2.name, alloc2.start, alloc2.end, alloc2.source
            ));
        }

        return Err(CodegenError::AddressConflict(error_msg));
    }

    // Emit collected string literals to DATA section
    string_collector.emit_strings(&mut emitter, &mut section_alloc)?;

    // Emit stdlib math functions if needed
    emit_stdlib_math_functions(&mut emitter, &mut section_alloc)?;

    // Generate interrupt vector table
    generate_interrupt_vectors(ast, &mut emitter)?;

    // Apply peephole optimizations
    let asm = emitter.finish();
    let lines = peephole::parse_assembly(&asm);
    let optimized = peephole::optimize(&lines);
    let final_asm = peephole::lines_to_string(&optimized);

    Ok((final_asm, section_alloc))
}

/// Generate the 6502 interrupt vector table at $FFFA-$FFFF
fn generate_interrupt_vectors(ast: &SourceFile, emitter: &mut Emitter) -> Result<(), CodegenError> {
    use crate::ast::{FnAttribute, Item};

    // Find interrupt handlers
    let mut nmi_handler: Option<String> = None;
    let mut reset_handler: Option<String> = None;
    let mut irq_handler: Option<String> = None;

    for item in &ast.items {
        if let Item::Function(func) = &item.node {
            let name = func.name.node.clone();

            for attr in &func.attributes {
                match attr {
                    FnAttribute::Nmi => {
                        if nmi_handler.is_some() {
                            return Err(CodegenError::UnsupportedOperation(
                                "Multiple NMI handlers defined".to_string(),
                            ));
                        }
                        nmi_handler = Some(name.clone());
                    }
                    FnAttribute::Reset => {
                        if reset_handler.is_some() {
                            return Err(CodegenError::UnsupportedOperation(
                                "Multiple RESET handlers defined".to_string(),
                            ));
                        }
                        reset_handler = Some(name.clone());
                    }
                    FnAttribute::Irq => {
                        if irq_handler.is_some() {
                            return Err(CodegenError::UnsupportedOperation(
                                "Multiple IRQ handlers defined".to_string(),
                            ));
                        }
                        irq_handler = Some(name.clone());
                    }
                    _ => {}
                }
            }
        }
    }

    // Only generate vector table if at least one handler is defined
    if nmi_handler.is_some() || reset_handler.is_some() || irq_handler.is_some() {
        emitter.emit_comment("============================");
        emitter.emit_comment("Interrupt Vector Table");
        emitter.emit_org(0xFFFA);

        // NMI vector at $FFFA
        if let Some(handler) = nmi_handler {
            emitter.emit_comment(&format!("NMI vector -> {}", handler));
            emitter.emit_word_label(&handler);
        } else {
            emitter.emit_comment("NMI vector (not used)");
            emitter.emit_word(0);
        }

        // RESET vector at $FFFC
        if let Some(handler) = reset_handler {
            emitter.emit_comment(&format!("RESET vector -> {}", handler));
            emitter.emit_word_label(&handler);
        } else {
            emitter.emit_comment("RESET vector (not used)");
            emitter.emit_word(0);
        }

        // IRQ/BRK vector at $FFFE
        if let Some(handler) = irq_handler {
            emitter.emit_comment(&format!("IRQ/BRK vector -> {}", handler));
            emitter.emit_word_label(&handler);
        } else {
            emitter.emit_comment("IRQ/BRK vector (not used)");
            emitter.emit_word(0);
        }
    }

    Ok(())
}
