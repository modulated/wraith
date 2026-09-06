# Wraith Language Specification

A systems programming language designed specifically for the 6502 processor, taking modern inspiration while remaining simple and explicit.

<!--
  The test suite compiles tagged examples on every run
  (tests/e2e/spec_examples.rs), so they cannot drift out of date:

    ```rust,compile           a complete translation unit, compiled as written.
                              Use for anything declaring its own fn / struct /
                              enum / static / const / import at the top level.

    ```rust,compile,fragment  a run of statements, compiled inside a generated
                              `#[reset] fn main() { … loop {} }`. Use for the
                              many examples that illustrate one expression or
                              declaration, so the prose does not have to show a
                              wrapper the reader does not care about. A fragment
                              must not declare top-level items — the suite
                              checks this.

  Leave a block as plain ```rust when it references peripherals or functions
  defined elsewhere in the prose, or when it deliberately shows code that does
  not compile (an error example). Making such a block self-contained and
  tagging it is a welcome change.
-->


## Table of Contents

- [Overview](#overview)
- [Basic Types](#basic-types)
- [Variables](#variables)
- [Functions](#functions)
- [Structs](#structs)
- [Enums](#enums)
- [Arrays and Slices](#arrays-and-slices)
- [Pointers](#pointers)
- [Strings](#strings)
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

- [x] Explain memory model and zero page usage (see [Zero Page Allocation](#zero-page-allocation))
- [x] Document calling conventions (see [Parameter Passing and Return Values](#parameter-passing-and-return-values), [Appendix C](#appendix-c-calling-convention))

### Design Philosophy

Wraith is a systems programming language designed specifically for the 6502 processor family. The language philosophy prioritizes:

1. **Explicitness over Convenience** - Every variable's type is declared, and narrowing or signedness changes must be written out with `as`. The only quiet conversions are the lossless ones: `u8` → `u16`, `i8` → `i16`, `bool` → `u8`, and integer literals adopting the type the context expects.
2. **Trust the Programmer** - Variables are mutable by default, no borrow checker, direct memory access.
3. **Zero Overhead** - Direct compilation to hand-optimized assembly with no runtime or hidden allocations.
4. **Hardware-Aware** - Language features map directly to 6502 capabilities (BCD types, interrupt handlers, zero page). The *processor* is what the language knows. It assumes nothing about the machine built around it: what peripherals exist, where they are decoded, how much memory there is, and what is ROM are all things you state — peripherals as `addr` declarations, memory as `wraith.toml` sections. No address is special to the compiler except the ones the 6502 itself fixes.
5. **Modern Syntax** - Rust-inspired syntax while remaining simple and explicit.

### Compilation Process

Wraith uses a multi-stage compilation process:

1. **Parsing** - Source `.wr` files are parsed into an Abstract Syntax Tree (AST)
2. **Semantic Analysis** - Type checking, constant evaluation, scope resolution
3. **Optimization** - Tail call optimization, dead code elimination, constant folding
4. **Code Generation** - Direct emission of 6502 assembly code
5. **Output** - Generates a flat, fully-placed `.ORG` `.asm` image, assembled by the bundled `flatasm` (or your own assembler; note the output is absolute, not relocatable — `ca65`'s linker model does not apply)

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
- Zero-page frame-based parameter passing for all types, including structs, arrays, and enums (passed as 2-byte pointers)
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
q8.8    // 16-bit signed fixed-point (8 integer + 8 fraction bits)
bool    // Boolean (represented as u8: 0 or 1)
char    // ASCII character (an 8-bit value holding one ASCII codepoint, 0-127)
```

A `char` literal is written with single quotes and supports the same escapes as
strings: `'A'`, `'0'`, `'\n'`, `'\t'`, `'\\'`, `'\''`, `'\0'`. Only ASCII
characters are allowed — `'é'` is a compile error. A `char` is a distinct
1-byte type: convert to and from `u8` with an explicit cast, and it
zero-extends to `u16`/`i16` like any other unsigned byte.

```rust,compile,fragment
let c: char = 'A';
let code: u8 = c as u8;      // 0x41
let back: char = 66 as char; // 'B'
```

A [string](#strings) is semantically an array of `char`: indexing a `str` yields
a `char`, and iterating one binds a `char` per character.

### Type Characteristics

- All types must be explicitly declared
- No type inference (beyond literals adopting the declared type)
- Narrowing and signedness changes require an explicit `as` cast; lossless widening (`u8`→`u16`, `i8`→`i16`, `bool`→`u8`) is implicit

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
```rust,compile,fragment
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

### Fixed-Point Type (`q8.8`)

`q8.8` is a signed 16-bit fixed-point number: 8 integer bits and 8 fraction
bits. The value is a two's-complement `i16` scaled by 256, so `1.5` is stored
as `0x0180`. It covers the fractional quantities a 6502 program draws with —
positions, velocities, angles — without a software float.

**Range and resolution:**
- Range: `-128.0` to `127.99609375`
- Resolution: `1/256` ≈ `0.0039`

**Literals.** A fractional literal (`1.5`, `0.25`, `3.75`) is a `q8.8` value in
a fixed-point context. It is scaled exactly at compile time — no binary float is
involved — and is out of range if its integer part does not fit. A *bare*
integer is not adopted as fixed-point (its bytes would be the raw value, not the
scaled one); write `3.0`, or cast with `3 as q8.8`.

```rust,compile,fragment
let pos: q8.8 = 1.5;      // 0x0180
let vel: q8.8 = 0.25;     // 0x0040
let two: q8.8 = 2.0;      // 0x0200
```

**Arithmetic.** Add and subtract are plain 16-bit two's-complement arithmetic —
no decimal mode, cheaper than BCD — and comparisons are signed 16-bit:

```rust,compile,fragment
let pos: q8.8 = 0.0;
let vel: q8.8 = 0.25;
pos = pos + vel;          // one 16-bit add, no shift
if pos < vel { }          // signed 16-bit compare
```

Multiply is `(a·b) >> 8`: the full 32-bit product of the two encodings with the
fraction shifted back out (stdlib `mulq88`, a signed widening shift-and-add).
Divide is `(a << 8) / b`: the dividend is widened to 24 bits so the quotient
keeps its 8 fraction bits (stdlib `divq88`, a signed restoring division). Both
**truncate** the dropped low bits and **wrap** on overflow, like every other
arithmetic result; divide by zero is the all-ones sentinel, as at every width:

```rust,compile,fragment
let a: q8.8 = 3.0;
let b: q8.8 = 2.0;
let area: q8.8 = a * b;   // 6.0
let half: q8.8 = a / b;   // 1.5
```

Modulo and the bitwise operators do not apply to a scaled value and are refused.

**Conversions.** Both directions need an explicit `as`:
- `<int> as q8.8` scales up (the integer becomes the whole part).
- `q8.8 as <int>` takes the integer part — an arithmetic shift right by 8, so it
  rounds toward negative infinity: `(-1.5) as i16` is `-2`.

```rust,compile,fragment
let n: u8 = 5;
let f: q8.8 = n as q8.8;  // 5.0 = 0x0500
let whole: u8 = f as u8;  // 5
```

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
```rust,compile,fragment
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
| `u8`, `i8`, `b8`, `bool`, `char` | 1 byte | 1 byte | See above |
| `u16`, `i16`, `b16`, `q8.8` | 2 bytes | 1 byte (6502 has no alignment requirements) | See above |
| `addr` | 2 bytes | 1 byte | 0x0000-0xFFFF |

**Memory Layout for Multi-byte Types:**
- Little-endian (low byte first, matching 6502 architecture)
- `u16` at address `0x1000`: low byte at `0x1000`, high byte at `0x1001`

**Accessing Multi-byte Components:**
```rust,compile,fragment
let value: u16 = 0x1234;
let low: u8 = value.low;    // 0x34
let high: u8 = value.high;  // 0x12
```

Both halves are assignable, and write one byte of the value in place:

```rust,compile,fragment
let value: u16 = 0x1234;
value.high = 0x56;          // value is now 0x5634
```

The target has to name storage — a `let` local or a `static`. A `const` is
ROM, a parameter is an immutable copy, and a call's result is not a place;
all three are rejected.

### Completion Status

All items completed.

---

## Variables

### Declaration Syntax

```rust,compile,fragment
let x: u8 = 42;
let delta: i16 = -500;
let flag: bool = true;
```

### Mutability

**All variables are mutable by default**. This is a low-level systems language that trusts the programmer.

```rust,compile,fragment
let x: u8 = 10;
x = 20;  // OK - variables are mutable
```

### Constants

Use the `const` keyword to declare compile-time constants. Constants are evaluated at compile time and cannot be reassigned.

```rust,compile
const MAX_SPRITES: u8 = 8;
const SCREEN_WIDTH: u16 = 320;
const PI_TIMES_100: u16 = 314;

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

### Mutable Globals (`static`)

`const` declares immutable data that lives in ROM. Use `static` for state that
must be **written at runtime and shared across functions** — including interrupt
handlers:

```rust,compile
static RX_HEAD: u8 = 0;
static RX_BUF: [u8; 64] = [0; 64];
static TICKS: u16 = 0;

#[irq]
fn on_irq() {
    TICKS = TICKS + 1;      // handler updates shared state
}

#[reset]
fn main() {
    RX_BUF[RX_HEAD] = 0x41;
    loop {}
}
```

This is the only way to share state between an interrupt handler and the main
program: local variables are allocated in per-function frames that the compiler
*colors* by the call graph, so a handler cannot see another function's locals.

**Characteristics:**
- Stored in the `BSS` section (RAM), allocated in declaration order
- Initial values are written by the reset handler, because RAM contents are
  undefined at power-on (large all-zero blocks use a compact fill loop)
- Arrays and structs are supported; arrays are indexed with absolute-indexed
  addressing
- A struct's initialiser may name any field kind: a number, a `bool`, an array,
  a nested struct, a function, a `str`, an enum variant, and `&OTHER` for
  another `static`. The last four are two bytes or a tag the assembler or the
  linker fills in, which is why they are initialisers rather than assignments:

  ```rust,compile
  enum Mode { Idle, Busy }
  struct Dev { name: str, mode: Mode, handler: fn(u8) -> u8, buf: &u8 }
  static SPARE: u8 = 0;
  fn echo(x: u8) -> u8 { return x; }
  static DEV: Dev = Dev { name: "uart", mode: Mode::Idle, handler: echo, buf: &SPARE };

  #[reset]
  fn main() { loop {} }
  ```
- `addr` may not be declared `static` — an `addr` names a fixed hardware
  location, so it stays `const`
- A two-byte `static` shared with an interrupt handler can tear unless its
  accesses are made indivisible — see [`atomic`](#interrupt-safe-statics-atomic)
  below

#### Interrupt-Safe Statics (`atomic`)

A two-byte load or store is two instructions. If an interrupt handler reads (or
writes) the same `static` between them, it sees — or leaves — a value that is
half old and half new. `TICKS` above is the classic case: the handler increments
it while the main program reads it.

Prefix the `static` with `atomic` and the compiler masks interrupts around every
whole-variable read and every assignment, so each access is indivisible:

```rust,compile
const OUT_LO: addr = 0x0200;
const OUT_HI: addr = 0x0201;
atomic static TICKS: u16 = 0;

#[irq]
fn on_irq() {
    TICKS = TICKS + 1;          // the whole read-modify-write is masked
}

#[reset]
fn main() {
    let now: u16 = TICKS;       // read both bytes with no handler in between
    OUT_LO = now.low;
    OUT_HI = now.high;
    loop {}
}
```

The guard is `PHP; SEI; …; PLP` — it **saves and restores** the interrupt-disable
flag rather than unconditionally re-enabling it, so it is correct inside a
handler (which already has interrupts masked) and when one atomic access nests in
another. A whole assignment is masked as a unit, so `TICKS = TICKS + 1` cannot
lose an update; the RHS is evaluated inside the mask, so keep it short (avoid a
call there — interrupts stay off across it).

Notes and limits:
- `atomic` is only for a **two-byte scalar** `static` (`u16`, `i16`, a pointer, a
  function pointer). A **one-byte** value is already atomic — a byte load or
  store is one instruction — so `atomic` on it warns and emits nothing.
- It does not apply to a `const` (immutable, never torn), to a local, or to an
  aggregate (array, struct, slice); those are compile errors.
- It makes each *access* indivisible, not a *transaction* spanning several
  statements. `a = TICKS; …; TICKS = a + f();` still has a gap; that needs a
  critical section around the whole sequence.

**Configuring RAM.** The `BSS` region defaults to `$0400-$07FF` (1 KB of user
RAM, clear of the zero page, the hardware stack at `$0100-$01FF`, and the
compiler's software-stack page at `$0200-$02FF`). Override it in `wraith.toml`
to match your board:

```toml
[[sections]]
name = "BSS"
start = 0x0400
end = 0x07FF
description = "User RAM for mutable globals"
```

The compiler warns if an `addr` declaration falls inside the `BSS` range, since
it would collide with allocated globals, and errors if the globals overflow the
configured region.

### Memory-Mapped Addresses

Use the `addr` keyword to declare memory-mapped I/O addresses:

```rust,compile
const LED: addr = 0x6000;      // Memory-mapped LED
const BUTTON: addr = 0x6001;   // Memory-mapped button

fn main() {
    LED = 1;                 // Write to address
    let state: u8 = BUTTON;  // Read from address
}
```

Addresses can be read from or written to like variables, but they represent fixed memory locations.
They can also be marked as read only or write only and this is enforced at compile time.

```rust
const LED: write addr = 0x6000;    // Write only address
const BUTTON: read addr = 0x6001;  // Read only address

fn main() {
    LED = 1;                 // Write to address - OK
    let x = LED;             // Read from write only address - compile time error
    let state: u8 = BUTTON;  // Read from address - OK
    BUTTON = 0;              // Write to read only address - compile time error
}
```

### Variable Scope

Variables follow block-scoped visibility rules:

```rust,compile
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

Wraith does **not** allow variable shadowing — redeclaring a name that is
already in scope is a compile error:

```rust
fn calculate() {
    let x: u8 = 5;
    let x: u16 = x as u16;  // ERROR: duplicate symbol 'x'
}
```

Bind a new name instead, and let the frame allocator share the storage:

```rust,compile
fn calculate() {
    let x: u8 = 5;
    let x_wide: u16 = x as u16;  // OK - a distinct name

    if x_wide > 10 {
        let x_small: u8 = 3;     // OK - a distinct name
    }
}
```

**Why:** a local's zero-page slot is assigned by call-graph coloring, not by
scope, so a shadowed name would not get fresh storage anyway — the old and
new bindings would alias. Rejecting the redeclaration makes that visible
instead of surprising.

### Zero Page Allocation

The 6502's zero page ($0000-$00FF) provides faster access (one fewer cycle than absolute addressing), shorter instruction encoding, and is required for indirect/indexed addressing modes. **Every local variable and function parameter is automatically allocated to zero page** - there is no `zp` keyword or manual opt-in; the compiler handles placement for you.

```rust,compile
fn fast_loop() {
    let counter: u8 = 0;   // Automatically allocated in zero page
    let temp: u16 = 0;     // 2 bytes, automatically allocated in zero page

    counter = counter + 1;  // Uses zero page addressing (faster)
}
```

#### Frame Allocation and Call-Graph Coloring

Each function gets a **frame**: a contiguous block of zero page holding its parameters followed by its local variables, sized to exactly what that function uses. Frames are placed using a static allocation strategy based on the program's call graph:

- If function `B` can be called from function `A` (directly or transitively), `B`'s frame is placed **above** `A`'s frame, so a callee can never overwrite its caller's live variables.
- If two functions can never be active at the same time (e.g. sibling functions that are never in the same call chain), their frames are allowed to **share the same addresses**. This is why a program with many small functions doesn't need one zero-page byte per variable in the whole program - only enough for the deepest call chain.
- This coloring is computed once at compile time; there is no runtime allocation or garbage collection involved, and non-recursive calls have zero overhead beyond the normal argument copy and `JSR`.

**Recursion:** a function that calls itself (directly or through a cycle of mutually recursive functions) is a special case - its own frame would otherwise be overwritten by the nested call before the outer call finishes using it. For these calls only, the compiler automatically saves the callee's frame to a small software stack before the call and restores it afterward. This is fully automatic and invisible to the programmer; see [Tail Call Optimization](#tail-call-optimization) for the (zero-overhead) tail-recursive case, which does not need this save/restore at all.

That software stack is a fixed 256-byte region, so a recursive function can only nest `256 / bytes_per_call` times before it overflows and silently corrupts data - the 6502 has no stack-limit detection, so this is not caught at runtime.

Two things go on that stack per call, and both count toward the limit: the callee's saved **frame**, and any **operand spilled across the call** (when a binary operation's right-hand side contains a call, the left-hand value cannot stay in a register or the zero-page pool across the `JSR`). A one-byte frame is not automatically safe — `return (n as u16) + s(n - 1)` saves one frame byte but also spills a two-byte operand, so it costs three bytes a level and tops out around 85.

