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
        TypeExpr::Pointer { pointee } => format!("&{}", format_type(pointee)),
        TypeExpr::Named(name) => name.clone(),
        TypeExpr::Function { params, ret } => {
            let params = params
                .iter()
                .map(format_type)
                .collect::<Vec<_>>()
                .join(", ");
            match ret {
                Some(r) => format!("fn({}) -> {}", params, format_type(r)),
                None => format!("fn({})", params),
            }
        }
    }
}

pub fn generate_item(
    item: &Spanned<Item>,
    emitter: &mut Emitter,
    info: &ProgramInfo,
    placement: &crate::codegen::placement::Placement,
    section_alloc: &mut SectionAllocator,
    string_collector: &mut StringCollector,
) -> Result<(), CodegenError> {
    match &item.node {
        Item::Function(func) => generate_function(
            func,
            emitter,
            info,
            placement,
            section_alloc,
            string_collector,
        ),
        Item::Static(stat) => generate_static(stat, emitter, info, section_alloc, string_collector),
        Item::Address(addr) => generate_address(addr, emitter, info),
        _ => Ok(()),
    }
}

/// Zero-page addresses an interrupt handler must preserve, in save order. The
/// list is pushed forward (LDA/PHA) in the prologue and popped in reverse
/// (PLA/STA) in the epilogue, wrapping the register save/restore. Because a
/// handler can preempt main code mid-expression, it must preserve the shared
/// codegen scratch/pools/math region plus the frame span its own call graph
/// touches (frames overlap main frames under unified coloring).
pub(super) fn interrupt_zp_save_addrs(info: &ProgramInfo, name: &str) -> Vec<u8> {
    let mut addrs = Vec::new();
    if let Some(si) = info.interrupt_save_info.get(name) {
        if si.save_scratch {
            addrs.extend(0x20u8..=0x3F); // codegen temps / pointer ops
            addrs.extend(0xF0u8..=0xFE); // binary-save + arg pools + scalar spill
        }
        if si.save_math {
            addrs.extend(0xD0u8..=0xDC); // mul16/div16 working storage + params
        }
        for (base, len) in &si.shared_frames {
            for i in 0..*len {
                addrs.push(base.wrapping_add(i));
            }
        }
    }
    addrs
}

pub(super) fn emit_interrupt_zp_save(emitter: &mut Emitter, addrs: &[u8]) {
    if addrs.is_empty() {
        return;
    }
    emitter.emit_comment("Save zero-page state the handler may clobber");
    for a in addrs {
        emitter.emit_inst("LDA", &format!("${:02X}", a));
        emitter.emit_inst("PHA", "");
    }
}

pub(super) fn emit_interrupt_zp_restore(emitter: &mut Emitter, addrs: &[u8]) {
    if addrs.is_empty() {
        return;
    }
    emitter.emit_comment("Restore zero-page state (reverse order)");
    for a in addrs.iter().rev() {
        emitter.emit_inst("PLA", "");
        emitter.emit_inst("STA", &format!("${:02X}", a));
    }
}

