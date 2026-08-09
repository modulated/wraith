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

    /// Reject a malformed memory map before it can miscompile silently.
    ///
    /// Placement trusts these sections completely — an overlap or a section laid
    /// over a hardware-fixed region does not error at placement time, it just
    /// puts two things at one address (or over the reset vector). Catch the
    /// structural mistakes here, at load, where the message can name the file.
    pub fn validate(&self) -> Result<(), String> {
        // Regions the 6502 fixes in hardware; nothing a section reserves may sit
        // on top of them. (The compiler's zero-page scratch is not configurable,
        // so a section straying into the zero page is always a mistake too.)
        const RESERVED: [(&str, u16, u16); 3] = [
            ("the zero page", 0x0000, 0x00FF),
            ("the hardware stack", 0x0100, 0x01FF),
            ("the interrupt vector table", 0xFFFA, 0xFFFF),
        ];
        // The software stack is a fixed one-page (256-byte) block addressed
        // `base,X` off an 8-bit pointer, so a smaller `STACK` section is silently
        // overrun. Kept in sync with `codegen::memory_layout::SOFTWARE_STACK_BYTES`.
        const SOFTWARE_STACK_BYTES: usize = 256;

        for s in &self.sections {
            if s.end < s.start {
                return Err(format!(
                    "section '{}' has end ({:#06X}) before start ({:#06X})",
                    s.name, s.end, s.start
                ));
            }
            for (what, lo, hi) in RESERVED {
                if s.start <= hi && s.end >= lo {
                    return Err(format!(
                        "section '{}' (${:04X}-${:04X}) overlaps {} (${:04X}-${:04X}), which is \
                         fixed by the hardware and cannot be reused",
                        s.name, s.start, s.end, what, lo, hi
                    ));
                }
            }
        }

        // Duplicate names: `get_section` returns the first match, so a second
        // definition is silently ignored — the addresses you think you set are
        // not the ones in effect.
        for (i, a) in self.sections.iter().enumerate() {
            if self.sections[..i].iter().any(|b| b.name == a.name) {
                return Err(format!("section '{}' is defined more than once", a.name));
            }
        }

        // Overlapping sections: two things placed at one address.
        for (i, a) in self.sections.iter().enumerate() {
            for b in &self.sections[..i] {
                if a.start <= b.end && a.end >= b.start {
                    return Err(format!(
                        "sections '{}' (${:04X}-${:04X}) and '{}' (${:04X}-${:04X}) overlap",
                        a.name, a.start, a.end, b.name, b.start, b.end
                    ));
                }
            }
        }

        if let Some(stack) = self.sections.iter().find(|s| s.name == "STACK")
            && stack.size() < SOFTWARE_STACK_BYTES
        {
            return Err(format!(
                "STACK section (${:04X}-${:04X}, {} bytes) is smaller than the fixed \
                 {}-byte software stack, which would overrun it",
                stack.start,
                stack.end,
                stack.size(),
                SOFTWARE_STACK_BYTES
            ));
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

    /// Validate a `wraith.toml` in the working directory, if one exists.
    ///
    /// Returns `Ok(())` when the file is absent (the default map is used) or
    /// present and well-formed, and `Err(message)` when it is present but
    /// malformed. The CLI calls this up front so a broken config fails fast with
    /// a clear message, rather than being silently swallowed by
    /// [`load_or_default`](Self::load_or_default) and replaced with the default
    /// map — which would compile against a memory layout the user never asked
    /// for.
    pub fn check_project_file() -> Result<(), String> {
        if Path::new("wraith.toml").exists() {
            Self::from_file("wraith.toml").map(|_| ())
        } else {
            Ok(())
        }
    }

    /// Try to load from wraith.toml in current directory, fall back to defaults.
    ///
    /// A malformed file falls back to the default map here; call
    /// [`check_project_file`](Self::check_project_file) first to surface the
    /// error instead of silently defaulting.
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
                // 4KB for constants/data, ending below $E000 so it never
                // collides with the memory-mapped I/O window ($E000-$EFFF, where
                // device `addr` registers live). That window is intentionally
                // left unmanaged — nothing but device registers may sit there.
                Section::new("DATA", 0xD000, 0xDFFF),
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

    #[test]
    fn the_default_config_validates() {
        Config::default_6502()
            .validate()
            .expect("default must be valid");
    }

    fn cfg(sections: Vec<Section>) -> Config {
        Config {
            sections,
            default_section: "CODE".to_string(),
        }
    }

    #[test]
    fn duplicate_section_names_are_rejected() {
        // get_section() returns the first, so a second CODE is silently dropped.
        let err = cfg(vec![
            Section::new("CODE", 0x8000, 0x9FFF),
            Section::new("CODE", 0xA000, 0xBFFF),
        ])
        .validate()
        .unwrap_err();
        assert!(err.contains("more than once"), "{err}");
    }

    #[test]
    fn overlapping_sections_are_rejected() {
        let err = cfg(vec![
            Section::new("CODE", 0x8000, 0xBFFF),
            Section::new("DATA", 0xB000, 0xCFFF), // overlaps CODE at $B000-$BFFF
        ])
        .validate()
        .unwrap_err();
        assert!(err.contains("overlap"), "{err}");
    }

    #[test]
    fn a_section_over_the_vector_table_is_rejected() {
        // A CODE section running to $FFFF would sit on the reset/IRQ vectors.
        let err = cfg(vec![Section::new("CODE", 0x8000, 0xFFFF)])
            .validate()
            .unwrap_err();
        assert!(err.contains("vector"), "{err}");
    }

    #[test]
    fn a_section_in_the_zero_page_is_rejected() {
        let err = cfg(vec![Section::new("CODE", 0x0000, 0x00FF)])
            .validate()
            .unwrap_err();
        assert!(err.contains("zero page"), "{err}");
    }

    #[test]
    fn a_section_over_the_hardware_stack_is_rejected() {
        let err = cfg(vec![Section::new("CODE", 0x0100, 0x01FF)])
            .validate()
            .unwrap_err();
        assert!(err.contains("hardware stack"), "{err}");
    }

    #[test]
    fn an_undersized_stack_section_is_rejected() {
        // The software stack is a fixed 256-byte page; a 128-byte section is
        // silently overrun.
        let err = cfg(vec![
            Section::new("CODE", 0x8000, 0xBFFF),
            Section::new("STACK", 0x0200, 0x027F), // 128 bytes
        ])
        .validate()
        .unwrap_err();
        assert!(err.contains("software stack"), "{err}");
    }

    #[test]
    fn a_full_page_stack_section_is_accepted() {
        cfg(vec![
            Section::new("CODE", 0x8000, 0xBFFF),
            Section::new("STACK", 0x0200, 0x02FF), // exactly 256 bytes
        ])
        .validate()
        .expect("a 256-byte STACK is valid");
    }
}
