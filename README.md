# Wraith

A systems programming language that compiles directly to 6502 assembly. Wraith takes inspiration from Rust's syntax while remaining low-level and explicit, designed specifically for the constraints and capabilities of the 6502 processor.

## Key Features

- **Direct 6502 Assembly Generation** - Compiles to compiler-optimized 6502 assembly code, not a generic bytecode
- **65C02 Support** - Targets the 65C02 by default (`--cpu 6502` for NMOS);
- **Opinionated** - Designed specifically for 6502 architecture with no runtime or abstraciton overhead
- **Low-Level Control** - Memory-mapped I/O, inline assembly, and explicit memory management
- **Modern Syntax** - Rust-inspired syntax with explicit types, AST and pattern matching
- **Tail Call Optimization** - Recursive functions optimized to loops when possible
- **Module System** - No more header files, no more macros
- **No Assumed Machine** - The compiler knows the 6502 and nothing else: no built-in peripherals, no fixed I/O range, no presumed memory size. Devices are `addr` declarations you write, memory is sections you configure, and only the addresses the processor itself mandates are reserved
- **Configurable Memory Sections** - Control code, data and RAM placement for different memory layouts in your bespoke 6502 computer
- **Mutable Globals** - `static` state in RAM, shareable between interrupt handlers and main code
- **Function Pointers & Vtables** - Call through struct fields (`device.read(reg)`), tables of drivers indexed at runtime (`DRIVERS[id].write(c)`), and per-instance state (`&State` in the vtable row) — see [`examples/device_drivers.wr`](examples/device_drivers.wr)
- **Slices** - `&[T]` views over arrays with runtime length, passable to and returnable from functions
- **Pointers** - `&x`, `*p`, `p[i]`, `p.field`, with an escape analysis that rejects a pointer outliving what it names
- **Bitfield Access** - `flags.set_bit(7)` / `clear_bit` / `toggle_bit` / `.bit(n)` on any integer, constant-folded to a mask
- **Compile-Time Tables** - `const SQR: [u8; 16] = [|i| => i * i];` folds to `.BYTE $00, $01, $04, $09, …` in ROM. The oldest 6502 trade — ROM for cycles — stated in the language instead of in a build script, with the length taken from the type so it is written once
- **Structure-of-Arrays Layout** - `#[soa]` on an array of structs stores one column per field, so `sprites[i].y` is `LDA col,Y` instead of multiplying the index by the element size first: 6 cycles against 19. Asked for by name, because a layout the compiler chose could be flipped back by one added `&sprites[i]` with nothing in the source to show for it — the compiler suggests it and refuses the uses that would break it
- **No Undefined Corners** - Divide by zero is the all-ones sentinel at every width and sign; a shift at or past the width shifts every bit out. Both are specified rather than left to whatever the emitted code happened to do, and the compiler refuses or warns where it can see the operand is constant
- **Explicit Aggregate Copies** - A whole array is never copied by an assignment. Copy element-wise or with `memcpy`, so a move the length of an array is visible at the place it happens rather than hidden behind an `=`
- **Rust-Style Diagnostics** - Several errors per run, each with a source span, caret and suggestion. A declaration that fails to check no longer hides the bodies below it, and a broken module reports every error once however many import paths reach it

## Quick Setup

### Prerequisites

- Rust toolchain (cargo) — builds the compiler, `wraith`, and the bundled
  assembler, `flatasm`; no external 6502 assembler is required

### Build and Run

```bash
# Build the Wraith compiler and the flatasm assembler
cargo build --release

# Compile a Wraith program -> my_program.asm
cargo run --release my_program.wr

# Assemble the output into a flat ROM image
cargo run --release --bin flatasm -- my_program.asm --rom
```

### Command-line options

```
Usage: wraith [OPTIONS] <input.wr>

  -h, --help              Print help
  -v, --version           Print version information
  -c, --comments LEVEL    Comment verbosity in the generated assembly
                          LEVEL: minimal, normal (default), verbose
      --cpu TARGET        Target CPU (default: 65c02)
                          TARGET: 65c02, 6502
  -o, --out DIR           Write the .asm output to DIR instead of alongside
                          the source. DIR is created if it does not exist.
      --completions SHELL Print a shell completion script and exit
                          SHELL: bash, zsh, fish
```

By default the assembly is written next to its source, so `src/main.wr`
produces `src/main.asm`. Use `-o` or `--out` to specify a build
directory, which keeps generated assembly out of the source tree:

```bash
wraith --out build src/main.wr    # writes build/main.asm
```

### Assembling to a binary (`flatasm`)

Wraith emits **absolute** assembly: every function is placed with `.ORG` at its
final address and the interrupt vector table is written directly at
`$FFFA`-`$FFFF`. That is a flat-image model, not the relocatable-segment model
that other assemblers (eg `ca65`/`ld65`).

```bash
# Full 64 KB image (byte i = memory address i)
flatasm my_program.asm

# $8000-$FFFF ROM image (32 KB), e.g. for a ROM at the top of memory
flatasm my_program.asm --rom

# An arbitrary address range
flatasm my_program.asm --start 0x8000 --end 0xFFFF
```

`flatasm` prints the resolved reset vector (`$FFFC`) on completion as a sanity
check and exits non-zero with a message on malformed input (undefined label,
out-of-range branch, unknown mnemonic).

### Environment variables

- `WRAITH_STD_PATH` — where non-relative imports (`import {memcpy} from
"mem.wr"`) look for the standard library. Defaults to `std/` relative to
  the working directory.

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

For complete language specification including syntax, types, and standard library, see [docs/specification.md](docs/specification.md).

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
- **DATA**: `0xD000-0xDFFF` (4KB) — constants and read-only data
- **STACK**: `0x0200-0x02FF` (256B) — **RAM** for the compiler's software stack
- **BSS**: `0x0400-0x07FF` (1KB) — **RAM** for mutable globals (`static`)

These are defaults, not a description of any particular machine. Every one is a
`wraith.toml` section, so the whole map is yours to define. The only addresses
the compiler fixes are those the 6502 itself mandates — the zero page (scratch
and function frames), the hardware stack at `$0100-$01FF`, and the vectors at
`$FFFA-$FFFF`.

Anything outside the sections you declare is left alone: the compiler places
nothing there and assumes nothing about it. Device registers live wherever your
hardware decodes them and are named with `const NAME: addr = ...`; leaving room
for them is a matter of how you size your sections, not something the compiler
has an opinion about.

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
  range.
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

[`device_drivers.wr`](examples/device_drivers.wr) is the longest of them and
the one closest to a real system: a driver is a struct of function pointers, a
table of those is the device list, and the kernel half of the program names no
driver and touches no register. One of its drivers is interrupt-driven with a
ring buffer at each end, so writing to the console queues a byte and returns;
the other is unbuffered, and nothing above the interface can tell. The devices
it drives are the ones its own test harness models — pick whatever your board
actually has. It runs against them in `tests/e2e/example_drivers.rs` rather
than only compiling, so what it claims is checked.

## Contributing

See [docs/ROADMAP.md](docs/ROADMAP.md) for planned features and development priorities.
