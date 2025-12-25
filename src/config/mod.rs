//! Configuration and memory layout

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

/// Memory section definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Section {
    pub name: String,
    pub start: u16,
    pub end: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

impl Section {
    pub fn new(name: impl Into<String>, start: u16, end: u16) -> Self {
        Self {
            name: name.into(),
            start,
            end,
            description: None,
        }
    }

    pub fn size(&self) -> usize {
        (self.end - self.start + 1) as usize
    }

    pub fn contains(&self, addr: u16) -> bool {
        addr >= self.start && addr <= self.end
    }
}

/// Configuration file structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub sections: Vec<Section>,
    #[serde(default = "default_section_name")]
    pub default_section: String,
}

fn default_section_name() -> String {
    "CODE".to_string()
}

impl Config {
    /// Load configuration from a TOML file
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, String> {
        let content = fs::read_to_string(path.as_ref())
            .map_err(|e| format!("Failed to read config file: {}", e))?;

        toml::from_str(&content)
            .map_err(|e| format!("Failed to parse config file: {}", e))
    }

    /// Try to load from wraith.toml in current directory, fall back to defaults
    pub fn load_or_default() -> Self {
        // Try to load from wraith.toml in current directory
        if let Ok(config) = Self::from_file("wraith.toml") {
            return config;
        }

        // Fall back to defaults
        Self::default()
    }

    /// Create default configuration for 6502
    pub fn default_6502() -> Self {
        Self {
            sections: vec![
                Section::new("STDLIB", 0xC000, 0xCFFF), // 4KB for standard library
                Section::new("CODE", 0x8000, 0xBFFF),   // 16KB for user code
                Section::new("DATA", 0xD000, 0xEFFF),   // 8KB for constants/data
            ],
            default_section: "CODE".to_string(),
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self::default_6502()
    }
}

/// Memory layout configuration
#[derive(Debug, Clone)]
pub struct MemoryConfig {
    pub sections: Vec<Section>,
    pub default_section_name: String,
}

impl MemoryConfig {
    /// Create from a Config
    pub fn from_config(config: Config) -> Self {
        Self {
            sections: config.sections,
            default_section_name: config.default_section,
        }
    }

    /// Load from wraith.toml or use defaults
    pub fn load_or_default() -> Self {
        Self::from_config(Config::load_or_default())
    }

    /// Create default memory layout for 6502
    pub fn default_6502() -> Self {
        Self::from_config(Config::default_6502())
    }

    /// Get a section by name
    pub fn get_section(&self, name: &str) -> Option<&Section> {
        self.sections.iter().find(|s| s.name == name)
    }

    /// Get the default section for functions without explicit section or org
    pub fn default_section(&self) -> &Section {
        // Use configured default section
        self.get_section(&self.default_section_name)
            .unwrap_or_else(|| panic!("Default section '{}' must exist in config", self.default_section_name))
    }
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self::default_6502()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert_eq!(config.sections.len(), 3);
        assert_eq!(config.default_section, "CODE");
    }

    #[test]
    fn test_section_size() {
        let section = Section::new("TEST", 0x8000, 0x8FFF);
        assert_eq!(section.size(), 4096);
    }

    #[test]
    fn test_section_contains() {
        let section = Section::new("TEST", 0x8000, 0x8FFF);
        assert!(section.contains(0x8000));
        assert!(section.contains(0x8FFF));
        assert!(!section.contains(0x7FFF));
        assert!(!section.contains(0x9000));
    }

    #[test]
    fn test_memory_config_default_section() {
        let config = MemoryConfig::default();
        let default = config.default_section();
        assert_eq!(default.name, "CODE");
        assert_eq!(default.start, 0x8000);
    }
}
