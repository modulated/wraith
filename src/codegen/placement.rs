//! Two-phase function placement.
//!
//! Addresses used to be chosen while each function was generated, from a bump
//! pointer per section. `#[org]` bypassed that pointer entirely: it emitted at
//! the requested address without telling the allocator, which then handed the
//! same addresses out again. Pinning anything at a section's base address made
//! that section unusable for everything else — `main` at `$8000` and the very
//! next auto-allocated function both landed on `$8000`.
//!
//! Placement is now decided before anything is emitted:
//!
//! 1. **Measure** every live function, by generating it into a throwaway
//!    emitter and counting instruction bytes.
//! 2. **Reserve** the ranges claimed by `#[org]`, after checking each is
//!    somewhere the memory map accounts for.
//! 3. **Allocate** everything else into the gaps left over.
//!
//! Order matters for reproducibility, not correctness: reservations are placed
//! first, then the rest are allocated in declaration order (imported modules
//! before the root, matching emission order), so the same source always
//! produces the same layout.

use rustc_hash::FxHashMap as HashMap;

use crate::ast::{FnAttribute, Function, Item, SourceFile, Spanned};
use crate::codegen::emitter::Emitter;
use crate::codegen::section_allocator::SectionAllocator;
use crate::codegen::stmt::generate_stmt;
use crate::codegen::{CodegenError, CommentVerbosity, StringCollector};
use crate::sema::ProgramInfo;

/// Where a function ended up, and how big it is.
#[derive(Debug, Clone, Copy)]
pub struct PlacedFunction {
    pub addr: u16,
    pub size: u16,
    pub source: AllocationSourceKind,
}

/// Which rule chose a function's address. Mirrors `AllocationSource` without
/// the section name, which is recovered from the function's own metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AllocationSourceKind {
    ExplicitOrg,
    Section,
    AutoAllocated,
}

/// Every function's address and size, keyed by name.
pub type Placement = HashMap<String, PlacedFunction>;

/// The 6502 fetches NMI, RESET and IRQ from here. Nothing else may live in it.
const VECTOR_TABLE_START: u16 = 0xFFFA;

/// Decide where every function goes, before any code is emitted.
///
/// `items` is the full generation order — imported modules first, then the root
/// module — filtered to the functions that will actually be emitted.
#[allow(clippy::too_many_arguments)]
pub fn plan(
    imported: &[Spanned<Item>],
    root: &SourceFile,
    program: &ProgramInfo,
    verbosity: CommentVerbosity,
    target: crate::codegen::TargetCpu,
    layout: &crate::codegen::memory_layout::MemoryLayout,
    section_alloc: &mut SectionAllocator,
    string_collector: &mut StringCollector,
) -> Result<Placement, CodegenError> {
    // Generation order: imported items are emitted before the root module's, so
    // measuring and allocating in the same order keeps addresses in the order a
    // reader of the output expects.
    let functions: Vec<&Function> = imported
        .iter()
        .chain(root.items.iter())
        .filter(|item| crate::codegen::is_live(item, program))
        .filter_map(|item| match &item.node {
            Item::Function(f) => Some(&**f),
            _ => None,
        })
        // Inline functions are expanded at their call sites and never emitted
        // on their own, so they occupy no address.
        .filter(|f| {
            !program
                .function_metadata
                .get(&f.name.node)
                .is_some_and(|m| m.is_inline)
        })
        .collect();

    // --- Phase 1: measure -------------------------------------------------
    let mut sizes: Vec<(&Function, u16)> = Vec::with_capacity(functions.len());
    let mut seen: HashMap<String, ()> = HashMap::default();
    for func in functions {
        // A name can appear twice when a module is reachable by two import
        // paths; only the first copy is emitted.
        if seen.insert(func.name.node.clone(), ()).is_some() {
            continue;
        }
        let size = measure(func, program, verbosity, target, layout, string_collector)?;
        sizes.push((func, size));
    }

    // --- Phase 2: reserve the pinned ranges -------------------------------
    let mut placement = Placement::default();
    for (func, size) in &sizes {
        let name = &func.name.node;
        let Some(org_addr) = program
            .function_metadata
            .get(name)
            .and_then(|m| m.org_address)
        else {
            continue;
        };
        let org_end = org_addr.saturating_add(size.saturating_sub(1));

        check_org_is_placeable(func, org_addr, org_end, section_alloc)?;

        section_alloc.reserve(name.clone(), org_addr, org_end);
        placement.insert(
            name.clone(),
            PlacedFunction {
                addr: org_addr,
                size: *size,
                source: AllocationSourceKind::ExplicitOrg,
            },
        );
    }

    // --- Phase 3: allocate the rest into what is left ---------------------
    for (func, size) in &sizes {
        let name = &func.name.node;
        if placement.contains_key(name) {
            continue; // pinned above
        }
        let section = program
            .function_metadata
            .get(name)
            .and_then(|m| m.section.clone());

        let (addr, source) = match &section {
            Some(section_name) => (
                section_alloc
                    .allocate(section_name, *size)
                    .map_err(CodegenError::SectionError)?,
                AllocationSourceKind::Section,
            ),
            None => (
                section_alloc
                    .allocate_default(*size)
                    .map_err(CodegenError::SectionError)?,
                AllocationSourceKind::AutoAllocated,
            ),
        };

        placement.insert(
            name.clone(),
            PlacedFunction {
                addr,
                size: *size,
                source,
            },
        );
    }

    Ok(placement)
}

