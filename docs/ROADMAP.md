# Wraith Compiler — Development Roadmap

Open work only. For what the language already does, see the
[Language Specification](specification.md); for building and running the
compiler, see the [README](../README.md).

The near-term goal driving most of the language work is a small operating system
for a 6502 homebrew machine: UART serial, keyboard input, a monochrome text
display, and a device protocol over a 6522 VIA.

---

## Blocking the OS

### Console and keyboard drivers

The device models and execution harness already exist (`tests/common/devices.rs`
— a TL16C550 UART and a 6522 VIA with real side effects), so this is Wraith code,
not compiler work. Pointers are in place, so a driver can be handed a caller's
buffer (`examples/pointers.wr`).

- **Keyboard** — scancode in via a VIA port with a CA1 strobe raising IRQ,
  decoded to ASCII, queued in a `static` ring buffer.
- **Text console** — memory-mapped framebuffer, cursor, line wrap, scroll (reuse
  `memcpy16` from `std/mem.wr`), clear-screen.
- **Monitor command loop** — `peek` / `poke` / `dump` / `load` / `run` / `help`
  over the modelled UART.

`examples/monitor/` holds an earlier sketch; neither `simple_monitor.wr` nor
`monitor_standalone.wr` compiles, and the set should be treated as reference
material to be rewritten here rather than as working examples.

---

## Language features

### Bitfield access — follow-ups

Single-bit access ships (`x.bit(n)`, `x.set_bit(n)`, `x.clear_bit(n)`,
`x.toggle_bit(n)`, constant bit index; lowered to `SMB`/`RMB` on the 65C02, an
`ORA`/`AND` read-modify-write otherwise), including on struct-field and
constant-index array-element targets that resolve to a fixed address
(`DEV.ctrl.set_bit(3)`, `t[1].flags.clear_bit(0)`). Remaining:

- **Bit-range slice** `flags.bits[7:4]` — extract/insert a contiguous field.
- **`BBR`/`BBS` fusion** — fold `if x.bit(n)` into a single bit-test-branch on the
  65C02. The assembler already recognizes the mnemonics; codegen does not emit
  them yet.
- **Bit mutation through a pointer or a runtime index** — `p.field.set_bit(n)`
  and `arr[i].flags.set_bit(n)` are rejected with a clear error. They need an
  indirect (`(zp),Y`) read-modify-write, where `SMB`/`RMB` do not apply.

### Const attributes (consider later)

`const` arrays always land in the `DATA` section. There is no known need to
change that today, but if one arises — say, keeping a small lookup table in the
same ROM region as the function that uses it under a bank-switched map — the
right shape is a placement attribute on `const`, reusing the `#[section("…")]`
mechanism functions already have:

```rust
#[section("CODE")]
const lookup_table: [u8; 16] = [0x00, 0x01, 0x04, 0x09, /* … */];
```

(This replaces an earlier "inline `data` directive" idea, which was a new keyword
for the same effect. On a 6502 there is no cache, so placement buys no locality —
only memory-map control — and an attribute delivers that without a new concept.
Not worth building until a concrete need appears.)

---

## Code generation & optimization

Nothing in this section is required for correctness — every item produces smaller
or faster code for programs that already compile.

### 65C02 instruction selection

The `--cpu 65c02` (default) / `--cpu 6502` switch and the `TargetCpu` plumbing
exist, and the assembler encodes the full WDC 65C02 base set. The CMOS path
already emits: the Rockwell `SMB`/`RMB` (single-bit set/clear); `STZ` for
reset-time zeroing; `PHX`/`PHY`/`PLX`/`PLY` in the interrupt prologue/epilogue;
accumulator `INC A`/`DEC A` (a flag-liveness-guarded fold of `CLC; ADC #$01` /
`SEC; SBC #$01`); and `JMP (abs,X)` for match jump-table dispatch (no zero-page
vector). Still to do:

- **`BRA rel`** — unconditional relative branch: 2 bytes against 3 for `JMP`
  within −128..+127. Awkward because the length depends on range, which is not
  known until placement, so it interacts with function-size measurement rather
  than being a straight substitution.
- **`TSB`/`TRB addr`** — test-and-set/reset bits without a mask in A. No obvious
  codegen site yet; would pair with a "test then set" idiom.

### Branch optimization intelligence

Status flags are discarded after every comparison, so a repeated test re-emits the
`CMP`:

```rust
if x > 5 { foo(); }
if x > 5 { bar(); }   // the second CMP is redundant if x is unchanged
```

