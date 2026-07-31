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
-   `set_stack_pointer(value: u8)` - TXS (set the hardware stack pointer)

### mem.wr

Memory manipulation functions. Destination and source regions are passed as
pointers; plain addresses are `u16` values.

**Available Functions:**

-   `memcpy(dest: &u8, src: &u8, len: u8)` - Copy memory
-   `memcpy16(dest: &u8, src: &u8, len: u16)` - Copy memory, 16-bit length
-   `memset(dest: &u8, value: u8, len: u8)` - Fill memory
-   `memset16(dest: &u8, value: u8, len: u16)` - Fill memory, 16-bit length
-   `memcmp(a: &u8, b: &u8, len: u8) -> u8` - Compare memory
-   `mem_read(address: u16) -> u8` - Read a byte from an absolute address
-   `mem_write(address: u16, value: u8)` - Write a byte to an absolute address
-   `mem_jump(address: u16)` - Jump to machine code at an absolute address (does not return)
-   `str_copy(dest: &u8, dest_size: u16, s: str) -> u16` - Copy a string into a buffer; returns bytes written

### char.wr

ASCII character classification and conversion. A string is an array of `char`,
so these pair directly with `str` indexing and `for c in s` iteration. Every
function is pure and `#[inline]`, so a call compiles to the test itself with no
call overhead.

**Classification** (all return `bool`):

-   `is_digit(c: char)` - '0'..='9'
-   `is_upper(c: char)` - 'A'..='Z'
-   `is_lower(c: char)` - 'a'..='z'
-   `is_alpha(c: char)` - an ASCII letter
-   `is_alnum(c: char)` - a letter or digit
-   `is_whitespace(c: char)` - space, tab, newline, or carriage return

**Conversion:**

-   `to_upper(c: char) -> char` - uppercase a letter (non-letters unchanged)
-   `to_lower(c: char) -> char` - lowercase a letter (non-letters unchanged)
-   `digit_value(c: char) -> u8` - value of '0'..='9' (0-9); `0xFF` for non-digits

### math.wr

Mathematical operations, all NMOS 6502-legal.

#### Comparison Operations

-   `min(a: u8, b: u8) -> u8` - Return the minimum of two values
-   `max(a: u8, b: u8) -> u8` - Return the maximum of two values
-   `clamp(value: u8, min_val: u8, max_val: u8) -> u8` - Clamp value between bounds

#### Absolute Value

-   `abs(x: i8) -> i8` - Absolute value of an i8 (`abs(-128)` wraps to -128)
-   `abs16(x: i16) -> i16` - Absolute value of an i16 (`abs16(-32768)` wraps)

#### Bit Manipulation

-   `set_bit(value: u8, bit: u8) -> u8` - Set bit (0-7)
-   `clear_bit(value: u8, bit: u8) -> u8` - Clear bit (0-7)
-   `test_bit(value: u8, bit: u8) -> u8` - Test if bit is set (returns 1 if set, 0 if clear)

#### Saturating Arithmetic

-   `saturating_add(a: u8, b: u8) -> u8` - Add with saturation at 255
-   `saturating_sub(a: u8, b: u8) -> u8` - Subtract with saturation at 0

#### Advanced Bit Operations

-   `count_bits(value: u8) -> u8` - Count number of set bits (population count)
-   `reverse_bits(value: u8) -> u8` - Reverse bit order (e.g., 0b11010010 → 0b01001011)
-   `swap_nibbles(value: u8) -> u8` - Swap high and low nibbles (e.g., 0xAB → 0xBA)

#### Multiplication and Division

-   `mul_wide(a: u8, b: u8) -> u16` - 8×8→16 multiply
-   `mul16(a: u16, b: u16) -> u16` - 16×16→16 multiply (also emitted on demand for the `*` operator on u16)
-   `div16(a: u16, b: u16) -> u16` - 16÷16 divide (also emitted on demand for `/` on u16)
-   `divmod(a: u8, b: u8) -> u16` - Divide with remainder: low byte is the quotient, high byte the remainder

#### Pseudo-Random Numbers

-   `rand() -> u8` - Next pseudo-random byte
-   `rand16() -> u16` - Next pseudo-random 16-bit value
-   `srand(seed: u16)` - Seed the generator
