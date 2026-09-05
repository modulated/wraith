//! Section-based memory allocation
//!
//! Manages allocation of addresses within memory sections.

use crate::ast::Span;
use crate::config::{MemoryConfig, Section};
use rustc_hash::FxHashMap as HashMap;

/// Information about an allocated address range
#[derive(Debug, Clone)]
pub struct Allocation {
    pub start: u16,
    pub end: u16,
    pub name: String,
    pub source: AllocationSource,
    /// Where the item was declared, when it came from source. `None` for
    /// compiler-reserved regions such as the interrupt vector table.
    pub span: Option<Span>,
}

#[derive(Debug, Clone)]
pub enum AllocationSource {
    ExplicitOrg,
    Section(String),
    AutoAllocated,
    /// A region the compiler must place at a fixed address the hardware
    /// defines, such as the 6502 interrupt vector table at $FFFA.
    Reserved,
}

impl AllocationSource {
    /// Was this address chosen by the programmer rather than the allocator?
    ///
    /// Two auto-allocated items can never overlap — the allocator hands out
    /// addresses in sequence — so any real conflict involves at least one of
    /// these, and the diagnostic should point at it.
    pub fn is_fixed(&self) -> bool {
        matches!(self, AllocationSource::ExplicitOrg)
    }
}

impl std::fmt::Display for AllocationSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AllocationSource::ExplicitOrg => write!(f, "explicit #[org]"),
            AllocationSource::Section(s) => write!(f, "section '{}'", s),
            AllocationSource::AutoAllocated => write!(f, "auto-allocated"),
            AllocationSource::Reserved => write!(f, "reserved by the hardware"),
        }
    }
}

/// A range pinned by `#[org]`, and the function that pinned it.
#[derive(Debug, Clone)]
pub struct Reservation {
    pub start: u16,
    pub end: u16,
    pub owner: String,
}

/// Tracks current offset within each section
pub struct SectionAllocator {
    config: MemoryConfig,
    /// Current offset within each section (relative to section start)
    offsets: HashMap<String, u16>,
    /// All allocations made (for conflict detection)
    pub allocations: Vec<Allocation>,
    /// Address ranges claimed by `#[org]`, sorted by start and non-overlapping.
    /// Allocation steps over these rather than handing out an address inside
    /// one; without that, pinning a function to a section's base address made
    /// the section unusable for everything else.
    reserved: Vec<Reservation>,
}

impl SectionAllocator {
    pub fn new(config: MemoryConfig) -> Self {
        let mut offsets = HashMap::default();
        for section in &config.sections {
            offsets.insert(section.name.clone(), 0);
        }
        Self {
            config,
            offsets,
            allocations: Vec::new(),
            reserved: Vec::new(),
        }
    }

    /// Claim `start..=end` for `owner` so no later allocation is placed inside.
    ///
    /// Reservations must all be made before any allocation: the cursor only
    /// moves forward, so a range reserved after an allocation has already been
    /// handed out past it cannot be honoured.
    pub fn reserve(&mut self, owner: String, start: u16, end: u16) {
        let pos = self.reserved.partition_point(|r| r.start < start);
        self.reserved.insert(pos, Reservation { start, end, owner });
    }

    /// Ranges currently reserved, in address order.
    pub fn reserved_ranges(&self) -> &[Reservation] {
        &self.reserved
    }

    /// The first address at or after `addr` where `size` bytes fit without
    /// touching a reserved range.
    ///
    /// Stepping past one reservation can land inside the next, so this repeats
    /// until the range is clear. Reservations are sorted and finite, and each
    /// step moves strictly forward, so it terminates.
    fn skip_reserved(&self, mut addr: u16, size: u16) -> Option<u16> {
        loop {
            let end = addr as u32 + size as u32 - 1;
            let clash = self
                .reserved
                .iter()
                .find(|r| addr <= r.end && end >= r.start as u32);
            match clash {
                // Resuming past a reservation that ends at $FFFF would wrap.
                Some(r) if r.end == u16::MAX => return None,
                Some(r) => addr = r.end + 1,
                None => return Some(addr),
            }
        }
    }

