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
(`DEV.ctrl.set_bit(3)`, `t[1].flags.clear_bit(0)`), and through a pointer
(`p.field.set_bit(n)`) or a runtime index (`t[i].flags.set_bit(n)`), which
desugar to an indirect `object = object <op> mask` read-modify-write.
`if x.bit(n)` / `if !x.bit(n)` on a zero-page byte fold into a single
`BBSn`/`BBRn` bit-test-branch on the 65C02. Remaining:

- **Bit-range slice** `flags.bits[7:4]` — extract/insert a contiguous field.

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
  errors; sema still stops at the first. This is a structural change with a real
  soundness risk — see the plan below before starting.
- **Interrupt hardware-stack depth check.** Frame coloring computes what a handler
  saves, but nothing checks that a handler's own call depth fits the hardware
  stack.
- **Compile the spec's examples as tests.** Done for the opt-in
  ` ```rust,compile ` blocks (`tests/e2e/spec_examples.rs`); the remaining plain
  ` ```rust ` fragments reference peripherals/functions defined elsewhere and are
  not self-contained. Tagging more of them (after making them self-contained)
  widens the net.
- **Error-message golden tests** — done for 15 of the top diagnostics
  (`tests/e2e/error_diagnostics.rs`), each pinning the `--> line:col` position, a
  message keyword, and the shared no-`Debug`-leak/has-a-caret invariant. Add more
  as new diagnostics land.

### Plan: sema-level multi-error reporting

Today every check in `src/sema/analyze/` returns `Result<_, SemaError>` and the
driver (`analyze_module` → `analyze_item` → `analyze_stmt`/`check_expr`)
`?`-propagates the first failure. The goal is to collect and report *all*
independent errors in one pass, as the parser already does.

**The load-bearing risk.** The historical defect class here is *state that
outlived its validity* — a register belief past a label, a merge list missing a
variant — producing silent miscompiles. Continuing analysis past an error means
deliberately running checks over partially-invalid state. So the invariant is:
**recover only at boundaries where the remaining work is independent of what
failed, and never fabricate a plausible-but-wrong type that later code trusts.**

**Recovery boundaries (where to catch, not `?`):**

- **Item** — a bad `fn`/`struct`/`static`/`const` is recorded; the next item is
  still analyzed. `analyze_item` and `register_item` become the coarse
  catch points, iterating all of `source.items` instead of stopping.
- **Statement** — within a function body, a failed `analyze_stmt` is recorded and
  the next statement is analyzed. Sibling statements are independent; a broken
  `let` should not hide a type error three lines down.
- **Never mid-expression.** A subexpression whose type could not be determined
  must *not* be guessed. Introduce a `Type::Error` (or `Type::Unknown`) sentinel:
  a failed `check_expr` records the error, returns `Type::Error`, and every
  operator/assignment/call check treats `Type::Error` on either side as
  "already reported — produce `Type::Error`, emit nothing." This is what stops
  one real error from cascading into ten bogus ones, and is the piece most likely
  to be wrong if rushed.

**Mechanism:**

1. Add `errors: Vec<SemaError>` to the analyzer, mirroring the existing
   `warnings: Vec<Warning>` field and its `.clone()` into `ProgramInfo`. Add a
   `record(&mut self, e: SemaError)` helper (dedupe by span+message; cap at, say,
   50 to bound pathological input).
2. Add `Type::Error`. Make it compatible with everything in the compatibility
   checks (so it never produces a *second* error) and make `type_size`/codegen
   never see it — analysis must abort before codegen if `!errors.is_empty()`.
3. Convert the two driver loops (`analyze_module`'s register and analyze passes)
   to record-and-continue. Convert `analyze_stmt`'s block walk likewise.
4. Change the public entry: `analyze` returns `Result<ProgramInfo, Vec<SemaError>>`
   (or keeps `ProgramInfo` carrying `errors`, with the CLI/tests checking it).
   The CLI renders each with `format_with_source_and_file` and exits non-zero if
   any; a partial `ProgramInfo` is **never** handed to codegen.

**Phasing (each independently green):**

- *Phase 1* — plumbing only: add `errors`, `record`, `Type::Error`, and the
  abort-before-codegen gate. Keep `?` everywhere; behavior is unchanged (still
  one error), but the machinery exists and is tested.
- *Phase 2* — item-level recovery: the two driver loops record-and-continue.
  Two unrelated broken items now both report. Highest value, lowest risk.
- *Phase 3* — statement-level recovery inside a body, guarded by `Type::Error`
  poisoning so a bad expression doesn't cascade.
- *Phase 4* — widen `check_expr`'s internal `?`s to record-and-poison where a
  subexpression failure is genuinely independent (e.g. each call argument).

**Testing:** a `tests/e2e/multi_error.rs` asserting that a program with N
independent errors reports N (not 1), that a single bad expression reports
*once* (no cascade — the key anti-goal), and that a file with any error produces
no `.asm`. The existing single-error golden tests
(`tests/e2e/error_diagnostics.rs`) must stay green throughout: first-error
position and message do not change.

**Explicitly out of scope:** partial code generation. If there is any error,
the compile fails with no output. This is a diagnostics-quality change, not an
error-recovery-codegen feature.

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
- **Small diagnostic/CLI nits.** The string-limit error says 256 but the limit is
  255; `-true` type-checks as bool; `-v` is `--version`; `--help` writes to stderr.
  (`-x as i16` already parses as `(-x) as i16`, matching Rust — unary binds
  tighter than `as`; `1_000` already lexes as `1000`. Both locked in
  `tests/e2e/operators.rs`.)
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
