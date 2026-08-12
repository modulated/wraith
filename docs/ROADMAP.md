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

`tests/fuzz_exec.rs` generates random programs, runs them on the emulator, and
checks the answers two independent ways: against an **oracle** (the generator
builds the program as a tree, so running it in Rust is exact) and against
**itself** in four surface forms — inline, inside a called function, inside a
`match` arm, inside a single-iteration loop — which must all agree. The second
catches what the first cannot: a misunderstanding the generator and the
compiler share, where one form still diverges.

What it generates is a small imperative language: `u8`/`u16`/`i8`/`i16`, ten
binary operators, casts, comparisons and boolean connectives, assignment,
`if`/`else`, counted `for` and condition-driven `while`, nested. Every variable's
final value is written out, so one program checks four results.

**What it reaches is documented, not asserted**: [`fuzz-coverage.md`](fuzz-coverage.md)
lists every construct in the language's AST with how many of a fixed sample of
programs contain it, the limits that apply where it is generated, and the reason
where it is not. That document is generated from the fuzzer and checked on every
run — and the variant lists are read from `src/ast/*.rs`, so a construct added to
the language appears there as uncovered and has to be given a reason before the
suite passes again. Read it before trusting any claim below about coverage.

It is deterministic and seeded per iteration, so a failure reports a seed that
reproduces it and CI sees the same programs every run. `WRAITH_FUZZ_ITERS` and
`WRAITH_FUZZ_SEED` widen the search locally. A failing program is **shrunk**
before it is reported — one-step simplifications, re-run each time, keeping the
same *kind* of failure — so what lands in the output is a handful of lines
rather than thirty of dense arithmetic.

Six real bugs so far, all silent:

1. Constant folding evaluated in `i64` and truncated once, while generated code
   wraps at every step: `(94 << 6) >> 3` folded to 240 where the same expression
   through a variable computed 16.
2. The same gap in the two places the width rule could not reach — a folded
   *comparison* (its type is `bool`, so it has no width of its own) and a folded
   *cast* (its type is the target, which hides the operand's width).
3. Widening casts extended by the destination's signedness rather than the
   source's, so `200u8 as i16` was −56 and `-1i8 as u16` was 255.
4. Short-circuit `&&` left its exit "with A already zero" — true as emitted,
   false once the peephole collapsed the left comparison into a bare branch.
   Fixed on both sides: the codegen loads the zero, and the peephole no longer
   collapses a boolean whose value is still live.
5. Two bare integer literals in one operator each fell back to their own default
   (`-5` to `i8`, `3` to `u8`) and the operator then rejected the pair, so
   `if (-5 - 3) < n` did not compile.
6. A conditional branch spanning the right operand of `&&` overflowed its
   ±127-byte range once that operand was large enough — an assembly-time failure
   with no source-level fix.

Regression tests: `tests/e2e/const_folding.rs`, `tests/e2e/int_conversions.rs`,
`tests/e2e/short_circuit.rs`.

**To widen**, in rough order of value — each needs the oracle extended to match,
and an oracle that is merely *probably* right is worse than no oracle:

- **Division by zero.** Currently sidestepped with nonzero positive literal
  divisors (positive also keeps `i8::MIN / -1` out). Pinning the behaviour first
  would let the generator use arbitrary divisors.
- **Precedence.** Expressions are fully parenthesised so a precedence
  disagreement cannot masquerade as a codegen bug; a separate generator that
  omits parentheses and compares against a parenthesised twin would test it
  metamorphically.
- **Mixed widths.** One type per program today, because mixed-width arithmetic
  brings the implicit widening rules into the oracle.
- **Calls with several arguments**, and recursion — the shapes where the spill
  and frame machinery live. The four surface forms exercise one call each; none
  passes an argument.
- **Aggregates.** Structs, arrays and slices, where several bugs have already
  been found by hand.
- **Shift counts at or past the width**, and constant expressions standing alone
  (typed by their own literals, not by the program around them — see the
  specification). Both are defined; neither is in the oracle.

### Code-size benchmark

`tests/code_size.rs` compiles every `examples/*.wr` and checks the bytes emitted
per section against `tests/code_size_baseline.txt`. A change either way fails,
with a per-program and overall delta; `WRAITH_BLESS_SIZES=1 cargo test --test
code_size` re-blesses, so an optimization's win shows up in the diff rather than
in a claim. It runs under plain `cargo test`, so CI already guards it.

Still to add: **cycle counts**, which need the instruction timing table the
disassembly item above would build. Size is the cheaper half of the measurement
and does not, on its own, say whether a change made a program *faster*.

### Sequencing

These items are not independent, and the natural order is not one-per-category:

1. ~~Execution-checked fuzzing first~~ — done for all four integer types,
   expressions and control flow; widen it further (see above) as the
   optimization work approaches.
2. **Then the known correctness bugs**, which are small and specific.
3. **Then the usability gaps** (array fields in structs is the largest).
4. ~~Then the size benchmark~~ — done; extend it with cycle counts when the
   timing table exists.
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
