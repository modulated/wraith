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

- [ ] Explain memory model and zero page usage
- [ ] Document calling conventions

### Design Philosophy

Wraith is a systems programming language designed specifically for the 6502 processor family. The language philosophy prioritizes:

1. **Explicitness over Convenience** - No type inference, no implicit conversions. Every operation must be explicit.
2. **Trust the Programmer** - Variables are mutable by default, no borrow checker, direct memory access.
3. **Zero Overhead** - Direct compilation to hand-optimized assembly with no runtime or hidden allocations.
4. **Hardware-Aware** - Language features map directly to 6502 capabilities (BCD types, interrupt handlers, zero page).
5. **Modern Syntax** - Rust-inspired syntax while remaining simple and explicit.

### Compilation Process

Wraith uses a multi-stage compilation process:

1. **Parsing** - Source `.wr` files are parsed into an Abstract Syntax Tree (AST)
2. **Semantic Analysis** - Type checking, constant evaluation, scope resolution
3. **Optimization** - Tail call optimization, dead code elimination, constant folding
4. **Code Generation** - Direct emission of 6502 assembly code
5. **Output** - Generates `.asm` file ready for your chosen 6502 assembler (ca65, DASM, etc.)

**Key Characteristics:**
- Compile-time constant evaluation and overflow checking
- Direct function calls use JSR/RTS, tail calls optimized to JMP
- Memory sections controlled via `wraith.toml` configuration
- No linking stage - generates complete assembly output

### Output Format

The compiler generates 6502 assembly code compatible with standard assemblers:
- Function labels for each `fn` declaration
- Memory-mapped addresses as absolute addressing
- Optimized register usage (A, X, Y)
- Stack-based parameter passing for complex types
- Section directives based on `wraith.toml` configuration

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

### Binary Coded Decimal (BCD) Types

BCD types (`b8` and `b16`) leverage the 6502's hardware decimal mode for efficient decimal arithmetic. Each nibble (4 bits) represents a single decimal digit (0-9).

**BCD Format:**
- `b8`: Two decimal digits (0-99), stored as `0xHH` where each H is 0-9
  - Example: `59` stored as `0x59`, NOT `0x3B`
- `b16`: Four decimal digits (0-9999), stored as `0xHHHH`
  - Example: `1234` stored as `0x1234`, NOT `0x04D2`

**Use Cases:**
- Game scores and timers (easy conversion to display)
- Financial calculations requiring exact decimal precision
- 7-segment display output
- ASCII digit conversion

**Operations:**
```rust
let score: b16 = 1000 as b16;
let points: b16 = 50 as b16;

// BCD addition uses 6502 decimal mode (SED/CLD)
score = score + points;  // 1000 + 50 = 1050 (stored as 0x1050)

// Subtraction also supported
score = score - points;  // 1050 - 50 = 1000

// Must cast to/from other types
let display: u8 = score as u8;  // Get low byte for display
```

**Important Notes:**
- BCD arithmetic is only valid for digits 0-9 in each nibble
- Invalid BCD values (nibbles A-F) produce undefined results
- Comparison operators work correctly on BCD values
- Multiplication and division require explicit loops or conversion to binary

### Type Overflow Behavior

**Compile-time Overflow:**
Constants are checked for overflow at compile time:
```rust
const VALID: u8 = 255;      // OK
const INVALID: u8 = 256;    // ERROR: constant overflow
const TOOBIG: b8 = 100;     // ERROR: BCD b8 max is 99
```

**Runtime Overflow:**
Runtime arithmetic wraps on overflow (no panic, no error):
```rust
let x: u8 = 255;
x = x + 1;           // Wraps to 0

let y: i8 = 127;
y = y + 1;           // Wraps to -128

let score: b16 = 9999 as b16;
score = score + (1 as b16);  // Wraps to 0000 in BCD
```

### Type Size and Alignment

All types are naturally aligned to their size:

| Type | Size | Alignment | Range |
|------|------|-----------|-------|
| `u8`, `i8`, `b8`, `bool` | 1 byte | 1 byte | See above |
| `u16`, `i16`, `b16` | 2 bytes | 1 byte (6502 has no alignment requirements) | See above |
| `addr` | 2 bytes | 1 byte | 0x0000-0xFFFF |

**Memory Layout for Multi-byte Types:**
- Little-endian (low byte first, matching 6502 architecture)
- `u16` at address `0x1000`: low byte at `0x1000`, high byte at `0x1001`

**Accessing Multi-byte Components:**
```rust
let value: u16 = 0x1234;
let low: u8 = value.low;    // 0x34
let high: u8 = value.high;  // 0x12
```

### Completion Status

All items completed.

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

### Variable Scope

Variables follow block-scoped visibility rules:

```rust
fn main() {
    let x: u8 = 10;    // Scope: entire function

    if true {
        let y: u8 = 20;  // Scope: only within if block
        x = x + y;       // OK - x is visible here
    }

    // y = 30;          // ERROR - y out of scope

    let z: u8 = x;     // OK - x still in scope
}
```

**Scope Rules:**
- Variables are visible from declaration point to end of containing block
- Inner blocks can access outer block variables
- Outer blocks cannot access inner block variables
- Function parameters have function scope

### Shadowing

Wraith supports variable shadowing - declaring a new variable with the same name:

```rust
fn calculate() {
    let x: u8 = 5;
    let x: u16 = x as u16;  // Shadows previous x, different type

    if x > 10 {
        let x: u8 = 3;      // Shadows again within block
        // x is u8 here
    }
    // x is u16 here
}
```

**Shadowing Characteristics:**
- Can change type of shadowed variable
- Previous variable becomes inaccessible
- Shadowing ends at block scope
- Useful for type conversions and reusing common names

### Zero Page Allocation

The 6502's zero page ($0000-$00FF) provides faster access and more addressing modes. Wraith allows explicit zero page allocation:

```rust
fn fast_loop() {
    zp let counter: u8 = 0;   // Allocated in zero page
    zp let temp: u16 = 0;     // Uses 2 bytes: $00xx and $00xx+1

    // Zero page variables enable faster addressing modes
    counter = counter + 1;     // Uses zero page addressing (faster)
}
```

**Zero Page Benefits:**
- Faster access (1 fewer cycle than absolute addressing)
- Shorter instruction encoding (saves ROM space)
- Required for some addressing modes (indirect, indexed)

**Zero Page Limitations:**
- Only 256 bytes total (including stack, which uses $0100-$01FF)
- Compiler automatically allocates temporary storage
- Manual ZP allocation may conflict with compiler usage
- Use sparingly for hot path variables only

**Strategy:**
- Compiler reserves $20-$7F for temporary storage
- User ZP variables should avoid compiler's range
- Function parameters may use zero page for small types
- Configuration for ZP ranges not yet implemented (future feature)

### Completion Status

All items completed.

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

Function attributes control code generation, placement, and calling conventions. They are specified using `#[attribute]` syntax before the function declaration.

#### `#[inline]`

Inlines the function body at each call site, eliminating JSR/RTS overhead:

```rust
#[inline]
fn add_two(x: u8) -> u8 {
    return x + 2;
}

fn main() {
    let result: u8 = add_two(5);  // Inline: no JSR, code inserted directly
}
```

**Characteristics:**
- No function label generated
- Arguments and locals embedded directly in caller
- Eliminates 12 cycle JSR/RTS overhead
- Increases code size if called multiple times
- Best for small, frequently-called functions

**When to Use:**
- Hot path functions called many times
- Small functions (< 10 instructions)
- Functions called from time-critical sections

#### `#[irq]` - Interrupt Request Handler

Marks function as IRQ (maskable interrupt) handler:

```rust
#[irq]
fn irq_handler() {
    // Handle timer interrupt, peripheral I/O, etc.
    let status: u8 = TIMER_STATUS;
    TIMER_STATUS = 0;  // Clear interrupt
}
```

