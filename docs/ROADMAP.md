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

`examples/monitor_standalone.wr` is an earlier sketch. It compiles, but predates
pointers and the device models, so treat it as reference material to be rewritten
rather than as the shape to build on.

---

## Language features

### Bit-range slice

`flags.bits[7:4]` — extract and insert a contiguous field. Single-bit access is
complete; this is the multi-bit generalization.

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

Two instructions in the WDC set have no codegen site yet:

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
  the registration pass (`register.rs` `bss_alloc`) assigns every mutable
  static a RAM address before liveness (`reachable_symbols`) is known, so a
  dropped static still reserves its bytes. A naive "allocate after the liveness
  walk" reorder does not work directly: initializer `&OTHER_STATIC` references
  and the flattened init bytes are resolved at registration time, in
  declaration order, from the already-assigned addresses. Recovering the space
  means splitting static registration into phases — declare symbols and collect
  refs, run liveness, then assign BSS addresses to the live statics (still in
  declaration order) and flatten their init bytes — and keeping the shared
  `bss_cursor` consistent before `finalize_frames` lays local-array blocks
  above it.
- **Automatic inlining of small functions.** `#[inline]` is explicit only; a leaf
  function smaller than its call sequence is always worth inlining, and the size
  is already measured in the first pass of `generate_function`.

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

Multi-error reporting covers declarations, bodies, statements and independent
subexpressions; two boundaries remain uncrossed because doing so safely needs
work the rest did not:

- **Recover from a failed declaration into the bodies.** Today a declaration that
  fails to register stops the walk, because its symbol is then missing and every
  use would report a bogus "cannot find" on top of the real error. Crossing this
  needs per-name suppression: remember which names failed to register and
  silence exactly their `UndefinedSymbol`s in the body pass.
- **Report several errors from one imported module.** `SemaError::InModule`
  carries a diagnostic already rendered against the child's source (its spans
  index a file the driver never read), so N child errors need N renders, each
  with its own `trail` clone, merged under the existing `is_replay` guard in
  `merge_imported` — or a diamond import reports the same module twice.

Also open:

- **Widen the spec-example harness.** Only the opt-in ` ```rust,compile ` blocks
  are compiled (`tests/e2e/spec_examples.rs`); the plain ` ```rust ` fragments
  reference peripherals defined elsewhere in the prose. Making more of them
  self-contained and tagging them widens the net.
- **More error-message golden tests** as new diagnostics land, in the
  exact-position style of `tests/e2e/error_diagnostics.rs`.

---

## Structure & maintainability

- **Shared exhaustive Stmt/Expr walker.** The dominant historical defect was
  hand-enumerated per-form walkers and merge lists that missed new variants. A
  single recursion helper shared by all analysis walkers would close it; the
  import-merge half is already done.
- **De-duplicate codegen.** Six near-identical unsigned compare routines; the
  ZeroPage/Absolute arms of `generate_index_assignment`; `generate_divide_i16` /
  `generate_modulo_i16`. The statement codegen is now split into a `stmt/` module (`assign`,
  `loops`, `match_stmt`, `asm_stmt`); the remaining win there is a shared
  `store_value_to_slot(ty, loc)` helper for the repeated byte-store sequences in
  `assign.rs`.
- **Turn string-matching e2e pockets into behavioral assertions** where behavior
  is assertable (`cpu_flags.rs`, and parts of `frames.rs` / `types.rs` /
  `control_flow.rs` / `memory.rs`).

---

## Polish

- **Match-arm binding slots.** Each arm allocates fresh frame slots; siblings could
  share (cf. `loop_bound_free`).

---

## Future / larger

- 65816 target support (16-bit mode).
- Optimization-level flags (`-O0` / `-O1` / `-O2`, and `-Os` for size).
