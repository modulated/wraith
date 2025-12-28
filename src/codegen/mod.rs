pub mod address_allocator;
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
    strings: HashMap<String, String>,  // content -> label
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
    pub fn emit_strings(&self, emitter: &mut Emitter, section_alloc: &mut SectionAllocator) -> Result<(), CodegenError> {
        if self.strings.is_empty() {
            return Ok(());
        }

        emitter.emit_comment("============================");
        emitter.emit_comment("String Literal Data");
        emitter.emit_comment("============================");

        for (content, label) in &self.strings {
            // Allocate in DATA section
            let data_size = 2 + content.len() as u16;  // length prefix + bytes
            let addr = section_alloc.allocate("DATA", data_size)
                .map_err(CodegenError::SectionError)?;

            emitter.emit_org(addr);
            emitter.emit_label(label);

            // Emit length as u16 little-endian
            let len = content.len() as u16;
            emitter.emit_raw(&format!("    .BYTE ${:02X}, ${:02X}  ; length = {}", len & 0xFF, (len >> 8) & 0xFF, len));

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
                    let bytes_str = chunk.iter()
                        .map(|b| format!("${:02X}", b))
                        .collect::<Vec<_>>()
                        .join(", ");

                    if i == 0 && chunk.len() < content.len() {
                        emitter.emit_raw(&format!("    .BYTE {}  ; bytes 0-{}", bytes_str, chunk.len() - 1));
                    } else if chunk.len() < 16 {
                        let start = i * 16;
                        emitter.emit_raw(&format!("    .BYTE {}  ; bytes {}-{}", bytes_str, start, start + chunk.len() - 1));
                    } else {
                        emitter.emit_raw(&format!("    .BYTE {}", bytes_str));
                    }
                }
            }
        }

        Ok(())
    }
}

pub fn generate(ast: &SourceFile, program: &ProgramInfo) -> Result<(String, SectionAllocator), CodegenError> {
    use crate::sema::table::{SymbolKind, SymbolLocation};
    use std::collections::{HashSet, HashMap};

    let mut emitter = Emitter::new();
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
                && emitted_addresses.insert(symbol.name.clone()) {
                    // Emit comment if this address was imported
                    if let Some(source) = import_sources.get(&symbol.name) {
                        emitter.emit_comment(&format!("Imported from {}", source));
                    }
                    emitter.emit_raw(&format!("{} = ${:04X}", symbol.name, addr));
                }
    }

    // Track which items have been emitted to avoid duplicates
    let mut emitted_items: HashSet<String> = HashSet::new();

    // Generate code for imported items FIRST
    // This ensures that imported functions are defined before they're called
    emitter.emit_comment("============================================================");
    emitter.emit_comment("Code from imported modules");
    emitter.emit_comment("============================================================");
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

        generate_item(item, &mut emitter, program, &mut section_alloc, &mut string_collector)?;
    }

    // Generate code for main module items
    emitter.emit_comment("============================================================");
    emitter.emit_comment("Code from main module");
    emitter.emit_comment("============================================================");
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
        generate_item(item, &mut emitter, program, &mut section_alloc, &mut string_collector)?;
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
                                "Multiple NMI handlers defined".to_string()
                            ));
                        }
                        nmi_handler = Some(name.clone());
                    }
                    FnAttribute::Reset => {
                        if reset_handler.is_some() {
                            return Err(CodegenError::UnsupportedOperation(
                                "Multiple RESET handlers defined".to_string()
                            ));
                        }
                        reset_handler = Some(name.clone());
                    }
                    FnAttribute::Irq => {
                        if irq_handler.is_some() {
                            return Err(CodegenError::UnsupportedOperation(
                                "Multiple IRQ handlers defined".to_string()
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
