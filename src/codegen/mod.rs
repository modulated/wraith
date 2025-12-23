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

    Ok(emitter.finish())
}
