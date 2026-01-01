//! Memory Layout Configuration
//!
//! Defines the memory layout for the 6502 architecture, including
//! zero page allocations and reserved regions.
//!
//! # Default Zero Page Layout
//!
//! ```text
//! $00-$1F (32 bytes): System reserved
//! $20-$2F (16 bytes): Temporary storage for codegen
//!   $20: Primary temp register (binary ops)
//!   $21: Loop end temp
//!   $22-$23: Arithmetic/enum operations
//! $30-$3F (16 bytes): Pointer operations scratch space
//!   $30-$31: Indirect addressing operations
//! $40-$7F (64 bytes): Variable allocation space
//! $80-$BF (64 bytes): Function parameter passing region
//! $C0-$FF (64 bytes): Available for future use
//! ```

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

    /// Variable allocation end (default $7F) - gives 64 bytes for variables
    pub variable_alloc_end: u8,

    /// Function parameter passing region (default $80)
    pub param_base: u8,

    /// Parameter region end (default $BF) - gives 64 bytes for parameters
    pub param_end: u8,
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
            variable_alloc_end: 0x7F,
            param_base: 0x80,
            param_end: 0xBF,
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

    /// Get the loop counter address
    pub fn loop_counter(&self) -> u8 {
        self.temp_storage_start + 0x10
    }

    /// Get the loop end temp address
    /// Note: temp_reg uses 2 bytes for u16 operations, so loop_end_temp must be at +2 or higher
    pub fn loop_end_temp(&self) -> u8 {
        self.temp_storage_start + 0x02
    }

    /// Get reserved regions for zero page allocator
    pub fn get_reserved_regions(&self) -> Vec<(u8, u8)> {
        vec![
            (self.system_reserved_start, self.system_reserved_end),
            (self.temp_storage_start, self.temp_storage_end),
            (self.pointer_ops_start, self.pointer_ops_end),
            (self.param_base, self.param_end),
        ]
    }

    /// Get the total variable space available (in bytes)
    pub fn variable_space(&self) -> u8 {
        self.variable_alloc_end - self.variable_alloc_start + 1
    }

    /// Get the total parameter space available (in bytes)
    pub fn param_space(&self) -> u8 {
        self.param_end - self.param_base + 1
    }
}
