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
            PrimitiveType::Char => "char".to_string(),
            PrimitiveType::B8 => "b8".to_string(),
            PrimitiveType::B16 => "b16".to_string(),
            PrimitiveType::Addr => "addr".to_string(),
        },
        TypeExpr::Array { element, size } => {
            format!("[{}; {}]", format_type(element), size)
        }
        TypeExpr::Slice { element } => format!("&[{}]", format_type(element)),
        TypeExpr::Pointer { pointee } => format!("&{}", format_type(pointee)),
        TypeExpr::Named(name) => name.clone(),
        TypeExpr::StringBuf { capacity } => format!("str<{}>", capacity),
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
///
/// The address list itself lives on `InterruptSaveInfo` so that sema's
/// hardware-stack depth check counts exactly the bytes this emits.
pub(super) fn interrupt_zp_save_addrs(info: &ProgramInfo, name: &str) -> Vec<u8> {
    info.interrupt_save_info
        .get(name)
        .map(|si| si.zp_save_addrs())
        .unwrap_or_default()
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

    // Start each function with an empty temp pool. Temps are per-function
    // scratch, but a few allocations are intentionally never freed within a
    // function (runtime enum construction hands the caller a pointer into the
    // pool), so without a per-function reset those bytes leak for the rest of
    // the program on the single shared emitter — eventually failing a later
    // function with a spurious "not enough temp storage". The measuring pass
    // already starts clean (`placement::measure` builds a fresh `Emitter` per
    // function), so this also keeps the two passes symmetric. Pool addresses are
    // all zero-page, so which one an allocation gets never changes an
    // instruction's length: resetting cannot affect the measured size.
    emitter.temp_alloc.reset();

    // Size-integrity check (see the end of this function): the allocator
    // reserved `function_size` bytes from the measuring pass. If the real pass
    // emits more, the next allocation overlaps this function's tail — the
    // failure mode this catches is silent binary corruption, so it is a hard
    // internal error, not a warning.
    let bytes_before = emitter.byte_count();

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
        emit_reset_prologue(emitter, info);
    }

    // Set current function context for tail call detection and inline asm scoping
    emitter.set_current_function(name.clone());

    // Check if function has tail recursion - if so, emit loop restart label.
    // The label goes *after* the prologues: the tail-call update writes the
    // new argument values into the frame params and jumps here, so anything
    // that rewrites params must not run again. In particular the
    // function-pointer prologue below copies the $E0 staging block over the
    // params — jumping in before it reloaded the *original* arguments on
    // every iteration and the loop never progressed.
    let has_tail_recursion = info
        .function_metadata
        .get(name)
        .map(|m| m.has_tail_recursion)
        .unwrap_or(false);

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
        if emitter.target.is_cmos() {
            // The 65C02 pushes X and Y directly; NMOS has to route them through
            // A (which is why A is saved first).
            emitter.emit_inst("PHA", "");
            emitter.emit_inst("PHX", "");
            emitter.emit_inst("PHY", "");
        } else {
            emitter.emit_inst("PHA", "");
            emitter.emit_inst("TXA", "");
            emitter.emit_inst("PHA", "");
            emitter.emit_inst("TYA", "");
            emitter.emit_inst("PHA", "");
        }
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

    if has_tail_recursion {
        emitter.emit_comment("Tail recursive function - loop optimization enabled");
        emitter.emit_label(&format!("{}_loop_start", name));
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
        if emitter.target.is_cmos() {
            emitter.emit_inst("PLY", "");
            emitter.emit_inst("PLX", "");
            emitter.emit_inst("PLA", "");
        } else {
            emitter.emit_inst("PLA", "");
            emitter.emit_inst("TAY", "");
            emitter.emit_inst("PLA", "");
            emitter.emit_inst("TAX", "");
            emitter.emit_inst("PLA", "");
        }
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

    // The measuring pass reserved `function_size` bytes for everything above.
    // Emitting more means the two passes disagree about the function's size,
    // and the next function's .ORG lands inside this one's tail.
    let emitted = emitter.byte_count() - bytes_before;
    if emitted > function_size {
        return Err(CodegenError::Internal(format!(
            "function '{}' emitted {} bytes but was measured at {} — \
             the measuring pass (placement::measure) does not match the real one",
            name, emitted, function_size
        )));
    }

    Ok(())
}

