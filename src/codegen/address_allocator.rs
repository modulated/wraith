//! Multi-pass address allocation with conflict detection
//!
//! This module implements a multi-pass compilation strategy:
//! 1. Collect metadata (org addresses, sections, etc.)
//! 2. Compile functions individually (without ORG directives)
//! 3. Allocate addresses and detect conflicts
//! 4. Emit final assembly with ORG directives

use crate::codegen::section_allocator::SectionAllocator;
use std::collections::HashMap;

/// Metadata about a function's placement requirements
#[derive(Debug, Clone)]
pub struct FunctionMetadata {
    pub name: String,
    pub org_address: Option<u16>,
    pub section: Option<String>,
}

/// A compiled function without address assignment
#[derive(Debug, Clone)]
pub struct CompiledFunction {
    pub name: String,
    pub assembly: String,  // Assembly code without .ORG
    pub size: u16,         // Size in bytes
}

/// An address range occupied by a function
#[derive(Debug, Clone)]
pub struct AddressRange {
    pub start: u16,
    pub end: u16,
    pub function_name: String,
    pub source: PlacementSource,
}

/// How a function's address was determined
#[derive(Debug, Clone)]
pub enum PlacementSource {
    ExplicitOrg(u16),
    Section(String),
    AutoAllocated,
}

impl std::fmt::Display for PlacementSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PlacementSource::ExplicitOrg(addr) => write!(f, "explicit #[org(0x{:04X})]", addr),
            PlacementSource::Section(name) => write!(f, "section '{}'", name),
            PlacementSource::AutoAllocated => write!(f, "auto-allocated"),
        }
    }
}

/// Address allocation conflict
#[derive(Debug)]
pub struct ConflictError {
    pub func1: String,
    pub range1: (u16, u16),
    pub source1: PlacementSource,
    pub func2: String,
    pub range2: (u16, u16),
    pub source2: PlacementSource,
}

impl ConflictError {
    pub fn to_diagnostic(&self) -> String {
        format!(
            "Address conflict: function '{}' at ${:04X}-${:04X} ({}) overlaps with '{}' at ${:04X}-${:04X} ({})",
            self.func1, self.range1.0, self.range1.1, self.source1,
            self.func2, self.range2.0, self.range2.1, self.source2
        )
    }
}

/// Manages address allocation with conflict detection
pub struct AddressAllocator {
    ranges: Vec<AddressRange>,
    section_allocator: SectionAllocator,
}

impl AddressAllocator {
    pub fn new(section_allocator: SectionAllocator) -> Self {
        Self {
            ranges: Vec::new(),
            section_allocator,
        }
    }

    /// Allocate addresses for all functions, detecting conflicts
    pub fn allocate(
        &mut self,
        metadata: &HashMap<String, FunctionMetadata>,
        compiled: &HashMap<String, CompiledFunction>,
    ) -> Result<HashMap<String, u16>, Vec<ConflictError>> {
        let mut addresses = HashMap::new();
        let mut errors = Vec::new();

        // Process functions with explicit org first, then the rest
        let mut functions: Vec<_> = metadata.keys().cloned().collect();
        functions.sort_by_key(|name| {
            // Sort by: explicit org first (by address), then auto-allocated (by name)
            metadata[name].org_address.map(|addr| (0, addr, name.clone()))
                .unwrap_or_else(|| (1, 0, name.clone()))
        });

        for func_name in functions {
            let meta = &metadata[&func_name];
            let func = match compiled.get(&func_name) {
                Some(f) => f,
                None => continue, // Skip if not compiled (imported functions, etc.)
            };

            let (addr, source) = if let Some(org_addr) = meta.org_address {
                // Explicit org address
                (org_addr, PlacementSource::ExplicitOrg(org_addr))
            } else if let Some(section) = &meta.section {
                // Section allocation
                match self.section_allocator.allocate(section, func.size) {
                    Ok(addr) => (addr, PlacementSource::Section(section.clone())),
                    Err(e) => {
                        eprintln!("Warning: section allocation failed for {}: {}", func_name, e);
                        continue;
                    }
                }
            } else {
                // Default allocation
                match self.section_allocator.allocate_default(func.size) {
                    Ok(addr) => (addr, PlacementSource::AutoAllocated),
                    Err(e) => {
                        eprintln!("Warning: default allocation failed for {}: {}", func_name, e);
                        continue;
                    }
                }
            };

            // Check for conflicts with existing allocations
            let range = AddressRange {
                start: addr,
                end: addr.saturating_add(func.size).saturating_sub(1),
                function_name: func_name.clone(),
                source: source.clone(),
            };

            if let Some(conflict) = self.find_conflict(&range) {
                errors.push(ConflictError {
                    func1: func_name.clone(),
                    range1: (range.start, range.end),
                    source1: source,
                    func2: conflict.function_name.clone(),
                    range2: (conflict.start, conflict.end),
                    source2: conflict.source.clone(),
                });
            } else {
                self.ranges.push(range);
                addresses.insert(func_name.clone(), addr);
            }
        }

        if errors.is_empty() {
            Ok(addresses)
        } else {
            Err(errors)
        }
    }

    /// Find if a range conflicts with any existing allocation
    fn find_conflict(&self, new_range: &AddressRange) -> Option<&AddressRange> {
        // Check if ranges overlap: two ranges overlap if one doesn't end before the other starts
        self.ranges.iter().find(|existing|
            !(new_range.end < existing.start || new_range.start > existing.end)
        )
    }
}
