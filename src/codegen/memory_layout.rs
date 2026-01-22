//! Memory Layout Configuration
//!
//! Defines the memory layout for the 6502 architecture, including
//! zero page allocations and reserved regions.
//!
//! # Default Zero Page Layout
//!
//! ```text
//! $00-$1F (32 bytes): System reserved
//! $20-$3F (32 bytes): Temporary storage for codegen (managed by TempAllocator)
//!   $20-$21: Primary temp register (binary ops, u16)
//!   $22-$23: Secondary temp (arithmetic/enum)
//!   $24-$2F: General temp pool
//!   $30-$3F: Pointer operations / overflow temp
//! $40-$7F (64 bytes): Variable allocation space
//! $80-$BF (64 bytes): Function parameter passing region
//! $C0-$CF (16 bytes): Extended variable space
//! $D0-$D8 (9 bytes):  Stdlib math working storage (mul16/div16)
//! $D9-$EF (23 bytes): Extended variable space (continued)
//! $F0-$F3 (4 bytes):  Binary op left operand save
//! $F4-$FE (11 bytes): Function argument evaluation temp
//! $FF:                Software stack pointer
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

    /// Get the jump table indirect pointer address (2 bytes for JMP indirect)
    /// Used by match statement jump table dispatch
    pub fn jump_ptr(&self) -> u8 {
        self.pointer_ops_start // $30 by default
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

/// Temporary storage allocator for codegen
///
/// Manages allocation of temporary zero-page locations to prevent conflicts
/// between different codegen phases (binary ops, function calls, etc.)
///
/// # Regions Managed
/// - Primary temp pool: $20-$3F (32 bytes)
/// - High temp pool: $F0-$F3 (4 bytes) - for binary op saves
/// - Arg temp pool: $F4-$FE (11 bytes) - for function arguments
#[derive(Debug, Clone)]
pub struct TempAllocator {
    /// Bitmap for $20-$3F (32 bytes, bit per byte)
    primary_pool: u32,
    /// Bitmap for $F0-$F3 (4 bytes)
    high_pool: u8,
    /// Bitmap for $F4-$FE (11 bytes)
    arg_pool: u16,
}

impl Default for TempAllocator {
    fn default() -> Self {
        Self::new()
    }
}

impl TempAllocator {
    /// Base addresses for each pool
    pub const PRIMARY_BASE: u8 = 0x20;
    pub const PRIMARY_SIZE: u8 = 32;
    pub const HIGH_BASE: u8 = 0xF0;
    pub const HIGH_SIZE: u8 = 4;
    pub const ARG_BASE: u8 = 0xF4;
    pub const ARG_SIZE: u8 = 11;

    pub fn new() -> Self {
        Self {
            primary_pool: 0,
            high_pool: 0,
            arg_pool: 0,
        }
    }

    /// Allocate `size` consecutive bytes from the primary temp pool ($20-$3F)
    /// Returns the starting address, or None if no space available
    pub fn alloc_primary(&mut self, size: u8) -> Option<u8> {
        Self::alloc_from_pool(
            &mut self.primary_pool,
            Self::PRIMARY_BASE,
            Self::PRIMARY_SIZE,
            size,
        )
    }

    /// Free previously allocated bytes in the primary pool
    pub fn free_primary(&mut self, addr: u8, size: u8) {
        Self::free_from_pool(&mut self.primary_pool, Self::PRIMARY_BASE, addr, size);
    }

    /// Allocate from high temp pool ($F0-$F3) - typically for binary op left operand
    pub fn alloc_high(&mut self, size: u8) -> Option<u8> {
        let mut pool = self.high_pool as u32;
        let result = Self::alloc_from_pool(&mut pool, Self::HIGH_BASE, Self::HIGH_SIZE, size);
        self.high_pool = pool as u8;
        result
    }

    /// Free previously allocated bytes in the high pool
    pub fn free_high(&mut self, addr: u8, size: u8) {
        let mut pool = self.high_pool as u32;
        Self::free_from_pool(&mut pool, Self::HIGH_BASE, addr, size);
        self.high_pool = pool as u8;
    }

    /// Allocate from arg temp pool ($F4-$FE) - for function argument evaluation
    pub fn alloc_arg(&mut self, size: u8) -> Option<u8> {
        let mut pool = self.arg_pool as u32;
        let result = Self::alloc_from_pool(&mut pool, Self::ARG_BASE, Self::ARG_SIZE, size);
        self.arg_pool = pool as u16;
        result
    }

    /// Free previously allocated bytes in the arg pool
    pub fn free_arg(&mut self, addr: u8, size: u8) {
        let mut pool = self.arg_pool as u32;
        Self::free_from_pool(&mut pool, Self::ARG_BASE, addr, size);
        self.arg_pool = pool as u16;
    }

    /// Reset all allocations (call at function boundaries)
    pub fn reset(&mut self) {
        self.primary_pool = 0;
        self.high_pool = 0;
        self.arg_pool = 0;
    }

    /// Check if a specific address range is free in primary pool
    pub fn is_primary_free(&self, addr: u8, size: u8) -> bool {
        if addr < Self::PRIMARY_BASE || addr + size > Self::PRIMARY_BASE + Self::PRIMARY_SIZE {
            return false;
        }
        let offset = addr - Self::PRIMARY_BASE;
        let mask = ((1u32 << size) - 1) << offset;
        (self.primary_pool & mask) == 0
    }

    /// Internal: allocate from a bitmap pool (static to avoid borrow issues)
    fn alloc_from_pool(pool: &mut u32, base: u8, pool_size: u8, size: u8) -> Option<u8> {
        if size == 0 || size > pool_size {
            return None;
        }

        // Find first fit: look for `size` consecutive zero bits
        let mask = (1u32 << size) - 1;
        for offset in 0..=(pool_size - size) {
            let shifted_mask = mask << offset;
            if (*pool & shifted_mask) == 0 {
                // Found free space, mark as allocated
                *pool |= shifted_mask;
                return Some(base + offset);
            }
        }
        None
    }

    /// Internal: free from a bitmap pool (static to avoid borrow issues)
    fn free_from_pool(pool: &mut u32, base: u8, addr: u8, size: u8) {
        if addr < base {
            return;
        }
        let offset = addr - base;
        let mask = ((1u32 << size) - 1) << offset;
        *pool &= !mask;
    }

    /// Get allocation statistics for debugging
    pub fn stats(&self) -> TempAllocStats {
        TempAllocStats {
            primary_used: self.primary_pool.count_ones() as u8,
            primary_total: Self::PRIMARY_SIZE,
            high_used: self.high_pool.count_ones() as u8,
            high_total: Self::HIGH_SIZE,
            arg_used: self.arg_pool.count_ones() as u8,
            arg_total: Self::ARG_SIZE,
        }
    }
}

/// Statistics about temp allocation usage
#[derive(Debug, Clone)]
pub struct TempAllocStats {
    pub primary_used: u8,
    pub primary_total: u8,
    pub high_used: u8,
    pub high_total: u8,
    pub arg_used: u8,
    pub arg_total: u8,
}