**Characteristics:**
- Generates RTI (Return from Interrupt) instead of RTS
- Preserves A, X, Y registers automatically
- Installed at IRQ vector ($FFFE)
- Can be disabled via SEI instruction

**IRQ Vector Setup:**
The compiler generates appropriate interrupt vectors. In bare-metal systems:
- IRQ vector at $FFFE/$FFFF points to this handler
- Handler must clear interrupt source to prevent retriggering

#### `#[nmi]` - Non-Maskable Interrupt Handler

Marks function as NMI (non-maskable interrupt) handler:

```rust
#[nmi]
fn nmi_handler() {
    // Handle critical interrupts (cannot be disabled)
    NMI_FLAG = 1;
    STATUS_LED = 0xFF;
}
```

**Characteristics:**
- Generates RTI instead of RTS
- Cannot be disabled (always active)
- Installed at NMI vector ($FFFA)
- Triggered by external NMI pin or internal events

**Common NMI Uses:**
- Watchdog timer
- Critical hardware errors
- V-blank interrupt (video systems)
- Power failure detection

#### `#[reset]` - Reset/Entry Point Handler

Marks function as the reset handler (system entry point):

```rust
#[reset]
fn reset_handler() {
    // Initialize system
    STACK_POINTER = 0xFF;
    STATUS_LED = 0;

    // Enable interrupts
    enable_interrupts();

    // Jump to main program
    main();

    // Should never return - infinite loop
    loop { }
}
```

**Characteristics:**
- Installed at RESET vector ($FFFC)
- First code executed on power-up or reset
- Should initialize hardware and stack
- Typically calls main() after setup
- Should never return (use infinite loop)

**Reset Handler Responsibilities:**
1. Initialize stack pointer
2. Clear/initialize memory
3. Configure hardware
4. Enable interrupts (if desired)
5. Call main program
6. Prevent return (infinite loop or halt)

#### `#[org(address)]` - Fixed Address Placement

Places function at a specific memory address:

```rust
#[org(0x8000)]
fn bootloader() {
    // Code placed at exactly $8000
}

#[org(0xC000)]
fn io_routines() {
    // Code placed at exactly $C000
}
```

**Characteristics:**
- Overrides section placement
- Exact address specified as parameter
- Useful for ROM-specific layouts
- Can create address conflicts if not careful

**Use Cases:**
- ROM bootloader at fixed address
- API entry points at known addresses
- Hardware-required code placement
- Cartridge banking boundaries

#### `#[section("name")]` - Section Placement

Places function in a named memory section defined in `wraith.toml`:

```rust
#[section("STDLIB")]
fn helper_function() {
    // Placed in STDLIB section
}

#[section("CODE")]
fn game_logic() {
    // Placed in CODE section (often the default)
}
```

**Characteristics:**
- Section must be defined in `wraith.toml`
- Multiple functions can share section
- Linker-like behavior for organizing code
- Section ranges defined in configuration

**Example wraith.toml:**
```toml
[[sections]]
name = "STDLIB"
start = 0x8000
end = 0x8FFF

[[sections]]
name = "CODE"
start = 0x9000
end = 0xBFFF

default_section = "CODE"
```

### Tail Call Optimization

Wraith automatically optimizes tail-recursive functions to use JMP instead of JSR, eliminating stack growth:

```rust
// Tail-recursive factorial - optimized to loop
fn factorial(n: u8, acc: u16) -> u16 {
    if n == 0 {
        return acc;
    }
    // Tail call - compiler uses JMP instead of JSR
    return factorial(n - 1, acc * (n as u16));
}
```

**Generated Assembly (conceptual):**
```assembly
factorial:
    ; Check if n == 0
    LDA n
    BEQ return_acc

    ; Calculate acc * n
    ; ... multiplication code ...

    ; Tail call optimization: JMP instead of JSR
    DEC n
    JMP factorial    ; <-- JMP, not JSR!

return_acc:
    ; Return accumulator
    RTS
```

**Benefits:**
- Constant stack usage (no growth)
- Faster execution (no JSR/RTS overhead)
- Enables deep recursion without stack overflow

**Requirements for Tail Call Optimization:**
- Function must call itself as the last operation
- Return value must be directly returned (no modification)
- No code after the recursive call

### Parameter Passing and Return Values

**Parameter Passing:**
- `u8`, `i8`, `b8`, `bool`: Via accumulator (A register) for first parameter, zero page for additional
- `u16`, `i16`, `b16`: Via A (low) and Y (high) registers for first parameter, zero page for additional
- Larger types (structs, arrays): Via zero page or stack
- Multiple parameters: First in registers, rest in zero page

**Return Values:**
- `u8` types: Accumulator (A register)
- `u16` types: A (low byte) and Y (high byte)
- Larger types: Via pointer passed as parameter (out parameter pattern)

**Example:**
```rust
fn add(a: u8, b: u8) -> u8 {
    return a + b;
}

fn add16(a: u16, b: u16) -> u16 {
    return a + b;
}

fn main() {
    let sum: u8 = add(5, 3);        // a in A, b in ZP, result in A
    let sum16: u16 = add16(100, 200); // a in A/Y, b in ZP, result in A/Y
}
```

### Completion Status

All items completed.

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

### Memory Layout

Structs are laid out sequentially in memory with no padding:

```rust
struct Point {
    x: u8,    // Offset 0
    y: u8,    // Offset 1
}           // Total size: 2 bytes

struct Entity {
    position: Point,  // Offset 0-1 (2 bytes)
    health: u8,       // Offset 2
    score: u16,       // Offset 3-4 (2 bytes, little-endian)
}                   // Total size: 5 bytes
```

**Layout Characteristics:**
- No padding or alignment (sequential bytes)
- Nested structs inlined directly
- Multi-byte fields stored little-endian
- Total struct size = sum of field sizes

### Nested Structs

Structs can contain other structs as fields:

```rust
struct Vector {
    x: i16,
    y: i16,
}

struct Sprite {
    position: Vector,
    velocity: Vector,
    color: u8,
}

fn update_sprite(s: Sprite) {
    // Access nested fields
    let px: i16 = s.position.x;
    let vy: i16 = s.velocity.y;

    // Modify nested fields
    s.position.x = s.position.x + s.velocity.x;
    s.position.y = s.position.y + s.velocity.y;
}
```

### Structs in Arrays

Arrays can contain struct elements:

```rust
struct Enemy {
    x: u8,
    y: u8,
    health: u8,
}

const MAX_ENEMIES: u8 = 8;
let enemies: [Enemy; 8] = [
    {x: 10, y: 20, health: 100},
    {x: 30, y: 40, health: 100},
    // ... rest initialized to zero if not specified
];

// Access array of structs
enemies[0].health = enemies[0].health - 10;
let x_pos: u8 = enemies[3].x;
```

### Passing Structs to Functions

Small structs (1-2 bytes) passed in registers/zero page, larger structs via pointer or stack:

```rust
// Small struct - passed efficiently
fn move_point(p: Point, dx: u8, dy: u8) -> Point {
    p.x = p.x + dx;
    p.y = p.y + dy;
    return p;
}

// Large struct - typically passed by reference in real usage
fn update_entity(e: Entity) {
    e.health = e.health - 1;
}
```

### Completion Status

All items completed.

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

Wraith supports enum variants that carry data, allowing you to create tagged unions (also known as sum types or discriminated unions). There are two forms: tuple variants and struct variants.

#### Tuple Variants

Tuple variants carry unnamed fields accessed by position:

```rust
enum Option {
    None,
    Some(u8),
}

enum Color {
    RGB(u8, u8, u8),
}

enum Result {
    Ok(u16),
    Err(u8),
}

// Creating tuple variant instances
let value: Option = Option::Some(42);
let red: Color = Color::RGB(255, 0, 0);
let success: Result = Result::Ok(1000);
```

**Pattern Matching with Tuple Variants** (⚠️ EXPERIMENTAL - Limited Testing):

