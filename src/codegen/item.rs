//! Item Code Generation
//!
//! Handles generation of functions and other items.

use crate::ast::{FnAttribute, Function, Item, PrimitiveType, Spanned, TypeExpr};
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

    // Check if this is an interrupt handler (need to know for size calculation)
    // Note: Reset is NOT an interrupt - it's the entry point, so no prologue/epilogue
    let is_interrupt = func.attributes.iter().any(|attr| matches!(
        attr,
        FnAttribute::Interrupt | FnAttribute::Nmi | FnAttribute::Irq
    ));

    // First pass: Generate function into temporary emitter to measure size
    let function_size = {
        let mut temp_emitter = Emitter::new();
        // Copy register state and label counter to avoid label conflicts
        temp_emitter.reg_state = emitter.reg_state.clone();
        temp_emitter.label_counter = emitter.label_counter;
        temp_emitter.match_counter = emitter.match_counter;

        // Include interrupt prologue size if needed (5 instructions = 10 bytes)
        if is_interrupt {
            temp_emitter.emit_inst("PHA", "");
            temp_emitter.emit_inst("TXA", "");
            temp_emitter.emit_inst("PHA", "");
            temp_emitter.emit_inst("TYA", "");
            temp_emitter.emit_inst("PHA", "");
        }

        // Generate function body to measure size
        generate_stmt(&func.body, &mut temp_emitter, info)?;

        // Include epilogue size
        if is_interrupt {
            // 6 instructions for epilogue
            temp_emitter.emit_inst("PLA", "");
            temp_emitter.emit_inst("TAY", "");
            temp_emitter.emit_inst("PLA", "");
            temp_emitter.emit_inst("TAX", "");
            temp_emitter.emit_inst("PLA", "");
            temp_emitter.emit_inst("RTI", "");
        } else if func.return_type.is_none() {
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

    // Check if this is an interrupt handler
    // Note: Reset is NOT an interrupt - it's the entry point, so no prologue/epilogue
    let is_interrupt = func.attributes.iter().any(|attr| matches!(
        attr,
        FnAttribute::Interrupt | FnAttribute::Nmi | FnAttribute::Irq
    ));

    // Emit interrupt prologue if needed
    if is_interrupt {
        emitter.emit_comment("Interrupt handler prologue - save registers");
        emitter.emit_inst("PHA", "");
        emitter.emit_inst("TXA", "");
        emitter.emit_inst("PHA", "");
        emitter.emit_inst("TYA", "");
        emitter.emit_inst("PHA", "");
    }

    // Body
    generate_stmt(&func.body, emitter, info)?;

    // Emit epilogue
    if is_interrupt {
        emitter.emit_comment("Interrupt handler epilogue - restore registers");
        emitter.emit_inst("PLA", "");
        emitter.emit_inst("TAY", "");
        emitter.emit_inst("PLA", "");
        emitter.emit_inst("TAX", "");
        emitter.emit_inst("PLA", "");
        emitter.emit_inst("RTI", "");
    } else {
        // Emit RTS for functions without explicit return (void functions)
        // Only emit if the last instruction wasn't already a terminal instruction (RTS, RTI, or JMP)
        // This avoids duplicate RTS when the function body ends with a return statement
        if func.return_type.is_none() && !emitter.last_was_terminal() {
            emitter.emit_inst("RTS", "");
        }
    }

    Ok(())
}

fn generate_static(
    stat: &crate::ast::Static,
    emitter: &mut Emitter,
    info: &ProgramInfo,
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

    // Look up the symbol to get its type and size
    let sym = info.table.lookup(name)
        .ok_or_else(|| CodegenError::SymbolNotFound(name.clone()))?;

    let type_size = sym.ty.size();

    // Emit initial value as data
    match &stat.init.node {
        crate::ast::Expr::Literal(crate::ast::Literal::Integer(val)) => {
            emitter.emit_comment(&format!("static {}: {} (size: {})", name, val, type_size));

            // Emit value as bytes (little-endian for multi-byte values)
            let value = *val as u64; // Convert to u64 for bit manipulation
            match type_size {
                1 => {
                    // Single byte (u8 or i8)
                    emitter.emit_byte((value & 0xFF) as u8);
                }
                2 => {
                    // Two bytes (u16 or i16) - little-endian
                    emitter.emit_byte((value & 0xFF) as u8);        // Low byte
                    emitter.emit_byte(((value >> 8) & 0xFF) as u8); // High byte
                }
                _ => {
                    // For larger types, emit as many bytes as needed
                    for i in 0..type_size {
                        emitter.emit_byte(((value >> (i * 8)) & 0xFF) as u8);
                    }
                }
            }
        }
        crate::ast::Expr::Literal(crate::ast::Literal::Array(elements)) => {
            emitter.emit_comment(&format!("static {} (array of {} elements)", name, elements.len()));

            // Emit each element as a byte
            for elem in elements {
                if let crate::ast::Expr::Literal(crate::ast::Literal::Integer(val)) = &elem.node {
                    emitter.emit_byte(*val as u8);
                } else {
                    return Err(CodegenError::UnsupportedOperation(
                        "Only integer literals supported in static array initialization".to_string()
                    ));
                }
            }
        }
        crate::ast::Expr::Literal(crate::ast::Literal::Bool(b)) => {
            emitter.emit_comment(&format!("static {} (bool)", name));
            emitter.emit_byte(if *b { 1 } else { 0 });
        }
        _ => {
            return Err(CodegenError::UnsupportedOperation(
                "Only constant literal expressions supported in static initialization".to_string()
            ));
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
