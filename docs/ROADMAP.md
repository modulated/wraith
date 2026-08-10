# Wraith Compiler — Development Roadmap

Open work only. For what the language already does, see the
[Language Specification](specification.md); for building and running the
compiler, see the [README](../README.md).

This is the language and compiler roadmap; it stays agnostic of any particular
program written in Wraith. Application work (device drivers, an OS, monitors)
lives in its own repository.

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

## Known limits found by stress testing

A differential battery (~150 programs with hand-computed results, run on the
emulator) turned these up. The silent-miscompile findings from that run are
fixed and regression-tested in `tests/e2e/match_ranges.rs`; what follows is what
it found and left standing.

### Arrays inside structs

A struct field wider than two bytes is rejected at initialization — "struct
field type with size 4 not yet supported" — so `struct S { a: [u8; 4], b: u8 }`
cannot be constructed. A `[u8; 2]` field is accepted but then `s.a[1]` fails
with "only variable array indexing is currently supported". Nested *struct*
fields of the same width work, because that path recurses field by field; the
array field has no equivalent. Making array fields work means giving them the
same treatment.

### `&const_array[i]` computes the wrong address

An immutable `const` stays at `SymbolLocation::Absolute(0)` — it is ROM data
referenced by label, and sema never learns the label's address. But
`generate_addr_of_element`'s `Absolute(base)` arm computes `base + offset`, so
for a const array that is `0 + offset`: `&A[1]` emits `LDA #$01 / LDX #$00` and
yields the pointer `$0001`, the *index* rather than the address. A store through
it silently scribbles on zero page ($00-$1F, system reserved).

Statics are fine (they have a real BSS address) and locals are fine (the slot
holds a runtime pointer); only the const/ROM case is wrong.

This also slips past the guard that already exists one level up: `A[1] = 9` on
a const is a clean sema error ("a const lives in ROM, so the store would
silently do nothing on real hardware"), but routing the same write through
`&A[1]` is accepted.

The address itself should be emitted label-relative (`#<A+1` / `#>A+1`, the form
the string and struct paths already use) rather than numerically. What to do
about *writes* through such a pointer is a design call: either track ROM
provenance and reject them the way direct const writes are rejected, or accept
that they are silent no-ops on hardware as they are for any ROM store.

### Slices are read-only, and only ever RAM-backed

`s[i]` reads, `for x in s` iterates, but `s[0] = 9` is rejected ("Can only index
arrays, pointers, and string buffers"). That is an unimplemented store path, not
a safety rule.

Mutability is safe to add *today* because a slice's backing store can only ever
be RAM: the source must be a zero-page local (`static` and `const` sources are
both rejected with "slice source array must be a zero-page local"). So there is
no ROM-backed slice to protect against.

That changes the moment slice sources widen, which the spec already promises
they do — "Data: Stored wherever array is allocated (const data, stack, etc.)"
describes a compiler that does not exist yet. When a `&[T]` can name a `const`
array or a string literal, writing through one has to be rejected, and the
language already has the pattern for it: `str` (ROM literal) is read-only while
`str<N>` (RAM buffer) is writable, each with its own diagnostic. Slices want the
same split rather than a fresh mechanism.

Order matters here: widening slice sources before adding the guard would make
ROM-backed slices writable in the window between, which is exactly the silent
no-op the `str`/`str<N>` split exists to prevent.

`for i in 0..s.len` also fails: `.len` is `u16` and the loop wants a `u8` bound,
so it needs `s.len as u8` spelled out. The mismatch is reported as a bare
"mismatched types" that does not mention the cast.

### Exclusive ranges in match patterns

`match n { 0..300 => … }` is a parse error ("expected FatArrow, found '..'");
only `..=` is accepted in a pattern. Ranges elsewhere (`for i in 0..n`) take
both forms, so the restriction is surprising.

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

- **Widen the spec-example harness further.** 82 of the spec's ~210 code blocks
  are now compiled on every run (`tests/e2e/spec_examples.rs`), via
  ` ```rust,compile ` for whole programs and ` ```rust,compile,fragment ` for
  statement runs wrapped in a generated `main`. Most of what is still untagged
  is untaggable by design — deliberate error examples, and the stdlib reference
  section's bodyless signatures. The remainder needs a peripheral or helper
  supplied to become self-contained, one block at a time.
- **More error-message golden tests** as new diagnostics land, in the
  exact-position style of `tests/e2e/error_diagnostics.rs` (33 cases).
  `TypeMismatch`, `InvalidUnaryOp`, `BreakOutsideLoop`, `EscapingPointer`,
  `InvalidAddrUsage`, `InstructionConflict`, `DuplicateSymbol` and
  `FrameRegionOverflow` are pinned; `ReturnTypeMismatch`, `OutOfZeroPage` and
  the import diagnostics are not.

---

## Structure & maintainability

- **Shared exhaustive Stmt/Expr walker.** The dominant historical defect was
  hand-enumerated per-form walkers and merge lists that missed new variants. A
  single recursion helper shared by all analysis walkers would close it; the
  import-merge half is already done.
- **Turn string-matching e2e pockets into behavioral assertions** where behavior
  is assertable. `cpu_flags.rs` and `frames.rs` are converted; `memory.rs`,
  `types.rs` and `control_flow.rs` were already behavioral or are asserting
  genuinely assembly-level properties (emitted data layout, the symbolic
  `PORT = $6000` form). What remains is the same judgement applied to the rest
  of `tests/e2e/`, keeping the assembly assertion wherever execution cannot see
  the property — that the spill avoids the hardware stack, that a given number
  of calls was emitted.

---

## Future / larger

- 65816 target support (16-bit mode).
- Optimization-level flags (`-O0` / `-O1` / `-O2`, and `-Os` for size).
