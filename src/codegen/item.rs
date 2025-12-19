//! Item Code Generation
//!
//! Handles generation of functions and other items.

use crate::ast::{Function, Item, Spanned};
use crate::codegen::stmt::generate_stmt;
use crate::codegen::{CodegenError, Emitter};
use crate::sema::ProgramInfo;

pub fn generate_item(
    item: &Spanned<Item>,
    emitter: &mut Emitter,
    info: &ProgramInfo,
) -> Result<(), CodegenError> {
    match &item.node {
        Item::Function(func) => generate_function(func, emitter, info),
        _ => Ok(()),
    }
}

fn generate_function(
    func: &Function,
    emitter: &mut Emitter,
    info: &ProgramInfo,
) -> Result<(), CodegenError> {
    let name = &func.name.node;

    // Check for #[org] attribute
    // TODO: Parse attributes properly in sema and store in ProgramInfo
    // For now, we'll just check if it's main and emit a default org if needed
    if name == "main" {
        emitter.emit_org(0x0801); // Default C64 start
    }

    emitter.emit_label(name);

    // Prologue (placeholder)
    // emitter.emit_inst("PHA", "");

    // Body
    generate_stmt(&func.body, emitter, info)?;

    // Epilogue (placeholder)
    // emitter.emit_inst("PLA", "");

    // Only emit RTS if the last statement wasn't a return (simple check)
    // Ideally we check control flow graph
    emitter.emit_inst("RTS", "");

    Ok(())
}
