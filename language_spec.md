# 6502 Language Specification

A C-like systems programming language designed specifically for the 6502 processor, taking inspiration from Rust's type system while remaining simple and explicit.

## Design Philosophy

-   **Simple and Explicit**: No type inference, explicit mutability, clear syntax
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

All variables are mutable by default for simplicity. This is a low-level systems language - trust the programmer.

```
x: u8 = 10;
x = 20;                      // OK - all variables are mutable
```

### Zero Page Hint

The `zp` keyword suggests the compiler allocate a variable to zero page for faster access. This is a **hint**, not a requirement - the compiler may ignore it if zero page space is exhausted.

```
zp fast_var: u8 = 0;         // Compiler will try to use zero page
zp counter: u16 = 0;         // Zero page hint
```

Zero page ($00-$FF) provides faster access (3 cycles vs 4 cycles) and enables special addressing modes. When register allocation optimization is enabled, the compiler may also promote frequently-used variables to zero page automatically, even without the `zp` hint.

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
inline fn fast_function(x: u8) -> u8 {
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

**Zero Page Convention:**

-   `$02-$0F`: Function arguments (14 bytes)
-   `$10-$1F`: Return values (16 bytes)
-   `$20-$3F`: Caller-saved temporaries (32 bytes)
-   `$40-$7F`: Callee-saved locals (64 bytes)
-   Hardware stack (`$0100-$01FF`): Only JSR/RTS return addresses

**Example:**

```
fn add(a: u8, b: u8) -> u8 {
    return a + b;
}

// Compiles to:
// Args: a=$02, b=$03
// Return: $10
// Caller loads $02, $03, JSR add, reads $10
```

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

### Attributes

```
#[zp_section]
struct FastData {
    counter: u8,
    temp: u8,
}
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

### Memory-Mapped I/O

```
VIC_BORDER: *u8 = 0xD020 as *u8;
VIC_BACKGROUND: *u8 = 0xD021 as *u8;
VIC_RASTER: *u8 = 0xD012 as *u8;

fn set_colors() {
    *VIC_BORDER = 0;
    *VIC_BACKGROUND = 6;
}
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
// Range-based
for i in 0..10 {      // 0 to 9
    // ...
}

for i in 0..=10 {     // 0 to 10 (inclusive)
    // ...
}

// Over slices
fn process(&[u8] data) {
    for item in data {
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

#[noreturn]             // Never returns
fn panic() { }

#[interrupt]            // Interrupt handler
fn irq() { }

#[org(0x8000)]         // Place at specific address
fn main() -> u8 { }
```

### Type Attributes

```
#[zp_section]
struct FastVars {
    u8 counter,
}

#[packed]
struct TightLayout {
    u8 a,
    u16 b,
}
```

## Memory Layout

### Zero Page ($00-$FF)

-   Fastest access (3 vs 4 cycles)
-   Required for some addressing modes
-   Limited to 256 bytes
-   Requested with `zp` keyword hint (compiler will try to honor)

### Stack ($0100-$01FF)

-   Hardware stack, 256 bytes
-   Used only for JSR/RTS (function return addresses)
-   Not used for argument passing or locals

### General Memory ($0200+)

-   Regular variables and data
-   Slower access than zero page
-   Unlimited (up to 64KB total address space)

## Complete Example

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

## Assembly Mapping Examples

### Variable Access

```
x: u8 = 42;

// Compiles to:
LDA #42
STA $0200    // x at $0200
```

### Zero Page Variable

```
zp x: u8 = 42;

// Compiles to:
LDA #42
STA $80      // x at $80 (zero page)
```

### 16-bit Addition

```
a: u16 = 100;
b: u16 = 200;
c: u16 = a + b;

// Compiles to:
CLC
LDA $0200    // a low
ADC $0202    // b low
STA $0204    // c low
LDA $0201    // a high
ADC $0203    // b high
STA $0205    // c high
```

### Function Call

```
fn add(a: u8, b: u8) -> u8 {
    return a + b;
}
result: u8 = add(5, 10);

// Compiles to:
LDA #5
STA $02      // arg0
LDA #10
STA $03      // arg1
JSR add
LDA $10      // return value
STA result

add:
    CLC
    LDA $02
    ADC $03
    STA $10
    RTS
```

### Struct Access

```
p: Point = Point { x: 10, y: 20 };
a: u8 = p.x;

// Compiles to:
LDA #10
STA $0200    // p.x
LDA #20
STA $0201    // p.y

LDA $0200    // load p.x
STA $0202    // store to a
```

### Array Indexing

```
arr: [u8; 5] = [1, 2, 3, 4, 5];
x: u8 = arr[2];

// Compiles to:
LDX #2
LDA arr_data,X
STA x
```

### Pointer Dereference

```
value: u8 = 42;
ptr: *u8 = &value;
x: u8 = *ptr;

// Compiles to:
LDA #42
STA $0200    // value

LDA #$00
STA $0201    // ptr low
LDA #$02
STA $0202    // ptr high

LDY #0
LDA ($0201),Y  // dereference
STA $0203    // x
```

### Match Statement Jump Table

```
match tag {
    0 => { case0(); },
    1 => { case1(); },
    2 => { case2(); },
}

// Compiles to:
LDA tag
ASL          // multiply by 2
TAX
LDA jump_table+1,X
PHA
LDA jump_table,X
PHA
RTS          // jump to address

jump_table:
    .word case0-1
    .word case1-1
    .word case2-1
```

## Reserved Keywords

```
asm       bool      break     else      enum      fn        for
i8        i16       if        in        inline    loop      match
return    struct    u8        u16       while     zp        as
true      false
```

Note: `mut` is reserved for future use but not currently active in the language.

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

## Future Considerations

Features not yet specified but may be added:

-   Module system for code organization
-   Basic preprocessor or build system integration
-   Expanded standard library (if any)
-   More sophisticated optimization hints
-   Debugging annotations
-   Macro system (currently excluded for simplicity)

## Design Rationale

### Why No Type Inference?

Keeps compilation simple and code explicit. On a resource-constrained system, clarity is paramount.

### Why Zero Page Hints?

The 6502's zero page is a critical performance feature. Giving programmers control while letting the compiler decide is a good balance.

### Why Fat Pointers for Slices?

Bounds checking can be optional, but having length available enables safer code and clearer APIs. The 3-byte overhead is acceptable.

### Why This Calling Convention?

Zero page access is fast (3-4 cycles vs 4-5 for absolute). Keeping the hardware stack clean simplifies debugging and reduces overhead.
