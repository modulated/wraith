# 6502 Language Specification

A systems programming language designed specifically for the 6502 processor, taking modern inspiration while remaining simple and explicit.

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

-   All types must be explicitly declared
-   No type inference
-   No implicit conversions (must use `as` keyword)

## Variables

### Declaration Syntax

```
let x: u8 = 42;
let delta: i16 = -500;
let flag: bool = true;
```

### Mutability

**All variables are mutable by default**. This is a low-level systems language that trusts the programmer.

```
let x: u8 = 10;
x = 20;                      
```

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
let LED: addr = 0x6000;      // Memory-mapped LED
let BUTTON: addr = 0x6001;   // Memory-mapped button

fn main() {
    LED = 1;                 // Write to address
    state: u8 = BUTTON;      // Read from address
}
```

Addresses can be read from or written to like variables, but they represent fixed memory locations.
They can also be marked as read only or write only and this is enforced at compile time.

```
let LED: write addr = 0x6000;    // Write only address
let BUTTON: read addr = 0x6001;  // Read only address

fn main() {
    LED = 1;                 // Write to address - OK
    let x = LED;             // Read from write only address - compile time error;
    let state: u8 = BUTTON;  // Read from address - OK
    BUTTON = 0;              // Write to read only address - compile time error;
}
```

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
- [ ] TODO - rewrite this section to be better
There are several function attributes:
- inline
- org
- irq
- nmi
- reset

- [ ] TODO: generate examples of all attributes

```
#[inline]         // Inlines the assembly without generating a jump to label and storing arguments/local variables
fn fast_function(x: u8) -> u8 {
    return x * 2;
}

#[irq]            // Interrupt request handler
fn irq() { }

#[nmi]            // Non maskable interrupt handler
fn nmi() { }

#[reset]          // Reset handler
fn reset() { }

#[org(0x8000)]         // Place generated machine code at specific address
fn main() -> u8 { }

#[section("STDLIB")]   // Place generated machine code in named section
fn imported_fn() { }
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
let p1: Point = { x: 10, y: 20 };
let p2: Point = { x: 5, y: 5 };
p2.x = 15;

x_coord: u8 = p1.x;
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

- [ ] TODO: ?implemented in language
      

## Arrays and Slices

### Fixed Arrays

```
let buffer: [u8; 10] = [0; 8];           // 10 bytes, all zeros
let data: [u16; 5] = [100, 200, 300, 400, 500];

buffer[5] = 42;
let value: u16 = data[2];
```

### Slices

```
const DATA: [u8; 6] = [0, 1, 2, 3, 4, 5];


array: [u8; 10] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
process_data(array);      // Automatic coercion
```

## Strings

### String Type

Strings in Wraith are length-prefixed byte sequences optimized for the 6502. The string type is declared as `str`.

```
let message: str = "Hello, World!";
let empty: str = "";
```

**Storage Format:**
- `[u8 length][byte data...]` - single byte length prefix followed by character data
- Maximum length: 255 bytes (enforced at compile time)
- Pointer size: 2 bytes (standard 6502 pointer)

### String Literals

String literals support escape sequences:

```
let msg1: str = "Hello\n";          // Newline
let msg2: str = "Tab\there";        // Tab
let msg3: str = "Quote: \"Hi\"";    // Escaped quotes
let msg4: str = "Backslash: \\";    // Backslash
```

### String Properties

Access string metadata:

```
let msg: str = "Hello";
let len: u16 = msg.len;      // Get length (5)
```

### String Indexing

Access individual characters by index:

```
let msg: str = "ABC";
let first: u8 = msg[0];    // 'A' (0x41)
let second: u8 = msg[1];   // 'B' (0x42)
```

**Note:** Indexing is bounds-checked at runtime in debug builds.

### String Concatenation

Concatenate strings at compile time using the `+` operator:

```
const GREETING: str = "Hello, " + "World!";
const PATH: str = "data/" + "level" + ".txt";
```

**Requirements:**
- Both operands must be compile-time constant strings
- Result must not exceed 256 bytes
- Evaluated entirely at compile time (zero runtime cost)

### String Slicing

Extract substrings at compile time:

```
const FULL: str = "Hello, World!";
const GREETING: str = FULL[0..5];     // "Hello"
const NAME: str = FULL[7..12];        // "World"
const COMMA: str = FULL[5..7];        // ", "
```

**Slice Syntax:**
- `start..end` - Exclusive end (standard)
- `start..=end` - Inclusive end
- Bounds must be constant expressions
- Empty slices are not allowed (compile error)
- Result is validated to fit within 256 bytes

### String Iteration

Iterate over characters in a string:

```
// Simple iteration
for c in message {
    // c is u8 (each character)
    process_char(c);
}

// With index
for (i, c) in message {
    // i is u8 (index), c is u8 (character)
    buffer[i] = c;
}
```

**Performance Note:** String iteration is optimized to use the X register as a counter, providing efficient 8-bit indexing on the 6502.

### String Pointer Caching

Frequently accessed strings are automatically cached in zero page for faster access:

```
fn process_string(s: str) {
    // Accessing the same string 3+ times triggers caching
    let len1 = s.len;
    let len2 = s.len;  // Uses cached pointer
    let len3 = s.len;  // Uses cached pointer
}
```

**Benefits:**
- ~60% faster access after initial setup
- Cache initialized once at function entry
- No manual intervention required

### Cross-Module String Pooling

Identical strings across different modules are automatically deduplicated using content-based hashing:

```
// file1.wr
pub const MSG: str = "Error";

// file2.wr
import { MSG } from "file1.wr";
const LOCAL: str = "Error";  // Shares storage with MSG
```

**Benefits:**
- Saves memory when multiple modules use the same strings
- Strings are identified by hash of content
- Automatic and transparent to the programmer

### Limitations

- Maximum string length: 255 bytes
- No runtime string mutation
- No runtime string concatenation
- String comparisons must be done manually (element by element)
- No built-in string search/replace operations

These limitations are intentional for the 6502 platform - strings are designed for static data like messages, labels, and constants rather than dynamic text processing.

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
fn process(data: [u8]) {
    for item in data {   // item is inferred as u8
        // process item
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
let small: u8 = 100;
let large: u16 = small as u16;

let signed: i8 = -10;
let unsigned: u8 = signed as u8; // will result in 246

let addr: u16 = 0x1000;
```

No implicit conversions - all casts must be explicit.
No error checking - casts that are invalid will overflow/underflow

## Inline Assembly

### Basic Assembly Block

```
fn increment() {
    asm {
        "clc",
        "adc #1"
    }
}
```

### Assembly with Variable Substitution

```
fn add_with_carry(a: u8, b: u8) -> u8 {
    result: u8;
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


## Modules and Imports

Wraith supports a simple file-based import system for code organization and reuse.

### Import Syntax

```
import {symbol1, symbol2, symbol3} from "module.wr";
```

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

## Reserved Keywords
- [ ] TODO: these may be incorrect - need to check later

```
addr      asm       bool      break     const     else      enum
fn        for       from      i8        i16       if        import
in        inline    loop      match     return    struct    u8
u16       while     as        true      false
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
