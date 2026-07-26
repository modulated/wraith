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

- Rust toolchain (cargo) — builds the compiler and the bundled `flatasm`
  assembler; no external 6502 assembler is required

### Build and Run

```bash
# Build the Wraith compiler and the flatasm assembler
cargo build --release

# Compile a Wraith program -> my_program.asm
cargo run --release my_program.wr

# Assemble the output into a flat ROM image
cargo run --release --bin flatasm -- my_program.asm -o my_program.rom --rom
```

### Assembling to a binary (`flatasm`)

Wraith emits **absolute** assembly: every function is placed with `.ORG` at its
final address and the interrupt vector table is written directly at
`$FFFA`-`$FFFF`. That is a flat-image model, not the relocatable-segment model
that `ca65`/`ld65` (and a linker `.cfg`) assume — `ca65`'s `.ORG` only moves the
logical program counter, it does not seek or pad the output, so it packs the
`.ORG` blocks together and drops the vectors, producing a broken image. Use the
bundled `flatasm` instead; it honours `.ORG` as an absolute seek and shares its
implementation with the compiler's own test harness.

```bash
# Full 64 KB image (byte i = memory address i)
flatasm my_program.asm -o my_program.bin

# $8000-$FFFF ROM image (32 KB), e.g. for a ROM at the top of memory
flatasm my_program.asm -o my_program.rom --rom

# An arbitrary address range
flatasm my_program.asm -o my_program.bin --start 0x8000 --end 0xFFFF
```

`flatasm` prints the resolved reset vector (`$FFFC`) on completion as a sanity
check and exits non-zero with a message on malformed input (undefined label,
out-of-range branch, unknown mnemonic). Run it via `cargo run --bin flatasm --`
from the source tree, or use the `flatasm` binary from `cargo build --release`
(`target/release/flatasm`).

### Command-line options

```
Usage: wraith [OPTIONS] <input.wr>

  -h, --help              Print help
  -v, --version           Print version information
  -c, --comments LEVEL    Comment verbosity in the generated assembly
                          LEVEL: minimal, normal (default), verbose
  -o, --out DIR           Write the .asm output to DIR instead of alongside
                          the source. DIR is created if it does not exist.
      --completions SHELL Print a shell completion script and exit
                          SHELL: bash, zsh, fish
```

By default the assembly is written next to its source, so `src/main.wr`
produces `src/main.asm`. `--out` keeps the file name but replaces the
directory, which keeps generated assembly out of the source tree:

```bash
wraith --out build src/main.wr    # writes build/main.asm
```

### Shell completion

Completion covers the flags, their argument values (verbosity levels, shell
names), directories after `--out`, and `.wr` files for the source argument.
The scripts live in [`completions/`](completions/) and the compiler can also
print them:

```bash
# bash — source it from ~/.bashrc, or install system-wide
wraith --completions bash > ~/.local/share/bash-completion/completions/wraith

# zsh — drop it somewhere on your $fpath, with compinit enabled in ~/.zshrc
wraith --completions zsh > ~/.zsh/completions/_wraith

# fish
wraith --completions fish > ~/.config/fish/completions/wraith.fish
```

Completion applies to the installed `wraith` binary, not to `cargo run`.

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
name = "STACK"
start = 0x0200
end = 0x02FF
description = "Compiler software stack (256B)"

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
- **STACK**: `0x0200-0x02FF` (256B) — **RAM** for the compiler's software stack
- **BSS**: `0x0400-0x07FF` (1KB) — **RAM** for mutable globals (`static`)

Every one of these is a `wraith.toml` section, so the whole map is yours to
define. The only addresses the compiler fixes are those the 6502 itself
mandates — the zero page (scratch and function frames), the hardware stack at
`$0100-$01FF`, and the vectors at `$FFFA-$FFFF`.

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

### The STACK section

`STACK` is one page of RAM holding Wraith's software stack, used to save a
callee's frame across a recursive call and to spill operands. Its size is fixed
at 256 bytes (the pointer is a single zero-page byte), but the page is yours to
place:

```toml
[[sections]]
name = "STACK"
start = 0x0200
end = 0x02FF
```

It must be RAM, must not overlap `BSS` or I/O, and is distinct from the 6502
hardware stack at `$0100-$01FF` (used by `JSR`/`RTS` and interrupts), which the
processor fixes and the compiler cannot move.

## Examples

Check the `examples/` directory for sample programs demonstrating:

- Tail-recursive optimization
- Interrupt handling
- Nested structs
- Mathematical operations
- Memory manipulation

## Contributing

See [ROADMAP.md](ROADMAP.md) for planned features and development priorities.