fn generate_function(
    func: &Function,
    emitter: &mut Emitter,
    info: &ProgramInfo,
    placement: &crate::codegen::placement::Placement,
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

    // Zero-page locations this handler must preserve (empty for non-handlers).
    // Computed once and emitted identically in the size-measuring and real passes.
    let interrupt_zp = if is_interrupt {
        interrupt_zp_save_addrs(info, name)
    } else {
        Vec::new()
    };

    // Where this function goes was decided before anything was emitted, so
    // that ranges pinned by #[org] could be reserved before the allocator
    // handed out the addresses around them (see `codegen::placement`).
    let placed = placement
        .get(name)
        .ok_or_else(|| CodegenError::Internal(format!("function '{}' was never placed", name)))?;
    let function_addr = placed.addr;
    let function_size = placed.size;
    let allocation_source = match placed.source {
        crate::codegen::placement::AllocationSourceKind::ExplicitOrg => {
            AllocationSource::ExplicitOrg
        }
        crate::codegen::placement::AllocationSourceKind::Section => AllocationSource::Section(
            info.function_metadata
                .get(name)
                .and_then(|m| m.section.clone())
                .unwrap_or_else(|| "CODE".to_string()),
        ),
        crate::codegen::placement::AllocationSourceKind::AutoAllocated => {
            AllocationSource::AutoAllocated
        }
    };
    emitter.emit_org(function_addr);

    // Record this allocation for conflict detection
    section_alloc.record_allocation(
        name.clone(),
        function_addr,
        function_size,
        allocation_source,
        Some(func.name.span),
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
        if let Some(frame) = info.function_frames.get(name) {
            emitter.emit_comment(&format!(
                "  Frame: ${:02X}-${:02X} ({} bytes: {} params + locals)",
                frame.base,
                frame.base.wrapping_add(frame.size.saturating_sub(1)),
                frame.size,
                frame.param_size
            ));
        }
        emitter.emit_comment("  Temps: $20-$3F=codegen scratch");
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

        // Mutable statics live in RAM, which holds garbage at power-on, so their
        // declared initial values must be written here before any user code runs.
        emit_static_inits(emitter, info);
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
        emit_interrupt_zp_save(emitter, &interrupt_zp);
    }

    // Address-taken function prologue: copy arguments from the fixed indirect-arg
    // staging block into this function's colored frame parameter slots. Every
    // caller (direct or indirect) writes args to the staging block, so this runs
    // on every entry. The params occupy [frame.base, frame.base + param_size).
    if info.address_taken_functions.contains(name)
        && let Some(frame) = info.function_frames.get(name)
        && frame.param_size > 0
    {
        emitter.emit_comment("Function-pointer prologue: copy staged args into frame");
        for i in 0..frame.param_size {
            emitter.emit_inst(
                "LDA",
                &format!(
                    "${:02X}",
                    crate::codegen::memory_layout::INDIRECT_ARG_BASE + i
                ),
            );
            emitter.emit_inst("STA", &format!("${:02X}", frame.base + i));
        }
        emitter.invalidate_registers();
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
        emit_interrupt_zp_restore(emitter, &interrupt_zp);
        emitter.emit_inst("PLA", "");
        emitter.emit_inst("TAY", "");
        emitter.emit_inst("PLA", "");
        emitter.emit_inst("TAX", "");
        emitter.emit_inst("PLA", "");
        emitter.emit_inst("RTI", "");
    } else {
        // Emit a trailing RTS whenever the body does not already end in a
        // terminal instruction (RTS, RTI, or JMP). This covers void functions
        // that fall off the end AND value-returning functions whose body ends in
        // an `asm { }` block that leaves the result in registers without an
        // explicit `return` (e.g. the stdlib math routines) — those would
        // otherwise fall through into the next function. `last_was_terminal`
        // already prevents a duplicate RTS after an explicit `return`.
        if !emitter.last_was_terminal() {
            emitter.emit_inst("RTS", "");
        }
    }

    Ok(())
}

