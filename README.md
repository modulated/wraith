# Wraith

A systems programming language that compiles directly to 6502 assembly. Wraith takes inspiration from Rust's syntax while remaining low-level and explicit, designed specifically for the constraints and capabilities of the 6502 processor.

## Key Features

- **Direct 6502 Assembly Generation** - Compiles to hand-optimized 6502 assembly code, not a generic bytecode
- **Opinionated Target** - Designed specifically for 6502 architecture with no abstraction overhead
- **Low-Level Control** - Memory-mapped I/O, inline assembly, and explicit memory management
- **No Runtime** - Zero-cost abstractions with no garbage collection or hidden allocations
- **Modern Syntax** - Rust-inspired syntax with explicit types and pattern matching
- **Tail Call Optimization** - Recursive functions optimized to loops when possible
- **Configurable Memory Sections** - Control code placement for different memory layouts

## Quick Setup

### Prerequisites
- Rust toolchain (cargo)
- A 6502 assembler (e.g., ca65, DASM, or your preferred assembler)

### Build and Run

```bash
# Build the Wraith compiler
cargo build --release

# Compile a Wraith program
cargo run --release <your-program.wr>

# This generates <your-program.asm>
# Assemble it with your 6502 assembler of choice
ca65 your-program.asm -o your-program.o
ld65 your-program.o -o your-program.bin
```

### Example Program

```rust
#[reset]
fn main() {
    let x: u8 = 42;
    let y: u8 = x + 8;

    // Memory-mapped output
    let OUTPUT: addr = 0x6000;
    OUTPUT = y;

    loop { }
}
```

## Documentation

For complete language specification including syntax, types, and standard library, see [specification.md](specification.md).

## Configuration

Wraith uses a `wraith.toml` configuration file to define memory sections for the 6502 target. The compiler looks for `wraith.toml` in the current directory when compiling. If not found, it uses default settings.

### Memory Sections

The configuration file defines memory sections where code and data can be placed:

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

### Default Configuration

If no `wraith.toml` is present, the compiler uses these defaults:
- **STDLIB**: `0x8000-0x8FFF` (4KB) - Standard library
- **CODE**: `0x9000-0xBFFF` (12KB) - User code (default)
- **DATA**: `0xC000-0xCFFF` (4KB) - Constants and data

Functions without an explicit `#[org]` or `#[section]` attribute are placed in the default section.

## Examples

Check the `examples/` directory for sample programs demonstrating:
- Tail-recursive optimization
- Interrupt handling
- Nested structs
- Mathematical operations
- Memory manipulation

## Contributing

See [ROADMAP.md](ROADMAP.md) for planned features and development priorities.

## License

[Add license information here]