    /// Allocate space in a specific section, returning the absolute address
    pub fn allocate(&mut self, section_name: &str, size: u16) -> Result<u16, String> {
        let section = self
            .config
            .get_section(section_name)
            .ok_or_else(|| format!("Unknown section: {}", section_name))?;
        let (sec_start, sec_end) = (section.start, section.end);

        let cursor = *self.offsets.get(section_name).unwrap();
        let addr = sec_start + cursor;

        let addr = self.skip_reserved(addr, size).ok_or_else(|| {
            format!(
                "Section '{}' has no {} free bytes left: every remaining range is \
                 claimed by an explicit #[org]",
                section_name, size
            )
        })?;

        // Check if allocation would overflow section
        // Use u32 arithmetic to avoid overflow on large allocations
        let end_addr = addr as u32 + size as u32 - 1;
        if end_addr > sec_end as u32 {
            let reserved_here: u32 = self
                .reserved
                .iter()
                .filter(|r| r.start >= sec_start && r.end <= sec_end)
                .map(|r| (r.end as u32 - r.start as u32) + 1)
                .sum();
            let detail = if reserved_here > 0 {
                format!(
                    " ({} byte{} of it reserved by explicit #[org] placement)",
                    reserved_here,
                    if reserved_here == 1 { "" } else { "s" }
                )
            } else {
                String::new()
            };
            return Err(format!(
                "Section '{}' overflow: tried to allocate {} bytes at ${:04X}, but section ends at ${:04X}{}",
                section_name, size, addr, sec_end, detail
            ));
        }

        // The cursor is relative to the section start and only moves forward,
        // so skipping a reservation is recorded here rather than re-derived.
        *self.offsets.get_mut(section_name).unwrap() = (addr - sec_start) + size;
        Ok(addr)
    }

    /// Allocate `size` bytes in a section at an address that is a multiple of
    /// `align`, skipping the padding before it. Used by `#[align]` to page-align
    /// a const table so indexed reads never cross a page boundary.
    ///
    /// Implemented by advancing the section cursor to the aligned boundary and
    /// then allocating normally, so the reserved-range and overflow handling is
    /// the same as [`allocate`](Self::allocate). (A reservation landing exactly
    /// on the boundary could nudge the result off it; that does not happen for
    /// DATA, which carries no `#[org]` reservations.)
    pub fn allocate_aligned(
        &mut self,
        section_name: &str,
        size: u16,
        align: u16,
    ) -> Result<u16, String> {
        let section = self
            .config
            .get_section(section_name)
            .ok_or_else(|| format!("Unknown section: {}", section_name))?;
        let (sec_start, sec_end) = (section.start, section.end);

        let cursor = *self.offsets.get(section_name).unwrap();
        let addr = sec_start as u32 + cursor as u32;
        let align = align.max(1) as u32;
        let aligned = addr.div_ceil(align) * align;
        if aligned + size.max(1) as u32 - 1 > sec_end as u32 {
            return Err(format!(
                "Section '{}' overflow: aligning {} bytes to a {}-byte boundary at ${:04X} \
                 runs past the section end ${:04X}",
                section_name, size, align, aligned, sec_end
            ));
        }
        *self.offsets.get_mut(section_name).unwrap() = (aligned as u16) - sec_start;
        self.allocate(section_name, size)
    }

    /// Allocate in the default section (CODE)
    pub fn allocate_default(&mut self, size: u16) -> Result<u16, String> {
        let default_section = self.config.default_section().clone();
        self.allocate(&default_section.name, size)
    }

    /// Get section info
    pub fn get_section(&self, name: &str) -> Option<&Section> {
        self.config.get_section(name)
    }

    /// Get remaining space in a section
    pub fn remaining(&self, section_name: &str) -> Option<usize> {
        let section = self.config.get_section(section_name)?;
        let offset = *self.offsets.get(section_name)?;
        Some((section.end - section.start + 1 - offset) as usize)
    }

    /// Record an allocation (for conflict detection)
    ///
    /// Everything that occupies address space in the output must be recorded,
    /// not just functions: an `#[org]` that lands on a string table or on the
    /// interrupt vectors is as broken as one that lands on another function,
    /// and is harder to spot by eye.
    pub fn record_allocation(
        &mut self,
        name: String,
        start: u16,
        size: u16,
        source: AllocationSource,
        span: Option<Span>,
    ) {
        // A zero-length item occupies nothing and cannot collide; recording it
        // as `start..=start-1` would wrap and produce a bogus conflict.
        if size == 0 {
            return;
        }
        let end = start.saturating_add(size).saturating_sub(1);
        self.allocations.push(Allocation {
            start,
            end,
            name,
            source,
            span,
        });
    }

    /// A short `NAME $start-$end` list of the configured sections, for
    /// diagnostics that need to say where an address could legally go.
    pub fn section_summary(&self) -> Option<String> {
        if self.config.sections.is_empty() {
            return None;
        }
        Some(
            self.config
                .sections
                .iter()
                .map(|s| format!("{} ${:04X}-${:04X}", s.name, s.start, s.end))
                .collect::<Vec<_>>()
                .join(", "),
        )
    }

