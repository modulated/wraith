//! Configuration and memory layout

/// Memory section definition
#[derive(Debug, Clone)]
pub struct Section {
    pub name: String,
    pub start: u16,
    pub end: u16,
}

impl Section {
    pub fn new(name: impl Into<String>, start: u16, end: u16) -> Self {
        Self {
            name: name.into(),
            start,
            end,
        }
    }

    pub fn size(&self) -> usize {
        (self.end - self.start + 1) as usize
    }

    pub fn contains(&self, addr: u16) -> bool {
        addr >= self.start && addr <= self.end
    }
}

/// Memory layout configuration
#[derive(Debug, Clone)]
pub struct MemoryConfig {
    pub sections: Vec<Section>,
}

impl MemoryConfig {
    /// Create default memory layout for 6502
    pub fn default_6502() -> Self {
        Self {
            sections: vec![
                Section::new("STDLIB", 0x8000, 0x8FFF), // 4KB for standard library
                Section::new("CODE", 0x9000, 0xBFFF),   // 12KB for user code
                Section::new("DATA", 0xC000, 0xCFFF),   // 4KB for constants/data
            ],
        }
    }

    /// Get a section by name
    pub fn get_section(&self, name: &str) -> Option<&Section> {
        self.sections.iter().find(|s| s.name == name)
    }

    /// Get the default section for functions without explicit section or org
    pub fn default_section(&self) -> &Section {
        // Default to CODE section
        self.get_section("CODE")
            .expect("Default CODE section must exist")
    }
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self::default_6502()
    }
}
