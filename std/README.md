# Wraith Standard Library

This directory contains the Wraith standard library modules.

## Available Modules

### intrinsics.wr
Low-level CPU control functions that map directly to 6502 instructions. All functions are inlined for zero overhead.

**Interrupt Control:**
- `enable_interrupts()` - CLI (Clear Interrupt Disable)
- `disable_interrupts()` - SEI (Set Interrupt Disable)

**Carry Flag:**
- `clear_carry()` - CLC (Clear Carry)
- `set_carry()` - SEC (Set Carry)

**Decimal Mode:**
- `clear_decimal()` - CLD (Clear Decimal Mode)
- `set_decimal()` - SED (Set Decimal Mode)

**Other:**
- `clear_overflow()` - CLV (Clear Overflow Flag)
- `nop()` - NOP (No Operation)
- `brk()` - BRK (Software Interrupt)
- `wait_for_interrupt()` - Busy-wait loop for interrupts

**Usage:**
```wraith
import { enable_interrupts, disable_interrupts } from "intrinsics.wr";

fn main() {
    disable_interrupts();  // Inlined to: SEI
    // Critical section
    enable_interrupts();   // Inlined to: CLI
}
```

### std.wr
Memory manipulation functions for 6502.

**Available Functions:**
- `memcpy(dest: u16, src: u16, len: u8)` - Copy memory
- `memset(dest: u16, value: u8, len: u8)` - Fill memory
- `memcmp(a: u16, b: u16, len: u8) -> u8` - Compare memory

## Import Syntax

Import specific symbols using:
```wraith
import { function1, function2 } from "module.wr";
```

Note: Module paths are relative to the standard library directory (std/).
For example, `"intrinsics.wr"` refers to `std/intrinsics.wr`.

## Implementation Notes

- Intrinsic functions use the `inline` keyword for zero-overhead abstraction
- All intrinsics are single instruction operations
- The compiler automatically inlines these functions at the call site
- No JSR/RTS overhead - just the raw 6502 instruction