    /// The configured section wholly containing `start..=end`, if any.
    ///
    /// A range that falls outside every section, or straddles the boundary
    /// between two, has no section to be checked against — capacity accounting
    /// does not see it, so it can silently overrun into whatever follows.
    pub fn containing_section(&self, start: u16, end: u16) -> Option<&Section> {
        self.config
            .sections
            .iter()
            .find(|s| start >= s.start && end <= s.end)
    }

    /// Check for conflicts in all recorded allocations
    pub fn check_conflicts(&self) -> Vec<(Allocation, Allocation)> {
        let mut conflicts = Vec::new();

        for (i, alloc1) in self.allocations.iter().enumerate() {
            for alloc2 in self.allocations.iter().skip(i + 1) {
                // Check if ranges overlap
                if !(alloc1.end < alloc2.start || alloc1.start > alloc2.end) {
                    conflicts.push((alloc1.clone(), alloc2.clone()));
                }
            }
        }

        conflicts
    }

    /// Get usage statistics for all sections
    pub fn get_statistics(&self) -> Vec<SectionStats> {
        self.config
            .sections
            .iter()
            .map(|section| {
                let used = *self.offsets.get(&section.name).unwrap_or(&0);
                let total = section.end - section.start + 1;
                SectionStats {
                    name: section.name.clone(),
                    used,
                    total,
                }
            })
            .collect()
    }
}

/// Statistics for a memory section
#[derive(Debug, Clone)]
pub struct SectionStats {
    pub name: String,
    pub used: u16,
    pub total: u16,
}

impl SectionStats {
    /// Get usage percentage
    pub fn percentage(&self) -> f32 {
        if self.total == 0 {
            0.0
        } else {
            (self.used as f32 / self.total as f32) * 100.0
        }
    }

    /// Format as human-readable string
    pub fn format(&self) -> String {
        format!(
            "{:8} - {:5}/{:5} bytes ({:5.2}%)",
            self.name,
            self.used,
            self.total,
            self.percentage()
        )
    }

    /// Format compact version (without section name, for use with labeled output)
    pub fn format_compact(&self) -> String {
        let used_kb = self.used as f32 / 1024.0;
        let total_kb = self.total as f32 / 1024.0;
        format!(
            "{:5.2}/{:5.2} KB ({:5.2}%)",
            used_kb,
            total_kb,
            self.percentage()
        )
    }
}

impl Default for SectionAllocator {
    fn default() -> Self {
        Self::new(MemoryConfig::load_or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_section_allocation() {
        let mut alloc = SectionAllocator::default();

        // Allocate in CODE section (0x8000-0xBFFF)
        let addr3 = alloc.allocate("CODE", 200).unwrap();
        assert_eq!(addr3, 0x8000);
    }

    #[test]
    fn test_section_overflow() {
        let mut alloc = SectionAllocator::default();

        // CODE is 0x8000-0xBFFF (16384 bytes)
        let result = alloc.allocate("CODE", 50000);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("overflow"));
    }

    #[test]
    fn test_remaining_space() {
        let mut alloc = SectionAllocator::default();

        let initial = alloc.remaining("CODE").unwrap();
        assert_eq!(initial, 16384); // 0x4000 (16KB)

        alloc.allocate("CODE", 1000).unwrap();
        let after = alloc.remaining("CODE").unwrap();
        assert_eq!(after, 16384 - 1000);
    }
}

#[cfg(test)]
mod reservation_tests {
    use super::*;
    use crate::config::{MemoryConfig, Section};

    /// A single 256-byte section at $8000, small enough to exhaust in a test.
    fn tiny() -> SectionAllocator {
        SectionAllocator::new(MemoryConfig {
            sections: vec![Section::new("CODE", 0x8000, 0x80FF)],
            ..Default::default()
        })
    }

    #[test]
    fn allocation_starts_after_a_reservation_at_the_section_base() {
        // The case that made #[org] unusable: pinning something at the base
        // used to hand the base straight back out to the next allocation.
        let mut a = tiny();
        a.reserve("pinned".into(), 0x8000, 0x800F);
        assert_eq!(a.allocate("CODE", 16).unwrap(), 0x8010);
    }

    #[test]
    fn a_gap_large_enough_is_used() {
        let mut a = tiny();
        a.reserve("pinned".into(), 0x8010, 0x801F);
        // 16 bytes fit exactly in $8000-$800F, before the reservation.
        assert_eq!(a.allocate("CODE", 16).unwrap(), 0x8000);
        // The next allocation has to step over the reservation.
        assert_eq!(a.allocate("CODE", 4).unwrap(), 0x8020);
    }

