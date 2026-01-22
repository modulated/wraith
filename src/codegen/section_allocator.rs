//! Section-based memory allocation
//!
//! Manages allocation of addresses within memory sections.

use crate::config::{MemoryConfig, Section};
use std::collections::HashMap;

/// Information about an allocated address range
#[derive(Debug, Clone)]
pub struct Allocation {
    pub start: u16,
    pub end: u16,
    pub name: String,
    pub source: AllocationSource,
}

#[derive(Debug, Clone)]
pub enum AllocationSource {
    ExplicitOrg,
    Section(String),
    AutoAllocated,
}

impl std::fmt::Display for AllocationSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AllocationSource::ExplicitOrg => write!(f, "explicit #[org]"),
            AllocationSource::Section(s) => write!(f, "section '{}'", s),
            AllocationSource::AutoAllocated => write!(f, "auto-allocated"),
        }
    }
}

/// Tracks current offset within each section
pub struct SectionAllocator {
    config: MemoryConfig,
    /// Current offset within each section (relative to section start)
    offsets: HashMap<String, u16>,
    /// All allocations made (for conflict detection)
    pub allocations: Vec<Allocation>,
}

impl SectionAllocator {
    pub fn new(config: MemoryConfig) -> Self {
        let mut offsets = HashMap::new();
        for section in &config.sections {
            offsets.insert(section.name.clone(), 0);
        }
        Self {
            config,
            offsets,
            allocations: Vec::new(),
        }
    }

    /// Allocate space in a specific section, returning the absolute address
    pub fn allocate(&mut self, section_name: &str, size: u16) -> Result<u16, String> {
        let section = self
            .config
            .get_section(section_name)
            .ok_or_else(|| format!("Unknown section: {}", section_name))?;

        let offset = self.offsets.get_mut(section_name).unwrap();
        let addr = section.start + *offset;

        // Check if allocation would overflow section
        // Use u32 arithmetic to avoid overflow on large allocations
        let end_addr = addr as u32 + size as u32 - 1;
        if end_addr > section.end as u32 {
            return Err(format!(
                "Section '{}' overflow: tried to allocate {} bytes at ${:04X}, but section ends at ${:04X}",
                section_name, size, addr, section.end
            ));
        }

        *offset += size;
        Ok(addr)
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
    pub fn record_allocation(
        &mut self,
        name: String,
        start: u16,
        size: u16,
        source: AllocationSource,
    ) {
        let end = start.saturating_add(size).saturating_sub(1);
        self.allocations.push(Allocation {
            start,
            end,
            name,
            source,
        });
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
