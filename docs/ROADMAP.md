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

## Testing and measurement infrastructure

Two gaps that gate the work below them: nothing currently checks that compiled
programs produce the *right answers* at scale, and nothing measures the size of
what is emitted. Both are prerequisites rather than features.

### Execution-checked fuzzing

`fuzz/fuzz_targets/fuzz_full_stack.rs` drives lex → parse → sema → codegen and
discards the result. It proves the compiler does not *crash*; nothing proves it
does not *lie*. Every miscompile found so far — the u16 match ranges, the
struct-argument pointer high byte, the X clobber across a recursive call, the
recursion depth model — compiled cleanly and returned a wrong answer, so the
existing fuzzer is structurally incapable of finding any of them.

A hand-written battery of ~150 programs with computed expected results found
four. That hit rate is the argument for automating it: generate random
well-typed programs, run them on the emulator harness (`tests/common/exec.rs`),
and compare against what they should produce.

The cheaper form, which needs no reference interpreter, is **metamorphic**:
generate a program and a semantically equivalent variant — `x * 2` against
`x + x`, an `if`/`else` chain against the equivalent `match`, a constant loop
bound against a runtime one holding the same value, a slice bound to a variable
against the same slice expression inline — compile both, and assert they agree.
Any disagreement is a compiler bug without anyone having to say what the right
answer was. The u16 match-range bug would have fallen out of the third of those
immediately.

### Code-size benchmark

There is no `benches/`, and CI runs only test/clippy/fmt plus a fuzz-target
build. So there is no way to tell whether an optimization helped, or whether an
unrelated change made every program bigger. Recording bytes emitted per example
program — and eventually cycle counts, which the disassembly item below would
supply — turns "smaller code" from an assertion into a measurement.

Nothing in *Code generation & optimization* can be evaluated until this exists.

### Sequencing

These items are not independent, and the natural order is not one-per-category:

1. **Execution-checked fuzzing first.** It is what makes the risky optimization
   work safe to attempt.
2. **Then the known correctness bugs**, which are small and specific.
3. **Then the usability gaps** (array fields in structs is the largest).
4. **Then the size benchmark**, before any optimization it would measure.
5. **Branch/flag tracking last.** It is the biggest single efficiency prize and
   the one whose predecessor already produced several silent miscompiles.
   Attempting it before differential testing exists is how the next silent
   miscompile gets written.

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

### A mutable slice type

Slices are read-only views and now borrow from any storage — a local, a
`static`, or a `const` array in ROM. That is deliberate: the descriptor is a
bare address and length, so it cannot know whether its target is writable, and
`s[i] = v` is rejected everywhere rather than being legal or not according to a
declaration somewhere else.

What is missing is the writable counterpart, the analogue of `str<N>` against
`str`. Whatever spelling it takes (`&mut [T]`, or a distinct type), the
constraint is that it must only be constructible from RAM-backed storage —
a local or a `static`, never a `const` — because a store into ROM is a silent
no-op on real hardware and nothing at run time will catch it.

Until then, code that needs to write a sub-range passes the array itself plus
explicit bounds.

Also still open:

- **Returning a slice expression.** `return v[1..4]` is rejected with "Slice
  expressions can only be used as assignment targets"; it has to be bound
  first. A returned slice leaves a *pointer* to its 4-byte descriptor in A:X and
  the caller copies through it, so the descriptor needs somewhere in the
  callee's frame to live — the same per-expression-site reservation
  `struct_temps` does for computed struct literals, keyed by the slice
  expression's span. Argument position needed no such thing because the
  argument staging slot is already a zero-page address the materializer can
  write straight into.
- **`for i in 0..s.len`** fails because `.len` is `u16` and the loop wants a
  `u8` bound, so it needs `s.len as u8` spelled out. The mismatch is reported as
  a bare "mismatched types" that does not mention the cast.

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
