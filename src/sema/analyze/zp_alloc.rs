//! Zero Page Memory Allocator
//!
//! Manages allocation of zero page addresses ($00-$FF) for the 6502.

use crate::ast::Span;
use crate::codegen::memory_layout::MemoryLayout;
use crate::sema::SemaError;

/// Zero page memory allocator
/// Manages allocation of zero page addresses ($00-$FF)
#[allow(dead_code)]
pub(super) struct ZeroPageAllocator {
    /// Next available address
    next_addr: u8,
    /// Reserved ranges (start, end) that cannot be allocated
    reserved: Vec<(u8, u8)>,
}

impl ZeroPageAllocator {
    pub fn new() -> Self {
        let layout = MemoryLayout::new();
        Self {
            next_addr: layout.variable_alloc_start,
            reserved: layout.get_reserved_regions(),
        }
    }

    /// Allocate a single byte in zero page
    pub fn allocate(&mut self) -> Result<u8, SemaError> {
        // Find next available address
        loop {
            let addr = self.next_addr;

            // Check if this address is reserved
            let is_reserved = self.reserved.iter().any(|(start, end)| addr >= *start && addr <= *end);

            if !is_reserved && addr != 0xFF {
                self.next_addr = addr + 1;
                return Ok(addr);
            }

            // Try next address
            self.next_addr += 1;

            if self.next_addr == 0 {
                // Wrapped around - out of zero page
                return Err(SemaError::OutOfZeroPage {
                    span: Span { start: 0, end: 0 }, // No span context in allocator
                });
            }
        }
    }

    /// Allocate multiple consecutive bytes
    #[allow(dead_code)]
    pub fn allocate_range(&mut self, count: u8) -> Result<u8, SemaError> {
        let start = self.next_addr;

        // Check if we have enough space
        if start as usize + count as usize > 0x100 {
            return Err(SemaError::OutOfZeroPage {
                span: Span { start: 0, end: 0 }, // No span context in allocator
            });
        }

        // Allocate each byte
        for _ in 0..count {
            self.allocate()?;
        }

        Ok(start)
    }

    /// Reset allocator (for new scope/function)
    #[allow(dead_code)]
    pub fn reset(&mut self) {
        let layout = MemoryLayout::new();
        self.next_addr = layout.variable_alloc_start;
    }
}
