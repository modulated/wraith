pub mod emitter;
pub mod expr;
pub mod item;
pub mod memory_layout;
pub mod regstate;
pub mod section_allocator;
pub mod stmt;

use crate::ast::SourceFile;
use crate::sema::ProgramInfo;
use emitter::Emitter;
use item::generate_item;
use section_allocator::SectionAllocator;

#[derive(Debug, Clone)]
pub enum CodegenError {
    Unknown,
    UnsupportedOperation(String),
    SymbolNotFound(String),
    SectionError(String),
}

pub fn generate(ast: &SourceFile, program: &ProgramInfo) -> Result<String, CodegenError> {
    use crate::sema::table::{SymbolKind, SymbolLocation};
    use std::collections::{HashSet, HashMap};

    let mut emitter = Emitter::new();
    let mut section_alloc = SectionAllocator::default();

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
        if symbol.kind == SymbolKind::Address {
            if let SymbolLocation::Absolute(addr) = symbol.location {
                if emitted_addresses.insert(symbol.name.clone()) {
                    // Emit comment if this address was imported
                    if let Some(source) = import_sources.get(&symbol.name) {
                        emitter.emit_comment(&format!("Imported from {}", source));
                    }
                    emitter.emit_raw(&format!("{} = ${:04X}", symbol.name, addr));
                }
            }
        }
    }

    // Generate code for all items except addresses (already emitted above)
    for item in &ast.items {
        // Skip address declarations - they were already emitted above
        if matches!(item.node, crate::ast::Item::Address(_)) {
            continue;
        }
        generate_item(item, &mut emitter, program, &mut section_alloc)?;
    }

    // Generate interrupt vector table
    generate_interrupt_vectors(ast, &mut emitter)?;

    Ok(emitter.finish())
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
