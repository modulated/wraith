# 6502 Language Specification

A C-like systems programming language designed specifically for the 6502 processor, taking inspiration from Rust's type system while remaining simple and explicit.

## Design Philosophy

- **Simple and Explicit**: No type inference, explicit mutability, clear syntax
- **6502-Optimized**: Zero page hints, efficient calling convention, minimal overhead
- **Memory-Conscious**: Direct control over memory layout and access patterns
- **No Runtime Overhead**: Compile-time features only, no garbage collection

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

- All types must be explicitly declared
- No type inference
- No implicit conversions (must use `as` keyword)

## Variables

### Declaration Syntax

```
u8 x = 42;
i16 delta = -500;
bool flag = true;
```

### Mutability

```
u8 immutable = 10;           // Cannot be changed
mut u8 mutable = 20;         // Can be changed
mutable = 30;                // OK
```

### Zero Page Hint

```
zp u8 fast_var = 0;          // Suggest compiler use zero page
zp mut u16 counter = 0;      // Mutable zero page variable
```

Zero page ($00-$FF) provides faster access (3 cycles vs 4 cycles) and enables special addressing modes.

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

- `$02-$0F`: Function arguments (14 bytes)
- `$10-$1F`: Return values (16 bytes)
- `$20-$3F`: Caller-saved temporaries (32 bytes)
- `$40-$7F`: Callee-saved locals (64 bytes)
- Hardware stack (`$0100-$01FF`): Only JSR/RTS return addresses

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
addr SCREEN = 0x0400;

// Read-only address (compiler error on write)
addr read RASTER = 0xD012;

// Write-only address (compiler error on read)
addr write BORDER = 0xD020;
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
    u8 x,
    u8 y,
}

struct Entity {
    Point position,
    u8 health,
    u16 score,
}
```

### Usage

```
Point p = Point { x: 10, y: 20 };
mut Point p2 = Point { x: 5, y: 5 };
p2.x = 15;

