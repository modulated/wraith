# 6502 Language Specification

A systems programming language designed specifically for the 6502 processor, taking modern inspiration while remaining simple and explicit.

## Design Philosophy

-   **Simple and Explicit**: No type inference, explicit code, clear syntax
-   **6502-Optimized**: Zero page hints, efficient calling convention, minimal overhead
-   **Memory-Conscious**: Direct control over memory layout and access patterns
-   **No Runtime Overhead**: Compile-time features only, no garbage collection

## Basic Types

### Primitive Types

```
u8      // 8-bit unsigned integer (0 to 255)
i8      // 8-bit signed integer (-128 to 127)
u16     // 16-bit unsigned integer (0 to 65535)
i16     // 16-bit signed integer (-32768 to 32767)
bool    // Boolean (actually u8: 0 or 1)
```

### Type Characteristics

-   All types must be explicitly declared
-   No type inference
-   No implicit conversions (must use `as` keyword)

## Variables

### Declaration Syntax

```
x: u8 = 42;
delta: i16 = -500;
flag: bool = true;
```

### Mutability

**All variables are mutable by default**. This is a low-level systems language that trusts the programmer.

```
x: u8 = 10;
x = 20;                      // OK - all variables are mutable
```

### Zero Page Hint

The `zp` keyword suggests the compiler allocate a variable to zero page for faster access. This is a **hint**, not a requirement - the compiler may ignore it if deemed not necessary.

```
zp fast_var: u8 = 0;         // Compiler will try to use zero page
zp counter: u16 = 0;         // Zero page hint
```

Zero page ($00-$FF) provides faster access (3 cycles vs 4 cycles) and enables special addressing modes. When register allocation optimization is enabled, the compiler may also promote frequently-used variables to zero page automatically, even without the `zp` hint.

### Constants

Use the `const` keyword to declare compile-time constants. Constants are evaluated at compile time and cannot be reassigned.

```
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

```
const INVALID: u8 = 256;     // ERROR: constant overflow (256 doesn't fit in u8)
```

### Memory-Mapped Addresses

Use the `addr` keyword to declare memory-mapped I/O addresses:

```
addr LED: u16 = 0x6000;      // Memory-mapped LED
addr BUTTON: u16 = 0x6001;   // Memory-mapped button

fn main() {
    LED = 1;                 // Write to address
    state: u8 = BUTTON;      // Read from address
}
```

Addresses can be read from or written to like variables, but they represent fixed memory locations.

## Functions

### Function Declaration

```
fn function_name(arg1: u8, arg2: u16) -> u8 {
    return arg1;
}

fn no_return(x: u8) {
    // No return statement needed
}
```

### Function Attributes

```
#[inline]
fn fast_function(x: u8) -> u8 {
    return x * 2;
}

#[noreturn]
fn infinite_loop() {
    loop { }
}

#[interrupt]
fn irq_handler() {
    // Compiler handles register save/restore
}
```

### Calling Convention

**Zero Page Parameter Passing:**

Arguments are passed via zero page memory for fast access. The 6502's zero page provides faster addressing (3-4 cycles vs 4-5 for absolute addressing) and enables special addressing modes.

**Memory Layout:**

-   `$00-$1F` (32 bytes): System reserved
-   `$20-$2F` (16 bytes): Temporary storage for compiler-generated code
-   `$30-$3F` (16 bytes): Pointer operations scratch space
-   `$40-$7F` (64 bytes): Local variable allocation
-   `$80-$BF` (64 bytes): Function parameter passing
-   `$C0-$FF` (64 bytes): Available for future expansion
-   Hardware stack (`$0100-$01FF`): Only JSR/RTS return addresses

**Parameter Passing:**

-   Arguments stored sequentially starting at `$80`: first arg at `$80`, second at `$81`, etc.
-   Return values passed in A register (with X for 16-bit values)
-   Caller stores arguments before JSR, callee reads from parameter locations
-   Maximum 64 bytes of parameters per function call

**Call Depth:**

With only return addresses on hardware stack (2 bytes per JSR/RTS), the maximum call depth is **128 function calls**. If parameters were also stack-based, depth would be reduced to ~20-40 calls depending on parameter count.

## Memory Mapped I/O

Wraith provides a dedicated syntax for declaring memory-mapped I/O addresses. These declarations define a named address that can be read from or written to like a variable.

### Declaration

```
// Read-write address (default)
SCREEN: addr = 0x0400;