/// Write each mutable static's declared initial value into its RAM location.
/// Emitted at the top of the reset handler. An all-zero region (the common
/// `= 0` / `[0; N]` case) becomes a compact fill loop; other values are written
/// byte by byte, coalescing runs that share a value so `A` is reloaded rarely.
fn emit_static_inits(emitter: &mut Emitter, info: &ProgramInfo) {
    use crate::sema::InitByte;

    // Only statics that survived dead-code elimination are declared in the
    // output; initializing a dropped one would write to an address no label
    // names.
    let live: Vec<&crate::sema::StaticInit> = info
        .static_inits
        .iter()
        .filter(|init| info.reachable_symbols.contains(&init.name))
        .collect();
    if live.is_empty() {
        return;
    }
    emitter.emit_comment("Initialize mutable statics (RAM is undefined at reset)");
    for init in live {
        let len = init.bytes.len();
        if len == 0 {
            continue;
        }
        // Large all-zero blocks: fill with a loop instead of `len` stores.
        if len > 8 && init.bytes.iter().all(|b| matches!(b, InitByte::Byte(0))) {
            let loop_label = emitter.next_label("bssz");
            emitter.emit_comment(&format!("{}: zero {} bytes", init.name, len));
            emitter.emit_inst("LDA", "#$00");
            emitter.emit_inst("LDX", "#$00");
            emitter.emit_label(&loop_label);
            for page in 0..len.div_ceil(256) {
                let base = init.addr as usize + page * 256;
                emitter.emit_inst("STA", &format!("${:04X},X", base));
            }
            emitter.emit_inst("INX", "");
            let count = if len >= 256 { 0u8 } else { len as u8 };
            emitter.emit_inst("CPX", &format!("#${:02X}", count));
            emitter.emit_inst("BNE", &loop_label);
            emitter.invalidate_registers();
            continue;
        }
        emitter.emit_comment(&format!("{} = {} byte(s)", init.name, len));
        // Track the last value loaded into A so runs of equal bytes reload rarely.
        let mut last: Option<u8> = None;
        for (i, b) in init.bytes.iter().enumerate() {
            match b {
                InitByte::Byte(v) => {
                    if last != Some(*v) {
                        emitter.emit_inst("LDA", &format!("#${:02X}", v));
                        last = Some(*v);
                    }
                }
                // Function addresses are only known to the assembler.
                InitByte::FnLow(f) => {
                    emitter.emit_inst("LDA", &format!("#<{}", f));
                    last = None;
                }
                InitByte::FnHigh(f) => {
                    emitter.emit_inst("LDA", &format!("#>{}", f));
                    last = None;
                }
            }
            emitter.emit_inst("STA", &format!("${:04X}", init.addr as usize + i));
        }
        emitter.invalidate_registers();
    }
}

