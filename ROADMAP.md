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

### 1. Console and keyboard drivers

The device models exist (see *What the emulator can simulate* below) but no
driver is written against them.

- Keyboard: scancode in via a VIA port with a CA1 strobe raising IRQ, decoded to
  ASCII, queued in a `static` ring buffer
- Text console: memory-mapped framebuffer, cursor, line wrap, scroll (reuse
  `memcpy16` from `std/mem.wr`), clear-screen
- Monitor command loop: `peek` / `poke` / `dump` / `load` / `run` / `help` over
  the modelled UART

Pointers are in place now, so a driver can be handed a caller's buffer.

**Complexity**: Medium — mostly Wraith code rather than compiler work.

---

## 🟡 HIGH PRIORITY

### 2. Standard library gaps — ✅ closed

The previously-missing pieces now ship:
- `abs`/`abs16` (`std/math.wr`).
- ASCII `char` helpers (`std/char.wr`): `is_digit`/`is_alpha`/`is_upper`/
  `is_lower`/`is_alnum`/`is_whitespace`, `to_upper`/`to_lower`, `digit_value`.
- `strcmp` (`std/string.wr`) — C-style −1/0/1 ordering (`s.len` and `str == str`
  already covered length and equality).
- `bcd_to_string`, `bcd16_to_string`, `string_to_bcd` (`std/string.wr`) — write
  a length-prefixed `[len][digits]` block into a caller `&u8` buffer / parse
  digits back to BCD, for displaying BCD counters on the console.

With these done, **bitfield access syntax (item 3) is the top remaining
priority** — device registers the driver work touches are almost all bitfields.

---

## 🟢 MEDIUM PRIORITY

### 3. Bitfield access syntax

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

### 4. Branch optimization intelligence

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

### 5. Disassembly output mode

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

### 6. Inline data directive

Lookup tables and sprite data colocated with the code that uses them, rather than
hoisted to a `static`.

```rust
data lookup_table: [u8; 16] = [0x00, 0x01, 0x04, 0x09, /* … */];
```

**Complexity**: Low. Const arrays and string literals already allocate from
`DATA` through `SectionAllocator`; inline data should do the same, so that it
stays visible to `#[org]` conflict detection.

---

### 7. Reclaim BSS from dropped statics

Unreachable statics are no longer emitted, but sema assigns their RAM addresses
before liveness is known, so the space stays reserved. Ordering BSS allocation
after the liveness walk would recover it.

**Complexity**: Low.

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

`examples/monitor/` holds an earlier monitor sketch. Neither
`simple_monitor.wr` nor `monitor_standalone.wr` compiles, and the set should be
treated as reference material to be rewritten as part of item (1) rather than
as working examples. `examples/pointers.wr` shows the buffer-passing shape they
will need.

---

## Recently Completed ✅

**OS enablement (2026)**
- Pointers and address-of: `&T`, `&x`, `*p`, `p[i]`, `p.field`, casts to and
  from `u16`, pointer-valued statics, and an escape analysis that rejects a
  pointer outliving the frame it names. `std/mem.wr` takes `&u8`, so a driver
  can be handed a caller's buffer (`examples/pointers.wr`)
- Local array data moved from the `CODE` section into RAM — writing to a local
  array was a silent no-op on a real board, invisible under the emulator's flat
  memory
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
- Two-phase function placement: sizes are measured, `#[org]` ranges reserved,
  and everything else allocated into the gaps, so pinned and auto-allocated
  functions can coexist
- Dead code elimination across the whole program, the file being compiled
  included, with warnings naming exactly what is dropped
- Const arrays allocate from the configured `DATA` section instead of a
  hardcoded `$C000` outside the memory map
- Spans carry a file id, so two modules can no longer collide on one map key
- `#[org]` collisions are compile errors, against functions, data, statics and
  the interrupt vector table, with source excerpts
- BCD `SED`/`CLD` peephole consolidation
- Inline-asm `{param}` substitution inside `#[inline]` functions (the whole
  `min`/`max`/`clamp` section of `std/math.wr` was uncallable)

**Tooling**
- `--out <dir>` and shell completion for bash, zsh and fish

See the [Language Specification](specification.md) for full documentation of
everything implemented.
