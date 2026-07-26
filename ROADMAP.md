# Wraith Compiler - Development Roadmap

_Updated: July 2026_

This roadmap contains **unimplemented work only**. For what the language already
does, see the [Language Specification](specification.md). For 65C02 instruction
selection and the historical bug log, see [TODO.md](TODO.md).

The near-term goal driving most of this list is a small operating system for a
6502 homebrew machine: UART serial, keyboard input, a monochrome text display,
and a device protocol over a 6522 VIA. Language work is ordered by what that
needs.

---

## 🔴 BLOCKING THE OS

### 1. `#[org]` placement should reserve its range

`#[org]` pins a function to an address but does not tell the section allocator,
which keeps handing the same addresses to auto-allocated functions. Pinning
anything at a section's base address therefore collides with the next function
placed — `examples/uart.wr` fails exactly this way, with `main` at `$8000` and
`uart_init` auto-allocated to `$8000`.

Collisions are now a compile error rather than silent corruption (see
[`#[org]` Placement Errors](specification.md#org-placement-errors)), so this is
loud rather than dangerous — but `#[org]` and auto-allocation still cannot be
used in the same program.

**Fix**: two-phase placement. Measure every function, reserve the explicitly
placed ranges, then allocate the rest around them. The allocator needs to become
a range list rather than a per-section bump pointer.

**Complexity**: Medium. Sizes are already measured in a first pass, so the
information exists; the allocator and the generation loop need reordering.

---

### 2. Address-of (`&x`)

`&x` does not parse. A driver cannot be handed a caller's buffer, so anything
that fills a buffer must either own it as a `static` or take an address as a
bare `u16` the compiler cannot check.

```rust
let buf: [u8; 64] = [0; 64];
uart_read_line(&buf, 64);   // parse error: expected expression, found '&'
```

**Action items**:
- Parse `&expr` and `&mut expr` as a unary operator
- Type it as a pointer to the operand (or reuse `Type::Slice` where a length is
  also wanted)
- Reject taking the address of a frame-allocated local that outlives its frame —
  frames are colored and reused, so the pointer would dangle

**Complexity**: Medium. Interacts with frame coloring, which is where the
soundness risk sits.

---

### 3. Console and keyboard drivers

The device models exist (see *What the emulator can simulate* below) but no
driver is written against them.

- Keyboard: scancode in via a VIA port with a CA1 strobe raising IRQ, decoded to
  ASCII, queued in a `static` ring buffer
- Text console: memory-mapped framebuffer, cursor, line wrap, scroll (reuse
  `memcpy16` from `std/mem.wr`), clear-screen
- Monitor command loop: `peek` / `poke` / `dump` / `load` / `run` / `help` over
  the modelled UART

Depends on (1) for placement and benefits from (2) for buffer passing.

**Complexity**: Medium — mostly Wraith code rather than compiler work.

---

## 🟡 HIGH PRIORITY

### 4. Standard library gaps

Present: `mul16`, `div16`, `divmod`, `mul_wide`, `memcpy`/`memcpy16`,
`memset`/`memset16`, `memcmp`, `str_copy`, PRNG (`rand`, `rand16`, `srand`), bit
helpers, saturating arithmetic.

Still missing:
- `abs(x: i8) -> i8` and `abs16(x: i16) -> i16`
- `strlen` / `strcmp` for the length-prefixed `str` representation
- `bcd_to_string(value: b8) -> str`, `bcd16_to_string(value: b16) -> str`, and
  `string_to_bcd` — needed to display BCD counters on the console

**Complexity**: Low for `abs`; Medium for the string conversions (6502 string
building).

---

### 5. Dead code elimination for the root module

Unreferenced items from *imported* modules are now dropped (see
[Unused Imports Are Not Emitted](specification.md#unused-imports-are-not-emitted)),
but the file being compiled always emits everything it defines, dead or not. It
warns instead.

The liveness machinery is already in place — `SemanticAnalyzer::reachable_symbols`
computes the closure over the whole program — so this is a question of whether
dropping a warned-about function is the behaviour we want, not of new analysis.

**Complexity**: Low (the analysis exists); the decision is a design call.

---

## 🟢 MEDIUM PRIORITY

### 6. Bitfield access syntax

Manual shifts and masks today. Device registers are almost entirely bitfields, so
this is the ergonomic gap the OS work runs into most often.

```rust
status.bit(7)              // read bit 7
status.set_bit(7)          // set bit 7
status.clear_bit(7)        // clear bit 7
flags.bits[7:4]            // the high nibble
```

On a 65C02 target these lower to `BBR`/`BBS`/`SMB`/`RMB` directly (see
[TODO.md](TODO.md)).

**Complexity**: Medium (parser + codegen).

---

### 7. Branch optimization intelligence

Status flags are discarded after every comparison, so a repeated test re-emits
the `CMP`.

```rust
if x > 5 { foo(); }
if x > 5 { bar(); }   // the second CMP is redundant if x is unchanged
```

Note the hazard: the register-state tracker that would underpin this has already
produced several silent miscompiles by outliving a label. Any flag tracking must
invalidate at labels and calls from the outset (`Emitter::emit_label` already
does this for registers).

**Complexity**: High (dataflow analysis, with a demonstrated correctness risk).

---

### 8. Disassembly output mode

Emit an annotated listing with resolved addresses and cycle counts, for
performance work and for reading what the peephole actually did.

```
9000: A9 00     LDA #$00        ; [2 cycles] load zero into A
9002: 85 40     STA $40         ; [3 cycles] store to x
```

The emulator harness already assembles Wraith output to a byte image
(`tests/common/exec.rs`), so address resolution is largely solved; the cycle
table is new.

**Complexity**: Medium.

---

## 🔵 LOWER PRIORITY

### 9. Inline data directive

Lookup tables and sprite data colocated with the code that uses them, rather than
hoisted to a `static`.

```rust
data lookup_table: [u8; 16] = [0x00, 0x01, 0x04, 0x09, /* … */];
```

**Complexity**: Low, but note the const-array path currently emits to a hardcoded
`$C000` org and does not go through the section allocator — item (10) should be
fixed first, or inline data will not be covered by conflict detection either.

---

### 10. Const-array placement through the allocator

Const arrays are emitted at a hardcoded `.ORG $C000` and never recorded as
allocations, so they are the one kind of output the `#[org]` conflict check
cannot see. Route them through `SectionAllocator` like string literals.

**Complexity**: Low.

---

### 11. Span identity across modules

`Span` is a bare byte offset with no file identity, so two modules can collide on
one. The inline-expansion path works around this by preferring the callee's own
symbol for the duration of its body; nothing else does. Adding a file id to
`Span` closes the class.

**Complexity**: Medium (touches every diagnostic).

---

## What the emulator can simulate

Device models and the execution harness are done; this is the substrate the OS
work builds on, listed here so it is not re-planned.

| File | Purpose |
| --- | --- |
| `tests/common/exec.rs` | Two-pass assembler, `TestBus`, `run()` / `run_with_devices()`, memory and register inspection |
| `tests/common/devices.rs` | TL16C550 UART (RBR/THR, IER, IIR, LCR+DLAB, LSR, divisor latches, RX FIFO, IRQ) and 6522 VIA (PORTA/B, DDRA/B, T1 counting CPU cycles, IFR/IER, CA1 strobe) |
| `tests/e2e/devices.rs` | 7 tests: polled TX/RX, FIFO drain, baud divisor through DLAB, IRQ-driven RX into a ring buffer, VIA port I/O, timer IRQ flag, timer IRQ reaching a handler |
| `tests/e2e/statics.rs` | 8 tests: mutable globals, reset-time initialization, IRQ/main sharing, non-overlap, u16 statics, static arrays, configurable stack page |
| `tests/e2e/vtable.rs` | 5 tests: calls through struct fields, runtime driver selection, multi-method vtables |
| `tests/e2e/interrupts_exec.rs` | 5 tests: IRQ handler execution, register preservation, main-state integrity, NMI edges, masking |
| `tests/e2e/frames.rs` | 14 tests: call-graph frame coloring, recursion, deep chains |

`examples/monitor/` and `examples/uart.wr` hold an earlier monitor sketch. They
do not currently compile — `uart.wr` on item (1) above, `simple_monitor.wr` on an
unresolved `uart_putc` — and should be treated as reference material to be
rewritten as part of item (3), not as working examples.

---

## Recently Completed ✅

**OS enablement (2026)**
- Mutable globals (`static`) allocated in a configurable BSS/RAM section
- Indirect calls through computed callees, enabling device vtables of function
  pointers
- Fully configurable memory map — only zero page, the hardware stack and the
  vectors are fixed; `CODE`/`DATA`/`STACK`/`BSS` all come from `wraith.toml`
- Emulator device framework (TL16C550 UART, 6522 VIA) with IRQ delivery

**Language**
- Glob imports (`import { * } from "m.wr"`), with unreferenced imported items
  dropped from the output
- First-class slices: creation, `.len`, indexing, re-slicing, passing, returning,
  inclusive ranges, iteration past 255 elements
- Runtime string `==` / `!=`
- Array literals adopt their declared element type (`let a: [u16; 3] = [0,0,0]`)
- Static frame allocation with call-graph coloring

**Enums** (both items from the previous roadmap)
- Tuple variant pattern matching — now covered by 13 execution tests, which found
  and fixed a jump-table dispatch bug that clobbered the payload pointer
- Struct variant pattern matching (`Message::Move { x, y }`) works and is tested

**Standard library**
- `mul16`, `div16`, `divmod`, `mul_wide`, `memcpy16`/`memset16`, PRNG
  (`rand`/`rand16`/`srand`)

**Diagnostics and correctness**
- `#[org]` collisions are compile errors, against functions, data, statics and
  the interrupt vector table, with source excerpts
- BCD `SED`/`CLD` peephole consolidation
- Inline-asm `{param}` substitution inside `#[inline]` functions (the whole
  `min`/`max`/`clamp` section of `std/math.wr` was uncallable)

**Tooling**
- `--out <dir>` and shell completion for bash, zsh and fish

See the [Language Specification](specification.md) for full documentation of
everything implemented.
