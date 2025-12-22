//! Item Code Generation
//!
//! Handles generation of functions and other items.

use crate::ast::{Function, Item, Spanned};
use crate::codegen::section_allocator::SectionAllocator;
use crate::codegen::stmt::generate_stmt;
use crate::codegen::{CodegenError, Emitter};
use crate::sema::ProgramInfo;

pub fn generate_item(
    item: &Spanned<Item>,
    emitter: &mut Emitter,
    info: &ProgramInfo,
    section_alloc: &mut SectionAllocator,
) -> Result<(), CodegenError> {
    match &item.node {
        Item::Function(func) => generate_function(func, emitter, info, section_alloc),
        Item::Static(stat) => generate_static(stat, emitter, info),
        Item::Address(addr) => generate_address(addr, emitter, info),
        _ => Ok(()),
    }
}

fn generate_function(
    func: &Function,
    emitter: &mut Emitter,
    info: &ProgramInfo,
    section_alloc: &mut SectionAllocator,
) -> Result<(), CodegenError> {
    let name = &func.name.node;

    // Determine function address
    // Priority: explicit org > section attribute > default section
    if let Some(metadata) = info.function_metadata.get(name) {
        if let Some(org_addr) = metadata.org_address {
            // Explicit org address takes precedence
            emitter.emit_org(org_addr);
        } else if let Some(section_name) = &metadata.section {
            // Allocate in specified section
            // Estimate function size (for now, use a conservative 256 bytes)
            // TODO: Calculate actual function size
            let addr = section_alloc
                .allocate(section_name, 256)
                .map_err(CodegenError::SectionError)?;
            emitter.emit_org(addr);
        } else {
            // Use default section (CODE)
            let addr = section_alloc
                .allocate_default(256)
                .map_err(CodegenError::SectionError)?;
            emitter.emit_org(addr);
        }
    } else {
        // No metadata - use default section
        let addr = section_alloc
            .allocate_default(256)
            .map_err(CodegenError::SectionError)?;
        emitter.emit_org(addr);
    }

    emitter.emit_label(name);

    // Prologue (placeholder)
    // emitter.emit_inst("PHA", "");

    // Body
    generate_stmt(&func.body, emitter, info)?;

    // Epilogue (placeholder)
    // emitter.emit_inst("PLA", "");

    // Emit RTS for functions without explicit return (void functions)
    // Functions with explicit return statements will have already emitted RTS
    // TODO: Properly track control flow to avoid duplicate RTS in some cases
    if func.return_type.is_none() {
        emitter.emit_inst("RTS", "");
    }

    Ok(())
}

fn generate_static(
    stat: &crate::ast::Static,
    emitter: &mut Emitter,
    _info: &ProgramInfo,
) -> Result<(), CodegenError> {
    // Generate static variable data
    // Look up the location from the symbol table
    let name = &stat.name.node;

    // Emit label for the static variable
    emitter.emit_label(name);

    // Emit initial value as data
    // For now, only support integer literals
    match &stat.init.node {
        crate::ast::Expr::Literal(crate::ast::Literal::Integer(val)) => {
            // TODO: Handle larger values
            emitter.emit_comment(&format!("static {}: {}", name, val));
            // In real 6502 assembly, we'd use .byte directive
            // For now, just emit a placeholder
            // The actual data emission would look like:
            // .byte $XX
        }
        crate::ast::Expr::Literal(crate::ast::Literal::Array(elements)) => {
            emitter.emit_comment(&format!("static {} (array of {} elements)", name, elements.len()));
            // Would emit .byte for each element
        }
        _ => {
            emitter.emit_comment(&format!("static {} (complex initializer)", name));
        }
    }

    Ok(())
}

fn generate_address(
    addr: &crate::ast::AddressDecl,
    emitter: &mut Emitter,
    info: &ProgramInfo,
) -> Result<(), CodegenError> {
    // Address declarations are memory-mapped I/O locations
    // Look up the actual address value from the symbol table
    let name = &addr.name.node;
    let access = match addr.access {
        crate::ast::AccessMode::Read => "read-only",
        crate::ast::AccessMode::Write => "write-only",
        crate::ast::AccessMode::ReadWrite => "read-write",
    };

    // Get the actual address value from resolved_symbols (using span for correct lookup)
    // Fallback to global table for top-level addresses
    let sym = info.resolved_symbols.get(&addr.name.span)
        .or_else(|| info.table.lookup(name));

    if let Some(sym) = sym {
        if let crate::sema::table::SymbolLocation::Absolute(addr_value) = sym.location {
            // Emit assembler equate: NAME = $ADDRESS
            emitter.emit_raw(&format!("{} = ${:04X}", name, addr_value));
            emitter.emit_comment(&format!("Memory-mapped {} ({})", name, access));
        } else {
            emitter.emit_comment(&format!("address {} ({}) - location type not absolute", name, access));
        }
    } else {
        return Err(CodegenError::SymbolNotFound(name.clone()));
    }

    Ok(())
}