    #[test]
    fn a_gap_one_byte_too_small_is_skipped() {
        let mut a = tiny();
        a.reserve("pinned".into(), 0x8010, 0x801F);
        // 17 bytes cannot fit in the 16-byte gap at $8000, so the whole
        // allocation moves past the reservation rather than overrunning it.
        assert_eq!(a.allocate("CODE", 17).unwrap(), 0x8020);
    }

    #[test]
    fn consecutive_reservations_are_stepped_over_together() {
        // Clearing one reservation can land inside the next, so the search has
        // to repeat rather than step once.
        let mut a = tiny();
        a.reserve("first".into(), 0x8000, 0x800F);
        a.reserve("second".into(), 0x8010, 0x801F);
        a.reserve("third".into(), 0x8020, 0x802F);
        assert_eq!(a.allocate("CODE", 8).unwrap(), 0x8030);
    }

    #[test]
    fn reservations_are_kept_in_address_order_regardless_of_insertion_order() {
        let mut a = tiny();
        a.reserve("late".into(), 0x8080, 0x808F);
        a.reserve("early".into(), 0x8000, 0x800F);
        a.reserve("middle".into(), 0x8040, 0x804F);
        let starts: Vec<u16> = a.reserved_ranges().iter().map(|r| r.start).collect();
        assert_eq!(starts, vec![0x8000, 0x8040, 0x8080]);
    }

    #[test]
    fn a_section_filled_by_reservations_reports_why() {
        let mut a = tiny();
        a.reserve("hog".into(), 0x8000, 0x80FF);
        let err = a.allocate("CODE", 1).unwrap_err();
        assert!(
            err.contains("#[org]"),
            "the error should blame the reservations, not the size: {err}"
        );
    }

    #[test]
    fn overflowing_a_partly_reserved_section_says_how_much_is_pinned() {
        let mut a = tiny();
        a.reserve("pinned".into(), 0x8000, 0x807F); // 128 of 256 bytes
        let err = a.allocate("CODE", 200).unwrap_err();
        assert!(err.contains("overflow"), "{err}");
        assert!(err.contains("128 bytes"), "{err}");
    }

    #[test]
    fn a_reservation_reaching_the_top_of_memory_does_not_wrap() {
        // Resuming at `end + 1` would wrap to $0000 and hand out an address
        // below the section, which the bounds check would then accept.
        let mut a = SectionAllocator::new(MemoryConfig {
            sections: vec![Section::new("CODE", 0xFF00, 0xFFFF)],
            ..Default::default()
        });
        a.reserve("top".into(), 0xFF00, 0xFFFF);
        assert!(a.allocate("CODE", 1).is_err());
    }

    #[test]
    fn allocation_is_unaffected_when_nothing_is_reserved() {
        let mut a = tiny();
        assert_eq!(a.allocate("CODE", 16).unwrap(), 0x8000);
        assert_eq!(a.allocate("CODE", 16).unwrap(), 0x8010);
    }

    /// A four-page section, to watch `allocate_aligned` skip to a page boundary.
    fn four_pages() -> SectionAllocator {
        SectionAllocator::new(MemoryConfig {
            sections: vec![Section::new("DATA", 0xD000, 0xD3FF)],
            default_section_name: "DATA".to_string(),
        })
    }

    #[test]
    fn aligned_allocation_rounds_up_to_the_next_page() {
        let mut a = four_pages();
        // A short table lands at the base, mid-page after it.
        assert_eq!(a.allocate("DATA", 3).unwrap(), 0xD000);
        // The aligned one skips the rest of the page to $D100.
        assert_eq!(a.allocate_aligned("DATA", 4, 256).unwrap(), 0xD100);
        // A following plain allocation packs right after it, no more padding.
        assert_eq!(a.allocate("DATA", 2).unwrap(), 0xD104);
    }

    #[test]
    fn aligned_allocation_at_a_boundary_does_not_move() {
        let mut a = four_pages();
        // Cursor already on a page boundary ($D000): alignment is a no-op.
        assert_eq!(a.allocate_aligned("DATA", 8, 256).unwrap(), 0xD000);
    }

    #[test]
    fn aligned_allocation_reports_overflow_past_the_section() {
        let mut a = four_pages();
        // Fill into the last page, then ask to align past the end.
        assert_eq!(a.allocate("DATA", 0x301).unwrap(), 0xD000); // through $D300
        assert!(a.allocate_aligned("DATA", 4, 256).is_err());
    }
}