/// Reject an `#[org]` that cannot work before anything is reserved for it.
fn check_org_is_placeable(
    func: &Function,
    org_addr: u16,
    org_end: u16,
    section_alloc: &SectionAllocator,
) -> Result<(), CodegenError> {
    let name = &func.name.node;

    // Checked before the section test: the vectors sit outside every section in
    // the default map, so the generic "not inside any section" message would be
    // technically right and would bury the part that matters — and a board whose
    // config *does* cover $FFFA would otherwise get no warning at all until it
    // failed to boot.
    if org_end >= VECTOR_TABLE_START {
        return Err(CodegenError::AddressConflict {
            message: format!("#[org] places '{}' over the interrupt vector table", name),
            notes: vec![
                format!("'{}' would occupy ${:04X}-${:04X}", name, org_addr, org_end),
                "the 6502 reads its NMI, RESET and IRQ vectors from $FFFA-$FFFF".to_string(),
                "code written there replaces the reset vector, so the machine will not start"
                    .to_string(),
            ],
            span: Some(func.name.span),
        });
    }

    if section_alloc
        .containing_section(org_addr, org_end)
        .is_none()
    {
        let sections = section_alloc
            .section_summary()
            .unwrap_or_else(|| "none configured".to_string());
        return Err(CodegenError::AddressConflict {
            message: format!(
                "#[org] address ${:04X} for '{}' is not inside any configured section",
                org_addr, name
            ),
            notes: vec![
                format!("'{}' would occupy ${:04X}-${:04X}", name, org_addr, org_end),
                format!("configured sections: {}", sections),
                "add a section covering this range to wraith.toml, or move the #[org]".to_string(),
            ],
            span: Some(func.name.span),
        });
    }

    // Two pinned functions cannot both be moved, so an overlap between them is
    // reported here rather than left to the post-generation conflict check —
    // the reservation list would otherwise hold two overlapping ranges and the
    // gap arithmetic would be meaningless.
    if let Some(clash) = section_alloc
        .reserved_ranges()
        .iter()
        .find(|r| org_addr <= r.end && org_end >= r.start)
    {
        return Err(CodegenError::AddressConflict {
            message: format!("'{}' overlaps '{}'", name, clash.owner),
            notes: vec![
                format!("'{}' would occupy ${:04X}-${:04X}", name, org_addr, org_end),
                format!(
                    "'{}' is pinned to ${:04X}-${:04X} (explicit #[org])",
                    clash.owner, clash.start, clash.end
                ),
                "both addresses are fixed, so neither can be moved out of the way".to_string(),
            ],
            span: Some(func.name.span),
        });
    }

    Ok(())
}

/// Measure a function by generating it into a throwaway emitter.
///
/// The emitter starts fresh, which is what the real one looks like at this
/// point too: every function emits its own label first, and `emit_label`
/// invalidates all register tracking. Seeding it from the previous function's
/// leftover state — as this did when it ran inline — could elide a load the
/// real pass then emits, measuring the function short.
fn measure(
    func: &Function,
    info: &ProgramInfo,
    verbosity: CommentVerbosity,
    target: crate::codegen::TargetCpu,
    layout: &crate::codegen::memory_layout::MemoryLayout,
    string_collector: &mut StringCollector,
) -> Result<u16, CodegenError> {
    let name = &func.name.node;

    let is_interrupt = func.attributes.iter().any(|attr| {
        matches!(
            attr,
            FnAttribute::Interrupt | FnAttribute::Nmi | FnAttribute::Irq
        )
    });
    let interrupt_zp = if is_interrupt {
        super::item::interrupt_zp_save_addrs(info, name)
    } else {
        Vec::new()
    };

    let mut e = Emitter::new(verbosity);
    e.target = target;
    e.memory_layout = layout.clone();
    e.set_current_function(name.clone());

    // The reset prologue (stack-pointer init + static initializers) is emitted
    // by the real pass before anything else; it must be measured too. Shared
    // helper so the two cannot drift.
    let is_reset = func
        .attributes
        .iter()
        .any(|attr| matches!(attr, FnAttribute::Reset));
    if is_reset {
        super::item::emit_reset_prologue(&mut e, info);
    }

    if is_interrupt {
        e.emit_inst("PHA", "");
        e.emit_inst("TXA", "");
        e.emit_inst("PHA", "");
        e.emit_inst("TYA", "");
        e.emit_inst("PHA", "");
        super::item::emit_interrupt_zp_save(&mut e, &interrupt_zp);
    }

    // The function-pointer prologue, which must match the real emit.
    if info.address_taken_functions.contains(name)
        && let Some(frame) = info.function_frames.get(name)
    {
        for _ in 0..frame.param_size {
            e.emit_inst("LDA", "$E0");
            e.emit_inst("STA", "$40");
        }
    }

    generate_stmt(&func.body, &mut e, info, string_collector)?;

    if is_interrupt {
        super::item::emit_interrupt_zp_restore(&mut e, &interrupt_zp);
        e.emit_inst("PLA", "");
        e.emit_inst("TAY", "");
        e.emit_inst("PLA", "");
        e.emit_inst("TAX", "");
        e.emit_inst("PLA", "");
        e.emit_inst("RTI", "");
    } else if !e.last_was_terminal() {
        e.emit_inst("RTS", "");
    }

    // Ten bytes of slack, kept from the original measurement: the peephole pass
    // only ever removes instructions, but the real emit can differ by a few
    // bytes where the measuring pass cannot reproduce the surrounding context
    // exactly.
    Ok(e.byte_count() + 10)
}
