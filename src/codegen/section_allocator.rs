//! Section-based memory allocation
//!
//! Manages allocation of addresses within memory sections.

use crate::config::{MemoryConfig, Section};
use std::collections::HashMap;

/// Tracks current offset within each section
pub struct SectionAllocator {
    config: MemoryConfig,
    /// Current offset within each section (relative to section start)
    offsets: HashMap<String, u16>,
}

impl SectionAllocator {
    pub fn new(config: MemoryConfig) -> Self {
        let mut offsets = HashMap::new();
        for section in &config.sections {
            offsets.insert(section.name.clone(), 0);
        }
        Self { config, offsets }
    }

    /// Allocate space in a specific section, returning the absolute address
    pub fn allocate(&mut self, section_name: &str, size: u16) -> Result<u16, String> {
        let section = self.config.get_section(section_name)
            .ok_or_else(|| format!("Unknown section: {}", section_name))?;

        let offset = self.offsets.get_mut(section_name).unwrap();
        let addr = section.start + *offset;

        // Check if allocation would overflow section
        if addr + size - 1 > section.end {
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
}

impl Default for SectionAllocator {
    fn default() -> Self {
        Self::new(MemoryConfig::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_section_allocation() {
        let mut alloc = SectionAllocator::default();

        // Allocate in STDLIB section
        let addr1 = alloc.allocate("STDLIB", 100).unwrap();
        assert_eq!(addr1, 0x8000);

        let addr2 = alloc.allocate("STDLIB", 50).unwrap();
        assert_eq!(addr2, 0x8064); // 0x8000 + 100

        // Allocate in CODE section
        let addr3 = alloc.allocate("CODE", 200).unwrap();
        assert_eq!(addr3, 0x9000);
    }

    #[test]
    fn test_section_overflow() {
        let mut alloc = SectionAllocator::default();

        // STDLIB is 0x8000-0x8FFF (4096 bytes)
        let result = alloc.allocate("STDLIB", 5000);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("overflow"));
    }

    #[test]
    fn test_remaining_space() {
        let mut alloc = SectionAllocator::default();

        let initial = alloc.remaining("STDLIB").unwrap();
        assert_eq!(initial, 4096); // 0x1000

        alloc.allocate("STDLIB", 1000).unwrap();
        let after = alloc.remaining("STDLIB").unwrap();
        assert_eq!(after, 4096 - 1000);
    }
}
