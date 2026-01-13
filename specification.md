# Wraith Language Specification

A systems programming language designed specifically for the 6502 processor, taking modern inspiration while remaining simple and explicit.

## Table of Contents

- [Overview](#overview)
- [Basic Types](#basic-types)
- [Variables](#variables)
- [Functions](#functions)
- [Structs](#structs)
- [Enums](#enums)
- [Arrays and Slices](#arrays-and-slices)
- [Pointers](#pointers)
- [Control Flow](#control-flow)
- [Type Casting](#type-casting)
- [Inline Assembly](#inline-assembly)
- [Modules and Imports](#modules-and-imports)
- [Standard Library](#standard-library)
- [Reserved Keywords](#reserved-keywords)
- [Operators](#operators)
- [Comments](#comments)

---

## Overview

- [ ] Add overview of design philosophy
- [ ] Document compilation process and output format
- [ ] Explain memory model and zero page usage
- [ ] Document calling conventions

Wraith compiles directly to 6502 assembly code, providing:
- Explicit type system with no inference
- Zero-cost abstractions
- Direct memory access and manipulation
- Inline assembly support
- Configurable memory layout via `wraith.toml`

---

## Basic Types

### Primitive Types

```
u8      // 8-bit unsigned integer (0 to 255)
i8      // 8-bit signed integer (-128 to 127)
b8      // 8-bit binary coded decimal (0 to 99)
u16     // 16-bit unsigned integer (0 to 65535)
i16     // 16-bit signed integer (-32768 to 32767)
b16     // 16-bit binary coded decimal (0 to 9999)
bool    // Boolean (represented as u8: 0 or 1)
```

### Type Characteristics

- All types must be explicitly declared
- No type inference
- No implicit conversions (must use `as` keyword)

### Completion Status

- [x] Basic primitive types documented
- [ ] Document BCD (Binary Coded Decimal) types usage and operations
- [ ] Add examples of type overflow behavior
- [ ] Document type size and alignment

---

## Variables

### Declaration Syntax

```rust
let x: u8 = 42;
let delta: i16 = -500;
let flag: bool = true;
```

### Mutability

**All variables are mutable by default**. This is a low-level systems language that trusts the programmer.

```rust
let x: u8 = 10;
x = 20;  // OK - variables are mutable
```

### Constants

Use the `const` keyword to declare compile-time constants. Constants are evaluated at compile time and cannot be reassigned.

```rust
const MAX_SPRITES: u8 = 8;
const SCREEN_WIDTH: u16 = 320;
const PI_TIMES_100: u8 = 314;

fn init() {
    for i in 0..MAX_SPRITES {
        // use constant
    }
}
```

Constants are checked for overflow at compile time:

```rust
const INVALID: u8 = 256;  // ERROR: constant overflow (256 doesn't fit in u8)
```

### Memory-Mapped Addresses

Use the `addr` keyword to declare memory-mapped I/O addresses:

```rust
let LED: addr = 0x6000;      // Memory-mapped LED
let BUTTON: addr = 0x6001;   // Memory-mapped button

fn main() {
    LED = 1;                 // Write to address
    let state: u8 = BUTTON;  // Read from address
}
```

Addresses can be read from or written to like variables, but they represent fixed memory locations.
They can also be marked as read only or write only and this is enforced at compile time.

```rust
let LED: write addr = 0x6000;    // Write only address
let BUTTON: read addr = 0x6001;  // Read only address

fn main() {
    LED = 1;                 // Write to address - OK
    let x = LED;             // Read from write only address - compile time error
    let state: u8 = BUTTON;  // Read from address - OK
    BUTTON = 0;              // Write to read only address - compile time error
}
```

### Completion Status

- [x] Variable declaration syntax documented
- [x] Mutability behavior documented
- [x] Constants documented with examples
- [x] Memory-mapped addresses documented
- [ ] Document variable scope rules
- [ ] Add shadowing behavior (if supported)
- [ ] Document zero page allocation strategy

---

## Functions

### Function Declaration

```rust
fn function_name(arg1: u8, arg2: u16) -> u8 {
    return arg1;
}

fn no_return(x: u8) {
    // No return statement needed
}
```

### Function Attributes

- [ ] TODO: Rewrite this section to be better
- [ ] TODO: Generate examples of all attributes

Function attributes control code generation and placement:

```rust
#[inline]         // Inlines the assembly without generating a jump to label
fn fast_function(x: u8) -> u8 {
    return x * 2;
}

#[irq]            // Interrupt request handler
fn irq() { }

#[nmi]            // Non-maskable interrupt handler
fn nmi() { }

#[reset]          // Reset handler (entry point)
fn reset() { }

#[org(0x8000)]    // Place generated machine code at specific address
fn main() -> u8 { }

#[section("STDLIB")]   // Place generated machine code in named section
fn imported_fn() { }
```

### Completion Status

- [x] Basic function declaration documented
- [ ] Document function calling convention
- [ ] Explain parameter passing (registers vs stack vs zero page)
- [ ] Document return value handling
- [ ] Add examples for each attribute
- [ ] Document tail call optimization behavior
- [ ] Explain inline attribute impact on code size

---

## Structs

### Declaration

```rust
struct Point {
    x: u8,
    y: u8,
}

struct Entity {
    position: Point,
    health: u8,
    score: u16,
}
```

### Usage

```rust
let p1: Point = { x: 10, y: 20 };
let p2: Point = { x: 5, y: 5 };
p2.x = 15;

let x_coord: u8 = p1.x;
```

### Completion Status

- [x] Basic struct declaration documented
- [x] Struct initialization syntax documented
- [x] Field access documented
- [ ] Document struct memory layout
- [ ] Add padding and alignment information
- [ ] Document nested struct behavior
- [ ] Add examples of structs in arrays
- [ ] Document struct passing to functions

---

## Enums

### Simple Enums

```rust
enum Direction {
    North = 0,
    South = 1,
    East = 2,
    West = 3,
}

let dir: Direction = Direction::North;
```

### Enums with Data (Tagged Unions)

- [ ] TODO: Document if implemented in language
- [ ] Add examples of tagged unions if supported
- [ ] Document memory layout of tagged unions

### Completion Status

- [x] Simple enum declaration documented
- [ ] Document default discriminant values
- [ ] Add pattern matching with enums
- [ ] Document enum memory representation

---

## Arrays and Slices

### Fixed Arrays

```rust
let buffer: [u8; 10] = [0; 10];  // 10 bytes, all zeros
let data: [u16; 5] = [100, 200, 300, 400, 500];

buffer[5] = 42;
let value: u16 = data[2];
```

### Slices (Fat Pointers)

```rust
const DATA: [u8; 6] = [0, 1, 2, 3, 4, 5];

let array: [u8; 10] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
process_data(array);  // Automatic coercion to slice
```

### Completion Status

- [x] Fixed array declaration documented
- [x] Array initialization syntax documented
- [x] Array indexing documented
- [ ] Document array bounds checking (compile-time vs runtime)
- [ ] Add slice syntax and operations
- [ ] Document slice memory representation
- [ ] Add multidimensional array examples
- [ ] Document array assignment and copying behavior

---

## Pointers

- [ ] TODO: Document if pointers are implemented
- [ ] Add pointer declaration syntax
- [ ] Document pointer arithmetic
- [ ] Add dereferencing examples
- [ ] Document pointer safety guarantees (if any)
- [ ] Add examples of pointers to structs
- [ ] Document null pointer handling

---

## Control Flow

### If/Else

```rust
if x > 10 {
    // ...
} else if x > 5 {
    // ...
} else {
    // ...
}
```

### While Loop

```rust
while condition {
    // ...
}

while x < 100 {
    x = x + 1;
}
```

### Loop (Infinite)

```rust
loop {
    if done {
        break;
    }
}
```

### For Loop

```rust
// Range-based (type inferred from range bounds)
for i in 0..10 {      // 0 to 9, i is inferred as u8
    // ...
}

for i in 0..=255 {    // 0 to 255 (inclusive), i is u8
    // ...
}

for i in 0..1000 {    // Larger range, i is inferred as u16
    // ...
}

// Explicit type annotation
for i: u8 in 0..10 {
    // ...
}

// Over slices (type inferred from slice element type)
fn process(data: [u8]) {
    for item in data {  // item is inferred as u8
        // process item
    }
}
```

### Match Statement

```rust
// Match on values
match value {
    0 => { },
    1..=10 => { },    // Range
    _ => { },         // Default
}

// Match on enums
match direction {
    Direction::North => { },
    Direction::South => { },
    Direction::East => { },
    Direction::West => { },
}

// Match on enum variants with data
match msg {
    Message::Quit => { },
    Message::Move { x, y } => {
        // use x, y
    },
    Message::Write(val) => {
        // use val
    },
}
```

### Completion Status

- [x] If/else documented
- [x] While loop documented
- [x] Infinite loop documented
- [x] For loop documented
- [x] Match statement documented
- [ ] Document continue statement
- [ ] Document break with labels (if supported)
- [ ] Add assembly output examples for each control flow construct
- [ ] Document short-circuit evaluation in conditions

---

## Type Casting

### Explicit Casting

```rust
let small: u8 = 100;
let large: u16 = small as u16;

let signed: i8 = -10;
let unsigned: u8 = signed as u8;  // Results in 246

let addr: u16 = 0x1000;
```

**No implicit conversions** - all casts must be explicit.
**No error checking** - casts that are invalid will overflow/underflow.

### Completion Status

- [x] Basic casting syntax documented
- [ ] Document all valid cast combinations
- [ ] Add truncation behavior examples
- [ ] Document sign extension behavior
- [ ] Add casting to/from bool

---

## Inline Assembly

### Basic Assembly Block

```rust
fn increment() {
    asm {
        "clc",
        "adc #1"
    }
}
```

### Assembly with Variable Substitution

```rust
fn add_with_carry(a: u8, b: u8) -> u8 {
    let result: u8;
    asm {
        "clc",
        "lda {a}",
        "adc {b}",
        "sta {result}"
    }
    return result;
}
```

Variables in `{}` are substituted with their memory locations.

### Completion Status

- [x] Basic assembly block documented
- [x] Variable substitution documented
- [ ] Document register clobbering
- [ ] Add examples of common assembly patterns
- [ ] Document inline assembly limitations
- [ ] Add examples using labels in assembly
- [ ] Document interaction with optimizer

---

## Modules and Imports

Wraith supports a simple file-based import system for code organization and reuse.

### Import Syntax

```rust
import {symbol1, symbol2, symbol3} from "module.wr";
```

### Import Resolution

**Relative imports**: Start with `./` or `../`

```rust
import {foo} from "./utils.wr";
import {bar} from "../lib/helper.wr";
```

**Non-relative imports**: Searched in standard library directory first, then current directory

```rust
import {memcpy} from "mem.wr";  // Searches stdlib first
```

### Limitations

These may be changed in future versions:

- No module hierarchy or namespaces (flat file imports only)
- No pub/private visibility (all symbols in a file are importable)
- No re-exports
- No wildcard imports (`import *`)

### Completion Status

- [x] Import syntax documented
- [x] Import resolution documented
- [x] Limitations documented
- [ ] Add circular import handling
- [ ] Document import order dependencies
- [ ] Add examples of organizing larger projects

---

## Standard Library

Wraith includes a small standard library optimized for 6502 architecture.

### Module: intrinsics.wr

Low-level CPU control functions that map directly to 6502 instructions. All functions are inlined for zero overhead.

#### Interrupt Control

- [ ] Document `enable_interrupts()` - CLI (Clear Interrupt Disable)
- [ ] Document `disable_interrupts()` - SEI (Set Interrupt Disable)

#### Carry Flag

- [ ] Document `clear_carry()` - CLC (Clear Carry)
- [ ] Document `set_carry()` - SEC (Set Carry)

#### Decimal Mode

- [ ] Document `clear_decimal()` - CLD (Clear Decimal Mode)
- [ ] Document `set_decimal()` - SED (Set Decimal Mode)

#### Other

- [ ] Document `clear_overflow()` - CLV (Clear Overflow Flag)
- [ ] Document `nop()` - NOP (No Operation)
- [ ] Document `brk()` - BRK (Software Interrupt)
- [ ] Document `wait_for_interrupt()` - Busy-wait loop for interrupts

### Module: mem.wr

Memory manipulation functions for 6502.

- [ ] Document `memcpy(dest: u16, src: u16, len: u8)` - Copy memory
- [ ] Document `memset(dest: u16, value: u8, len: u8)` - Fill memory
- [ ] Document `memcmp(a: u16, b: u16, len: u8) -> u8` - Compare memory

### Module: math.wr

Mathematical operations optimized for 6502. Leverages 65C02-specific instructions for efficient bit manipulation.

#### Comparison Operations

- [ ] Document `min(a: u8, b: u8) -> u8` - Return minimum
- [ ] Document `max(a: u8, b: u8) -> u8` - Return maximum
- [ ] Document `clamp(value: u8, min_val: u8, max_val: u8) -> u8` - Clamp value

#### Bit Manipulation (65C02)

Uses 65C02 SMB/RMB/BBS instructions for atomic bit operations.

- [ ] Document `set_bit(value: u8, bit: u8) -> u8` - Set bit (0-7)
- [ ] Document `clear_bit(value: u8, bit: u8) -> u8` - Clear bit (0-7)
- [ ] Document `test_bit(value: u8, bit: u8) -> u8` - Test if bit is set

#### Saturating Arithmetic

- [ ] Document `saturating_add(a: u8, b: u8) -> u8` - Add with saturation at 255
- [ ] Document `saturating_sub(a: u8, b: u8) -> u8` - Subtract with saturation at 0

#### Advanced Bit Operations

- [ ] Document `count_bits(value: u8) -> u8` - Count set bits (population count)
- [ ] Document `reverse_bits(value: u8) -> u8` - Reverse bit order
- [ ] Document `swap_nibbles(value: u8) -> u8` - Swap high and low nibbles

### Completion Status

- [ ] Add usage examples for each stdlib function
- [ ] Document performance characteristics
- [ ] Add assembly output examples
- [ ] Document 65C02 vs 6502 compatibility
- [ ] Add examples combining multiple stdlib functions

---

## Reserved Keywords

- [ ] TODO: Verify these keywords are correct

```
addr      asm       bool      break     const     else      enum
fn        for       from      i8        i16       if        import
in        inline    loop      match     return    struct    u8
u16       while     as        true      false     let       b8
b16       read      write
```

### Completion Status

- [ ] Verify complete list of reserved keywords
- [ ] Document future reserved keywords
- [ ] Add examples of keyword usage

---

## Operators

### Arithmetic

```rust
+   -   *   /   %     // Add, subtract, multiply, divide, modulo
<<  >>                // Left shift, right shift
```

### Comparison

```rust
==  !=  <   >   <=  >=
```

### Logical

```rust
&&  ||  !
```

### Bitwise

```rust
&   |   ^   ~         // AND, OR, XOR, NOT
```

### Assignment

```rust
=   +=  -=  *=  /=  %=    // Assignment and compound assignment
&=  |=  ^=  <<=  >>=      // Bitwise compound assignment
```

### Completion Status

- [x] All operators listed
- [ ] Document operator precedence
- [ ] Add overflow behavior for arithmetic operators
- [ ] Document short-circuit evaluation for logical operators
- [ ] Add examples of operator usage
- [ ] Document operator implementation in assembly

---

## Comments

```rust
// Single line comment

/*
   Multi-line
   comment
*/
```

### Completion Status

- [x] Comment syntax documented
- [ ] Add documentation comment syntax (if supported)
- [ ] Document comment handling in inline assembly

---

## Appendices

### Appendix A: Code Generation

- [ ] Document register allocation strategy
- [ ] Explain zero page usage
- [ ] Document stack usage
- [ ] Add optimization passes overview

### Appendix B: Memory Layout

- [ ] Document default memory map
- [ ] Explain section placement
- [ ] Add examples of custom memory layouts

### Appendix C: Calling Convention

- [ ] Document parameter passing
- [ ] Explain return value handling
- [ ] Add calling convention for interrupt handlers

### Appendix D: Examples

- [ ] Add complete program examples
- [ ] Include common patterns and idioms
- [ ] Add performance optimization examples

---

## Revision History

- 2026-01-13: Initial skeleton created with checkboxes for incremental completion