u8 x_coord = p.x;
```

### Attributes

```
#[zp_section]
struct FastData {
    u8 counter,
    u8 temp,
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

Direction dir = Direction::North;
```

### Enums with Data (Tagged Unions)

```
enum Message {
    Quit,                           // tag only
    Move { u8 x, u8 y },           // tag + 2 bytes
    Write(u8),                      // tag + 1 byte
    ChangeColor(u8, u8),           // tag + 2 bytes
}

Message msg = Message::Move { x: 10, y: 20 };
```

**Memory Layout:**

- First byte: tag (which variant)
- Following bytes: variant data
- Total size: 1 + max(variant sizes)

## Arrays and Slices

### Fixed Arrays

```
[u8; 10] buffer = [0; 10];           // 10 bytes, all zeros
[u16; 5] data = [100, 200, 300, 400, 500];

buffer[5] = 42;
u16 value = data[2];
```

### Slices (Fat Pointers)

```
&[u8] slice;              // Read-only slice (3 bytes: ptr_lo, ptr_hi, len)
&[mut u8] mut_slice;      // Mutable slice

// Arrays automatically coerce to slices in function calls
fn process_data(&[u8] data) {
    for u8 i in 0..data.len {
        // use data[i]
    }
}

[u8; 10] array = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
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
*u8 ptr;              // Pointer to u8
*mut u8 mut_ptr;      // Mutable pointer to u8
*Point struct_ptr;    // Pointer to struct
```

### Pointer Operations

```
u8 x = 42;
*u8 ptr = &x;              // Take address
u8 value = *ptr;           // Dereference

mut u8 y = 10;
*mut u8 mut_ptr = &y;
*mut_ptr = 20;             // Modify through pointer

// Pointer arithmetic
*u8 screen = 0x0400 as *u8;
*(screen + 10) = 65;       // Write 'A' at offset 10
```

### Memory-Mapped I/O

```
*mut u8 VIC_BORDER = 0xD020 as *mut u8;
*mut u8 VIC_BACKGROUND = 0xD021 as *mut u8;
*u8 VIC_RASTER = 0xD012 as *u8;

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
for u8 i in 0..10 {      // 0 to 9
    // ...
}

for u8 i in 0..=10 {     // 0 to 10 (inclusive)
    // ...
}

// Over slices
fn process(&[u8] data) {
    for u8 item in data {
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
u8 small = 100;
u16 large = small as u16;

i8 signed = -10;
u8 unsigned = signed as u8;

u16 addr = 0x1000;
*u8 ptr = addr as *u8;
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
fn add_with_carry(u8 a, u8 b) -> u8 {
    u8 result;
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

- Fastest access (3 vs 4 cycles)
- Required for some addressing modes
- Limited to 256 bytes
- Suggested with `zp` keyword

### Stack ($0100-$01FF)

- Hardware stack, 256 bytes
- Used only for JSR/RTS (function return addresses)
- Not used for argument passing or locals

### General Memory ($0200+)

- Regular variables and data
- Slower access than zero page
- Unlimited (up to 64KB total address space)

## Complete Example

```
struct Sprite {
    u8 x,
    u8 y,
    u8 color,
}

[Sprite; 8] sprites;

fn init_sprites() {
    for u8 i in 0..8 {
        sprites[i] = Sprite { x: 0, y: 0, color: 1 };
    }
}

fn update_sprite(u8 index, i8 dx, i8 dy) {
    if index >= 8 {
        return;
    }

    mut *Sprite sprite = &sprites[index];
    sprite.x = ((sprite.x as i16) + (dx as i16)) as u8;
    sprite.y = ((sprite.y as i16) + (dy as i16)) as u8;
}

fn clear_memory(&[mut u8] buffer) {
    for u8 i in 0..buffer.len {
        buffer[i] = 0;
    }
}

*mut u8 SCREEN = 0x0400 as *mut u8;
*mut u8 BORDER = 0xD020 as *mut u8;

fn main() -> u8 {
    // Initialize display
    *BORDER = 0;

    // Setup sprites
    init_sprites();
    update_sprite(0, 5, -3);

    // Clear screen buffer
    mut [u8; 256] buffer = [0; 256];
    clear_memory(buffer);

    return 0;
}
```

## Assembly Mapping Examples

### Variable Access

```
u8 x = 42;

// Compiles to:
LDA #42
STA $0200    // x at $0200
```

### Zero Page Variable

```
zp u8 x = 42;

// Compiles to:
LDA #42
STA $80      // x at $80 (zero page)
```

### 16-bit Addition

```
u16 a = 100;
u16 b = 200;
u16 c = a + b;

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
fn add(u8 a, u8 b) -> u8 {
    return a + b;
}
u8 result = add(5, 10);

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
Point p = Point { x: 10, y: 20 };
u8 a = p.x;

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
[u8; 5] arr = [1, 2, 3, 4, 5];
u8 x = arr[2];

// Compiles to:
LDX #2
LDA arr_data,X
STA x
```

### Pointer Dereference

```
u8 value = 42;
*u8 ptr = &value;
u8 x = *ptr;

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
mut       return    struct    u8        u16       while     zp
as        true      false
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

## Future Considerations

Features not yet specified but may be added:

- Module system for code organization
- Basic preprocessor or build system integration
- Expanded standard library (if any)
- More sophisticated optimization hints
- Debugging annotations
- Macro system (currently excluded for simplicity)

## Design Rationale

### Why No Type Inference?

Keeps compilation simple and code explicit. On a resource-constrained system, clarity is paramount.

### Why Explicit Mutability?

Makes data flow clear and helps prevent bugs. The compiler can optimize immutable data.

### Why Zero Page Hints?

The 6502's zero page is a critical performance feature. Giving programmers control while letting the compiler decide is a good balance.

### Why Fat Pointers for Slices?

Bounds checking can be optional, but having length available enables safer code and clearer APIs. The 3-byte overhead is acceptable.

### Why This Calling Convention?

Zero page access is fast (3-4 cycles vs 4-5 for absolute). Keeping the hardware stack clean simplifies debugging and reduces overhead.