To catch this early, the compiler emits a **compile-time warning** naming the computed depth whenever that per-call cost makes the software stack run out before the hardware stack does. (The 6502 hardware stack independently caps *any* non-tail recursion at roughly 128 nested calls — two bytes of return address per `JSR` in page 1 — so a function that outlasts that is not flagged, since the software stack tells the programmer nothing new.) Tail-recursive functions are exempt entirely: they become loops and push nothing.

The warning is not an error. Recursion depth is a runtime property, and a function that only ever nests a few levels is safe; the warning reports the ceiling so the choice is an informed one. Suggested fixes are tail recursion or an explicit loop.

**Interrupt handlers** (`#[irq]`/`#[nmi]`) can preempt main-line code at any point, including mid-expression. The compiler tracks which zero-page scratch and frames a handler's call graph can touch and automatically saves/restores that state in the handler's prologue/epilogue, so an interrupt firing during an in-progress calculation cannot corrupt it. See [Appendix C: Calling Convention](#appendix-c-calling-convention).

**Zero Page Limitations:**
- Only 256 bytes total; the frame region is a fixed 144-byte window (see [Appendix B](#appendix-b-memory-layout)) shared, via coloring, by every function in the program
- A single function's parameters + locals must fit in that window along with everything reachable from its callers - a function with very large local arrays/structs, or an unusually deep call chain of large frames, can exceed it
- If the coloring cannot fit the program, compilation fails with a "zero-page frame region overflow" error naming the offending call chain, rather than silently corrupting memory - reduce local buffer sizes or restructure the call graph to fix it
- There is no manual override to place a specific variable outside its frame; use an `addr` declaration (see [Memory-Mapped Addresses](#memory-mapped-addresses)) for state that must live at a fixed absolute address instead

### Completion Status

All items completed.

---

## Functions

### Function Declaration

```rust,compile
fn function_name(arg1: u8, arg2: u16) -> u8 {
    return arg1;
}

fn no_return(x: u8) {
    // No return statement needed
}
```

#### Returns are checked

A function that declares a return type must return a value of that type on
every path, and a function that declares none must not return a value at all.
Both are compile errors rather than warnings, because neither is detectable at
run time: the calling convention passes the result in the accumulator, so a
caller reads a value either way — a missing return just hands it whatever the
last statement happened to leave there.

```rust
fn incomplete(n: u8) -> u8 {
    if n == 0 { return 1; }
}                              // ERROR: missing return in function 'incomplete'

fn nothing_to_give() {
    return 5;                  // ERROR: return type mismatch, expected void
}
```

A path "returns" if it ends in a `return`, or if it cannot complete at all.
That makes each of these complete:

- an `if`/`else` where **both** arms return (one arm alone is not enough — the
  other falls through);
- a `loop` with no `break` out of it, which never completes, so a trailing
  `loop {}` is a valid way to end a value-returning function;
- a `match` that returns in every arm **and** covers every value, either through
  a wildcard arm or by naming every variant of an enum;
- a whole-function `asm` block, which is trusted to leave the result in the
  accumulator (this is how much of the standard library is written).

A `while` or `for` loop never counts, even if its body returns: it may run zero
times, so the path that skips it still falls through.

Ordinary conversion rules apply to the returned value — lossless widening is
implicit, narrowing needs an explicit cast:

```rust,compile,fragment
let widened: u16 = 3;      // a `-> u16` function may `return` a u8
```

### Function Attributes

Function attributes control code generation, placement, and calling conventions. They are specified using `#[attribute]` syntax before the function declaration.

Attributes are not only for functions: [`#[soa]`](#columns-instead-of-records-soa)
and [`#[align]`](#page-alignment-align) go on a `static` or `const` array and
choose how it is laid out. An attribute that does not apply to the declaration it
is written on is an error, rather than being ignored.

#### `#[inline]`

Inlines the function body at each call site, eliminating JSR/RTS overhead:

```rust,compile
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

```rust,compile
const TIMER_STATUS: addr = 0x6004;

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

```rust,compile
const NMI_FLAG: addr = 0x0300;
const STATUS_LED: addr = 0x6000;

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

```rust,compile
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

Every parameter, of every type and in every position (including the first), is passed via zero page - there is no register-based fast path for a lone leading argument. This is what makes the calling convention uniform and safe under [frame allocation](#zero-page-allocation): a call site evaluates each argument expression, stages the results in a temporary pool, then copies them into the callee's own frame immediately before `JSR`.

- `u8`, `i8`, `b8`, `bool`: 1 byte in the callee's frame
- `u16`, `i16`, `b16`: 2 bytes (low byte, then high byte) in the callee's frame
- Structs, arrays, and enums: passed by reference as a 2-byte pointer in the callee's frame, pointing at the caller's storage for that value

**Return Values:**
- `u8`, `i8`, `b8`, `bool`: Accumulator (A register)
- `u16`, `i16`, `b16`: A (low byte) and Y (high byte)
- Arrays, strings, and enum values: a 2-byte pointer in A (low byte) and X (high byte)

**Example:**
```rust,compile
fn add(a: u8, b: u8) -> u8 {
    return a + b;
}

fn add16(a: u16, b: u16) -> u16 {
    return a + b;
}

fn main() {
    let sum: u8 = add(5, 3);          // a and b both in add's frame, result in A
    let sum16: u16 = add16(100, 200); // a and b both in add16's frame, result in A/Y
}
```

Because parameters live in the callee's own frame rather than shared fixed registers or a fixed address range, a caller's own parameters and locals are never at risk of being overwritten by a call it makes - see [Zero Page Allocation](#zero-page-allocation) for how frames are placed to guarantee this, and [Appendix C](#appendix-c-calling-convention) for the full call sequence.

### Function Pointers

A function's bare name used as a value is its address. The type is written
`fn(params) -> ret` (the `-> ret` is omitted for a function returning nothing):

```rust,compile
fn double(x: u8) -> u8 { return x + x; }

fn main() {
    let f: fn(u8) -> u8 = double;
    let y: u8 = f(21);      // 42
}
```

A function pointer is a 2-byte code address. Calls through one go via an
indirect trampoline rather than a direct `JSR`, so they cost a few extra cycles.

**Indirect arguments.** A function whose address is taken receives its arguments
through a fixed staging block and copies them into its frame in a prologue, so
direct and indirect callers agree on where arguments live.

The staging block is a fixed size at a fixed address, so a parameter has to be
one or two bytes with a settled register convention. That admits the scalars,
a pointer, a `str`, an enum, and a struct — which goes by reference, indirectly
just as it does directly. An array or a slice may not be passed to an indirect
call; pass a `&T` instead.

### Vtables and Dynamic Dispatch

Function pointers can be stored in struct fields and called through them. This
is how a driver or device interface is expressed: the calling code names only the
struct, not the implementation.

```rust,compile
// Two devices at whatever addresses the machine puts them.
const PORT_A_IN:  read  addr = 0x6000;
const PORT_A_OUT: write addr = 0x6001;
const PORT_B_IN:  read  addr = 0x6010;

struct Device {
    read:  fn(u8) -> u8,
    write: fn(u8),
}

fn a_read(reg: u8) -> u8  { return PORT_A_IN; }
fn a_write(v: u8)         { PORT_A_OUT = v; }
fn b_read(reg: u8) -> u8  { return PORT_B_IN; }

static DEV: Device = Device { read: a_read, write: a_write };

fn main() {
    // Bind a different driver at runtime; callers are unaffected.
    DEV.read = b_read;

    let status: u8 = DEV.read(5);   // dispatched through the vtable
    DEV.write(0x41);
}
```

Any expression whose type is a function pointer may be called, so a field, a
variable, or a returned pointer all work. A bare `name(...)` still compiles to a
direct `JSR`; only computed callees pay the indirect cost.

`read` and `write` are *contextual* keywords: they act as access modifiers only
in `const NAME: read addr = ...`, and are ordinary identifiers everywhere else,
so `struct Device { read, write }` and `dev.write(v)` are legal.

#### Device tables

An array of those structs is a device list, indexed by device number. Both
halves of dispatch come out of one table: through a bound vtable for "how does
this device work", and by index for "which device".

```rust,compile
struct Driver {
    init:  fn(),
    write: fn(u8),
}

fn a_init() { }
fn a_write(v: u8) { }
fn b_init() { }
fn b_write(v: u8) { }

static DRIVERS: [Driver; 2] = [
    Driver { init: a_init, write: a_write },
    Driver { init: b_init, write: b_write },
];
static CONSOLE: Driver = Driver { init: a_init, write: a_write };

fn register(id: u8) {
    CONSOLE = DRIVERS[id];   // the whole vtable, copied
    CONSOLE.init();
}

fn main() {
    register(1);
    CONSOLE.write(0x41);     // through the bound vtable
    DRIVERS[0].write(0x42);  // by device number
}
```

Registration is a whole-struct assignment (see [Copying Structs](#copying-structs)),
so `CONSOLE` ends up with its own copy of the pointers rather than an alias into
the table.

#### Per-instance state

A vtable row may carry data as well as pointers, which is what lets one driver
serve several devices of the same kind — peripherals on a shared bus, say. A
`&T` is passed like any other pointer:

```rust,compile
struct State { count: u8 }
static S0: State = State { count: 0 };
static S1: State = State { count: 0 };

fn poll(s: &State) -> u8 {
    s.count = s.count + 1;
    return s.count;
}

struct Peripheral {
    state: &State,
    poll:  fn(&State) -> u8,
}

static PERIPHS: [Peripheral; 2] = [
    Peripheral { state: &S0, poll: poll },
    Peripheral { state: &S1, poll: poll },
];

fn main() {
    let i: u8 = 1;
    let n: u8 = PERIPHS[i].poll(PERIPHS[i].state);
}
```

A driver reached only through a table is not reported as dead code, so device
entry points do not need to be called directly to avoid a warning.

### Completion Status

All items completed.

---

## Structs

### Declaration

```rust,compile
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

```rust,compile
struct Point { x: u8, y: u8 }

#[reset]
fn main() {
    let p1: Point = { x: 10, y: 20 };
    let p2: Point = { x: 5, y: 5 };
    p2.x = 15;

    let x_coord: u8 = p1.x;
    loop {}
}
```

A field may be named `len`, `low` or `high`. On a struct-typed object the
field wins over the built-in accessor (which applies only to slices, arrays
and strings for `.len`, and to `u16`/`i16` for `.low`/`.high`):

```rust,compile
struct Entry { len: u8, flags: u8 }

#[reset]
fn main() {
    let e: Entry = { len: 4, flags: 1 };
    let n: u8 = e.len;   // the field: 4

    let a: [u8; 4] = [1, 2, 3, 4];
    let m: u16 = a.len;  // the built-in: 4
    loop {}
}
```

### Memory Layout

Structs are laid out sequentially in memory with no padding:

```rust,compile
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
- Array fields inlined directly too — `len * element size` bytes, in place
- Multi-byte fields stored little-endian
- Total struct size = sum of field sizes

An *array* of structs is interleaved by default — each record's bytes together,
one record after the next — but can be stored the other way up, one column per
field, with [`#[soa]`](#columns-instead-of-records-soa).

An array field is part of the struct's bytes, not a pointer to them, so
reaching an element is the struct's base plus the field's offset plus the
scaled index:

```rust,compile
struct Row { tag: u8, cells: [u16; 3], flags: u8 }   // 1 + 6 + 1 = 8 bytes

#[reset]
fn main() {
    let r: Row = Row { tag: 1, cells: [10, 20, 30], flags: 0 };
    let i: u8 = 2;
    r.cells[i] = 40;
    let c: u16 = r.cells[1];
    loop {}
}
```

### Nested Structs

Structs can contain other structs as fields:

```rust,compile
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

#[reset]
fn main() { loop {} }
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

### Columns Instead of Records (`#[soa]`)

An array of structs is stored interleaved by default: each record's fields sit
together, one record after the next. `#[soa]` on a `static` or `const` array
stores it the other way up — one column per field, each holding that field for
every element.

```rust,compile
struct Sprite { x: u8, y: u8, hp: u8 }

#[soa]
static SPRITES: [Sprite; 8] = [Sprite { x: 0, y: 0, hp: 0 }; 8];
const OUT: addr = 0x0900;

#[reset]
fn main() {
    let i: u8 = 3;
    SPRITES[i].y = 40;
    OUT = SPRITES[i].y;
    loop {}
}
```

Nothing about the *source* changes: it is still an array of records, indexed and
read the same way. What changes is the addressing mode.

**Interleaved**, `SPRITES[i].y` must multiply the index by the element size
before it can index at all — on a three-byte record that is seven instructions
and nineteen cycles for one byte, and the multiply is recomputed for every field
read:

```
STA $22 / CLC / ADC $22 / CLC / ADC $22 / TAY / LDA base,Y
```

**In columns**, the index scales by the *field's* own size, which for a byte
field is not at all:

```
TAY / LDA col,Y
```

A two-byte field costs one `ASL A` rather than a multiply, so columns still win
wherever a field is narrower than the record.

#### What it costs

An element is no longer contiguous, so **it has no address**. Every use that
would need one is a compile error:

```rust
let e: Sprite = SPRITES[1];    // error: no address of its own
let p: &Sprite = &SPRITES[1];  // error
draw(SPRITES[1]);              // error, if `draw` takes a Sprite
SPRITES[1] = other;            // error
let some: &[Sprite] = SPRITES[0..2];  // error: a slice needs contiguous elements
```

A single *field* still has an address — it is one entry in one column — so
`&SPRITES[1].hp` is fine.

This is why the layout is asked for by name rather than inferred. If the
compiler chose it, adding one `&SPRITES[i]` would silently flip the whole array
back and turn every access from an index into a multiply, with nothing in the
source to show for it. Named, that same line is an error, and the decision stays
where it was written.

#### Restrictions

- The attribute goes on a `static` or `const` array, not on the struct. Whether
  columns pay is a property of how a *collection* is traversed, not of the
  record type: the same `Sprite` may be a hardware register block in one place
  and a pool in another.
- Every field must be a scalar of one or two bytes — a primitive, a pointer, a
  function pointer or a fieldless enum. A field that is itself a struct or an
  array would need its own nested columns, which is a separate feature.

#### The compiler will suggest it

An array of structs that is only ever reached one field at a time, and whose
fields would all take columns, is pointed out:

```
warning: `A` is only ever read one field at a time, so every access multiplies
the index by 2; `#[soa]` would store it as one column per field and index
directly. The cost is that an element would no longer have an address
```

The *recommendation* is inferred; the layout is not. The suggestion is
deliberately quiet: a single mention that is not a field read — a `&`, a slice,
a whole-element binding — and it says nothing, because a suggestion the reader
has to dismiss is worse than one never made.

### Page Alignment (`#[align]`)

`#[align]` on a `const` array places the table on a 256-byte page boundary
(`$xx00`):

```rust,compile
#[align]
const SQUARES: [u8; 16] = [|i| => i * i];
const OUT: addr = 0x0900;

#[reset]
fn main() {
    let i: u8 = 5;
    OUT = SQUARES[i];
    loop {}
}
```

On the 6502 an indexed read (`LDA table,X`) costs an extra cycle whenever
`base + index` crosses a page boundary, and the crossing depends on the index —
so an unaligned table gives a hot loop data-dependent timing. A table that starts
on a page boundary and fits within a page never crosses: every access is the fast
path, the timing is fixed, and the element's offset is simply the low byte of its
address.

The attribute is **bare** — it takes no argument. The page is the only alignment
that changes anything on this machine (there is no cache, and sub-page boundaries
do not affect the indexed-read penalty), so there is nothing else to ask for;
`#[align(256)]` is rejected in favour of `#[align]`.

The cost is the padding between the previous item and the next page boundary, so
alignment is worth it for a table read in a loop, not for one touched once.

#### Restrictions

- `#[align]` goes on a **`const` array** — a read-only table in ROM. It is not
  yet supported on a mutable `static` (whose RAM is repacked to reclaim dropped
  globals) and is meaningless on a scalar, a string, or an `addr`; each is an
  error rather than a silent no-op.

### Passing Structs to Functions

All structs are passed **by reference**: the callee receives a 2-byte pointer
to the caller's storage (see [Calling Convention](#appendix-c-calling-convention)).
Field writes through a struct parameter modify the *caller's* struct:

```rust,compile
struct Entity { health: u8 }

fn update_entity(e: Entity) {
    e.health = e.health - 1;  // mutates the caller's Entity
}

#[reset]
fn main() { let e: Entity = { health: 3 }; update_entity(e); loop {} }
```

To work on a copy, bind the parameter to a local — binding copies (see
[Copying Structs](#copying-structs)) — or return a fresh struct:

```rust,compile
struct Point { x: u8, y: u8 }

fn move_point(p: Point, dx: u8, dy: u8) -> Point {
    return Point { x: p.x + dx, y: p.y + dy };
}
```

### Copying Structs

Binding and assignment **copy**; arguments are passed **by reference**. The two
conventions are deliberate: `let q: Point = p` gives `q` storage of its own, so
writing `q` leaves `p` alone, while `f(p)` hands over an address so the callee
can write through to the caller's struct.

Any struct-valued *place* can be copied — a local, a `static`, a nested field,
an array element (at a constant or a runtime index), or a by-reference
parameter:

```rust
struct Driver { init: fn(), read: fn() -> u8 }
static DRIVERS: [Driver; 2] = [ /* … */ ];
static CONSOLE: Driver = /* … */;

let d: Driver = DRIVERS[id];   // copies the whole struct
CONSOLE = DRIVERS[id];         // so does assignment
DRIVERS[i] = DRIVERS[j];       // including element to element
```

A struct value that is *not* a place — an expression with no address of its own
— has to be a struct literal or a call returning a struct, both of which are
copied the same way. Anything else is a compile error rather than a partial
copy.

### Returning Structs by Value

A function may return a struct by value. What travels back is the struct's
**address**, in `A:X`; the caller copies the bytes out of it immediately after
the call, so returning and binding a struct is a true copy:

```rust,compile
struct Point { x: u8, y: u8 }

fn make() -> Point {
    return Point { x: 7, y: 9 };
}

fn main() {
    let p: Point = make();   // full struct copied into p
    p = make();              // reassignment copies too
}
```

Any way of naming the struct may be returned, and each yields its address
differently — a local from its frame slot, a by-reference parameter from the
pointer in its slot, a `static` from its fixed address, and a literal from the
pointer it already produces:

```rust,compile
struct S { f: u8, a: [u8; 3] }
static G: S = S { f: 1, a: [2, 3, 4] };

fn from_local(x: u8) -> S { let s: S = S { f: x, a: [0; 3] }; return s; }
fn from_param(p: S) -> S { return p; }
fn from_static() -> S { return G; }
fn from_literal(x: u8) -> S { return S { f: x, a: [5, 6, 7] }; }

#[reset]
fn main() { let s: S = from_local(1); s = from_static(); loop {} }
```

Because the address names storage inside the callee's frame, it is valid only
until the caller has copied it — which is what the caller does with it, and the
only thing it may do with it.

#### Where a struct literal lives

A struct literal whose fields are all constants is emitted as bytes in the
`CODE` section, and the expression evaluates to a pointer at them. It costs no
RAM and no cycles to build. "Constant" means the field folds to a number, so
`(-1)` and `2 * 3` qualify; an array field is laid out inline there like
anywhere else, and a field the literal omits is zeroed for its whole width.

A literal with a *computed* field has no bytes until the program runs, so it is
assembled at run time into a block of RAM reserved for that literal. The block
is per literal site and is colored by the call graph exactly like local array
data, so two functions that can never be active at once share the space. This
is what makes `move_point` above work; the two forms are interchangeable in
source, and only the cost differs:

```rust,compile
struct Point { x: u8, y: u8 }

#[reset]
fn main() {
    let dx: u8 = 3;
    let fixed: Point = Point { x: 1, y: 2 };         // bytes in ROM
    let computed: Point = Point { x: dx + 1, y: 2 }; // assembled in RAM
    loop {}
}
```

Initializing a variable directly writes the fields straight into the variable's
own storage, with no intermediate block at all.

### Completion Status

All items completed.

---

## Enums

### Simple Enums

```rust,compile
enum Direction {
    North = 0,
    South = 1,
    East = 2,
    West = 3,
}

#[reset]
fn main() {
    let dir: Direction = Direction::North;
    loop {}
}
```

#### Discriminants

Each variant has a one-byte discriminant. Writing `= N` sets it; omitting it
continues from the previous variant, starting at 0:

```rust,compile
enum Code { A = 10, B, C = 20, D }   // 10, 11, 20, 21
```

Because a discriminant is one byte, three things are compile errors rather than
silent surprises: a value outside `0-255`, two variants sharing a value (the
second would be unreachable, since dispatch selects on the tag), and a variant
following `= 0xFF`, which has no number left to take.

#### Casting to an Integer

A unit variant's discriminant *is* its runtime value, so `as` yields exactly the
number written in the declaration. This is how an enum naming hardware states is
written to a register:

```rust,compile
const PORT_DIR: addr = 0x6003;   // a port's data-direction register

pub enum Direction {
    OUTPUT = 0xFF,
    INPUT  = 0x00,
}

pub fn set_port_direction(direction: Direction) {
    PORT_DIR = direction as u8;  // stores $FF or $00
}
```

The cast must be written out. There is no implicit conversion, in keeping with
the rest of the language, so assigning a `Direction` straight to a `u8` is an
error that names both types.

### Enums with Data (Tagged Unions)

Wraith supports enum variants that carry data, allowing you to create tagged unions (also known as sum types or discriminated unions). There are two forms: tuple variants and struct variants.

#### Tuple Variants

Tuple variants carry unnamed fields accessed by position:

```rust,compile
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

#[reset]
fn main() {
    // Creating tuple variant instances
    let value: Option = Option::Some(42);
    let red: Color = Color::RGB(255, 0, 0);
    let success: Result = Result::Ok(1000);
    loop {}
}
```

**Pattern Matching with Tuple Variants** (⚠️ EXPERIMENTAL - Limited Testing):

```rust,compile
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

✅ **Status**: Tuple variant pattern matching with data extraction works and is covered by execution tests.

#### Struct Variants

Struct variants carry named fields:

```rust,compile
enum Message {
    Quit,
    Move { x: u8, y: u8 },
    Write { text: str },
    ChangeColor { r: u8, g: u8, b: u8 },
}

#[reset]
fn main() {
    // Creating struct variant instances
    let msg: Message = Message::Move { x: 10, y: 20 };
    let color: Message = Message::ChangeColor { r: 255, g: 128, b: 0 };
    loop {}
}
```

**Pattern Matching with Struct Variants**:

```rust
match msg {
    Message::Move { x, y } => {
        // x and y are bound to the field values
    }
    _ => {}
}
```

✅ **Status**: Struct variant pattern matching with field extraction works and is covered by execution tests. Fields are checked against the variant definition: an unknown field name or a value of the wrong type is a compile error.

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
```rust,compile
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

```rust,compile
enum Input {
    None,                          // Unit variant (tag only)
    Key(u8),                       // Tuple variant (tag + 1 byte)
    MouseClick { x: u8, y: u8 },   // Struct variant (tag + 2 bytes)
}

#[reset]
fn main() {
    let input1: Input = Input::None;
    let input2: Input = Input::Key(65);  // 'A' key
    let input3: Input = Input::MouseClick { x: 100, y: 50 };
    loop {}
}
```

#### Current Limitations

1. **Struct variant pattern matching**: Cannot extract fields from struct variants in match arms (planned feature)
2. **Tuple variant testing**: Pattern matching with data extraction is minimally tested and may have bugs
3. **Complex nesting**: Deeply nested enums with data may have codegen issues
4. **Size calculations**: Each variant can have different sizes, making the enum size equal to the largest variant + 1 byte for the tag

### Default Discriminant Values

If not specified, discriminants start at 0 and increment:

```rust,compile
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

```rust,compile
enum State {
    Off = 0,
    On = 1,
}

#[reset]
fn main() {
    let s: State = State::On;  // Stored as u8 value 1
    let raw: u8 = s as u8;     // Cast to u8: 1
    loop {}
}
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

```rust,compile,fragment
let buffer: [u8; 10] = [0; 10];  // 10 bytes, all zeros
let data: [u16; 5] = [100, 200, 300, 400, 500];

buffer[5] = 42;
let value: u16 = data[2];
```

### Generated Tables

A table whose entries are a function of their index can be written as
`[|i| => <expression>]`. Every entry is computed at compile time, so the
program starts with the numbers already in place.

```rust,compile
const SQR: [u8; 16] = [|i| => i * i];
const ROW: [u16; 4] = [|i| => 0x0400 + (i as u16) * 40];
const OUT: addr = 0x0900;

#[reset]
fn main() {
    let k: u8 = 7;
    OUT = SQR[k];
    loop {}
}
```

`SQR` above becomes sixteen bytes of ROM: `$00 $01 $04 $09 $10 …`. No code
runs to build it.

**The length comes from the type.** It is not written in the expression, so
a table's count is stated once, in its declaration. A generated table
therefore needs a declared array type — there is nowhere else for the length
to come from.

**The index is a `u8`.** It is named by the closure's parameter (`i` above,
but any name will do) and takes the values `0` through `len - 1`. Because it
is a `u8`, a generated table holds at most 256 entries.

**The body is ordinary arithmetic at the element type.** It may name other
constants, and it wraps exactly the way the same expression would at run
time, so a table and a loop over the same expression cannot disagree:

```rust,compile
const NARROW: [u8; 4] = [|i| => (i * 100) / 2];
const WIDE: [u16; 4] = [|i| => ((i as u16) * 100) / 2];
const OUT: addr = 0x0900;

#[reset]
fn main() {
    OUT = NARROW[3] + WIDE[3].low;
    loop {}
}
```

`NARROW` is `0, 50, 100, 22` — at `i = 3` the product overflows a `u8` to 44
before the divide, which is what the equivalent `u8` loop computes. `WIDE`
is `0, 50, 100, 150`. A wider intermediate needs a written cast, and the
cast is the reader's decision rather than the compiler's.

**The body must be constant.** It becomes data before the program runs, so
it cannot read a `static`, call a function, or index another array.

A generated table may be declared as a `const` (ROM data), as a `static`
(written to RAM at startup) or as a local array (stored into its frame on
entry):

```rust,compile
const LOG2: [u8; 8] = [|i| => i / 2];      // ROM
static COUNTS: [u8; 4] = [|i| => i + 1];   // RAM, written at startup
const OUT: addr = 0x0900;

#[reset]
fn main() {
    let mask: [u8; 4] = [|i| => 1 << i];   // frame
    OUT = LOG2[7] + COUNTS[2] + mask[3];
    loop {}
}
```

### Slices

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

```rust,compile,fragment
let data: [u8; 5] = [1, 2, 3, 4, 5];
let i: u8 = 3;
if i < 5 {
    let x: u8 = data[i];  // Safe
}
```

### Slice Operations

Slices are references to a sub-range of an array, carrying a base pointer and a
runtime length. A slice value is produced by slicing an array with `arr[a..b]`
(or `arr[a..=b]`) and bound to a `&[T]` variable:

```rust,compile,fragment
let a: [u8; 6] = [1, 2, 3, 4, 5, 6];
let s: &[u8] = a[1..5];   // elements a[1]..a[4]

let n: u16 = s.len;       // runtime length (here 4)
let first: u8 = s[0];     // s[0] == a[1]
for i in 0..s.len {
    // iterate a slice by index
}
```

Bounds may be constants or computed at run time (`a[i..j]`), and slices of
`u8` and `u16` element arrays are supported (the index is scaled by the element
width). A slice can be passed to a function, which reads its length and
elements through the descriptor:

```rust
fn sum(s: &[u8]) -> u8 {
    let acc: u8 = 0;
    for i in 0..s.len {
        acc = acc + s[i as u8];
    }
    return acc;
}

let s: &[u8] = a[1..5];
let total: u8 = sum(s);
```

Slices support the full set of view operations:

```rust
let s: &[u8] = a[1..5];
let s2: &[u8] = s[1..3];       // re-slice a slice (offsets compose)
s = a[2..6];                   // reassign to a new view
for x in s { /* iterate elements */ }

total(a[1..5]);                // a slice expression may be an argument
total(s[1..3]);                // including a re-slice

fn middle(v: &[u8]) -> &[u8] { return v[1..4]; }  // and may be returned
```

**Slice Characteristics:**
- Size: 4 bytes (2-byte base address + 2-byte length)
- View into array data; length tracked at runtime
- Created with `arr[a..b]` (constant or runtime bounds)
- `.len`, indexing `s[i]` (read-only), and `for x in s` iteration
- Re-sliceable (`s[a..b]`), reassignable, and passed to / returned from
  functions by value

Bounds may be `u8`/`i8` or `u16`/`i16` (the latter lets constant-bounds slices
exceed 255 elements), inclusive ranges accept a runtime end, and `for x in s`
iterates the full length with a 16-bit counter.

**Slices are read-only.** `s[i]` reads, `for x in s` iterates, and a slice can
be re-sliced, reassigned, and passed to or returned from functions — but
`s[i] = v` is a compile error:

```rust
let a: [u8; 6] = [1, 2, 3, 4, 5, 6];
let s: &[u8] = a[1..4];
s[0] = 9;      // ERROR: cannot write through a slice: `&[T]` is a read-only view
a[1] = 9;      // OK - write to the array it borrows from
```

This is what lets a slice borrow from *any* storage. The source array may be a
local, a `static`, or a `const`:

```rust,compile
const TABLE: [u8; 4] = [10, 20, 30, 40];   // ROM
static BUFFER: [u8; 4] = [0; 4];           // RAM

fn total(v: &[u8]) -> u8 {
    let acc: u8 = 0;
    let n: u8 = v.len as u8;
    for i in 0..n { acc = acc + v[i]; }
    return acc;
}

#[reset]
fn main() {
    let rom: &[u8] = TABLE[0..4];
    let ram: &[u8] = BUFFER[0..4];
    let a: u8 = total(rom);
    let b: u8 = total(ram);
    loop {}
}
```

A `const` array lives in ROM, where a store is a silent no-op on real hardware.
Since a slice descriptor is just a base address and a length, it carries no
record of which storage it came from — so the rule cannot depend on the source
without making the same expression legal or not according to a declaration
elsewhere. Rejecting every write keeps `&[T]` one thing, and mirrors the split
between `str` (may be a ROM literal, read-only) and `str<N>` (owns RAM,
writable). A writable slice type would be the analogue of `str<N>`; it does not
exist yet.

Indexing takes a `u8`/`i8`, because indexed addressing goes through an 8-bit
register. `.len` is a `u16`, so `for i in 0..s.len` types `i` as one and needs
`s[i as u8]` — or bind the bound first (`let n: u8 = s.len as u8;`) and the loop
variable is a `u8` throughout. `for x in s` sidesteps the question entirely.

Current limits: element widths above 2 bytes are not yet supported, runtime
(non-constant) slice bounds must be `u8`, and there is no runtime bounds
checking.

### Slice Memory Representation

```rust,compile
const DATA: [u8; 6] = [0, 1, 2, 3, 4, 5];

fn process(slice: &[u8]) {
    // slice references DATA
    // Length is known from array type
}
```

**Memory Layout:**
- Slice parameter: 4 bytes total (base address + length)
- Base address: 2 bytes pointing to first element
- Length: 2 bytes for element count
- Data: stays wherever the borrowed array lives — a `const` array's ROM, a
  `static`'s RAM, or a local's frame block. The descriptor holds an address, so
  a slice never copies the data it views.

### Multidimensional Arrays

An array element may itself be an array. The nesting is laid out row-major and
inline — a `[[u8; 8]; 4]` is 32 contiguous bytes — and `m[i][j]` indexes it with
no pointer chase. A `const`, a `static` and a `let` local all initialise from a
nested literal, and a multidimensional array may be passed to a function.

```rust,compile,fragment
let screen: [[u8; 8]; 4] = [
    [0, 0, 0, 0, 0, 0, 0, 0],
    [0, 1, 1, 1, 1, 1, 1, 0],
    [0, 1, 0, 0, 0, 0, 1, 0],
    [0, 0, 0, 0, 0, 0, 0, 0],
];
screen[1][3] = 2;                  // row 1, column 3
let pixel: u8 = screen[2][5];
```

A fill applies at each level, so `[[0; 8]; 4]` is thirty-two zeroes:

```rust,compile,fragment
let grid: [[u16; 3]; 2] = [[0; 3]; 2];
```

Flattening to one dimension — `screen[r * 8 + c]` over a `[u8; 32]` — is still a
fine choice where the index arithmetic is cheaper written out; it is no longer
the only one.

### Array Assignment and Copying

An array is initialised once, from a literal, and is never assigned as a whole
afterwards. Both of these are refused:

```rust
let src: [u8; 3] = [1, 2, 3];
let dst: [u8; 3] = [0; 3];

dst = src;         // ERROR: cannot assign a whole array to `dst`
dst = [4, 5, 6];   // ERROR: same
```

The binding form is refused for the same reason, and says so at the
declaration: `let dst: [u8; 3] = src;` is *a local array must be initialized
with an array literal*.

The reason is cost. `dst = src` on a 6502 is a loop over the elements — no
instruction moves more than one byte — so an assignment that looks like a
register move would emit an unbounded copy whose length is the array's, and a
`[u8; 256]` would silently cost 256 stores and a loop in a language whose whole
point is that you can see what the code will do. Making the copy explicit puts
that cost where the reader can count it. (Nothing about an array's
*representation* changes here: it is still a block of elements, not a
reference. What is refused is one *statement*, not one semantics.)

There are two ways to copy. At these lengths the loop is the smaller of the
two — `memcpy` is a call, and its body has to be linked in — so the choice is
about what you have rather than about size:

```rust,compile
import { memcpy } from "std/mem.wr";

#[reset]
fn main() {
    let src: [u8; 3] = [1, 2, 3];
    let dst: [u8; 3] = [0; 3];

    // Element-wise: no import, no call, and the compiler sees every store.
    for i in 0..3 {
        dst[i] = src[i];
    }

    // Or with an explicit copy: one call whatever the length, and the only
    // form that takes a length decided at run time. `memcpy` counts *bytes*,
    // so a `[u16; N]` passes `N * 2`.
    memcpy(&dst[0], &src[0], 3);

    loop {}
}
```

`memcpy` copies bytes and does not overlap-check, so it is `memcpy` and not
`memmove`: a destination inside the source range is the caller's problem. It
lives in `std/mem.wr` alongside `memcpy16` for lengths past 255, `memset`, and
`memcmp`.

`&array[0]` is the address of the first element; `&array` is the same address —
see [Pointers](#pointers). An array *field* works the same way, so `memcpy` can
name one as its destination:

```rust,compile
import { memcpy } from "std/mem.wr";
struct D { f: [u8; 3], t: u8 }

#[reset]
fn main() {
    let d: D = D { f: [0; 3], t: 9 };
    let src: [u8; 3] = [1, 2, 3];
    memcpy(&d.f[0], &src[0], 3);
    loop {}
}
```

A **slice** is the one aggregate that *is* assigned as a whole, because a slice
is two numbers rather than a block of elements: `sl = TBL[1..4]` rebinds the
descriptor and copies nothing. That is the distinction the refusal draws — an
assignment that costs two bytes is allowed; one that costs the array's length
is spelled out.

### Completion Status

All items completed.

---

## Pointers

A pointer is the address of a value. It is written `&T`, taken with `&x`, and
read through with `*p`.

```rust,compile,fragment
let x: u8 = 41;
let p: &u8 = &x;
*p = *p + 1;        // x is now 42
```

Pointers exist so that a function can be handed a *caller's* buffer or device
struct. Without them a driver has to own its buffer as a `static`, or take a
bare `u16` the compiler cannot check.

```rust
fn read_line(buf: &u8, max: u8) -> u8 { ... }

let line: [u8; 64] = [0; 64];
let n: u8 = read_line(&line, 64);
```

### Representation

Two bytes, little-endian — the same 16-bit address the hardware uses. A pointer
occupies 2 bytes wherever it is stored, and `&Node` inside `struct Node` is
therefore fine: the size of a pointer never depends on the size of what it
points at.

```rust,compile
struct Node { value: u8, next: &Node }   // 3 bytes
```

### What you can do with one

| Form | Meaning |
| --- | --- |
| `&x` | the address of a variable, element, or field |
| `*p` | read or write the value it points at |
| `p[i]` | the *i*-th element from `p`, scaled by the element width |
| `p.field` | a field of the struct it points at — no `(*p).field` needed |
| `p as u16` | the address as a number |
| `n as &T` | a number as an address |

`&arr` on an array gives `&T`, a pointer to the first element — not
`&[T; N]`. That is what `memcpy(&dst, &src, n)` relies on.

`p[i]` has no bounds check: a pointer carries no length. When the length
matters, use a slice (`&[T]`), which carries one.

`p[i]` and `p.field` compose freely, and to any depth: `p.cells[i]` reaches an
array field through a pointer, `p.inner.v` follows a chain two levels down, and
`&x.cells[0]` takes the address of an element of a field. Where the whole chain
is constant it folds to the address at compile time; where it is not — anything
through a pointer, or an element at a run-time index — the base is computed and
the offsets added to it.

Two pointers of the same type compare for equality with `==` / `!=` — the null
check a linked list needs. Ordering (`<`, `>`, …) and arithmetic do not apply:
arithmetic is indexing (`p[i]`, scaled by the element width, rather than `p + n`
on a raw byte offset), and a relative order between two addresses is rarely
meaningful.

```rust
if p == 0 as &Node { ... }   // null check
```

Nor is there `&mut`: the language has one pointer kind, and no `mut` keyword to
distinguish a second.

### What is rejected

`&` needs something with an address that outlives the expression, so these do
not compile:

```rust
&5              // a literal has no address
&f()            // nor does a call result
&(a + b)        // nor an arithmetic result
&SOME_CONST     // a const lives in ROM, referenced by label, not by address
&SOME_ADDR      // an `addr` declaration; its read/write mode is checked at
                // the name, and a pointer would launder that check away
&some_function  // a function name is already its address
```

A `str`, a slice and an enum value are already references, so `&` on them is
rejected too — pass them directly.

Casts are checked in both directions. An address is 16 bits, so only `as u16`,
`as i16` or another pointer type keeps it whole; and only an integer can become
a pointer.

```rust
let n: u8 = p as u8;      // rejected: discards the page
let q: &u8 = true as &u8; // rejected: a bool is not an address
```

### Lifetimes

There is no borrow checker. There is a narrower check that rejects the shapes
that actually dangle.

Locals live in zero-page frames allocated by colouring the call graph: a
callee's frame never overlaps a live caller's. That is why passing `&local`
*down* is always safe — and it is not a new guarantee, since struct arguments
have been passed by reference all along. What is not safe is a pointer going
the other way. Once a function returns, an unrelated function may occupy those
bytes.

So these are errors:

```rust
fn leak() -> &u8 { let x: u8 = 1; return &x; }   // returning a local's address
static SAVED: &u8 = 0x0400 as &u8;
fn stash() { let x: u8 = 1; SAVED = &x; }         // storing it past the frame
```

Taking `&local` inside a recursive function is rejected as well. A recursive
call copies the frame to the software stack and copies it back, so the same
bytes serve every depth and a pointer taken at one depth names a different
invocation's variable at the next. Hoist the value to a `static`.

The subtle case is laundering through a parameter. A pointer *parameter* has
unknown provenance — the callee cannot see where it came from — so storing one
in a global has to be allowed, or a registration function could not be written
at all. The caller is checked instead:

```rust
fn keep(d: &u8) { SAVED = d; }    // fine on its own

keep(&COUNT);                      // fine: a static outlives everything
let x: u8 = 1; keep(&x);           // rejected here, at the call site
```

The analysis is flow-insensitive: a variable's provenance is the meet of every
value assigned to it anywhere in the function. `let p = &GLOBAL; p = &x;`
makes `p` local throughout.

An **indirect** call gets the same rule, asked of every function it could
reach. Which one runs is unknown, but the candidates are not — only a function
whose address was taken is reachable through a pointer — so a pointer to a
local is rejected if *any* of them stores that parameter beyond the call.

Frame colouring holds across an indirect call as it does across a direct one:
a function that dispatches through a pointer is given a colouring edge to every
address-taken function, so the callee's frame cannot overlap the caller's and a
pointer into that frame stays valid.

### Pointers in statics

A `static` can hold a pointer, initialised either from a literal address or
from another static's address:

```rust,compile
static COUNT: u8 = 0;
static P: &u8 = &COUNT;
static DEVICE: &u8 = 0x6000 as &u8;
```

Statics are laid out in declaration order, so a static's initializer can only
name one declared above it; a forward reference is an error rather than a
silent zero.

---

## Strings

### String Type

Strings in Wraith are length-prefixed byte sequences optimized for the 6502. The string type is declared as `str`.

```rust,compile,fragment
let message: str = "Hello, World!";
let empty: str = "";
```

**Storage Format:**
- `[u8 length][byte data...]` - single byte length prefix followed by character data
- Maximum length: 255 bytes (enforced at compile time)
- A `str` value is a 2-byte pointer to this length-prefixed data

A bare `str` is **immutable**: it may point at a string literal in ROM, so its
bytes cannot be written. For runtime editing, use a string buffer (below).

### Mutable String Buffers (`str<N>`)

A `str<N>` is a fixed-capacity, mutable string that owns `N` bytes of RAM (plus
the one-byte length prefix, so the backing block is `1 + N` bytes). `N` is a
compile-time constant, `0`–`255`. At runtime a `str<N>` **is** a `str` — a
2-byte pointer to `[len][bytes]` — so every `str` operation (`.len`, indexing,
`==`, iteration, passing to a `fn(s: str)`) works on it unchanged.

```rust,compile
fn main() {
    let s: str<16> = "cat";   // capacity 16, current length 3, in RAM
    s[0] = 'b';               // "bat"  — edit a character in place
    s[2] = 'd';               // "bad"
    let n: u16 = s.len;       // 3 (editing characters does not change the length)
}
```

**Rules:**
- Must be initialized with a string literal that fits within `N`
  (a longer literal is a compile-time error).
- `s[i] = value` writes one character; `value` is a `char` (a string is an array
  of `char`). A numeric byte needs `as char`, e.g. `s[i] = 0x2E as char`.
  A constant index past capacity is a compile error; a variable index is **not**
  bounds-checked at runtime (same rule as [array indexing](#array-bounds-checking)).
- Writing through a bare `str` (`s[i] = …` where `s: str`) is a compile error —
  declare it as a `str<N>` buffer instead.
- The backing RAM is colored by the call graph like local arrays, so a buffer
  costs `1 + N` bytes in the BSS section for the duration of its scope.

Editing the length (append/truncate) and higher-level helpers (`push`, `append`,
`copy_from`) are intended to live in the standard library, built on top of
`s[i] = …`; the byte-level write is the compiler primitive.

### String Literals

String literals support escape sequences:

```rust,compile,fragment
let msg1: str = "Hello\n";          // Newline
let msg2: str = "Tab\there";        // Tab
let msg3: str = "Quote: \"Hi\"";    // Escaped quotes
let msg4: str = "Backslash: \\";    // Backslash
```

### String Properties

Access string metadata:

```rust,compile,fragment
let msg: str = "Hello";
let len: u16 = msg.len;      // Get length (5)
```

### String Indexing

A string is semantically an array of `char`, so indexing yields a `char`. Use
`as u8` when you want the raw byte value (for arithmetic or a hardware register):

```rust,compile,fragment
let msg: str = "ABC";
let first: char = msg[0];       // 'A'
let second: char = msg[1];      // 'B'
if first == 'A' { /* ... */ }   // compare chars directly
let byte: u8 = msg[0] as u8;    // 0x41, for byte-level work
```

**Bounds checking** follows the same rules as [array indexing](#array-bounds-checking): a constant index out of range is a compile-time error, while a variable index is **not** bounds-checked at runtime (reading past the end yields undefined data). It is the programmer's responsibility to keep variable indices within `msg.len`.

### String Concatenation

Concatenate strings at compile time using the `+` operator:

```rust,compile
const GREETING: str = "Hello, " + "World!";
const PATH: str = "data/" + "level" + ".txt";
```

**Requirements:**
- Both operands must be compile-time constant strings
- Result must not exceed 255 bytes
- Evaluated entirely at compile time (zero runtime cost)

### String Comparison

Compare two strings for equality with `==` / `!=` (result is `bool`). The
comparison runs at runtime: the length bytes are compared first, then each
character.

```rust,compile,fragment
let a: str = "hello";
let b: str = "hello";
if a == b { /* equal */ }
if a != "world" { /* differs */ }
```

### String Slicing

Extract substrings at compile time:

```rust,compile
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
- Result is validated to fit within 255 bytes

### String Iteration

Iterate over characters in a string:

```rust,compile
fn process_char(c: char) { }

#[reset]
fn main() {
    let message: str = "hi";
    let buffer: [u8; 8] = [0; 8];

    // Simple iteration
    for c in message {
        // c is char (each character)
        process_char(c);
    }

    // With index
    for (i, c) in message {
        // i is u8 (index), c is char (character)
        buffer[i] = c as u8;   // buffer is [u8; N], so cast the char to its byte
    }
    loop {}
}
```

**Performance Note:** String iteration is optimized to use the X register as a counter, providing efficient 8-bit indexing on the 6502.

### String Parameter Access

A `str` parameter is a 2-byte pointer that, like every other parameter, is passed directly in its own zero-page frame slot (see [Zero Page Allocation](#zero-page-allocation)). Because it's already in zero page, every access reads it in place - there is no separate pointer-caching layer:

```rust,compile
fn process_string(s: str) {
    let len1: u16 = s.len;
    let len2: u16 = s.len;  // Reads the same zero-page slot again
    let len3: u16 = s.len;
}
```

### Cross-Module String Pooling

Identical strings across different modules are automatically deduplicated using content-based hashing:

```rust
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
- No runtime string concatenation (concatenation is compile-time only)
- No built-in string search/replace operations

These limitations are intentional for the 6502 platform - strings are designed for static data like messages, labels, and constants rather than dynamic text processing.

### Completion Status

All items completed.

---

## Control Flow

### If/Else

```rust,compile,fragment
let x: u8 = 12;
if x > 10 {
    // ...
} else if x > 5 {
    // ...
} else {
    // ...
}
```

### While Loop

```rust,compile,fragment
let condition: bool = false;
while condition {
    // ...
}

let x: u8 = 0;
while x < 100 {
    x = x + 1;
}
```

### Loop (Infinite)

```rust,compile,fragment
let done: bool = true;
loop {
    if done {
        break;
    }
}
```

### For Loop

```rust,compile
// Over slices (type inferred from slice element type)
fn process(data: &[u8]) {
    for item in data {  // item is inferred as u8
        // process item
    }
}

#[reset]
fn main() {
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
    loop {}
}
```

### Match Statement

```rust
// Match on values
match value {
    0 => { },
    1..=10 => { },    // Range, inclusive of 10
    11..20 => { },    // Range, exclusive of 20
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

Both range spellings mean here what they mean in a `for` range: `a..=b` covers
`b`, `a..b` stops one short of it. An exclusive end may therefore name the value
just past the type — `0..256` covers a whole `u8` — but a range that covers
nothing (`5..5`, or a reversed `9..=3`) is rejected, since the arm could never
run.

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
if i < array.len && array[i] == target {
    // array[i] only evaluated if i < array.len
}

// Check address validity before reading
const STATUS: addr = 0x6000;
if STATUS != 0 && STATUS == 0xFF {
    // Second check only evaluated if STATUS != 0
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

**Only lossless widening is implicit** (`u8` → `u16`, `i8` → `i16`, `bool` → `u8`); everything else needs an explicit `as` cast.
**No error checking** - casts that are invalid will overflow/underflow.

### Integer Literals in Binary Operations

A binary operation requires both operands to have the same type; two **variables**
of different widths (e.g. `u16 + u8`) are a type error and must be reconciled with
an explicit cast:

```rust,compile,fragment
let a: u16 = 300;
let b: u8 = 5;
let c: u16 = a + (b as u16);   // explicit widening required
```

The single exception is an operand built only from **integer literals**: it
adopts the other operand's integer type when its values fit, in any operand
position and for any operator (arithmetic *and* comparison). This is a
compile-time typing of the literals, not a runtime conversion, so the
no-implicit-conversion rule is preserved. A negated literal (`-5`) counts, as
does a whole subexpression of literals — `(37 >> 1)` is as free to be `i8` as
`18` is. A cast does not: it names the type it produces.

When **both** operands are literals there is nothing to adopt from, so the pair
takes the narrowest type that holds every literal written in it — signed if any
of them is negative — unless a declared type is in scope and holds them all, in
which case that wins:

```rust,compile,fragment
let n: i8 = 1;
if (-5 - 3) < n { }            // ok: -5 and 3 are both i8; -8 < 1

let big: i16 = 3 - 5;          // i16, from the declaration: -2, not 254
```

Note what this means for a constant expression standing on its own: its type
comes from the literals in it, not from the code around it. In a program full of
`i8` values, `0 >= (3 << 7)` is still a `u8` comparison, and the shift wraps at
eight bits accordingly. Anchor it with a variable, or annotate it, if the
program's width is what you meant.

The same rule applies wherever a target type is known, notably array elements:

```rust
let a: [u16; 3] = [1, 2, 300];   // elements are u16, from the declaration
let b: [u16; 4] = [0; 4];        // fill value is u16
let c: [i16; 2] = [-100, -200];  // signed elements
let d: [u8; 2]  = [1, 300];      // error: 300 does not fit a u8 element
```

```rust
let a: u16 = 300;
if a < 5 { ... }               // ok: `5` adopts u16
let d: u16 = 1 + a;            // ok: `1` adopts u16

let s: i16 = -300;
if s < -5 { ... }              // ok: `-5` adopts i16 (signed compare)

let e: u8 = 5;
let f: u16 = e + 300;          // error: `e` is u8 and 300 does not fit u8
```

### Valid Cast Combinations

**Integer Widening (Safe):**
```rust,compile,fragment
let small: u8 = 100;
let large: u16 = small as u16;  // 100 -> 100 (zero-extended)

let signed: i8 = -10;
let wide: i16 = signed as i16;  // -10 -> -10 (sign-extended)
```

**Integer Narrowing (Truncation):**
```rust,compile,fragment
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
```rust,compile,fragment
let bcd: b8 = 0x42 as b8;    // Binary 42 -> BCD 42
let bin: u8 = bcd as u8;     // BCD 42 -> Binary 0x42

let score: b16 = 1234 as b16; // Binary -> BCD 1234
let raw: u16 = score as u16;  // BCD 1234 -> 0x1234
```

**Boolean Conversions:**
```rust,compile,fragment
let flag: bool = true;
let num: u8 = flag as u8;    // true -> 1, false -> 0

let value: u8 = 42;
let is_set: bool = value as bool;  // 0 -> false, nonzero -> true
```

### Truncation Behavior

When casting to a smaller type, high bytes are discarded:

```rust,compile,fragment
let value: u16 = 0xABCD;
let low: u8 = value as u8;    // 0xCD (low byte)
let high: u8 = (value >> 8) as u8;  // 0xAB (high byte, shifted first)

// Multi-step truncation
let big: u16 = 0x1234;
let small: u8 = big as u8;    // 0x34
```

### Sign Extension

Signed casts preserve the sign by extending the sign bit:

```rust,compile,fragment
let small: i8 = -1;          // 0xFF in binary
let large: i16 = small as i16;  // 0xFFFF (sign extended)

let positive: i8 = 127;      // 0x7F
let wide: i16 = positive as i16; // 0x007F (zero extended for positive)
```

Which extension a widening cast performs is decided by the **source** type, not
the destination. A signed source carries its sign into the new high byte; an
unsigned one carries zero. The two mixed cases follow from that and are worth
stating outright:

```rust,compile,fragment
let big: u8 = 200;
let as_signed: i16 = big as i16;   // 200 — a u8 has no sign bit to extend

let neg: i8 = -1;
let as_unsigned: u16 = neg as u16; // 0xFFFF — the value, reinterpreted
```

Narrowing is unaffected: it keeps the low byte whichever way the signs go.

**Manual Sign Extension (if needed):**
```rust,compile
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

```rust,compile
fn increment() {
    asm {
        "clc",
        "adc #1"
    }
}
```

### Assembly with Variable Substitution

```rust,compile
fn add_with_carry(a: u8, b: u8) -> u8 {
    let result: u8 = 0;
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

```rust,compile
fn custom_operation() -> u8 {
    let result: u8 = 0;
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
```rust,compile
fn read_timer() -> u8 {
    let value: u8 = 0;
    asm {
        "LDA $6004",  // read a device status register
        "STA {value}",
    }
    return value;
}
```

**Bit Manipulation:**
```rust,compile
fn set_interrupt_mask(mask: u8) {
    asm {
        "LDA {mask}",
        "STA $6005",   // a device's interrupt-enable register
    }
}
```

**Timing-Critical Code:**
```rust,compile
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

```rust,compile
fn find_byte(haystack: u16, needle: u8, len: u8) -> u8 {
    let result: u8 = 0;
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

```rust,compile
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

#### Glob Imports

A `*` imports every `pub` item of a module, so a library can be pulled in
without listing its API:

```rust,compile
import { * } from "math.wr";     // every pub item
import * from "math.wr";         // braces optional around a bare *
import { min, * } from "math.wr"; // legal; naming min is redundant
```

A glob respects visibility exactly as a named import does: private items stay
private, and referring to one is the same error as importing it by name. Unused
glob imports are not warned about — bringing in names you may not use is what
the wildcard asked for — and, per the next section, the ones you don't use cost
nothing in the output.

### Unreachable Code Is Not Emitted

The compiler emits only the functions and data the program can actually reach,
computed as a transitive closure over calls and references from the program's
entry points. This applies to imported modules and to the file being compiled
alike.

Importing a module makes its **whole file** available, not just the symbols
named — an imported function may call private siblings the importing program can
never refer to — so without this, using one helper from a library dragged in all
of it:

```rust,compile
import { * } from "math.wr";   // ~18 functions

#[reset]
fn main() {
    let q: u16 = div16(1000, 7);   // only div16 and its callees are emitted
    loop {}
}
```

A glob import and an explicit list of the symbols actually used compile to
identical assembly, so `*` costs nothing over naming each one.

#### What Counts as Reachable

Execution starts, or arrives from outside the call graph, at these **roots**:

- the `#[reset]`, `#[irq]` and `#[nmi]` handlers, which the hardware enters
  through the vector table, and `main`, the conventional entry point when no
  `#[reset]` is given;
- functions pinned by `#[org]` or `#[section]`, since fixing an address is
  usually how something outside the program is given a way in;
- functions whose address is taken, reachable through a pointer that no call
  edge records;
- names referenced outside any function body, such as in a `static`'s
  initializer.

From there, an item is kept when it is called or referenced — directly or
transitively — from something already kept, including by a `JSR`/`JMP` naming it
inside an inline `asm` block. The walk is deliberately conservative: keeping too
much only wastes ROM, while dropping something live is a jump into whatever
follows it.

Unreachable items **in the file being compiled** are reported before being
dropped, so the warnings name exactly what was removed:

```
warning: unused function: `never_called`
warning: unused constant or static: `LOOKUP_TABLE`
```

Items from imported modules are dropped silently — you did not write them, and a
library you use part of is not a mistake.

#### Files With No Entry Point

A file with no `#[reset]`, no interrupt handler, no `main` and no placed
function has no reachable code at all. That means the compiler cannot tell what
runs, not that everything is dead, so the whole module is emitted and nothing is
reported as unused. A half-written file still compiles to something you can
inspect.

### Module Visibility

**All items are private by default.** Only items marked with `pub` can be imported from other modules.

#### Visibility Rules

- Functions, constants, structs, enums, and address declarations are private unless marked `pub`
- Private items cannot be imported by other modules
- Public items marked with `pub` can be imported
- Local variables, function parameters, and pattern bindings are always private

#### Example: Public Items

```rust,compile
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

```rust,compile
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

```rust,compile
import {memcpy} from "mem.wr";  // Searches stdlib first
```

The standard library directory defaults to `std/` relative to the working
directory and can be overridden with the `WRAITH_STD_PATH` environment
variable — useful when building from another directory or with a vendored
copy of the library:

```bash
WRAITH_STD_PATH=/opt/wraith/std wraith main.wr
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
```rust,compile
import {memset} from "mem.wr";  // stdlib import

const SCREEN: addr = 0x0400;

fn init_graphics() {
    memset(0x0400 as &u8, 0x20, 255);
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

```rust,compile
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
```rust,compile
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
```rust,compile
import { enable_interrupts } from "std/intrinsics.wr";

fn setup_hardware() { }

#[reset]
fn reset_handler() {
    // Initialize hardware first
    setup_hardware();

    // Enable interrupts before the main loop
    enable_interrupts();

    loop {}
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
```rust,compile
import { disable_interrupts, enable_interrupts } from "std/intrinsics.wr";

static SHARED: u16 = 0;

fn update_shared_data() { SHARED = SHARED + 1; }

fn critical_update() {
    disable_interrupts();

    // Critical section - no IRQ interrupts
    update_shared_data();

    enable_interrupts();
}

#[reset]
fn main() { critical_update(); loop {} }
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
```rust,compile
import { clear_decimal, set_decimal } from "std/intrinsics.wr";

fn bcd_add(a: u8, b: u8) -> u8 {
    set_decimal();
    let result: u8 = a + b;  // BCD addition
    clear_decimal();
    return result;
}

#[reset]
fn main() { let s: u8 = bcd_add(0x19, 0x01); loop {} }
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
```rust,compile
import { set_stack_pointer } from "std/intrinsics.wr";

#[reset]
fn reset_handler() {
    set_stack_pointer(0xFF);  // Initialize stack to top
    loop {}
}
```

### Module: mem.wr

Memory manipulation functions optimized for 6502.

**Import:**
```rust,compile
import { memcpy, memset, memcmp, mem_read, mem_write } from "mem.wr";
```

#### Memory Block Operations

##### `memcpy(dest: &u8, src: &u8, len: u8)`

Copy `len` bytes from source address to destination address.

```rust
fn memcpy(dest: &u8, src: &u8, len: u8)
```

**Parameters:**
- `dest`: Destination address
- `src`: Source address
- `len`: Number of bytes to copy (max 255)

**Note:** Uses indexed addressing with Y register. Memory regions can overlap.

**Example:**
```rust
static SOURCE_DATA: [u8; 5] = [1, 2, 3, 4, 5];

memcpy(0x0400 as &u8, &SOURCE_DATA, 5);
```

`SOURCE_DATA` is a `static` rather than a `const` because a `const` lives in
ROM and is referenced by label, not by address — `&CONST` is rejected.

##### `memset(dest: &u8, value: u8, len: u8)`

Fill `len` bytes at destination with a constant value.

```rust
fn memset(dest: &u8, value: u8, len: u8)
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
```rust,compile
import { memset } from "std/mem.wr";

#[reset]
fn main() {
    // Clear screen with spaces (0x20)
    memset(0x0400 as &u8, 0x20, 255);
    loop {}
}
```

Note that an `addr` declaration in rvalue position is a *load* from that
address, not the address itself, so `SCREEN as u16` would pass the byte
currently stored there. Write the address, or take `&` of a static.

##### `memcmp(a: &u8, b: &u8, len: u8) -> u8`

Compare two memory regions for equality.

```rust
fn memcmp(a: &u8, b: &u8, len: u8) -> u8
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
static EXPECTED: [u8; 4] = [0x12, 0x34, 0x56, 0x78];

if memcmp(&EXPECTED, 0x6000 as &u8, 4) == 1 {
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
```rust,compile
import { mem_read } from "std/mem.wr";

#[reset]
fn main() {
    let value: u8 = mem_read(0x0400);  // Read from $0400
    loop {}
}
```

##### `mem_write(address: u16, value: u8)`

Write a byte to an address using indirect addressing.

```rust
fn mem_write(address: u16, value: u8)
```

**Equivalent to:** `*(address) = value` in C

**Uses:** 6502 indirect indexed addressing mode `STA (addr),Y`

**Example:**
```rust,compile
import { mem_write } from "std/mem.wr";

#[reset]
fn main() {
    mem_write(0x0400, 42);  // Write 42 to $0400
    loop {}
}
```

##### `mem_jump(address: u16)`

Transfer execution to the specified address.

```rust
fn mem_jump(address: u16)
```

**Maps to:** `JMP (address)` - indirect jump

**Warning:** Execution may not return unless the target code explicitly returns. Typically used for monitor/debugger "Go" commands.

**Example:**
```rust,compile
import { mem_jump } from "std/mem.wr";

#[reset]
fn main() {
    // Jump to code at $8000
    mem_jump(0x8000);
    // Execution continues at $8000
    loop {}
}
```

### Module: math.wr

Mathematical operations optimized for 6502/65C02. Focus on unsigned 8-bit values with efficient assembly implementations.

**Import:**
```rust,compile
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
```rust,compile
import { min } from "std/math.wr";

#[reset]
fn main() {
    let current_health: u8 = 120;
    let health: u8 = min(current_health, 100);  // Cap at 100
    loop {}
}
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
```rust,compile
import { max } from "std/math.wr";

#[reset]
fn main() {
    let base_damage: u8 = 0;
    let damage: u8 = max(base_damage, 1);  // Minimum 1 damage
    loop {}
}
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
```rust,compile
import { clamp } from "std/math.wr";

#[reset]
fn main() {
    let user_input: u8 = 20;
    let volume: u8 = clamp(user_input, 0, 15);  // Clamp to 0-15 range
    loop {}
}
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
```rust,compile
import { set_bit } from "std/math.wr";

#[reset]
fn main() {
    let flags: u8 = 0b00000000;
    flags = set_bit(flags, 3);  // Set bit 3 -> 0b00001000
    loop {}
}
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
```rust,compile
import { clear_bit } from "std/math.wr";

#[reset]
fn main() {
    let flags: u8 = 0b11111111;
    flags = clear_bit(flags, 5);  // Clear bit 5 -> 0b11011111
    loop {}
}
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
```rust,compile
import { test_bit } from "std/math.wr";

#[reset]
fn main() {
    let status: u8 = 0b00010000;
    if test_bit(status, 4) == 1 {
        // Bit 4 is set
    }
    loop {}
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
```rust,compile
import { saturating_add } from "std/math.wr";

#[reset]
fn main() {
    let health: u8 = 250;
    health = saturating_add(health, 10);  // Result: 255, not wrap to 4
    loop {}
}
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
```rust,compile
import { saturating_sub } from "std/math.wr";

#[reset]
fn main() {
    let ammo: u8 = 3;
    ammo = saturating_sub(ammo, 5);  // Result: 0, not wrap to 254
    loop {}
}
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
```rust,compile
import { count_bits } from "std/math.wr";

#[reset]
fn main() {
    let bits: u8 = count_bits(0b10110101);  // Returns 5
    loop {}
}
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
```rust,compile
import { reverse_bits } from "std/math.wr";

#[reset]
fn main() {
    let reversed: u8 = reverse_bits(0xA5);  // 0xA5 -> 0xA5 (palindrome)
    let test: u8 = reverse_bits(0x01);      // 0x01 -> 0x80
    loop {}
}
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
```rust,compile
import { swap_nibbles } from "std/math.wr";

#[reset]
fn main() {
    let swapped: u8 = swap_nibbles(0x12);  // 0x12 -> 0x21
    let color: u8 = swap_nibbles(0xF0);    // 0xF0 -> 0x0F
    loop {}
}
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
**Temporary Storage:** Zero page $20-$27 (parameters `a`/`b` are read from their normal zero-page frame slots like any other function parameter - see [Zero Page Allocation](#zero-page-allocation))

**Example:**
```rust,compile
import { mul16 } from "std/math.wr";

#[reset]
fn main() {
    let area: u16 = mul16(320, 200);  // Screen area calculation
    loop {}
}
```

##### `div16(a: u16, b: u16) -> u16`

Divide two 16-bit unsigned integers using non-restoring division.

```rust
fn div16(a: u16, b: u16) -> u16
```

**Algorithm:** Non-restoring division (optimized for 6502)
**Cycles:** ~1200-1400 (16 iterations of shift-subtract)
**Returns:** a ÷ b (quotient), or 0xFFFF if b == 0
**Temporary Storage:** Zero page $20-$27 (parameters `a`/`b` are read from their normal zero-page frame slots like any other function parameter - see [Zero Page Allocation](#zero-page-allocation))

**Example:**
```rust,compile
import { div16 } from "std/math.wr";

#[reset]
fn main() {
    let total_score: u16 = 4200;
    let num_players: u16 = 7;
    let average: u16 = div16(total_score, num_players);

    // Division by zero handling
    let result: u16 = div16(100, 0);  // Returns 0xFFFF
    if result == 0xFFFF {
        // Handle division by zero
    }
    loop {}
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

The following **38 keywords** are reserved in Wraith and cannot be used as identifiers:

```
addr      as        asm       b8        b16       bool      break
carry     const     continue  else      enum      false     fn
for       from      i8        i16       if        import    in
let       loop      match     negative  overflow  pub       read
return    static    str       struct    true      u8        u16
while     write     zero
```

### Keywords by Category

**Control Flow (9 keywords):**
```
if        else      while     loop      for
match     return    break     continue
```

**Variable Declaration (3 keywords):**
```
let       const     static
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
```rust,compile,fragment
let count: u8 = 10;         // Unsigned 8-bit
let delta: i16 = -500;      // Signed 16-bit
let score: b16 = 1234 as b16;  // BCD 16-bit
let flag: bool = true;      // Boolean
```

**Variable Declaration:**
```rust,compile
const MAX: u8 = 100;        // Compile-time constant

#[reset]
fn main() {
    let x: u8 = 42;         // Mutable variable (automatically zero-page allocated)
    loop {}
}
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
```rust,compile
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

### Bitfield Access

Individual bits of an integer are read and written with built-in methods, where
`n` is a compile-time constant in range for the value's width (0-7 for an 8-bit
value, 0-15 for a 16-bit one):

```rust,compile,fragment
let flags: u8 = 0;
flags.set_bit(7);        // set bit 7  -> 0x80
flags.clear_bit(3);      // clear bit 3
flags.toggle_bit(0);     // flip bit 0
if flags.bit(7) { }      // read bit 7 as a bool
```

- `x.bit(n) -> bool` reads a bit; `set_bit`/`clear_bit`/`toggle_bit` mutate `x`
  in place and need a plain variable target (a local, `static`, or writable
  `addr` register).
- The bit index must be a **compile-time constant**. For a runtime index, use
  `std/math.wr`'s `set_bit`/`clear_bit`/`test_bit`.
- On the **65C02** target, a zero-page `set_bit`/`clear_bit` compiles to a single
  `SMB`/`RMB` instruction; on NMOS **6502**, or for an absolute/`addr` target, it
  is an `ORA`/`AND` read-modify-write. `toggle_bit` is always `EOR`.

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
```rust,compile,fragment
let p: bool = true;
let q: bool = false;
let a: u8 = 1;
let b: u8 = 2;

let x: u8 = 2 + 3 * 4;      // 14, not 20 (multiplication before addition)
let y: u8 = (2 + 3) * 4;    // 20 (parentheses override)
let z: bool = !p && q;      // (!p) && q (NOT before AND)
let w: u8 = a + b << 2;     // (a + b) << 2 (addition before shift)
```

### Arithmetic Overflow Behavior

All arithmetic operators wrap on overflow with no error checking:

```rust,compile,fragment
let x: u8 = 255 + 1;     // 0 (wraps)
let y: u8 = 0 - 1;       // 255 (wraps)
let z: u8 = 200 * 2;     // 144 (400 % 256)

let i: i8 = 127 + 1;     // -128 (signed overflow wraps)
let j: i8 = -128 - 1;    // 127 (signed underflow wraps)
```

**Shifts:**

`<<` and `>>` take a count in bits. A count **at or past the type's width**
shifts every bit out: the result is `0`, or `-1` for `>>` on a negative signed
value, since an arithmetic right shift feeds the sign bit back in.

```rust
let a: u8 = 200;
let n: u8 = 9;
let x: u8 = a << n;        // 0 — every bit shifted out
let y: i8 = (-100) >> n;   // -1 — the sign bit is what comes in
```

The count is **not masked** to the width. A 6502 has no barrel shifter, so a
variable shift is a loop that simply performs the count; there is nothing to
mask and no cost to defining it this way. Masking (as x86 and Java do) would
make `x << 8` on a `u8` mean `x << 0`, which is both surprising and useless,
and would cost an `AND` on every variable shift to produce.

A count the compiler can see is at or past the width is a **warning**, not an
error — the behaviour is defined, and clearing a value by shifting it out is a
real if unusual idiom:

```rust,compile,fragment
let a: u8 = 1;
let n: u8 = 3;

let x: u8 = a << 8;        // warning: shifting a `u8` by 8 always yields 0
let y: u8 = a << 7;        // fine
let z: u8 = a << n;        // fine: `n` is only known at run time
```

**Division and Modulo:**

Division truncates toward zero, and the remainder takes the dividend's sign:
`-23 / 5` is `-4` and `-23 % 5` is `-3`.

Division and modulo **by zero are defined**, not undefined: both yield the
all-ones value of the type.

| Expression | Result |
|---|---|
| `u8 x / 0`, `u8 x % 0` | `0xFF` |
| `u16 x / 0`, `u16 x % 0` | `0xFFFF` |
| `i8 x / 0`, `i8 x % 0` | `-1` |
| `i16 x / 0`, `i16 x % 0` | `-1` |

This is what the hardware sequence already produces rather than a value chosen
for its own sake: shift-and-subtract division with a zero divisor succeeds at
every trial subtraction, so the quotient fills with ones. Defining it costs
nothing — the check is three instructions and was already being emitted — and
it means a program that divides by zero gets the same answer every time instead
of whatever an uninitialized byte held.

RISC-V's M extension defines the same value for the same reason. It differs in
one detail: there the *remainder* of `x % 0` is the dividend, where here it is
all-ones like the quotient. One value for both is simpler to state and to rely
on.

There is **no runtime check and no trap**. A zero divisor is not an error at
run time; it produces the sentinel and execution continues.

A divisor the compiler can see is zero is a *compile-time* error:

```rust
let x: u8 = a / 0;         // error: division by zero
let y: u8 = a % (3 - 3);   // error: modulo by zero
let z: u8 = a / b;         // fine: `b` is only known at run time
```

The sentinel exists for the second case — a divisor that is zero only
sometimes, which no compiler can catch. A divisor that is *always* zero says
nothing about the dividend and is a mistake rather than a choice, so it is
refused where it can be seen.

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
- Common pattern: `if i < len && array[i] == value`

### Completion Status

- [ ] Document operator implementation in assembly

---

## Comments

Wraith supports three types of comments: single-line comments, multi-line comments, and documentation comments.

### Single-Line Comments

Single-line comments start with `//` and continue to the end of the line:

```rust,compile
fn calculate() -> u8 {
    let x: u8 = 42;  // Initialize x to 42
    // This entire line is a comment
    return x;
}
```

### Multi-Line Comments

Multi-line comments begin with `/*` and end with `*/`. They can span multiple lines:

```rust,compile
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

```rust,compile
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

```rust,compile
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

```rust,compile
fn test() {
    let x: u8 = 10 /* comment in middle */ + 5;  // Valid, equals 15
}
```

However, comments inside assembly string literals are **not** processed by Wraith:

```rust,compile,fragment
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

- [x] Document register allocation strategy
- [x] Explain zero page usage
- [x] Document stack usage
- [x] Add optimization passes overview

#### Register Allocation Strategy

Wraith does not use a general-purpose register allocator; the 6502's three registers are assigned fixed roles by convention, and anything that doesn't fit is spilled to zero page:

- **A (accumulator)**: The primary value register. Holds the result of almost every expression, the low byte of 16-bit values, and the low byte of pointer-like return values (arrays, strings, enums).
- **Y**: The high byte of a 16-bit value in registers (parameter evaluation, return values), or the loop index in indexed-addressing code (`(ptr),Y`).
- **X**: The high byte of a pointer-like return value (A:X convention), a loop counter, or the index register when reading/writing the software stack at $0200,X.

Because there's no allocator to spill "extra" live values, the compiler uses small dedicated zero-page pools instead - see the next section.

#### Zero Page Usage

See [Appendix B](#appendix-b-memory-layout) for the full byte-level map. In summary:
- $00-$1F is left untouched (system/platform reserved)
- $20-$3F is a pool of codegen scratch temporaries (binary operation operands, pointer dereferencing, enum tag/data access)
- $40-$CF is the **frame region**: every function's parameters and locals, statically colored by the call graph (see [Zero Page Allocation](#zero-page-allocation))
- $D0-$DC is working storage and call parameters for the compiler-generated 16-bit math routines (`mul16`/`div16`/`mod16`)
- $F0-$F3 and $F4-$FE are small pools for a binary operation's left-operand save and for staging function-call arguments before they're copied into the callee's frame
- $FF holds the software stack pointer described below

#### Stack Usage

Wraith uses **two** stacks, for different purposes:

1. **Hardware stack ($0100-$01FF)** - used exactly as the 6502 uses it natively: `JSR`/`RTS` return addresses, and (in interrupt handlers only) `PHA`/`PLA` save/restore of the A, X, and Y registers around the handler body.
2. **Software stack ($0200-$02FF, indexed by zero-page $FF)** - a compiler-managed stack used for two things, both invisible to the programmer:
   - Saving and restoring a function's own frame across a **recursive** call (a call to itself, or to another function that can call back into it), so the nested invocation cannot destroy values the outer invocation still needs. Non-recursive calls never touch this stack - they have no save/restore overhead at all.
   - Spilling a binary operation's left operand when its right operand contains a function call (e.g. `f(a) + f(b)`), since the fast register-based save (Y for `u8`, a small zero-page pool for `u16`) would otherwise be clobbered by the call.

Recursion inside an interrupt handler's call graph is a compile error, because the software stack is not safe to use reentrantly if an interrupt can fire in the middle of a push/pop sequence.

#### Optimization Passes Overview

Applied roughly in this order during compilation:

1. **Constant folding** - compile-time evaluation of constant expressions (including const-only casts, BCD range validation)
2. **Tail call optimization** - a function's tail-recursive self-call is rewritten to update its own parameters and `JMP` back to the top of the function instead of `JSR`, giving constant stack usage regardless of recursion depth
3. **Dead code elimination** - statements after a `return`/`break`/`continue` in the same block are dropped
4. **Strength reduction** - e.g. multiplication/division by a power of two becomes a shift
5. **Peephole optimization** - a pass over the emitted instruction stream that removes redundant loads/stores, dead stores, no-op register transfers, unreachable code after a terminator, and simplifies comparison-against-zero and branch-over-jump patterns

### Appendix B: Memory Layout

- [x] Document default memory map
- [x] Explain section placement
- [x] Add examples of custom memory layouts

#### Default Memory Map

**Zero page ($0000-$00FF):**

| Range | Size | Purpose |
|-------|------|---------|
| $00-$1F | 32 bytes | System/platform reserved |
| $20-$3F | 32 bytes | Codegen scratch temporaries |
| $40-$CF | 144 bytes | Frame region (all function parameters and locals) |
| $D0-$D8 | 9 bytes | `mul16`/`div16`/`mod16` working storage |
| $D9-$DC | 4 bytes | `mul16`/`div16`/`mod16` call parameters |
| $DD-$DE | 2 bytes | PRNG state (`rand`/`rand16`/`srand` in `std/math.wr`) |
| $DF | 1 byte | Reserved |
| $E0-$E7 | 8 bytes | Argument staging for address-taken functions (indirect calls) |
| $E8-$ED | 6 bytes | Reserved |
| $EE-$EF | 2 bytes | Indirect-call vector (function-pointer trampoline) |
| $F0-$F3 | 4 bytes | Binary operation left-operand save |
| $F4-$FE | 11 bytes | Function-call argument staging |
| $FF | 1 byte | Software stack pointer |

**Other memory:**

| Range | Purpose |
|-------|---------|
| $0100-$01FF | Hardware stack (JSR/RTS, interrupt register save) |
| $0200-$02FF | Default `STACK` section — software stack (recursion frame save/restore, operand spill) |
| $0400-$07FF | Default `BSS` section (1KB) — **RAM** for mutable globals (`static`) |
| $8000-$BFFF | Default `CODE` section (16KB) |
| $D000-$DFFF | Default `DATA` section (4KB) |
| $FFFA-$FFFF | 6502 hardware vectors (NMI, RESET, IRQ) |

The four sections are *defaults*, not a machine description. Only the zero page,
the hardware stack and the vectors are fixed — the 6502 mandates those. Every
other range above comes from `wraith.toml` and is yours to move.

Addresses outside every declared section are left untouched: the compiler places
nothing there and assumes nothing about them. That is where device registers go,
declared with `addr` (see [Memory-Mapped Addresses](#memory-mapped-addresses)),
wherever the machine happens to decode them. Sizing a section so it stops clear
of your hardware is a configuration decision, not something the compiler knows.

Only `BSS` is written at runtime; `CODE` and `DATA` are read-only on a ROM-based
machine. See [Mutable Globals](#mutable-globals-static) for how `static` storage
is allocated and initialized, and the configuration notes below for choosing the
range.

#### Section Placement

Code and data are placed into named **sections**, either the default `CODE`/`DATA` sections above or sections you define in `wraith.toml` (see the `#[section]` and `#[org]` function attributes under [Function Attributes](#function-attributes)). A function with no placement attribute goes into the configured default section; `#[section("NAME")]` places it in a named section; `#[org(address)]` places it at an exact address, overriding section placement entirely.

#### How Placement Works

Addresses are decided before any code is emitted, in three steps: every function
is measured, the ranges claimed by `#[org]` are reserved, and everything else is
allocated into what is left. A pinned function therefore does not have to sit
out of the way of the rest of the program — the allocator routes around it, and
`#[org]` can be used at a section's base address:

```rust
#[org(0x8000)]          // the base of CODE
#[reset]
fn main() { … }

fn helper() { … }       // allocated after main, not on top of it
```

Data placed in a section works the same way: a function pinned into `DATA` is
stepped over by the string table and const arrays that share it.

Auto-allocated functions are placed in declaration order, so the same source
always produces the same layout.

#### `#[org]` Placement Errors

A pinned address cannot be moved, so where two of them cannot both be satisfied
the compiler reports it rather than choosing. Every case where a placement
cannot work is a compile error, reported against the function with a source
excerpt:

- **Overlapping another pinned function.** Two `#[org]` ranges that intersect
  cannot both be honoured. The size used is the measured size of the generated
  code, so a function that merely *grows* into its pinned neighbour is caught
  too. (An `#[org]` overlapping something the allocator placed is not an error:
  the allocator moves that other thing instead.)
- **Overlapping the interrupt vector table.** `$FFFA-$FFFF` holds the NMI, RESET
  and IRQ vectors that the 6502 fetches in hardware. Code placed there replaces
  the reset vector, so the machine never starts — reported specifically rather
  than as a generic overlap.
- **Outside every configured section.** An address covered by no section is not
  accounted for by capacity checks and, in a ROM image, may hold nothing at run
  time. The error lists the configured sections so the address can be moved or a
  section added.
- **Overrunning the end of its section.** The whole range must fit, not just the
  start address.

Two functions whose ranges merely touch — one ending exactly where the next
begins — do not conflict.

```rust
#[org(0xFFF8)]
fn boot() { }        // error: places 'boot' over the interrupt vector table

#[org(0x0100)]
fn helper() { }      // error: $0100 is not inside any configured section
```

#### Configuring RAM (`BSS`)

The `BSS` section is where every `static` is allocated, in declaration order.
It is the one region the program writes to at runtime, so it must name real RAM:

```toml
[[sections]]
name = "BSS"
start = 0x0400
end = 0x07FF
description = "User RAM for mutable globals"
```

Constraints to observe when choosing the range:

- **Avoid the reserved low pages.** The zero page holds codegen scratch and
  function frames, `$0100-$01FF` is the hardware stack (fixed by the processor),
  and the `STACK` section holds the software stack. The default starts at `$0400`
  to clear all three.
- **Avoid memory-mapped I/O.** The compiler warns when an `addr` declaration
  falls inside `BSS`, because a `static` placed there would collide with the
  device register.
- **Budget the space.** Exceeding the region is a compile error naming the range.
  Large buffers dominate: an 80×25 text framebuffer is 2000 bytes, well beyond
  the 1 KB default, so either enlarge `BSS`, choose a smaller geometry, or place
  video memory outside `BSS` and reach it with `addr`.
- A configuration that omits `BSS` falls back to `$0400-$07FF`.

Because statics are allocated in declaration order and never reused, moving a
`static` in the source changes the addresses of the ones after it — relevant only
if you depend on fixed addresses from a debugger or external tooling.

#### Configuring the software stack (`STACK`)

The `STACK` section is one page of RAM used to save a callee's frame across a
recursive call and to spill operands across call-bearing sub-expressions:

```toml
[[sections]]
name = "STACK"
start = 0x0200
end = 0x02FF
```

Its size is fixed at 256 bytes (the stack pointer is a single zero-page byte),
but the page itself is configurable. It must be RAM and must not overlap `BSS`
or memory-mapped I/O. It is distinct from the 6502 **hardware** stack at
`$0100-$01FF`, which the processor mandates for `JSR`/`RTS` and interrupt entry
and which cannot be relocated.

#### What the compiler fixes

Only what the hardware or the zero-page addressing modes require:

| Region | Why it is fixed |
|--------|-----------------|
| `$0000-$00FF` | Zero page — codegen scratch, function frames, pointers; zero-page and indirect addressing modes only reach this page |
| `$0100-$01FF` | 6502 hardware stack (`JSR`/`RTS`, interrupt entry) |
| `$FFFA-$FFFF` | NMI / RESET / IRQ vectors |

Everything else — code, constant data, the software stack and mutable-global
RAM — is placed by `wraith.toml` and can be moved to match the board.

#### Custom Memory Layout Example

```toml
[[sections]]
name = "STDLIB"
start = 0x8000
end = 0x8FFF
description = "Standard library functions (4KB)"

[[sections]]
name = "CODE"
start = 0x9000
end = 0xBFFF
description = "User code (12KB)"

[[sections]]
name = "DATA"
start = 0xC000
end = 0xCFFF
description = "Constants and data (4KB)"

default_section = "CODE"
```

### Appendix C: Calling Convention

- [x] Document parameter passing
- [x] Explain return value handling
- [x] Add calling convention for interrupt handlers

#### The Full Call Sequence

For a call `f(arg1, arg2, ...)`:

1. **Evaluate arguments** - each argument expression is evaluated in turn and its result staged into the $F4-$FE temp pool (not directly into `f`'s frame yet - this lets an argument's own evaluation freely use zero-page scratch, and lets a later argument be a call to `f` itself, without corrupting an already-evaluated earlier argument)
2. **Save the callee's frame, if this is a recursive call** - if `f` can (transitively) call back to the current function, the current contents of `f`'s frame are pushed to the software stack, so a nested/re-entrant invocation cannot destroy the outer invocation's still-needed values
3. **Copy staged arguments into `f`'s frame** - from the temp pool into `f`'s actual parameter slots
4. **`JSR f`**
5. **Restore the callee's frame, if step 2 saved one** - popped back from the software stack, with the return value preserved across the pop
6. **Read the return value** - from A, A+Y, or A+X depending on the return type (see below)

A tail-recursive self-call (`return f(...)` where `f` is the enclosing function) skips this entirely: arguments are copied straight into the function's own frame and control jumps back to the top of the function with `JMP` - no `JSR`, no frame save, no stack growth.

#### Return Value Handling

| Return type | Location |
|---|---|
| `u8`, `i8`, `b8`, `bool` | A |
| `u16`, `i16`, `b16` | A (low byte), Y (high byte) |
| Array, string, enum | A (low byte of pointer), X (high byte of pointer) |
| Void | (no value) |

#### Calling Convention for Interrupt Handlers

A function marked `#[irq]` or `#[nmi]` is installed at the corresponding hardware vector and compiled with an automatic prologue/epilogue:

1. **Prologue**: `PHA`; `TXA`/`PHA`; `TYA`/`PHA` (save A, X, Y to the hardware stack), followed by saving any zero-page scratch/frame state the handler's call graph can touch (see below)
2. **Handler body**
3. **Epilogue**: restore the zero-page state saved above, then `PLA`/`TAY`; `PLA`/`TAX`; `PLA` (restore Y, X, A in reverse order), then `RTI` instead of `RTS`

Because an interrupt can preempt main-line code at any point - including in the middle of an expression that has live values in the shared zero-page scratch pools - the compiler computes, per handler, which of that scratch and which other functions' frames the handler's own call graph reaches, and saves/restores exactly that state around the handler body. This makes interrupt handlers safe to write using the same language features as regular code, with two restrictions:

- **No recursion** is allowed anywhere in an interrupt handler's call graph (the software stack used for frame save/restore is not reentrant)
- The save/restore adds latency proportional to how much state the handler's call graph touches; handlers that call few/simple functions have a smaller, cheaper save set

### Appendix D: Examples

- [ ] Add complete program examples
- [ ] Include common patterns and idioms
- [ ] Add performance optimization examples

---

## Revision History

- 2026-08-25: Multidimensional arrays (`[[T; N]; M]`) implemented — a local now
  initialises from a nested literal the way a `const` and `static` already did,
  and `m[i][j]` indexing and passing one to a function were already in place.
  And `let mut x` now reports that the language has no `mut` (locals are mutable
  by default) instead of failing with a misleading "expected `:`".
- 2026-08-23 (0.7.0):

  *Language.* Compile-time generated tables, written
  `const SQR: [u8; 16] = [|i| => i * i];` — folded before the program runs,
  with the length taken from the declared type. Structure-of-arrays layout for an array of structs, `#[soa]`,
  storing one column per field so an indexed field read costs an index rather
  than a multiply; every whole-element use columns cannot support is refused,
  and a warning suggests the attribute where it would pay. Whole-array
  assignment is refused — copy element-wise or with `memcpy`, so a move the
  length of an array is visible in the source. `static` struct initialisers
  accept `str`, enum and `&T` fields. An attribute written on a declaration it
  does not apply to is an error rather than being ignored.

  *Defined behaviour.* Divide by zero yields the all-ones sentinel at every
  width and sign, and a divisor the compiler can see is zero is refused. A
  shift count at or past the width shifts every bit out, and warns when the
  count is constant. A `for` bound whose sign or width the counter cannot hold
  is refused.

  *Miscompiles fixed*, each with a regression test. A struct returned by value
  came back as its first byte; a struct passed to an inlined call came through
  as its first two bytes; an enum field of a struct was stored as a pointer's
  low byte and read by dereferencing it; `*p` on a pointer-to-pointer dropped
  the high byte; two-byte `static`s and `static` struct fields stored one byte;
  re-pointing a `str` local wrote one byte of two; implicit widening took the
  sign from the destination rather than the source, at seven sites, and a match
  arm widened by zero regardless of its sign; a tail call rebound its
  parameters at the wrong widths; a function-pointer argument was sized as one
  byte; `&x.f[0]`, `&m[i][j]`, `p.a[i]` and `mk(6).f1` computed the wrong
  address or were rejected; and `arr[i] = f()` called `f` twice.

  *Diagnostics.* Several errors are reported per run with rustc-style spans. A
  failed declaration no longer hides the bodies below it — only its own name is
  suppressed — an unknown type is reported where it is written, and a broken
  module reports every error once however many import paths reach it. Every
  `SemaError` variant is now pinned by a golden test or carries a written
  reason for not being.

  *Compiler.* Argument staging for the four call forms merged into one routine
  and one width table. Frame colouring given an edge for indirect calls, and
  arguments spill to the software stack when the pool will not hold them. BSS
  is repacked so a dropped `static` gives its bytes back. Two-branch
  comparisons fuse into their branch. The differential fuzzer gained pointers
  with alias modelling, aggregates, slices, function-pointer dispatch, mixed
  widths and cross-call arguments; 137 of this document's examples are compiled
  on every run.
- 2026-08-09 (0.6.0): Pointer equality (`==`/`!=` on the same pointer type);
  bit mutation through a pointer or a runtime index; 65C02 `BBR`/`BBS` fusion for
  `if x.bit(n)`; function-pointer dispatch tables (`handlers[i](x)`) fixed for
  `static`, `const` and local tables; `u16` `&`/`|`/`^` fixed to combine the high
  byte; a 16-bit xorshift PRNG replacing the LFSR; self-referential by-value
  structs and index-scaling overflow rejected; multi-error reporting and
  rustc-style diagnostics; an interrupt hardware-stack depth warning; and a
  memory map that reserves `$E000-$EFFF` for memory-mapped I/O.
- 2026-01-13: Initial skeleton created with checkboxes for incremental completion
