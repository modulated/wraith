///! Memory Layout Configuration
///!
///! Defines the memory layout for the 6502 architecture, including
///! zero page allocations and reserved regions.

/// Memory layout configuration for 6502 code generation
#[derive(Debug, Clone)]
pub struct MemoryLayout {
    /// System reserved zero page (usually $00-$1F)
    pub system_reserved_start: u8,
    pub system_reserved_end: u8,

    /// Temporary storage for codegen operations (default $20-$2F)
    pub temp_storage_start: u8,
    pub temp_storage_end: u8,

    /// Pointer operations scratch space (default $30-$3F)
    pub pointer_ops_start: u8,
    pub pointer_ops_end: u8,

    /// Variable allocation start (default $40)
    pub variable_alloc_start: u8,

    /// Function parameter passing region (default $50)
    pub param_base: u8,
}

impl Default for MemoryLayout {
    fn default() -> Self {
        Self {
            system_reserved_start: 0x00,
            system_reserved_end: 0x1F,
            temp_storage_start: 0x20,
            temp_storage_end: 0x2F,
            pointer_ops_start: 0x30,
            pointer_ops_end: 0x3F,
            variable_alloc_start: 0x40,
            param_base: 0x50,
        }
    }
}

impl MemoryLayout {
    /// Create a new memory layout with default configuration
    pub fn new() -> Self {
        Self::default()
    }

    /// Get the temporary register address (first byte of temp storage)
    pub fn temp_reg(&self) -> u8 {
        self.temp_storage_start
    }

    /// Get the loop counter address (second byte of temp storage)
    pub fn loop_counter(&self) -> u8 {
        self.temp_storage_start + 0x01
    }

    /// Get the loop end temp address
    pub fn loop_end_temp(&self) -> u8 {
        self.temp_storage_start + 0x10
    }

    /// Get reserved regions for zero page allocator
    pub fn get_reserved_regions(&self) -> Vec<(u8, u8)> {
        vec![
            (self.system_reserved_start, self.system_reserved_end),
            (self.temp_storage_start, self.temp_storage_end),
            (self.pointer_ops_start, self.pointer_ops_end),
        ]
    }
}