/// The reset-handler prologue: software-stack pointer init ($FF points at the
/// $0200 page), then the mutable-static initializers (RAM is undefined at
/// power-on, so their declared values must be written before any user code
/// runs). Shared between the measuring pass (`placement::measure`) and the
/// real one — if the two ever drift, the allocator hands out addresses the
/// emitted bytes overflow, and the next function is spliced into this one.
pub(crate) fn emit_reset_prologue(emitter: &mut Emitter, info: &ProgramInfo) {
    emitter.emit_comment("Initialize software stack pointer for parameter preservation");
    emitter.emit_inst("LDA", "#$00");
    emitter.emit_inst("STA", "$FF"); // Stack pointer at $FF, stack at $0200-$02FF

    // Mutable statics live in RAM, which holds garbage at power-on, so their
    // declared initial values must be written here before any user code runs.
    emit_static_inits(emitter, info);
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
        // Large all-zero blocks: fill with loops instead of `len` stores.
        // Full 256-byte pages share one loop (X wraps through 0, so CPX #$00
        // is the full-page trip count); the trailing partial page gets its
        // own, shorter loop. One loop for both used to run the partial page's
        // STA for a full 256 iterations, zeroing `256 - rem` bytes past the
        // allocation.
        if len > 8 && init.bytes.iter().all(|b| matches!(b, InitByte::Byte(0))) {
            let full_pages = len / 256;
            let rem = len % 256;
            // On the 65C02 `STZ addr,X` stores zero directly, so the loop needs
            // no `LDA #$00` at all. On NMOS, load A once and store it.
            let cmos = emitter.target.is_cmos();
            let store = if cmos { "STZ" } else { "STA" };
            emitter.emit_comment(&format!("{}: zero {} bytes", init.name, len));
            if !cmos {
                emitter.emit_inst("LDA", "#$00");
            }
            if full_pages > 0 {
                let loop_label = emitter.next_label("bssz");
                emitter.emit_inst("LDX", "#$00");
                emitter.emit_label(&loop_label);
                for page in 0..full_pages {
                    let base = init.addr as usize + page * 256;
                    emitter.emit_inst(store, &format!("${:04X},X", base));
                }
                emitter.emit_inst("INX", "");
                emitter.emit_inst("CPX", "#$00");
                emitter.emit_inst("BNE", &loop_label);
            }
            if rem > 0 {
                let loop_label = emitter.next_label("bssz");
                emitter.emit_inst("LDX", "#$00");
                emitter.emit_label(&loop_label);
                let base = init.addr as usize + full_pages * 256;
                emitter.emit_inst(store, &format!("${:04X},X", base));
                emitter.emit_inst("INX", "");
                emitter.emit_inst("CPX", &format!("#${:02X}", rem as u8));
                emitter.emit_inst("BNE", &loop_label);
            }
            emitter.invalidate_registers();
            continue;
        }
        emitter.emit_comment(&format!("{} = {} byte(s)", init.name, len));
        let cmos = emitter.target.is_cmos();
        // Track the last value loaded into A so runs of equal bytes reload rarely.
        let mut last: Option<u8> = None;
        for (i, b) in init.bytes.iter().enumerate() {
            let addr = format!("${:04X}", init.addr as usize + i);
            match b {
                // On the 65C02 a zero byte is a bare `STZ` that leaves A (and so
                // the `last` cache for surrounding non-zero bytes) untouched.
                InitByte::Byte(0) if cmos => {
                    emitter.emit_inst("STZ", &addr);
                    continue;
                }
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
                // Rewritten to a label pair by `resolve_string_inits` before
                // anything is emitted. Reaching one here means that pass was
                // skipped, which would otherwise put a byte of nothing in RAM.
                InitByte::StrLow(t) | InitByte::StrHigh(t) => {
                    unreachable!("unresolved string initializer for {t:?}")
                }
            }
            emitter.emit_inst("STA", &addr);
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

/// Emit a const array to the data section.
fn emit_const_array(
    stat: &crate::ast::Static,
    emitter: &mut Emitter,
    info: &ProgramInfo,
    section_alloc: &mut SectionAllocator,
    _string_collector: &mut StringCollector,
) -> Result<(), CodegenError> {
    let name = &stat.name.node;

    // Take the resolved type from the symbol table rather than re-deriving a
    // width from the syntax. The old reading looked only for a primitive
    // element and fell back to 1 byte, so `[Entry; 4]` reserved four bytes for
    // four structs — the data ran off the end of its own allocation and into
    // whatever the DATA section handed out next.
    let ty = info
        .resolved_symbols
        .get(&stat.name.span)
        .or_else(|| info.table.lookup(name))
        .map(|sym| sym.ty.clone())
        .ok_or_else(|| CodegenError::SymbolNotFound(name.clone()))?;

    let byte_size = crate::sema::init::size_of(&ty, &info.type_registry);

    // Take an address from the DATA section rather than emitting at a fixed
    // .ORG. Every const array used to be written at $C000, so two of them
    // overlapped, the address ignored whatever the memory map said, and none of
    // them were visible to the #[org] conflict check.
    //
    // `#[align]` (sema has already confirmed it is a const array) rounds that
    // address up to a page boundary, so `LDA table,X` never crosses a page and
    // the element offset is the low byte. The label moves with the `.ORG`, so
    // every reference to the table still resolves through it.
    let addr = if stat.align.is_some() {
        section_alloc
            .allocate_aligned("DATA", byte_size as u16, 256)
            .map_err(CodegenError::SectionError)?
    } else {
        section_alloc
            .allocate("DATA", byte_size as u16)
            .map_err(CodegenError::SectionError)?
    };
    section_alloc.record_allocation(
        name.clone(),
        addr,
        byte_size as u16,
        AllocationSource::Section("DATA".to_string()),
        Some(stat.name.span),
    );

    emitter.emit_comment(&format!("Const array: {} ({} bytes)", name, byte_size));
    emitter.emit_data_org(addr);
    emitter.emit_data_label(name);

    // A const lives in ROM, so unlike a `static` there is no "undefined until
    // the reset handler runs" excuse for a value the compiler cannot work out.
    let bytes = crate::sema::init::flatten_top(
        &stat.init,
        &ty,
        info,
        false,
        info.soa_arrays.get(name.as_str()),
    )
    .map_err(|e| {
        CodegenError::UnsupportedOperation(format!("in const '{}': {}", name, e.message))
    })?;

    emit_init_bytes(&bytes, emitter);
    Ok(())
}

/// How codegen resolves the names inside a `const` initializer.
///
/// Semantic analysis folded every constant expression already and left the
/// results keyed by span, so this is a lookup rather than an evaluation.
impl crate::sema::init::InitContext for ProgramInfo {
    fn generated_table(&self, span: crate::ast::Span) -> Option<&[i64]> {
        self.generated_tables.get(&span).map(|v| v.as_slice())
    }

    fn registry(&self) -> &crate::sema::type_defs::TypeRegistry {
        &self.type_registry
    }

    fn integer(&self, expr: &Spanned<crate::ast::Expr>) -> Option<i64> {
        if let crate::ast::Expr::Literal(crate::ast::Literal::Integer(n)) = &expr.node {
            return Some(*n);
        }
        if let Some(crate::sema::const_eval::ConstValue::Integer(n)) =
            self.folded_constants.get(&expr.span)
        {
            return Some(*n);
        }
        // Fall back to evaluating it. A `const` array's element expressions are
        // flattened here, not type-checked as ordinary expressions, so nothing
        // has folded them and there is no entry to look up: `-5` is
        // `Unary(Neg, 5)`, not a bare literal, and `const A: [i8; 2] = [-5, 1]`
        // was rejected as "not a compile-time constant" the moment it was used.
        // The `static` path works because sema's own `InitContext` evaluates
        // rather than pattern-matching; this brings codegen's into line.
        crate::sema::const_eval::eval_const_expr_with_env(expr, &self.const_env)
            .ok()
            .and_then(|v| v.as_integer())
    }

    fn function_name(&self, expr: &Spanned<crate::ast::Expr>) -> Option<String> {
        let crate::ast::Expr::Variable(n) = &expr.node else {
            return None;
        };
        self.table
            .lookup(n)
            .filter(|s| matches!(s.ty, crate::sema::types::Type::Function(..)))
            .map(|_| n.clone())
    }

    fn address_of(
        &self,
        operand: &Spanned<crate::ast::Expr>,
    ) -> Result<u16, crate::sema::init::InitError> {
        let mut operand = operand;
        while let crate::ast::Expr::Paren(inner) = &operand.node {
            operand = inner;
        }
        let crate::ast::Expr::Variable(n) = &operand.node else {
            return Err(crate::sema::init::InitError {
                message: "only the address of a static can appear in constant data".to_string(),
                span: operand.span,
                fatal: true,
            });
        };
        match self
            .table
            .lookup(n)
            .map(|s| (s.kind.clone(), s.location.clone()))
        {
            Some((
                crate::sema::table::SymbolKind::Variable,
                crate::sema::table::SymbolLocation::Absolute(a),
            )) => Ok(a),
            _ => Err(crate::sema::init::InitError {
                message: format!(
                    "cannot take the address of '{}' in constant data; only a static has a \
                     fixed address",
                    n
                ),
                span: operand.span,
                fatal: true,
            }),
        }
    }
}

/// Emit flattened initializer bytes as assembler data.
///
/// A function reference occupies two bytes that only the assembler can fill in,
/// so an `FnLow`/`FnHigh` pair collapses back into a single `.WORD label` — the
/// same thing the interrupt vector table emits.
fn emit_init_bytes(bytes: &[crate::sema::InitByte], emitter: &mut Emitter) {
    use crate::sema::InitByte;

    let mut run: Vec<u8> = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        match &bytes[i] {
            InitByte::Byte(v) => {
                run.push(*v);
                i += 1;
            }
            InitByte::FnLow(f) => {
                emit_byte_directives(&run, emitter);
                run.clear();
                emitter.emit_data_directive(&format!(".WORD {}", f));
                // Skip the matching FnHigh; the .WORD covers both bytes.
                i += if matches!(bytes.get(i + 1), Some(InitByte::FnHigh(_))) {
                    2
                } else {
                    1
                };
            }
            InitByte::StrLow(t) | InitByte::StrHigh(t) => {
                unreachable!("unresolved string initializer for {t:?}")
            }
            InitByte::FnHigh(f) => {
                // Only reachable if a pair was split, which the flattener never
                // does; emit the high byte alone rather than losing it.
                emit_byte_directives(&run, emitter);
                run.clear();
                emitter.emit_data_directive(&format!(".BYTE >{}", f));
                i += 1;
            }
        }
    }
    emit_byte_directives(&run, emitter);
}

/// Emit a byte slice as `.BYTE` directives, collapsing long runs of zeros.
///
/// `.RES n` only advances the assembler's address, and the image starts zeroed,
/// so a run of zeros costs one line instead of `n/16`. This matters for the
/// padding a partly-written table leaves behind as much as for `[0; 256]`.
fn emit_byte_directives(bytes: &[u8], emitter: &mut Emitter) {
    const MIN_RUN: usize = 16;

    let mut i = 0;
    let mut pending: Vec<u8> = Vec::new();
    while i < bytes.len() {
        if bytes[i] == 0 {
            let run = bytes[i..].iter().take_while(|b| **b == 0).count();
            if run >= MIN_RUN {
                flush_byte_lines(&pending, emitter);
                pending.clear();
                emitter.emit_data_directive(&format!(".RES {}", run));
                i += run;
                continue;
            }
        }
        pending.push(bytes[i]);
        i += 1;
    }
    flush_byte_lines(&pending, emitter);
}

fn flush_byte_lines(bytes: &[u8], emitter: &mut Emitter) {
    for chunk in bytes.chunks(16) {
        let byte_str = chunk
            .iter()
            .map(|b| format!("${:02X}", b))
            .collect::<Vec<_>>()
            .join(", ");
        emitter.emit_data_directive(&format!(".BYTE {}", byte_str));
    }
}
