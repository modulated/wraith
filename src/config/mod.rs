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

        let config: Self =
            toml::from_str(&content).map_err(|e| format!("Failed to parse config file: {}", e))?;
        config.validate()?;
        Ok(config)
    }

    /// Check a user-supplied config for the mistakes that used to panic deep
    /// in the compiler: a section with `end < start` (the size subtraction
    /// underflows) and a `default_section` naming a section that doesn't
    /// exist.
    pub fn validate(&self) -> Result<(), String> {
        for s in &self.sections {
            if s.end < s.start {
                return Err(format!(
                    "section '{}' has end ({:#06X}) before start ({:#06X})",
                    s.name, s.end, s.start
                ));
            }
        }
        if !self.sections.iter().any(|s| s.name == self.default_section) {
            return Err(format!(
                "default_section '{}' is not one of the configured sections: [{}]",
                self.default_section,
                self.sections
                    .iter()
                    .map(|s| s.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        Ok(())
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
                Section::new("CODE", 0x8000, 0xBFFF), // 16KB for user code
                Section::new("DATA", 0xD000, 0xEFFF), // 8KB for constants/data
                // Compiler's software stack: one page of RAM used to save a
                // callee's frame across a recursive call and to spill operands.
                // Only the size (256 bytes) is fixed; the page is configurable.
                Section::new("STACK", 0x0200, 0x02FF),
                // User RAM for mutable globals (`static`): 1KB clear of the zero
                // page ($0000-$00FF), the hardware stack ($0100-$01FF) and the
                // software-stack page. Override this in wraith.toml to match a
                // different board.
                Section::new("BSS", 0x0400, 0x07FF),
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
        // Config::from_file validates this, and the built-in default is
        // well-formed, so a miss here means a MemoryConfig was constructed
        // by hand with a bad name — a compiler bug, not a user mistake.
        self.get_section(&self.default_section_name)
            .unwrap_or_else(|| {
                panic!(
                    "Default section '{}' must exist in config",
                    self.default_section_name
                )
            })
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
        // CODE (ROM), DATA (const data), STACK (software stack), BSS (RAM)
        assert_eq!(config.sections.len(), 4);
        assert_eq!(config.default_section, "CODE");
        let bss = config.sections.iter().find(|s| s.name == "BSS").unwrap();
        assert_eq!(bss.start, 0x0400);
        assert_eq!(bss.end, 0x07FF);
        let stack = config.sections.iter().find(|s| s.name == "STACK").unwrap();
        assert_eq!(stack.start, 0x0200);
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

    #[test]
    fn a_section_with_end_before_start_is_rejected() {
        // This used to panic deep in the compiler (size subtraction underflow).
        let toml = r#"
            default_section = "CODE"
            [[sections]]
            name = "CODE"
            start = 32768
            end = 16384
        "#;
        let config: Config = toml::from_str(toml).unwrap();
        let err = config.validate().unwrap_err();
        assert!(err.contains("CODE"), "{err}");
    }

    #[test]
    fn a_missing_default_section_is_rejected() {
        // This used to panic in default_section().
        let toml = r#"
            default_section = "ROM"
            [[sections]]
            name = "CODE"
            start = 32768
            end = 49151
        "#;
        let config: Config = toml::from_str(toml).unwrap();
        let err = config.validate().unwrap_err();
        assert!(err.contains("ROM"), "{err}");
    }
}
