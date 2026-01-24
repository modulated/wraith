//! Item Code Generation
//!
//! Handles generation of functions and other items.

use crate::ast::{FnAttribute, Function, Item, PrimitiveType, Spanned, TypeExpr};
use crate::codegen::section_allocator::{AllocationSource, SectionAllocator};
use crate::codegen::stmt::generate_stmt;
use crate::codegen::{CodegenError, Emitter, StringCollector};
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
            PrimitiveType::B8 => "b8".to_string(),
            PrimitiveType::B16 => "b16".to_string(),
            PrimitiveType::Addr => "addr".to_string(),
        },
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
    string_collector: &mut StringCollector,
) -> Result<(), CodegenError> {
    match &item.node {
        Item::Function(func) => {
            generate_function(func, emitter, info, section_alloc, string_collector)
        }
        Item::Static(stat) => generate_static(stat, emitter, info, string_collector),
        Item::Address(addr) => generate_address(addr, emitter, info),
        _ => Ok(()),
    }
}

fn generate_function(
    func: &Function,
    emitter: &mut Emitter,
    info: &ProgramInfo,
    section_alloc: &mut SectionAllocator,
    string_collector: &mut StringCollector,
) -> Result<(), CodegenError> {
    let name = &func.name.node;

    // Skip code generation for inline functions - they're expanded at call sites
    if let Some(metadata) = info.function_metadata.get(name)
        && metadata.is_inline
    {
        return Ok(());
    }

    // Check if this is an interrupt handler (need to know for size calculation)
    // Note: Reset is NOT an interrupt - it's the entry point, so no prologue/epilogue
    let is_interrupt = func.attributes.iter().any(|attr| {
        matches!(
            attr,
            FnAttribute::Interrupt | FnAttribute::Nmi | FnAttribute::Irq
        )
    });

    // First pass: Generate function into temporary emitter to measure size
    let function_size = {
        let mut temp_emitter = Emitter::new(emitter.verbosity);
        // Copy register state and label counter to avoid label conflicts
        temp_emitter.reg_state = emitter.reg_state.clone();
        temp_emitter.label_counter = emitter.label_counter;
        temp_emitter.match_counter = emitter.match_counter;
        // Set current function for inline asm variable scoping
        temp_emitter.set_current_function(name.clone());

        // Include interrupt prologue size if needed (5 instructions = 10 bytes)
        if is_interrupt {
            temp_emitter.emit_inst("PHA", "");
            temp_emitter.emit_inst("TXA", "");
            temp_emitter.emit_inst("PHA", "");
            temp_emitter.emit_inst("TYA", "");
            temp_emitter.emit_inst("PHA", "");
        }

        // Generate function body to measure size
        generate_stmt(&func.body, &mut temp_emitter, info, string_collector)?;

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
    let (function_addr, allocation_source) =
        if let Some(metadata) = info.function_metadata.get(name) {
            if let Some(org_addr) = metadata.org_address {
                // Explicit org address takes precedence
                emitter.emit_org(org_addr);
                (org_addr, AllocationSource::ExplicitOrg)
            } else if let Some(section_name) = &metadata.section {
                // Allocate in specified section using actual measured size
                let addr = section_alloc
                    .allocate(section_name, function_size)
                    .map_err(CodegenError::SectionError)?;
                emitter.emit_org(addr);
                (addr, AllocationSource::Section(section_name.clone()))
            } else {
                // Use default section (CODE)
                let addr = section_alloc
                    .allocate_default(function_size)
                    .map_err(CodegenError::SectionError)?;
                emitter.emit_org(addr);
                (addr, AllocationSource::AutoAllocated)
            }
        } else {
            // No metadata - use default section
            let addr = section_alloc
                .allocate_default(function_size)
                .map_err(CodegenError::SectionError)?;
            emitter.emit_org(addr);
            (addr, AllocationSource::AutoAllocated)
        };

    // Record this allocation for conflict detection
    section_alloc.record_allocation(
        name.clone(),
        function_addr,
        function_size,
        allocation_source,
    );

    // Emit function header comment with signature and location
    emitter.emit_comment(&format!("Function: {}", name));

    // Parameters
    if !func.params.is_empty() {
        let params_str: Vec<String> = func
            .params
            .iter()
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

    // Document zero-page usage in verbose mode
    if emitter.is_verbose() {
        emitter.emit_comment(&format!(
            "  Temps: $20-${:02X}=available scratch",
            emitter.memory_layout.param_base - 1
        ));
        emitter.emit_comment(&format!(
            "  Params: ${:02X}-$81=parameter area",
            emitter.memory_layout.param_base
        ));
    }

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

    // Initialize software stack pointer for reset handler
    let is_reset = func
        .attributes
        .iter()
        .any(|attr| matches!(attr, FnAttribute::Reset));
    if is_reset {
        emitter.emit_comment("Initialize software stack pointer for parameter preservation");
        emitter.emit_inst("LDA", "#$00");
        emitter.emit_inst("STA", "$FF"); // Stack pointer at $FF, stack at $0200-$02FF
    }

    // Set current function context for tail call detection and inline asm scoping
    emitter.set_current_function(name.clone());

    // Check if function has tail recursion - if so, emit loop restart label
    let has_tail_recursion = info
        .function_metadata
        .get(name)
        .map(|m| m.has_tail_recursion)
        .unwrap_or(false);

    if has_tail_recursion {
        emitter.emit_comment("Tail recursive function - loop optimization enabled");
        emitter.emit_label(&format!("{}_loop_start", name));
    }

    // Copy struct parameter pointers to local storage
    // This ensures nested calls don't clobber the struct pointer in param space
    if let Some(metadata) = info.function_metadata.get(name)
        && !metadata.struct_param_locals.is_empty()
    {
        emitter.emit_comment("Copy struct param pointers to local storage");
        let param_base = emitter.memory_layout.param_base;
        let mut param_offset = 0u8;

        // Iterate through params to find struct params and their offsets
        for param in &func.params {
            let param_name = &param.name.node;
            let param_type = info.resolved_types.get(&param.ty.span);

            // Check if this param has a local copy
            if let Some(&local_addr) = metadata.struct_param_locals.get(param_name) {
                let param_addr = param_base + param_offset;
                emitter.emit_comment(&format!(
                    "Copy '{}' pointer ${:02X} -> ${:02X}",
                    param_name, param_addr, local_addr
                ));
                emitter.emit_inst("LDA", &format!("${:02X}", param_addr));
                emitter.emit_inst("STA", &format!("${:02X}", local_addr));
                emitter.emit_inst("LDA", &format!("${:02X}", param_addr + 1));
                emitter.emit_inst("STA", &format!("${:02X}", local_addr + 1));
                param_offset += 2; // Struct pointers are 2 bytes
            } else if let Some(ty) = param_type {
                // Non-struct param - advance by its size
                // Arrays are passed as 2-byte pointers, not by value
                if matches!(ty, crate::sema::types::Type::Array(_, _)) {
                    param_offset += 2;
                } else {
                    param_offset += ty.size() as u8;
                }
            } else {
                // Fallback: assume 1 byte
                param_offset += 1;
            }
        }
    }

    // Check if this is an interrupt handler
    // Note: Reset is NOT an interrupt - it's the entry point, so no prologue/epilogue
    let is_interrupt = func.attributes.iter().any(|attr| {
        matches!(
            attr,
            FnAttribute::Interrupt | FnAttribute::Nmi | FnAttribute::Irq
        )
    });

    // Emit interrupt prologue if needed
    if is_interrupt {
        emitter.emit_comment("Interrupt handler prologue - save registers");
        if emitter.is_verbose() {
            emitter.emit_comment("Stack: [return_lo, return_hi, P, A, X, Y] (6 bytes pushed)");
        }
        emitter.emit_inst("PHA", "");
        emitter.emit_inst("TXA", "");
        emitter.emit_inst("PHA", "");
        emitter.emit_inst("TYA", "");
        emitter.emit_inst("PHA", "");
    }

    // Body
    generate_stmt(&func.body, emitter, info, string_collector)?;

    // Clear current function context
    emitter.clear_current_function();

    // Emit epilogue
    if is_interrupt {
        emitter.emit_comment("Interrupt handler epilogue - restore registers");
        if emitter.is_verbose() {
            emitter.emit_comment("Restore Y, X, A in reverse order (LIFO)");
        }
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
    string_collector: &mut StringCollector,
) -> Result<(), CodegenError> {
    // Handle const arrays specially - they need to emit data
    if !stat.mutable {
        // Check if this is a const array - if so, emit it to data section
        if matches!(stat.ty.node, TypeExpr::Array { .. }) {
            return emit_const_array(stat, emitter, info, string_collector);
        }
        // Skip code generation for other const (non-mutable) statics
        // They are compile-time constants that get folded into the code
        return Ok(());
    }

    // Generate storage for mutable statics only
    let name = &stat.name.node;

    // Emit label for the static variable
    emitter.emit_label(name);

    // Look up the symbol to get its type and size
    let sym = info
        .table
        .lookup(name)
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
                    emitter.emit_byte((value & 0xFF) as u8); // Low byte
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
            emitter.emit_comment(&format!(
                "static {} (array of {} elements)",
                name,
                elements.len()
            ));

            // Emit each element as a byte
            for elem in elements {
                if let crate::ast::Expr::Literal(crate::ast::Literal::Integer(val)) = &elem.node {
                    emitter.emit_byte(*val as u8);
                } else {
                    return Err(CodegenError::UnsupportedOperation(
                        "Only integer literals supported in static array initialization"
                            .to_string(),
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
                "Only constant literal expressions supported in static initialization".to_string(),
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
    let sym = info
        .resolved_symbols
        .get(&addr.name.span)
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

/// Emit a const array to the data section
fn emit_const_array(
    stat: &crate::ast::Static,
    emitter: &mut Emitter,
    info: &ProgramInfo,
    _string_collector: &mut StringCollector,
) -> Result<(), CodegenError> {
    let name = &stat.name.node;

    emitter.emit_comment(&format!("Const array: {}", name));

    // Emit data label
    emitter.emit_data_label(name);

    // Emit array data based on initialization expression
    match &stat.init.node {
        crate::ast::Expr::Literal(crate::ast::Literal::ArrayFill { value, count }) => {
            emit_array_fill_data(value, *count, emitter, info)?;
        }
        crate::ast::Expr::Literal(crate::ast::Literal::Array(elements)) => {
            emit_array_literal_data(elements, emitter, info)?;
        }
        _ => {
            return Err(CodegenError::UnsupportedOperation(
                "Const arrays must have literal initializers".to_string(),
            ));
        }
    }

    Ok(())
}

/// Emit data for an array fill literal ([value; count])
fn emit_array_fill_data(
    value: &Spanned<crate::ast::Expr>,
    count: usize,
    emitter: &mut Emitter,
    info: &ProgramInfo,
) -> Result<(), CodegenError> {
    // Evaluate the fill value as a constant
    let val = if let crate::ast::Expr::Literal(crate::ast::Literal::Integer(n)) = &value.node {
        *n
    } else if let Some(const_val) = info.folded_constants.get(&value.span) {
        if let crate::sema::const_eval::ConstValue::Integer(n) = const_val {
            *n
        } else {
            return Err(CodegenError::UnsupportedOperation(
                "Array fill value must be an integer".to_string(),
            ));
        }
    } else {
        return Err(CodegenError::UnsupportedOperation(
            "Array fill value must be a constant".to_string(),
        ));
    };

    // Emit repeated bytes (using .BYTE for portability)
    emit_repeated_bytes(val as u8, count, emitter);

    Ok(())
}

/// Emit data for an array literal ([1, 2, 3, ...])
fn emit_array_literal_data(
    elements: &[Spanned<crate::ast::Expr>],
    emitter: &mut Emitter,
    info: &ProgramInfo,
) -> Result<(), CodegenError> {
    let mut bytes = Vec::new();

    // Collect all bytes
    for elem in elements {
        let val = if let crate::ast::Expr::Literal(crate::ast::Literal::Integer(n)) = &elem.node {
            *n
        } else if let Some(const_val) = info.folded_constants.get(&elem.span) {
            if let crate::sema::const_eval::ConstValue::Integer(n) = const_val {
                *n
            } else {
                return Err(CodegenError::UnsupportedOperation(
                    "Array elements must be integers".to_string(),
                ));
            }
        } else {
            return Err(CodegenError::UnsupportedOperation(
                "Array elements must be constants".to_string(),
            ));
        };
        bytes.push(val as u8);
    }

    // Emit as .BYTE directives (max 16 per line for readability)
    for chunk in bytes.chunks(16) {
        let byte_str = chunk
            .iter()
            .map(|b| format!("${:02X}", b))
            .collect::<Vec<_>>()
            .join(", ");
        emitter.emit_data_directive(&format!(".BYTE {}", byte_str));
    }

    Ok(())
}

/// Emit repeated bytes for array fill with non-zero value
fn emit_repeated_bytes(val: u8, count: usize, emitter: &mut Emitter) {
    // Emit as .BYTE directives (max 16 per line)
    let bytes = vec![val; count];
    for chunk in bytes.chunks(16) {
        let byte_str = chunk
            .iter()
            .map(|b| format!("${:02X}", b))
            .collect::<Vec<_>>()
            .join(", ");
        emitter.emit_data_directive(&format!(".BYTE {}", byte_str));
    }
}