fn generate_static(
    stat: &crate::ast::Static,
    emitter: &mut Emitter,
    info: &ProgramInfo,
    section_alloc: &mut SectionAllocator,
    string_collector: &mut StringCollector,
) -> Result<(), CodegenError> {
    // Handle const declarations specially - they may need to emit data
    if !stat.mutable {
        // Check if this is a const array - if so, emit it to data section
        if matches!(stat.ty.node, TypeExpr::Array { .. }) {
            return emit_const_array(stat, emitter, info, section_alloc, string_collector);
        }

        // Handle const strings - register folded constants with string collector
        if matches!(&stat.ty.node, TypeExpr::Named(name) if name == "str") {
            // Check if the init expression was folded to a string constant
            if let Some(const_val) = info.folded_constants.get(&stat.init.span) {
                if let crate::sema::const_eval::ConstValue::String(s) = const_val {
                    // Register the string so it gets emitted to the data section
                    string_collector.add_string(s.clone());
                }
            } else if let crate::ast::Expr::Literal(crate::ast::Literal::String(s)) =
                &stat.init.node
            {
                // Direct string literal - register it
                string_collector.add_string(s.clone());
            }
        }

        // Skip code generation for other const (non-mutable) statics
        // They are compile-time constants that get folded into the code
        return Ok(());
    }

    // A mutable `static` lives in RAM (the BSS section), not ROM. Sema assigned
    // it a concrete address, so emit only an assembler equate here — no bytes.
    // Its initial value is written by the reset handler (see emit_static_init),
    // because ROM data cannot be pre-loaded into RAM on a bare machine.
    let name = &stat.name.node;
    let sym = info
        .table
        .lookup(name)
        .ok_or_else(|| CodegenError::SymbolNotFound(name.clone()))?;
    if let crate::sema::table::SymbolLocation::Absolute(addr) = sym.location {
        let size = crate::codegen::expr::type_byte_size(&sym.ty, info).max(1);
        emitter.emit_comment(&format!(
            "static {} @ ${:04X} ({} bytes, RAM)",
            name, addr, size
        ));
        emitter.emit_raw(&format!("{} = ${:04X}", name, addr));
        // Statics occupy RAM for the whole program, so an #[org] placed over
        // one is a conflict even though no bytes are emitted here.
        section_alloc.record_allocation(
            name.clone(),
            addr,
            size as u16,
            AllocationSource::Section("BSS".to_string()),
            Some(stat.name.span),
        );
    } else {
        return Err(CodegenError::Internal(format!(
            "mutable static '{}' was not assigned a RAM address",
            name
        )));
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
    section_alloc: &mut SectionAllocator,
    _string_collector: &mut StringCollector,
) -> Result<(), CodegenError> {
    let name = &stat.name.node;

    // Element width so u16/i16/b16 arrays emit two little-endian bytes each.
    let elem_size = match &stat.ty.node {
        crate::ast::TypeExpr::Array { element, .. } => match &element.node {
            crate::ast::TypeExpr::Primitive(p) => p.size_bytes(),
            _ => 1,
        },
        _ => 1,
    };

    let elem_count = match &stat.ty.node {
        crate::ast::TypeExpr::Array { size, .. } => *size,
        _ => 0,
    };
    let byte_size = (elem_count * elem_size) as u16;

    // Take an address from the DATA section rather than emitting at a fixed
    // .ORG. Every const array used to be written at $C000, so two of them
    // overlapped, the address ignored whatever the memory map said, and none of
    // them were visible to the #[org] conflict check.
    let addr = section_alloc
        .allocate("DATA", byte_size)
        .map_err(CodegenError::SectionError)?;
    section_alloc.record_allocation(
        name.clone(),
        addr,
        byte_size,
        AllocationSource::Section("DATA".to_string()),
        Some(stat.name.span),
    );

    emitter.emit_comment(&format!("Const array: {} ({} bytes)", name, byte_size));
    emitter.emit_data_org(addr);
    emitter.emit_data_label(name);

    // Emit array data based on initialization expression
    match &stat.init.node {
        crate::ast::Expr::Literal(crate::ast::Literal::ArrayFill { value, count }) => {
            emit_array_fill_data(value, *count, elem_size, emitter, info)?;
        }
        crate::ast::Expr::Literal(crate::ast::Literal::Array(elements)) => {
            emit_array_literal_data(elements, elem_size, emitter, info)?;
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
    elem_size: usize,
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

    let total_bytes = count * elem_size.max(1);

    // Zero-fill optimization: use .RES directive for zeros
    if val == 0 && total_bytes >= 16 {
        emitter.emit_comment(&format!(
            "Zero-filled array optimized: {} bytes",
            total_bytes
        ));
        emitter.emit_data_directive(&format!(".RES {}", total_bytes));
    } else {
        // Emit each element as `elem_size` little-endian bytes, `count` times.
        let mut bytes = Vec::with_capacity(total_bytes);
        for _ in 0..count {
            push_le_bytes(&mut bytes, val, elem_size);
        }
        emit_byte_directives(&bytes, emitter);
    }

    Ok(())
}

/// Emit data for an array literal ([1, 2, 3, ...])
fn emit_array_literal_data(
    elements: &[Spanned<crate::ast::Expr>],
    elem_size: usize,
    emitter: &mut Emitter,
    info: &ProgramInfo,
) -> Result<(), CodegenError> {
    let mut bytes = Vec::new();

    // Collect each element as `elem_size` little-endian bytes so u16/i16/b16
    // arrays keep their high byte instead of being truncated.
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
        push_le_bytes(&mut bytes, val, elem_size);
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

/// Append `size` little-endian bytes of `value` (1 for u8/bool, 2 for u16).
fn push_le_bytes(bytes: &mut Vec<u8>, value: i64, size: usize) {
    let bits = value as u64;
    for i in 0..size.max(1) {
        bytes.push(((bits >> (i * 8)) & 0xFF) as u8);
    }
}

/// Emit a byte slice as `.BYTE` directives, max 16 per line for readability.
fn emit_byte_directives(bytes: &[u8], emitter: &mut Emitter) {
    for chunk in bytes.chunks(16) {
        let byte_str = chunk
            .iter()
            .map(|b| format!("${:02X}", b))
            .collect::<Vec<_>>()
            .join(", ");
        emitter.emit_data_directive(&format!(".BYTE {}", byte_str));
    }
}