```rust
enum Option {
    None,
    Some(u8),
}

fn unwrap_or_default(opt: Option) -> u8 {
    match opt {
        Option::Some(value) => {
            // 'value' is extracted from the enum
            return value;
        }
        Option::None => {
            return 0;
        }
    }
}
```

⚠️ **Status**: Tuple variant pattern matching with data extraction has code generation support but is **experimental and minimally tested**. The implementation exists in the compiler but may have edge cases or bugs. Thorough testing is ongoing.

#### Struct Variants

Struct variants carry named fields:

```rust
enum Message {
    Quit,
    Move { x: u8, y: u8 },
    Write { text: str },
    ChangeColor { r: u8, g: u8, b: u8 },
}

// Creating struct variant instances
let msg: Message = Message::Move { x: 10, y: 20 };
let color: Message = Message::ChangeColor { r: 255, g: 128, b: 0 };
```

**Pattern Matching with Struct Variants** (❌ NOT YET IMPLEMENTED):

```rust
// This syntax is NOT currently supported:
match msg {
    Message::Move { x, y } => {  // ❌ Compilation error
        // Field extraction not implemented
    }
    _ => {}
}
```

❌ **Status**: Struct variant pattern matching with field extraction is **not yet implemented**. The compiler will return an error: "Pattern bindings for struct variants not yet implemented". For now, you can only match on struct variants without extracting their fields.

#### Memory Layout

Tagged unions are represented in memory with a discriminant tag followed by field data:

```
Memory layout for enum variants:
+------+--------+--------+--------+
| Tag  | Field0 | Field1 | Field2 |
| (u8) |   ...  |   ...  |   ...  |
+------+--------+--------+--------+
```

**Example**:
```rust
enum Color {
    RGB(u8, u8, u8),  // Tag 0
}

// Memory layout of Color::RGB(255, 128, 64):
// Byte 0: 0x00  (tag for RGB variant)
// Byte 1: 0xFF  (red = 255)
// Byte 2: 0x80  (green = 128)
// Byte 3: 0x40  (blue = 64)
```

**Important Notes**:
- The tag is always a `u8` (1 byte)
- Fields are laid out sequentially after the tag
- Total size = 1 byte (tag) + sum of field sizes
- Enum expressions evaluate to a pointer to the enum data (returned in A:X registers)
- Pattern matching loads the tag byte and compares it to variant discriminants

#### Mixed Variant Types

You can mix unit, tuple, and struct variants in the same enum:

```rust
enum Input {
    None,                          // Unit variant (tag only)
    Key(u8),                       // Tuple variant (tag + 1 byte)
    MouseClick { x: u8, y: u8 },   // Struct variant (tag + 2 bytes)
}

let input1: Input = Input::None;
let input2: Input = Input::Key(65);  // 'A' key
let input3: Input = Input::MouseClick { x: 100, y: 50 };
```

#### Current Limitations

1. **Struct variant pattern matching**: Cannot extract fields from struct variants in match arms (planned feature)
2. **Tuple variant testing**: Pattern matching with data extraction is minimally tested and may have bugs
3. **Complex nesting**: Deeply nested enums with data may have codegen issues
4. **Size calculations**: Each variant can have different sizes, making the enum size equal to the largest variant + 1 byte for the tag

### Default Discriminant Values

If not specified, discriminants start at 0 and increment:

```rust
enum Status {
    Idle,      // 0 (implicit)
    Running,   // 1 (implicit)
    Stopped,   // 2 (implicit)
}

enum Priority {
    Low = 10,
    Medium,    // 11 (continues from previous)
    High,      // 12
    Critical = 100,
}
```

**Rules:**
- First variant defaults to 0 if not specified
- Subsequent variants increment by 1 from previous
- Explicit values override auto-increment
- Values must fit in u8 (0-255)

### Pattern Matching with Enums

Use match statements for exhaustive enum handling:

```rust
enum Direction {
    North = 0,
    South = 1,
    East = 2,
    West = 3,
}

fn move_player(dir: Direction) {
    match dir {
        Direction::North => {
            y = y - 1;
        },
        Direction::South => {
            y = y + 1;
        },
        Direction::East => {
            x = x + 1;
        },
        Direction::West => {
            x = x - 1;
        },
    }
}

// Enum in conditions
if current_dir == Direction::North {
    // moving up
}
```

### Memory Representation

Enums are stored as single bytes (u8):

```rust
enum State {
    Off = 0,
    On = 1,
}

let s: State = State::On;  // Stored as u8 value 1
let raw: u8 = s as u8;     // Cast to u8: 1
```

**Characteristics:**
- Size: 1 byte (u8)
- Values: 0-255
- Can be cast to/from u8
- Used directly in comparisons
- Efficient switch/match compilation

### Completion Status

- [ ] Complete testing for tuple variant pattern matching (in progress)
- [ ] Implement struct variant pattern matching (planned)

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

### Array Bounds Checking

**Compile-time Checking:**
Constant indices are checked at compile time:

```rust
let data: [u8; 5] = [1, 2, 3, 4, 5];
let x: u8 = data[10];  // ERROR: index out of bounds (caught at compile time)
```

**Runtime Behavior:**
Variable indices have NO runtime bounds checking:

```rust
let data: [u8; 5] = [1, 2, 3, 4, 5];
let i: u8 = get_index();  // Unknown at compile time
let x: u8 = data[i];      // NO bounds check - undefined if i >= 5
```

**Programmer Responsibility:**
- Always ensure indices are within bounds
- Use constants when possible for compile-time checking
- Add manual checks for variable indices if needed

```rust
if i < 5 {
    let x: u8 = data[i];  // Safe
}
```

### Slice Operations

Slices are fat pointers (pointer + length) that reference array data:

```rust
// Function taking slice
fn sum_values(values: [u8]) -> u16 {
    let total: u16 = 0;
    for v in values {
        total = total + (v as u16);
    }
    return total;
}

// Arrays automatically coerce to slices
let data: [u8; 5] = [10, 20, 30, 40, 50];
let result: u16 = sum_values(data);  // Passes as slice
```

**Slice Characteristics:**
- Size: 2 bytes (pointer to data)
- Length tracked separately
- Read-only view of array data
- No slice syntax (e.g., `arr[1..3]`) - pass whole array only

### Slice Memory Representation

```rust
const DATA: [u8; 6] = [0, 1, 2, 3, 4, 5];

fn process(slice: [u8]) {
    // slice is a pointer to DATA
    // Length is known from array type
}
```

**Memory Layout:**
- Slice parameter: 2-byte pointer to first element
- Length: Tracked by compiler/type system
- Data: Stored wherever array is allocated (const data, stack, etc.)

### Multidimensional Arrays

Wraith supports arrays of arrays for multidimensional data:

```rust
// 2D array: 4 rows × 8 columns
let screen: [[u8; 8]; 4] = [
    [0, 0, 0, 0, 0, 0, 0, 0],
    [0, 1, 1, 1, 1, 1, 1, 0],
    [0, 1, 0, 0, 0, 0, 1, 0],
    [0, 0, 0, 0, 0, 0, 0, 0],
];

// Access elements
screen[1][3] = 2;  // Row 1, column 3
let pixel: u8 = screen[2][5];
```

**Memory Layout:**
- Row-major order (rows stored sequentially)
- No padding between elements
- Total size: rows × columns × element_size

### Array Assignment and Copying

Arrays are value types - assignment copies all elements:

```rust
let source: [u8; 3] = [1, 2, 3];
let dest: [u8; 3] = source;  // Copies all 3 bytes

source[0] = 10;  // dest is unchanged (independent copy)
```

**For large arrays**, use `memcpy` for efficiency:

```rust
let source: [u8; 100] = [...];
let dest: [u8; 100];
memcpy(&dest as u16, &source as u16, 100);
```

### Completion Status

All items completed.

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

