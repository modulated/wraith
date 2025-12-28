# Wraith Standard Library

This directory contains the Wraith standard library modules.

## Available Modules

### intrinsics.wr

Low-level CPU control functions that map directly to 6502 instructions. All functions are inlined for zero overhead.

#### Interrupt Control

-   `enable_interrupts()` - CLI (Clear Interrupt Disable)
-   `disable_interrupts()` - SEI (Set Interrupt Disable)

#### Carry Flag

-   `clear_carry()` - CLC (Clear Carry)
-   `set_carry()` - SEC (Set Carry)

#### Decimal Mode

-   `clear_decimal()` - CLD (Clear Decimal Mode)
-   `set_decimal()` - SED (Set Decimal Mode)

#### Other

-   `clear_overflow()` - CLV (Clear Overflow Flag)
-   `nop()` - NOP (No Operation)
-   `brk()` - BRK (Software Interrupt)
-   `wait_for_interrupt()` - Busy-wait loop for interrupts

### mem.wr

Memory manipulation functions for 6502.

**Available Functions:**

-   `memcpy(dest: u16, src: u16, len: u8)` - Copy memory
-   `memset(dest: u16, value: u8, len: u8)` - Fill memory
-   `memcmp(a: u16, b: u16, len: u8) -> u8` - Compare memory

### math.wr

Mathematical operations optimized for u8 on 65C02. Leverages 65C02-specific instructions (SMB/RMB/BBS) for efficient bit manipulation.

#### Comparison Operations

-   `min(a: u8, b: u8) -> u8` - Return the minimum of two values
-   `max(a: u8, b: u8) -> u8` - Return the maximum of two values
-   `clamp(value: u8, min_val: u8, max_val: u8) -> u8` - Clamp value between bounds

#### Bit Manipulation (65C02)

Uses 65C02 SMB/RMB/BBS instructions for atomic bit operations. All functions use zero page $20 for temporary storage.

-   `set_bit(value: u8, bit: u8) -> u8` - Set bit (0-7) using SMB instructions
-   `clear_bit(value: u8, bit: u8) -> u8` - Clear bit (0-7) using RMB instructions
-   `test_bit(value: u8, bit: u8) -> u8` - Test if bit is set using BBS instructions (returns 1 if set, 0 if clear)

#### Saturating Arithmetic

-   `saturating_add(a: u8, b: u8) -> u8` - Add with saturation at 255
-   `saturating_sub(a: u8, b: u8) -> u8` - Subtract with saturation at 0

#### Advanced Bit Operations

-   `count_bits(value: u8) -> u8` - Count number of set bits (population count)
-   `reverse_bits(value: u8) -> u8` - Reverse bit order (e.g., 0b11010010 → 0b01001011)
-   `swap_nibbles(value: u8) -> u8` - Swap high and low nibbles (e.g., 0xAB → 0xBA)
