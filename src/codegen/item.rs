//! Item Code Generation
//!
//! Handles generation of functions and other items.

use crate::ast::{Function, Item, PrimitiveType, Spanned, TypeExpr};
use crate::codegen::section_allocator::SectionAllocator;
use crate::codegen::stmt::generate_stmt;
use crate::codegen::{CodegenError, Emitter};
use crate::sema::ProgramInfo;

/// Format a type for display in comments
fn format_type(ty: &Spanned<TypeExpr>) -> String {
    match &ty.node {
        TypeExpr::Primitive(prim) => match prim {
            PrimitiveType::U8 => "u8".to_string(),
            PrimitiveType::U16 => "u16".to_string(),
            PrimitiveType::I8 => "i8".to_string(),
            PrimitiveType::I16 => "i16".to_string(),
            PrimitiveType::Bool => "bool".to_string(),
        },
        TypeExpr::Pointer { pointee, mutable } => {
            if *mutable {
                format!("*mut {}", format_type(pointee))
            } else {
                format!("*{}", format_type(pointee))
            }
        }
        TypeExpr::Array { element, size } => {
            format!("[{}; {}]", format_type(element), size)
        }
        TypeExpr::Slice { element, mutable } => {
            if *mutable {
                format!("&mut [{}]", format_type(element))
            } else {
                format!("&[{}]", format_type(element))
            }
        }
        TypeExpr::Named(name) => name.clone(),
    }
}

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

    // First pass: Generate function into temporary emitter to measure size
    let function_size = {
        let mut temp_emitter = Emitter::new();
        // Copy register state and other context
        temp_emitter.reg_state = emitter.reg_state.clone();

        // Generate function body to measure size
        generate_stmt(&func.body, &mut temp_emitter, info)?;

        // Add RTS if needed (for void functions)
        if func.return_type.is_none() {
            temp_emitter.emit_inst("RTS", "");
        }

        // Get the actual size + 10 bytes padding for safety
        temp_emitter.byte_count() + 10
    };

    // Determine function address
    // Priority: explicit org > section attribute > default section
    let function_addr = if let Some(metadata) = info.function_metadata.get(name) {
        if let Some(org_addr) = metadata.org_address {
            // Explicit org address takes precedence
            emitter.emit_org(org_addr);
            org_addr
        } else if let Some(section_name) = &metadata.section {
            // Allocate in specified section using actual measured size
            let addr = section_alloc
                .allocate(section_name, function_size)
                .map_err(CodegenError::SectionError)?;
            emitter.emit_org(addr);
            addr
        } else {
            // Use default section (CODE)
            let addr = section_alloc
                .allocate_default(function_size)
                .map_err(CodegenError::SectionError)?;
            emitter.emit_org(addr);
            addr
        }
    } else {
        // No metadata - use default section
        let addr = section_alloc
            .allocate_default(function_size)
            .map_err(CodegenError::SectionError)?;
        emitter.emit_org(addr);
        addr
    };

    // Emit function header comment with signature and location
    emitter.emit_comment(&format!("Function: {}", name));

    // Parameters
    if !func.params.is_empty() {
        let params_str: Vec<String> = func.params.iter()
            .map(|p| format!("{}: {}", p.name.node, format_type(&p.ty)))
            .collect();
        emitter.emit_comment(&format!("  Params: {}", params_str.join(", ")));
    } else {
        emitter.emit_comment("  Params: none");
    }

    // Return type
    if let Some(ref ret_ty) = func.return_type {
        emitter.emit_comment(&format!("  Returns: {}", format_type(ret_ty)));
    } else {
        emitter.emit_comment("  Returns: void");
    }

    // Location
    emitter.emit_comment(&format!("  Location: ${:04X}", function_addr));

    // Attributes
    if let Some(metadata) = info.function_metadata.get(name) {
        let mut attrs = Vec::new();
        if metadata.is_inline {
            attrs.push("inline");
        }
        if let Some(ref section) = metadata.section {
            attrs.push(section.as_str());
        }
        if !attrs.is_empty() {
            emitter.emit_comment(&format!("  Attributes: {}", attrs.join(", ")));
        }
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
    // Skip code generation for const (non-mutable statics)
    // They are compile-time constants that get folded into the code
    if !stat.mutable {
        return Ok(());
    }

    // Generate storage for mutable statics only
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
    let name = &addr.name.node;

    // Get the actual address value from resolved_symbols (using span for correct lookup)
    // Fallback to global table for top-level addresses
    let sym = info.resolved_symbols.get(&addr.name.span)
        .or_else(|| info.table.lookup(name));

    if let Some(sym) = sym {
        if let crate::sema::table::SymbolLocation::Absolute(addr_value) = sym.location {
            // Emit assembler equate: NAME = $ADDRESS
            emitter.emit_raw(&format!("{} = ${:04X}", name, addr_value));
        }
    } else {
        return Err(CodegenError::SymbolNotFound(name.clone()));
    }

    Ok(())
}