### Continue Statement

Skip the rest of the current loop iteration and continue with the next:

```rust
for i in 0..10 {
    if i == 5 {
        continue;  // Skip when i is 5
    }
    process(i);  // Not called when i == 5
}

let counter: u8 = 0;
while counter < 100 {
    counter = counter + 1;

    if counter % 2 == 0 {
        continue;  // Skip even numbers
    }

    process_odd(counter);
}
```

**Behavior:**
- Jumps to start of next iteration
- Works in `for`, `while`, and `loop`
- Skips remaining code in current iteration

### Break Statement

Exit from a loop immediately:

```rust
loop {
    let input: u8 = read_input();

    if input == 0 {
        break;  // Exit loop
    }

    process(input);
}

// break in for loop
for i in 0..100 {
    if check_condition(i) {
        break;  // Exit early
    }
}
```

**Note:** Break with labels (e.g., `'outer: loop` and `break 'outer`) is not currently supported.

### Short-Circuit Conditions

Conditions in if/while use short-circuit evaluation:

```rust
// Check bounds before array access
if i < array.length && array[i] == target {
    // array[i] only evaluated if i < array.length
}

// Check null before dereference
if ptr != 0 && *ptr == value {
    // *ptr only dereferenced if ptr != 0
}
```

See [Operators](#operators) section for full short-circuit documentation.

### Completion Status

- [ ] Add assembly output examples for each control flow construct

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

### Valid Cast Combinations

**Integer Widening (Safe):**
```rust
let small: u8 = 100;
let large: u16 = small as u16;  // 100 -> 100 (zero-extended)

let signed: i8 = -10;
let wide: i16 = signed as i16;  // -10 -> -10 (sign-extended)
```

**Integer Narrowing (Truncation):**
```rust
let large: u16 = 0x1234;
let small: u8 = large as u8;  // 0x1234 -> 0x34 (truncate high byte)

let wide: i16 = -300;
let narrow: i8 = wide as i8;  // Truncates, may lose sign
```

**Signed ↔ Unsigned:**
```rust
let unsigned: u8 = 200;
let signed: i8 = unsigned as i8;  // 200 -> -56 (reinterpret bits)

let negative: i8 = -10;
let positive: u8 = negative as u8;  // -10 -> 246 (reinterpret bits)
```

**BCD Conversions:**
```rust
let bcd: b8 = 0x42 as b8;    // Binary 42 -> BCD 42
let bin: u8 = bcd as u8;     // BCD 42 -> Binary 0x42

let score: b16 = 1234 as b16; // Binary -> BCD 1234
let raw: u16 = score as u16;  // BCD 1234 -> 0x1234
```

**Boolean Conversions:**
```rust
let flag: bool = true;
let num: u8 = flag as u8;    // true -> 1, false -> 0

let value: u8 = 42;
let is_set: bool = value as bool;  // 0 -> false, nonzero -> true
```

### Truncation Behavior

When casting to a smaller type, high bytes are discarded:

```rust
let value: u16 = 0xABCD;
let low: u8 = value as u8;    // 0xCD (low byte)
let high: u8 = (value >> 8) as u8;  // 0xAB (high byte, shifted first)

// Multi-step truncation
let big: u16 = 0x1234;
let small: u8 = big as u8;    // 0x34
```

### Sign Extension

Signed casts preserve the sign by extending the sign bit:

```rust
let small: i8 = -1;          // 0xFF in binary
let large: i16 = small as i16;  // 0xFFFF (sign extended)

let positive: i8 = 127;      // 0x7F
let wide: i16 = positive as i16; // 0x007F (zero extended for positive)
```

**Manual Sign Extension (if needed):**
```rust
fn sign_extend_u8_to_u16(value: u8) -> u16 {
    if value >= 128 {  // Negative in i8
        return (value as u16) | 0xFF00;  // Sign extend
    }
    return value as u16;  // Positive, zero extend
}
```

### Completion Status

All items completed.

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

### Register Clobbering

Inline assembly can modify registers without compiler tracking:

```rust
fn custom_operation() -> u8 {
    let result: u8;
    asm {
        "LDA #$42",      // Load accumulator
        "CLC",
        "ADC #$10",
        "STA {result}",  // Store result
    }
    return result;  // A, X, Y may be clobbered
}
```

**Best Practices:**
- Store important values before inline asm
- Assume A, X, Y registers are clobbered
- Use variable substitution to save/restore state
- Keep assembly blocks short and focused

### Common Assembly Patterns

**Reading Hardware Registers:**
```rust
fn read_timer() -> u8 {
    let value: u8;
    asm {
        "LDA $D012",  // Read VIC-II raster register (C64 example)
        "STA {value}",
    }
    return value;
}
```

**Bit Manipulation:**
```rust
fn set_interrupt_mask(mask: u8) {
    asm {
        "LDA {mask}",
        "STA $D01A",   // VIA interrupt enable register
    }
}
```

**Timing-Critical Code:**
```rust
#[inline]
fn wait_cycles(count: u8) {
    asm {
        "LDX {count}",
        "loop:",
        "DEX",
        "BNE loop",
    }
}
```

**Direct Memory Block Operations:**
```rust
fn fast_clear(addr: u16, len: u8) {
    asm {
        "LDA #$00",
        "LDY {len}",
        "loop:",
        "STA ({addr}),Y",
        "DEY",
        "BNE loop",
    }
}
```

### Labels in Assembly

Use labels for loops and branches within asm blocks:

```rust
fn find_byte(haystack: u16, needle: u8, len: u8) -> u8 {
    let result: u8;
    asm {
        "LDA {needle}",
        "LDY #$00",
        "search_loop:",
        "CMP ({haystack}),Y",
        "BEQ found",
        "INY",
        "CPY {len}",
        "BNE search_loop",
        // Not found
        "LDA #$FF",
        "JMP done",
        "found:",
        "TYA",  // Transfer index to A
        "done:",
        "STA {result}",
    }
    return result;
}
```

**Label Scope:**
- Labels are local to the asm block
- Must be unique within the block
- Cannot reference labels outside the block

### Limitations

**What inline assembly CANNOT do:**
- Access local variables by name (must use substitution)
- Call Wraith functions directly (use JSR to label)
- Automatically save/restore registers
- Type checking on operations
- Bounds checking on memory access

**Size Limits:**
- No hard limit on asm block size
- Large blocks may impact optimization
- Consider using separate function for large asm

### Optimizer Interaction

Inline assembly is treated as opaque by the optimizer:

```rust
fn example() {
    let x: u8 = 10;

    // Optimizer cannot see what asm does
    asm {
        "NOP",
        "NOP",
    }

    let y: u8 = x + 5;  // Optimizer assumes x unchanged
}
```

**Implications:**
- Variables used in asm won't be optimized away
- Code motion around asm blocks is limited
- Asm blocks act as optimization barriers
- Use `#[inline]` on functions with small asm blocks for better optimization

### Completion Status

All items completed.

---

## Modules and Imports

Wraith supports a simple file-based import system for code organization and reuse.

### Import Syntax

```rust
import {symbol1, symbol2, symbol3} from "module.wr";
```

### Module Visibility

**All items are private by default.** Only items marked with `pub` can be imported from other modules.

#### Visibility Rules

- Functions, constants, structs, enums, and address declarations are private unless marked `pub`
- Private items cannot be imported by other modules
- Public items marked with `pub` can be imported
- Local variables, function parameters, and pattern bindings are always private

#### Example: Public Items

```rust
// file: math_utils.wr

// Public function - can be imported
pub fn add(a: u8, b: u8) -> u8 {
    return a + b;
}

// Private function - cannot be imported
fn internal_helper() -> u8 {
    return 42;
}

// Public constant - can be imported
pub const MAX_VALUE: u8 = 255;

// Private constant - cannot be imported
const INTERNAL_CONSTANT: u8 = 10;

// Public struct - can be imported
pub struct Point {
    x: u8,
    y: u8,
}

// Private struct - cannot be imported
struct InternalData {
    value: u8,
}

// Public enum - can be imported
pub enum Color {
    Red,
    Green,
    Blue,
}

// Private enum - cannot be imported
enum InternalState {
    Idle,
    Running,
}

// Public address - can be imported
pub const LED_PORT: addr = 0x6000;

// Private address - cannot be imported
const INTERNAL_PORT: addr = 0x6001;

#[reset]
fn main() {}
```

#### Using Public Items

```rust
// file: main.wr
import {add, MAX_VALUE, Point, Color, LED_PORT} from "math_utils.wr";

fn main() {
    // Can use all public items
    let sum: u8 = add(10, 20);
    let max: u8 = MAX_VALUE;
    let p: Point = Point { x: 5, y: 10 };
    let c: Color = Color::Red;
    LED_PORT = 1;

    // ERROR: Cannot import private items
    // import {internal_helper} from "math_utils.wr";  // Compile error!
}
```

#### Error: Importing Private Items

Attempting to import a private item results in a clear error:

```rust
import {internal_helper} from "math_utils.wr";
```

**Error Message:**
```
error: import error
  --> 1:9
    |
  1 | import {internal_helper} from "math_utils.wr";
    |         ^^^^^^^^^^^^^^^ symbol 'internal_helper' is private and cannot be imported
```

#### Visibility and API Design

The `pub` keyword enables explicit API boundaries:

```rust
// file: graphics_lib.wr

// Public API - stable interface
pub fn draw_sprite(x: u8, y: u8, sprite_id: u8) {
    setup_vram();
    write_sprite_data(x, y, sprite_id);
}

pub fn clear_screen() {
    fill_vram(0);
}

// Private implementation - can be changed without affecting users
fn setup_vram() {
    // Internal implementation
}

fn write_sprite_data(x: u8, y: u8, sprite_id: u8) {
    // Internal implementation
}

fn fill_vram(value: u8) {
    // Internal implementation
}
```

Users of `graphics_lib.wr` can only import `draw_sprite` and `clear_screen`, ensuring the internal implementation details remain encapsulated.

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

### Circular Import Detection

Wraith detects circular imports at compile time and reports an error:

```rust
// file: a.wr
import {b_function} from "b.wr";

fn a_function() {
    b_function();
}

// file: b.wr
import {a_function} from "a.wr";  // ERROR: circular import

fn b_function() {
    a_function();
}
```

**Error Message:**
```
error: circular import detected: a.wr -> b.wr -> a.wr
```

**Solution:** Restructure code to eliminate circular dependencies:
- Extract shared functionality to a third module
- Use forward declarations (if available)
- Reorganize module boundaries

**Example Fix:**
```rust
// file: common.wr
fn shared_function() { }

// file: a.wr
import {shared_function} from "common.wr";

// file: b.wr
import {shared_function} from "common.wr";
```

### Import Order and Dependencies

**Import Processing:**
1. Imports are processed depth-first
2. Each file is only processed once (subsequent imports are skipped)
3. Symbols must be defined before use within a file
4. No forward declarations - define functions/types before using them

**Import Order Best Practices:**
```rust
// Good: Import from most general to most specific
import {memcpy, memset} from "mem.wr";        // Standard library
import {helper_fn} from "./utils.wr";         // Local utilities
import {config} from "./config.wr";           // Local configuration
```

### Organizing Larger Projects

**Recommended Project Structure:**
```
my-project/
├── wraith.toml              # Memory configuration
├── main.wr                  # Entry point with #[reset]
├── lib/                     # Reusable modules
│   ├── graphics.wr         # Graphics routines
│   ├── input.wr            # Input handling
│   └── sound.wr            # Sound routines
├── game/                    # Game-specific code
│   ├── player.wr           # Player logic
│   ├── enemy.wr            # Enemy logic
│   └── levels.wr           # Level data
└── data/                    # Constants and tables
    ├── sprites.wr          # Sprite data
    └── maps.wr             # Map data
```

**Example main.wr:**
```rust
import {init_graphics, draw_sprite} from "./lib/graphics.wr";
import {read_input} from "./lib/input.wr";
import {update_player} from "./game/player.wr";

#[reset]
fn main() {
    init_graphics();

    loop {
        let input: u8 = read_input();
        update_player(input);
    }
}
```

**Example lib/graphics.wr:**
```rust
import {memset} from "mem.wr";  // stdlib import

const SCREEN: addr = 0x0400;

fn init_graphics() {
    memset(SCREEN as u16, 0x20, 255);
}

fn draw_sprite(x: u8, y: u8, sprite_id: u8) {
    // Drawing code
}
```

### Module Organization Best Practices

1. **One Responsibility Per Module**
   - Each `.wr` file should handle one clear area of functionality
   - Example: `graphics.wr`, `input.wr`, `physics.wr`

2. **Keep Related Code Together**
   - Group related functions, structs, and constants in the same file
   - Example: Player struct and update_player() in same file

3. **Minimize Cross-Module Dependencies**
   - Prefer importing from stdlib over custom modules when possible
   - Avoid long chains of imports (A imports B imports C...)

4. **Use Clear Naming**
   - Module names should describe their purpose
   - Avoid generic names like `utils.wr` or `helpers.wr`

5. **Document Module Purpose**
   - Add comments at the top of each file explaining its purpose
   - List major exports

```rust
// lib/graphics.wr
// Graphics system for 6502 display
// Exports: init_graphics(), draw_sprite(), clear_screen()

import {memset} from "mem.wr";

fn init_graphics() { }
fn draw_sprite(x: u8, y: u8, sprite_id: u8) { }
fn clear_screen() { }
```

### Limitations

These may be changed in future versions:

- **No module hierarchy or namespaces** - Flat file imports only
  - Cannot do `graphics::sprite::draw()`
  - All imports are at file level

- **No re-exports** - Cannot re-export imported symbols
  - Each module must import directly from the source

- **No wildcard imports** - Cannot use `import * from "module.wr"`
  - Must explicitly list each symbol to import

### Completion Status

All items completed.

---

## Standard Library

Wraith includes a small standard library optimized for 6502 architecture.

### Module: intrinsics.wr

Low-level CPU control functions that map directly to 6502 instructions. All functions are inlined for zero overhead.

**Import:**
```rust
import { enable_interrupts, disable_interrupts, nop } from "intrinsics.wr";
```

#### Interrupt Control

##### `enable_interrupts()`

Enable maskable interrupts (IRQ) by clearing the interrupt disable flag.

```rust
#[inline]
fn enable_interrupts()
```

**Maps to:** `CLI` (Clear Interrupt Disable)
**Cycles:** 2
**Use:** After calling, the CPU will respond to IRQ interrupts. NMI interrupts are always enabled.

**Example:**
```rust
#[reset]
fn reset_handler() {
    // Initialize hardware first
    setup_hardware();

    // Enable interrupts before main loop
    enable_interrupts();

    main();
}
```

##### `disable_interrupts()`
Disable maskable interrupts (IRQ) by setting the interrupt disable flag.

```rust
#[inline]
fn disable_interrupts()
```

**Maps to:** `SEI` (Set Interrupt Disable)
**Cycles:** 2
**Use:** Create critical sections that must not be interrupted. NMI cannot be disabled.

**Example:**
```rust
fn critical_update() {
    disable_interrupts();

    // Critical section - no IRQ interrupts
    update_shared_data();

    enable_interrupts();
}
```

#### Carry Flag Control

##### `clear_carry()`
Clear the carry flag before addition operations.

```rust
#[inline]
fn clear_carry()
```

**Maps to:** `CLC` (Clear Carry)
**Cycles:** 2
**Note:** Compiler normally handles this automatically for addition. Use for manual multi-byte arithmetic.

##### `set_carry()`
Set the carry flag before subtraction operations.

```rust
#[inline]
fn set_carry()
```

**Maps to:** `SEC` (Set Carry)
**Cycles:** 2
**Note:** Compiler normally handles this automatically for subtraction. Use for manual multi-byte arithmetic.

#### Decimal Mode Control

##### `clear_decimal()`

Switch CPU to binary arithmetic mode.

```rust
#[inline]
fn clear_decimal()
```

**Maps to:** `CLD` (Clear Decimal Mode)
**Cycles:** 2
**Note:** In binary mode (default), ADC and SBC perform normal binary addition/subtraction. Most programs run in binary mode.

##### `set_decimal()`

Switch CPU to Binary-Coded Decimal (BCD) arithmetic mode.

```rust
#[inline]
fn set_decimal()
```

**Maps to:** `SED` (Set Decimal Mode)
**Cycles:** 2
**Use:** In BCD mode, ADC and SBC treat values as packed BCD digits (0-9). Useful for decimal display calculations.

**Example:**
```rust
fn bcd_add(a: u8, b: u8) -> u8 {
    set_decimal();
    let result: u8 = a + b;  // BCD addition
    clear_decimal();
    return result;
}
```

**Note:** Wraith's `b8` and `b16` types automatically manage decimal mode.

#### Other CPU Control

##### `clear_overflow()`

Clear the overflow (V) flag in the processor status register.

```rust
#[inline]
fn clear_overflow()
```

**Maps to:** `CLV` (Clear Overflow)
**Cycles:** 2
**Note:** The overflow flag is set by ADC/SBC for signed arithmetic overflow.

##### `nop()`

Execute a no-operation instruction (2 cycle delay).

```rust
#[inline]
fn nop()
```

**Maps to:** `NOP` (No Operation)
**Cycles:** 2
**Uses:**
- Timing delays
- Code alignment
- Placeholder for future instructions

##### `brk()`

Trigger a software interrupt/breakpoint.

```rust
#[inline]
fn brk()
```

**Maps to:** `BRK` (Break)
**Cycles:** 7

**Behavior:**
1. Pushes PC+2 to stack
2. Pushes status flags to stack (with B flag set)
3. Sets interrupt disable flag
4. Jumps to IRQ/BRK vector at $FFFE

**Uses:**
- Debugging breakpoints
- System call interface
- Error handlers

**Note:** Most debuggers/emulators treat BRK as a breakpoint.

##### `set_stack_pointer(value: u8)`

Set the stack pointer to a specific value.

```rust
#[inline]
fn set_stack_pointer(value: u8)
```

**Maps to:** `LDX #value; TXS`
**Cycles:** 4 (2 for LDX, 2 for TXS)

**Note:** The 6502 stack lives in page 1 ($0100-$01FF). Common usage: `set_stack_pointer(0xFF)` to initialize SP to $01FF (top of stack).

**Example:**
```rust
#[reset]
fn reset_handler() {
    set_stack_pointer(0xFF);  // Initialize stack to top
    main();
}
```

### Module: mem.wr

Memory manipulation functions optimized for 6502.

**Import:**
```rust
import { memcpy, memset, memcmp, mem_read, mem_write } from "mem.wr";
```

#### Memory Block Operations

##### `memcpy(dest: u16, src: u16, len: u8)`

Copy `len` bytes from source address to destination address.

```rust
fn memcpy(dest: u16, src: u16, len: u8)
```

**Parameters:**
- `dest`: Destination address
- `src`: Source address
- `len`: Number of bytes to copy (max 255)

**Note:** Uses indexed addressing with Y register. Memory regions can overlap.

**Example:**
```rust
const SOURCE_DATA: [u8; 5] = [1, 2, 3, 4, 5];
let SCREEN_BUFFER: addr = 0x0400;

memcpy(SCREEN_BUFFER as u16, &SOURCE_DATA as u16, 5);
```

##### `memset(dest: u16, value: u8, len: u8)`

Fill `len` bytes at destination with a constant value.

```rust
fn memset(dest: u16, value: u8, len: u8)
```

**Parameters:**
- `dest`: Destination address
- `value`: Byte value to fill with
- `len`: Number of bytes to fill (max 255)

**Use Cases:**
- Clear screen buffers
- Initialize arrays
- Zero memory regions

**Example:**
```rust
let SCREEN: addr = 0x0400;

// Clear screen with spaces (0x20)
memset(SCREEN as u16, 0x20, 255);
```

##### `memcmp(a: u16, b: u16, len: u8) -> u8`

Compare two memory regions for equality.

```rust
fn memcmp(a: u16, b: u16, len: u8) -> u8
```

**Parameters:**
- `a`: First memory region address
- `b`: Second memory region address
- `len`: Number of bytes to compare (max 255)

**Returns:**
- `1` if regions are equal
- `0` if regions differ

**Example:**
```rust
const EXPECTED: [u8; 4] = [0x12, 0x34, 0x56, 0x78];
let TEST_DATA: addr = 0x6000;

if memcmp(&EXPECTED as u16, TEST_DATA as u16, 4) == 1 {
    // Memory matches
}
```

#### Indirect Memory Access

##### `mem_read(address: u16) -> u8`

Read a byte from an address using indirect addressing.

```rust
fn mem_read(address: u16) -> u8
```

**Equivalent to:** `byte = *(address)` in C

**Uses:** 6502 indirect indexed addressing mode `LDA (addr),Y`

**Example:**
```rust
let value: u8 = mem_read(0x0400);  // Read from $0400
```

##### `mem_write(address: u16, value: u8)`

Write a byte to an address using indirect addressing.

```rust
fn mem_write(address: u16, value: u8)
```

**Equivalent to:** `*(address) = value` in C

**Uses:** 6502 indirect indexed addressing mode `STA (addr),Y`

**Example:**
```rust
mem_write(0x0400, 42);  // Write 42 to $0400
```

##### `mem_jump(address: u16)`

Transfer execution to the specified address.

```rust
fn mem_jump(address: u16)
```

**Maps to:** `JMP (address)` - indirect jump

**Warning:** Execution may not return unless the target code explicitly returns. Typically used for monitor/debugger "Go" commands.

**Example:**
```rust
// Jump to code at $8000
mem_jump(0x8000);
// Execution continues at $8000
```

### Module: math.wr

Mathematical operations optimized for 6502/65C02. Focus on unsigned 8-bit values with efficient assembly implementations.

**Import:**
```rust
import { min, max, clamp, set_bit, clear_bit, saturating_add, mul16, div16 } from "math.wr";
```

#### Comparison Operations

##### `min(a: u8, b: u8) -> u8`

Return the smaller of two u8 values.

```rust
#[inline]
fn min(a: u8, b: u8) -> u8
```

**Cycles:** ~8 (1 comparison + 1 conditional branch)
**Optimization:** Uses CMP and BCC to avoid boolean intermediate

**Example:**
```rust
let health: u8 = min(current_health, 100);  // Cap at 100
```

##### `max(a: u8, b: u8) -> u8`

Return the larger of two u8 values.

```rust
#[inline]
fn max(a: u8, b: u8) -> u8
```

**Cycles:** ~8 (1 comparison + 1 conditional branch)
**Optimization:** Uses CMP and BCS to avoid boolean intermediate

**Example:**
```rust
let damage: u8 = max(base_damage, 1);  // Minimum 1 damage
```

##### `clamp(value: u8, min_val: u8, max_val: u8) -> u8`

Clamp a value between min and max bounds (inclusive).

```rust
#[inline]
fn clamp(value: u8, min_val: u8, max_val: u8) -> u8
```

**Cycles:** ~12-16 (best case: in range, worst case: clamped twice)
**Optimization:** Two comparisons with early exit

**Example:**
```rust
let volume: u8 = clamp(user_input, 0, 15);  // Clamp to 0-15 range
```

#### Bit Manipulation (65C02)

Uses 65C02 SMB/RMB/BBS instructions for efficient bit operations.

**Note:** These functions use zero page $20 for temporary storage.

##### `set_bit(value: u8, bit: u8) -> u8`

Set a specific bit (0-7) in a byte using 65C02 SMB instructions.

```rust
#[inline]
fn set_bit(value: u8, bit: u8) -> u8
```

**Cycles:** ~18-20
**Uses:** 65C02 SMB (Set Memory Bit) instructions
**Temporary Storage:** Zero page $20

**Example:**
```rust
let flags: u8 = 0b00000000;
flags = set_bit(flags, 3);  // Set bit 3 -> 0b00001000
```

##### `clear_bit(value: u8, bit: u8) -> u8`

Clear a specific bit (0-7) in a byte using 65C02 RMB instructions.

```rust
#[inline]
fn clear_bit(value: u8, bit: u8) -> u8
```

**Cycles:** ~18-20
**Uses:** 65C02 RMB (Reset Memory Bit) instructions
**Temporary Storage:** Zero page $20

**Example:**
```rust
let flags: u8 = 0b11111111;
flags = clear_bit(flags, 5);  // Clear bit 5 -> 0b11011111
```

##### `test_bit(value: u8, bit: u8) -> u8`

Test if a specific bit (0-7) is set using 65C02 BBS instructions.

```rust
#[inline]
fn test_bit(value: u8, bit: u8) -> u8
```

**Cycles:** ~20-22
**Uses:** 65C02 BBS (Branch on Bit Set) instructions
**Returns:** 1 if bit is set, 0 if clear
**Temporary Storage:** Zero page $20

**Example:**
```rust
let status: u8 = 0b00010000;
if test_bit(status, 4) == 1 {
    // Bit 4 is set
}
```

#### Saturating Arithmetic

##### `saturating_add(a: u8, b: u8) -> u8`

Add two u8 values with saturation at 255 (no wrap-around).

```rust
#[inline]
fn saturating_add(a: u8, b: u8) -> u8
```

**Cycles:** ~6-8 (optimized to leave result in accumulator)
**Returns:** a + b, or 255 if overflow would occur

**Example:**
```rust
let health: u8 = 250;
health = saturating_add(health, 10);  // Result: 255, not wrap to 4
```

##### `saturating_sub(a: u8, b: u8) -> u8`

Subtract b from a with saturation at 0 (no wrap-around).

```rust
#[inline]
fn saturating_sub(a: u8, b: u8) -> u8
```

**Cycles:** ~6-8 (optimized to leave result in accumulator)
**Returns:** a - b, or 0 if underflow would occur

**Example:**
```rust
let ammo: u8 = 3;
ammo = saturating_sub(ammo, 5);  // Result: 0, not wrap to 254
```

#### Advanced Bit Operations

##### `count_bits(value: u8) -> u8`

Count the number of set bits (1s) in a byte.

```rust
#[inline]
fn count_bits(value: u8) -> u8
```

**Cycles:** ~58-76 (8 iterations, optimized using Y register)
**Returns:** Number of 1 bits in the value (0-8)

**Example:**
```rust
let bits: u8 = count_bits(0b10110101);  // Returns 5
```

##### `reverse_bits(value: u8) -> u8`

Reverse the bits in a byte (bit 0 ↔ bit 7, bit 1 ↔ bit 6, etc.).

```rust
#[inline]
fn reverse_bits(value: u8) -> u8
```

**Cycles:** ~66-76
**Temporary Storage:** Zero page $20
**Example:** `0b11010010` → `0b01001011`

**Example:**
```rust
let reversed: u8 = reverse_bits(0xA5);  // 0xA5 -> 0xA5 (palindrome)
let test: u8 = reverse_bits(0x01);      // 0x01 -> 0x80
```

##### `swap_nibbles(value: u8) -> u8`

Swap the high and low nibbles (4-bit halves) of a byte.

```rust
#[inline]
fn swap_nibbles(value: u8) -> u8
```

**Cycles:** ~10-14 (optimized to leave result in accumulator)
**Example:** `0xAB` → `0xBA`

**Example:**
```rust
let swapped: u8 = swap_nibbles(0x12);  // 0x12 -> 0x21
let color: u8 = swap_nibbles(0xF0);    // 0xF0 -> 0x0F
```

#### 16-bit Arithmetic

##### `mul16(a: u16, b: u16) -> u16`

Multiply two 16-bit unsigned integers using shift-and-add algorithm.

```rust
fn mul16(a: u16, b: u16) -> u16
```

**Algorithm:** Shift-and-add method (optimized for 6502)
**Cycles:** ~800-1000 (depends on number of set bits in multiplier)
**Returns:** a × b (lower 16 bits if result overflows)
**Temporary Storage:** Zero page $20-$27, parameters in $80-$83

**Example:**
```rust
let area: u16 = mul16(320, 200);  // Screen area calculation
```

##### `div16(a: u16, b: u16) -> u16`

Divide two 16-bit unsigned integers using non-restoring division.

```rust
fn div16(a: u16, b: u16) -> u16
```

**Algorithm:** Non-restoring division (optimized for 6502)
**Cycles:** ~1200-1400 (16 iterations of shift-subtract)
**Returns:** a ÷ b (quotient), or 0xFFFF if b == 0
**Temporary Storage:** Zero page $20-$27, parameters in $80-$83

**Example:**
```rust
let average: u16 = div16(total_score, num_players);

// Division by zero handling
let result: u16 = div16(100, 0);  // Returns 0xFFFF
if result == 0xFFFF {
    // Handle division by zero
}
```

### 65C02 vs 6502 Compatibility

**65C02-Specific Features:**
- Bit manipulation functions (`set_bit`, `clear_bit`, `test_bit`) use SMB/RMB/BBS instructions
- These instructions are NOT available on original 6502 (only 65C02 and later)
- If targeting original 6502, avoid these functions or implement alternatives

**6502-Compatible Functions:**
- All other stdlib functions work on both 6502 and 65C02
- `min`, `max`, `clamp`, `saturating_add`, `saturating_sub` - 6502 compatible
- `count_bits`, `reverse_bits`, `swap_nibbles` - 6502 compatible
- `mul16`, `div16` - 6502 compatible
- All `mem.wr` functions - 6502 compatible
- All `intrinsics.wr` functions - 6502 compatible

### Completion Status

- [ ] Add assembly output examples

---

## Reserved Keywords

The following **37 keywords** are reserved in Wraith and cannot be used as identifiers:

```
addr      as        asm       b8        b16       bool      break
carry     const     continue  else      enum      false     fn
for       from      i8        i16       if        import    in
let       loop      match     negative  overflow  pub       read
return    str       struct    true      u8        u16       while
write     zero      zp
```

### Keywords by Category

**Control Flow (9 keywords):**
```
if        else      while     loop      for
match     return    break     continue
```

**Variable Declaration (3 keywords):**
```
let       const     zp
```

**Type Keywords (8 keywords):**
```
u8        i8        u16       i16
b8        b16       bool      str
```

**Function and Type Declarations (3 keywords):**
```
fn        struct    enum
```

**Module System (3 keywords):**
```
import    from      pub
```

**CPU Status Flags - Read-Only (4 keywords):**
```
carry     zero      overflow  negative
```

**Type Casting and Iteration (2 keywords):**
```
as        in
```

**Memory and I/O (4 keywords):**
```
addr      asm       read      write
```

**Boolean Literals (2 keywords):**
```
true      false
```

### Future Reserved Keywords

No additional keywords are currently planned for future versions.

### Keyword Usage Examples

**Type Keywords:**
```rust
let count: u8 = 10;         // Unsigned 8-bit
let delta: i16 = -500;      // Signed 16-bit
let score: b16 = 1234 as b16;  // BCD 16-bit
let flag: bool = true;      // Boolean
```

**Variable Declaration:**
```rust
let x: u8 = 42;             // Mutable variable
const MAX: u8 = 100;        // Compile-time constant
zp let counter: u8 = 0;     // Zero-page variable (faster access)
```

**CPU Status Flags:**
```rust
fn check_arithmetic() {
    let result: u8 = add_numbers(250, 10);
    if carry {
        // Overflow occurred
    }
    if zero {
        // Result was zero
    }
}
```

**Memory-Mapped I/O:**
```rust
let LED: addr = 0x6000;           // Memory-mapped address
let BUTTON: read addr = 0x6001;   // Read-only address
let OUTPUT: write addr = 0x6002;  // Write-only address
```

**Inline Assembly:**
```rust
fn custom_operation() {
    asm {
        "LDA #$42",
        "STA $6000"
    }
}
```

### Notes

- All keywords are **case-sensitive** (e.g., `if` is a keyword, but `If` or `IF` are valid identifiers)
- Keywords cannot be used as variable names, function names, struct names, or any other identifiers
- There is no mechanism to escape keywords (unlike Rust's `r#` syntax)
- **Note:** `inline` is NOT a reserved keyword - it appears only in function attributes as `#[inline]`

### Completion Status

All items completed.

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

### Operator Precedence

Operators are listed from highest to lowest precedence:

| Precedence | Operator | Description | Associativity |
|------------|----------|-------------|---------------|
| 1 (highest) | `()` | Grouping/Function call | Left-to-right |
| 2 | `.` `[]` | Member access, Array indexing | Left-to-right |
| 3 | `!` `~` `-` (unary) | Logical NOT, Bitwise NOT, Negation | Right-to-left |
| 4 | `*` `/` `%` | Multiplication, Division, Modulo | Left-to-right |
| 5 | `+` `-` | Addition, Subtraction | Left-to-right |
| 6 | `<<` `>>` | Left shift, Right shift | Left-to-right |
| 7 | `<` `<=` `>` `>=` | Comparison | Left-to-right |
| 8 | `==` `!=` | Equality | Left-to-right |
| 9 | `&` | Bitwise AND | Left-to-right |
| 10 | `^` | Bitwise XOR | Left-to-right |
| 11 | `\|` | Bitwise OR | Left-to-right |
| 12 | `&&` | Logical AND | Left-to-right |
| 13 | `\|\|` | Logical OR | Left-to-right |
| 14 (lowest) | `=` `+=` `-=` etc. | Assignment operators | Right-to-left |

**Examples:**
```rust
let x: u8 = 2 + 3 * 4;      // 14, not 20 (multiplication before addition)
let y: u8 = (2 + 3) * 4;    // 20 (parentheses override)
let z: bool = !a && b;      // (!a) && b (NOT before AND)
let w: u8 = a + b << 2;     // (a + b) << 2 (addition before shift)
```

### Arithmetic Overflow Behavior

All arithmetic operators wrap on overflow with no error checking:

```rust
let x: u8 = 255 + 1;     // 0 (wraps)
let y: u8 = 0 - 1;       // 255 (wraps)
let z: u8 = 200 * 2;     // 144 (400 % 256)

let i: i8 = 127 + 1;     // -128 (signed overflow wraps)
let j: i8 = -128 - 1;    // 127 (signed underflow wraps)
```

**Division and Modulo:**
- Division by zero: Result is undefined (no runtime check)
- Modulo by zero: Result is undefined (no runtime check)
- Programmer responsibility to check divisor

### Short-Circuit Evaluation

Logical operators `&&` and `||` use short-circuit evaluation:

```rust
// && stops evaluating if first operand is false
if x > 0 && expensive_check(x) {
    // expensive_check() is NOT called if x <= 0
}

// || stops evaluating if first operand is true
if quick_check() || slow_check() {
    // slow_check() is NOT called if quick_check() returns true
}
```

**Benefits:**
- Avoids unnecessary computation
- Prevents errors (e.g., array bounds checking)
- Common pattern: `if ptr != 0 && *ptr == value`

### Completion Status

- [ ] Document operator implementation in assembly

---

## Comments

Wraith supports three types of comments: single-line comments, multi-line comments, and documentation comments.

### Single-Line Comments

Single-line comments start with `//` and continue to the end of the line:

```rust
fn calculate() -> u8 {
    let x: u8 = 42;  // Initialize x to 42
    // This entire line is a comment
    return x;
}
```

### Multi-Line Comments

Multi-line comments begin with `/*` and end with `*/`. They can span multiple lines:

```rust
/*
   This is a multi-line comment.
   It can span across multiple lines.
   Useful for longer explanations or temporarily disabling code blocks.
*/
fn complex_function() {
    /* You can also use multi-line comments inline */ let x: u8 = 10;
}
```

**Note**: Multi-line comments do **not** nest. The first `*/` closes the comment block:

```rust
/* This is /* NOT a nested comment */ and this causes an error */
```

### Documentation Comments

Documentation comments use triple slashes (`///`) and are used to document functions, structs, and other items. These are commonly used in the standard library:

```rust
/// Enable interrupts by clearing the interrupt disable flag
/// Maps to: CLI (Clear Interrupt Disable)
/// Cycles: 2
#[inline]
fn enable_interrupts() {
    asm {
        "CLI"
    }
}

/// Add two u8 values with saturation at 255
/// Returns: a + b, or 255 if overflow would occur
/// Cycles: ~6-8
fn saturating_add(a: u8, b: u8) -> u8 {
    // implementation
}
```

Documentation comments are typically placed immediately before the item they document and should describe:
- What the function/struct/item does
- Parameter meanings (if not obvious)
- Return value semantics
- Performance characteristics (cycle counts for 6502)
- Hardware mapping (for intrinsics)

### Comments in Inline Assembly

Comments can be used within inline assembly blocks. Both comment styles work:

```rust
fn example_asm() {
    asm {
        // Single-line comment in assembly
        "LDA #$42",     // Load accumulator with 0x42

        /*
           Multi-line comment explaining
           the next few instructions
        */
        "STA $6000",
        "RTS"           // Return from subroutine
    }
}
```

**Important**: Assembly string literals themselves are passed directly to the assembler and should use the assembler's comment syntax (typically `;` for 6502 assemblers):

```rust
fn with_assembler_comments() {
    asm {
        "LDA #$42    ; Assembler comment (inside the string)",
        // Wraith comment (outside the string)
        "STA $6000"
    }
}
```

### Best Practices

**DO:**
- Use `///` documentation comments for public API functions
- Include cycle counts for performance-critical functions
- Comment non-obvious bit manipulation or hardware interactions
- Explain "why" rather than "what" in regular comments
- Use comments to mark TODO items or known limitations

```rust
/// Fast integer division by 10 using multiplication and shifts
/// Cycles: ~45 (much faster than div16)
fn div10_fast(value: u8) -> u8 {
    // Using multiply by 0xCD and shift right by 11 bits
    // This works because 0xCD / 2048 ≈ 1/10 for u8 range
    // TODO: Verify accuracy for values > 200
}
```

**DON'T:**
- Over-comment obvious code
- Leave commented-out code in production
- Use comments to describe what the code literally does (if it's clear)

```rust
// BAD: Obvious comment
let x: u8 = 42;  // Set x to 42

// GOOD: Explains why
let x: u8 = 42;  // Magic number from hardware spec (p. 23)

// BAD: Commented-out code
// let old_value: u8 = some_old_function();

// GOOD: TODO with context
// TODO: Replace with hardware timer when available (issue #42)
let delay: u8 = software_delay(100);
```

### Comment Preprocessor Interaction

Comments are stripped during lexical analysis and do not affect code generation. This means:

```rust
fn test() {
    let x: u8 = 10 /* comment in middle */ + 5;  // Valid, equals 15
}
```

However, comments inside assembly string literals are **not** processed by Wraith:

```rust
asm {
    "LDA #$42  ; This semicolon comment goes to the assembler",
    // This slash comment is processed by Wraith
}
```

### Completion Status

All items completed.

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