High complexity, with a demonstrated correctness risk: the register-state tracker
that would underpin this has already produced several silent miscompiles by
outliving a label. Any flag tracking must invalidate at labels and calls from the
outset (`Emitter::emit_label` already does this for registers).

### Smaller code

- **Reclaim BSS from dropped statics.** Dead statics are no longer emitted, but
  sema assigns their RAM addresses before liveness is known, so the space stays
  reserved. Ordering BSS allocation after the liveness walk recovers it.
- **Consolidate duplicate enum variant data.** Two constructions of the same
  variant with the same payload emit the same bytes twice.
- **Move const enum payloads out of the instruction stream.** They are emitted
  inline with a `JMP` over them; placing them in `DATA` removes the jump.
- **Automatic inlining of small functions.** `#[inline]` is explicit only; a leaf
  function smaller than its call sequence is always worth inlining, and the size
  is already measured in the first pass of `generate_function`.
- **stdlib math ROM over-allocation** (~49 bytes across `mul16`/`div16`/`mod16`);
  **copy loops unrolled per byte** (6 code bytes per byte copied — a `DEX/BNE`
  loop pays off beyond ~3 bytes); **`LDA #$00; LDY #$00` → `LDA #$00; TAY`**.

---

## Tooling

### Disassembly output mode

An annotated listing with resolved addresses and cycle counts, for performance
work and for reading what the peephole actually did:

```
9000: A9 00     LDA #$00        ; [2 cycles] load zero into A
9002: 85 40     STA $40         ; [3 cycles] store to x
```

The emulator harness already assembles Wraith output to a byte image
(`tests/common/exec.rs`), so address resolution is largely solved; the cycle
table is new.

---

## Correctness & diagnostics

- **Sema-level multi-error reporting.** The parser recovers and reports multiple
  errors; sema still stops at the first.
- **Interrupt hardware-stack depth check.** Frame coloring computes what a handler
  saves, but nothing checks that a handler's own call depth fits the hardware
  stack.
- **Compile the spec's examples as tests.** A harness that extracts the ` ```rust `
  blocks from `specification.md` and asserts they compile would permanently catch
  the class of spec examples that don't.
- **Error-message golden tests** for the top ~20 diagnostics, in the exact-position
  style of `tests/e2e/import_diagnostics.rs`.

---

## Structure & maintainability

- **Shared exhaustive Stmt/Expr walker.** The dominant historical defect was
  hand-enumerated per-form walkers and merge lists that missed new variants. A
  single recursion helper shared by all analysis walkers would close it; the
  import-merge half is already done.
- **De-duplicate codegen.** Six near-identical unsigned compare routines; the
  ZeroPage/Absolute arms of `generate_index_assignment`; `generate_divide_i16` /
  `generate_modulo_i16`; the `emit_signed_lt` closure copied verbatim into two
  files. `stmt.rs` (~3,950 lines) wants a `store_value_to_slot(ty, loc)` helper.
- **Turn string-matching e2e pockets into behavioral assertions** where behavior
  is assertable (`cpu_flags.rs`, and parts of `frames.rs` / `types.rs` /
  `control_flow.rs` / `memory.rs`).

---

## Polish

- **Dead code.** `address_allocator.rs` (188 lines) is unused; `TempAllocator::reset`
  is never called (and the "reset at function boundaries" comment at
  `aggregate.rs` is false — runtime enum construction leaks pool bytes
  program-wide); `is_primary_free`, `TempAllocStats`,
  `ParseErrorKind::InvalidInteger` / `InvalidType` are dead.
- **u8 div/mod by zero** loads a never-initialized zero-page byte while the comment
  says "leave A as-is".
- **`-x as i16` precedence** parses as `(-x) as i16`; Rust/C parse `-(x as i16)`.
  Align or document.
- **Small diagnostic/CLI nits.** The string-limit error says 256 but the limit is
  255; `-true` type-checks as bool; `1_000` lexes as `1` + `_000`; `-v` is
  `--version`; `--help` writes to stderr.
- **Match-arm binding slots.** Each arm allocates fresh frame slots; siblings could
  share (cf. `loop_bound_free`).
- **Test isolation.** `tests/visibility_errors.rs` writes fixed filenames into the
  shared temp dir, risking parallel-run flakiness.
- **Repo hygiene.** A committed `.DS_Store` and an empty `fuzz/pGNi` AFL artifact
  can go; the spec's revision history is never updated.

---

## Future / larger

- 65816 target support (16-bit mode).
- Optimization-level flags (`-O0` / `-O1` / `-O2`, and `-Os` for size).
