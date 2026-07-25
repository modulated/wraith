# Wraith

A systems programming language that compiles directly to 6502 assembly. Wraith takes inspiration from Rust's syntax while remaining low-level and explicit, designed specifically for the constraints and capabilities of the 6502 processor.

## Key Features

- **Direct 6502 Assembly Generation** - Compiles to compiler-optimized 6502 assembly code, not a generic bytecode
- **Opinionated** - Designed specifically for 6502 architecture with no runtime or abstraciton overhead
- **Low-Level Control** - Memory-mapped I/O, inline assembly, and explicit memory management if required
- **Modern Syntax** - Rust-inspired syntax with explicit types and pattern matching
- **Tail Call Optimization** - Recursive functions optimized to loops when possible
- **Module System** - No more header files, no more macros
- **Configurable Memory Sections** - Control code, data and RAM placement for different memory layouts
- **Mutable Globals** - `static` state in RAM, shareable between interrupt handlers and main code
- **Function Pointers & Vtables** - Call through struct fields (`device.read(reg)`) for driver-style dispatch
- **Slices** - `&[T]` views over arrays with runtime length, passable to and returnable from functions

## Quick Setup

### Prerequisites

- Rust toolchain (cargo)
- A 6502 assembler (e.g., ca65, DASM, or your preferred 6502 assembler)

### Build and Run

```bash
# Build the Wraith compiler
cargo build --release

# Compile a Wraith program
cargo run --release my_program.wr

# This generates my_program.asm
# Assemble it with your 6502 assembler of choice
ca65 my_program.asm -o my_program.o
ld65 my_program.o -o my_program.bin
```

## Documentation

For complete language specification including syntax, types, and standard library, see [specification.md](specification.md).

## Configuration

Wraith uses a `wraith.toml` configuration file to define memory sections for the 6502 target. The compiler looks for `wraith.toml` in the current directory when compiling. If not found, it uses default settings.

### Memory Sections

The configuration file defines memory sections where code and data can be placed:

```toml
[[sections]]
name = "MY_AWESOME_LIBRARY"
start = 0x8000
end = 0x8FFF
description = "Custom Library functions (4KB)"

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

[[sections]]
name = "BSS"
start = 0x0400
end = 0x07FF
description = "User RAM for mutable globals (1KB)"

default_section = "CODE"
```

### Default Configuration

If no `wraith.toml` is present, the compiler uses these defaults:

- **CODE**: `0x8000-0xBFFF` (16KB) — user code (default)
- **DATA**: `0xD000-0xEFFF` (8KB) — constants and read-only data
- **BSS**: `0x0400-0x07FF` (1KB) — **RAM** for mutable globals (`static`)

Functions without an explicit `#[org]` or `#[section]` attribute are placed in the default section.

### The BSS section (RAM)

`BSS` is the only section the compiler writes to at runtime: every `static` is
allocated there, in declaration order, and the reset handler writes their
initial values (RAM contents are undefined at power-on).

Point it at whatever RAM your board actually has:

```toml
[[sections]]
name = "BSS"
start = 0x0400
end = 0x07FF
```

Things to keep in mind when choosing the range:

- **Do not overlap the reserved low pages.** The zero page (`$0000-$00FF`) holds
  codegen scratch and call-graph-colored function frames, `$0100-$01FF` is the
  6502 hardware stack, and `$0200-$02FF` is Wraith's software stack (used for
  recursion and operand spills). The default starts at `$0400` to clear all three.
- **Do not overlap memory-mapped I/O.** The compiler warns if an `addr`
  declaration falls inside `BSS`, since a `static` allocated there would collide
  with the device register.
- **Size it for your data.** Overflowing the region is a compile error naming the
  range. A text framebuffer is often the largest consumer (an 80×25 screen is
  2000 bytes — more than the 1 KB default), so either enlarge `BSS`, use a
  smaller geometry, or map video memory separately with `addr`.
- If a config omits `BSS` entirely, the compiler falls back to `$0400-$07FF`.

## Examples

Check the `examples/` directory for sample programs demonstrating:

- Tail-recursive optimization
- Interrupt handling
- Nested structs
- Mathematical operations
- Memory manipulation

## Contributing

See [ROADMAP.md](ROADMAP.md) for planned features and development priorities.