// Read-only address (compiler error on write)
RASTER: addr read = 0xD012;

// Write-only address (compiler error on read)
BORDER: addr write = 0xD020;
```

### Usage

```
fn update_screen() {
    // Write to address
    SCREEN = 0;

    // Read from address
    if RASTER > 100 {
        BORDER = 1;
    }
}
```

## Structs

### Declaration

```
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

```
p: Point = Point { x: 10, y: 20 };
p2: Point = Point { x: 5, y: 5 };
p2.x = 15;

x_coord: u8 = p.x;
```

## Enums

### Simple Enums

```
enum Direction {
    North = 0,
    South = 1,
    East = 2,
    West = 3,
}

dir: Direction = Direction::North;
```

### Enums with Data (Tagged Unions)

```
enum Message {
    Quit,                           // tag only
    Move { u8 x, u8 y },           // tag + 2 bytes
    Write(u8),                      // tag + 1 byte
    ChangeColor(u8, u8),           // tag + 2 bytes
}

msg: Message = Message::Move { x: 10, y: 20 };
```

**Memory Layout:**

-   First byte: tag (which variant)
-   Following bytes: variant data
-   Total size: 1 + max(variant sizes)

## Arrays and Slices

### Fixed Arrays

```
buffer: [u8; 10] = [0; 10];           // 10 bytes, all zeros
data: [u16; 5] = [100, 200, 300, 400, 500];

buffer[5] = 42;
value: u16 = data[2];
```

### Slices (Fat Pointers)

```
slice: &[u8];              // Slice (3 bytes: ptr_lo, ptr_hi, len)

// Arrays automatically coerce to slices in function calls
fn process_data(&[u8] data) {
    for i in 0..data.len {
        // use data[i]
    }
}

array: [u8; 10] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
process_data(array);      // Automatic coercion
```

**Slice Memory Layout:**

```
Byte 0: Pointer low byte
Byte 1: Pointer high byte
Byte 2: Length
```

## Pointers

### Pointer Types

```
ptr: *u8;              // Pointer to u8
struct_ptr: *Point;    // Pointer to struct
```

### Pointer Operations

```
x: u8  = 42;
ptr: *u8  = &x;              // Take address
value: u8 = *ptr;           // Dereference

y: u8  = 10;
ptr2: *u8  = &y;
*ptr2 = 20;                 // Modify through pointer

// Pointer arithmetic
screen: *u8 = 0x0400 as *u8;
*(screen + 10) = 65;       // Write 'A' at offset 10
```

## Control Flow

### If/Else

```
if x > 10 {
    // ...
} else if x > 5 {
    // ...
} else {
    // ...
}
```

### While Loop

```
while condition {
    // ...
}

while x < 100 {
    x = x + 1;
}
```

### Loop (Infinite)

```
loop {
    if done {
        break;
    }
}
```

### For Loop

```
// Range-based (type inferred from range bounds)
for i in 0..10 {      // 0 to 9, i is inferred as u8
    // ...
}

for i in 0..=255 {     // 0 to 255 (inclusive), i is u8
    // ...
}

for i in 0..1000 {     // Larger range, i is inferred as u16
    // ...
}

// Explicit type annotation
for i: u8 in 0..10 {
    // ...
}

// Over slices (type inferred from slice element type)
fn process(&[u8] data) {
    for item in data {   // item is inferred as u8
        // use item
    }
}
```

### Match Statement

```
// Match on values
match value {
    0 => { },
    1..=10 => { },       // Range
    _ => { },            // Default
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

## Type Casting

### Explicit Casting

```
small: u8 = 100;
large: u16 = small as u16;

signed: i8 = -10;
unsigned: u8 = signed as u8;

addr: u16 = 0x1000;
ptr: *u8 = addr as *u8;
```

No implicit conversions - all casts must be explicit.
No error checking - casts that are invalid will overflow/underflow

## Inline Assembly

### Basic Assembly Block

```
fn wait_for_vblank() {
    asm {
        "lda $D011"
        "bpl *-3"
    }
}
```

### Assembly with Variable Substitution

```
fn add_with_carry(a: u8, b: u8) -> u8 {
    result: u8;
    asm {
        "clc"
        "lda {a}"
        "adc {b}"
        "sta {result}"
    }
    return result;
}
```

Variables in `{}` are substituted with their memory locations.

## Attributes

### Function Attributes

```
#[inline]               // Suggest inlining
fn small_function() { }

#[irq]            // Interrupt request handler
fn irq() { }

#[nmi]            // Non maskable interrupt handler
fn nmi() { }

#[reset]          // Reset handler
fn reset() { }

#[org(0x8000)]         // Place at specific address
fn main() -> u8 { }

#[section("STDLIB")]   // Place in named memory section
fn imported_fn() { }
```

## Modules and Imports

Wraith supports a simple file-based import system for code organization and reuse.

### Import Syntax

```
import {symbol1, symbol2, symbol3} from "module.wr";
```

### Example

TODO - add example

### Import Resolution

-   **Relative imports**: Start with `./` or `../`

    ```
    import {foo} from "./utils.wr";
    import {bar} from "../lib/helper.wr";
    ```

-   **Non-relative imports**: Searched in standard library directory first, then current directory
    ```
    import {memcpy} from "std.wr";  // Searches stdlib first
    ```

### Limitations

These may be changed in future versions.

-   No module hierarchy or namespaces (flat file imports only)
-   No pub/private visibility (all symbols in a file are importable)
-   No re-exports
-   No wildcard imports (`import *`)

## Example Program

```
struct Sprite {
    x: u8,
    y: u8,
    color: u8,
}

sprites: [Sprite; 8];

fn init_sprites() {
    for i in 0..8 {
        sprites[i] = Sprite { x: 0, y: 0, color: 1 };
    }
}

fn update_sprite(index: u8, dx: i8, dy: i8) {
    if index >= 8 {
        return;
    }

    sprite: *Sprite = &sprites[index];
    sprite.x = ((sprite.x as i16) + (dx as i16)) as u8;
    sprite.y = ((sprite.y as i16) + (dy as i16)) as u8;
}

fn clear_memory(buffer: &[u8]) {
    for i in 0..buffer.len {
        buffer[i] = 0;
    }
}

SCREEN: *u8 = 0x0400 as *u8;
BORDER: *u8 = 0xD020 as *u8;

fn main() -> u8 {
    // Initialize display
    *BORDER = 0;

    // Setup sprites
    init_sprites();
    update_sprite(0, 5, -3);

    // Clear screen buffer
    buffer: [u8; 256] = [0; 256];
    clear_memory(buffer);

    return 0;
}
```

## Reserved Keywords

```
addr      asm       bool      break     const     else      enum
fn        for       from      i8        i16       if        import
in        inline    loop      match     return    struct    u8
u16       while     zp        as        true      false
```

## Operators

### Arithmetic

```
+   -   *   /   %        // Add, subtract, multiply, divide, modulo
<<  >>                   // Left shift, right shift
```

### Comparison

```
==  !=  <   >   <=  >=
```

### Logical

```
&&  ||  !
```

### Bitwise

```
&   |   ^   ~            // AND, OR, XOR, NOT
```

### Assignment

```
=   +=  -=  *=  /=  %=   // Assignment and compound assignment
&=  |=  ^=  <<=  >>=     // Bitwise compound assignment
```

## Comments

```
// Single line comment

/*
   Multi-line
   comment
*/
```
