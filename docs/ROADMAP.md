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

*Partly done, and the entry as originally written pointed at the wrong
redundancy.* It proposed tracking status flags across statements so a repeated
`if x > 5` would not re-emit the `CMP`. That case barely occurs: an `if` ends
in a label and usually contains a call, and both invalidate flags, so a tracker
would find almost nothing to elide.

The waste was elsewhere and larger. A comparison feeding an `if` or `while`
built a 0/1 in `A` and then compared *that* against zero to branch — eleven
instructions to decide one branch. `collapse_boolean_compares` in the peephole
already fused the common case, but it matched a *one-branch* comparison tail,
and only four of the six comparisons end in one. Unsigned `<=` and `>` have no
single 6502 branch (`A > m` is `!Z && C`), so they ended in two, missed the
window, and materialised the boolean. Both shapes are now matched — they differ
from each other as well: `>` sends its branches to different labels, `<=` sends
both to the same one. A comparison-heavy program drops from 89 instructions to
74.

**The guard was refusing for a property the rewrite cannot affect.** Everything
the collapse deletes — a `CMP #$00`, two `LDA`s and a `JMP` — writes N, Z and C
and never V, so V holds the same bit before and after. Requiring it dead anyway
refused 48 of the 225 candidate sites in the example corpus. Excluding V from
the guard removes **115 instructions** across the corpus, 60 of them from
`monitor_standalone`.

Finding that took an instrumented run reporting every guard's value per site,
which is what should have been done first: two earlier guesses (that `RTS` was
being treated as reading every flag; that the shapes simply did not match) were
both measured, both wrong, and one of those measurements was itself wrong —
see the benchmark note below. The remaining refusals are now known rather than
guessed at: 30 sites with A genuinely live, and 44 whose labels have other
entrants.

**The benchmark could not see any of this.** `tests/code_size.rs` measured the
section allocator's reservations, which are made at *placement* — before the
peephole runs. Every "code size unchanged" it reported for a peephole change
was measuring something that could not move, including the ones in this
branch's earlier commits. It now records the emitted instruction count
alongside the section bytes; the sections still matter, since they are what has
to fit the memory map, but they are not the whole picture.

**Where the ceiling is.** Fusing in the peephole means recognising a shape
codegen just finished emitting. Emitting the branch directly — a
`generate_condition_branch` path threaded through comparisons, `&&`, `||` and
`!` — would need no pattern matching, would cover the compound conditions the
peephole cannot see through, and is the version worth building if this area is
returned to.

### Smaller code

- **Reclaim BSS from dropped statics.** *Done.* Registration hands out BSS
  addresses in declaration order, long before liveness is known — an
  initializer's `&OTHER` has to resolve to a number as it is flattened — so a
  dropped static used to keep its bytes. Rather than defer allocation, the
  layout is repacked between `reachable_symbols` and `finalize_frames`: live
  statics keep their relative order and the gaps close. Sizes come from the
  gaps between consecutive addresses, so nothing re-derives a type's width.

  The lesson worth keeping is where an address lives by that point. There are
  *three* copies — the symbol table, the per-use snapshots in
  `resolved_symbols`, and `inline_param_symbols`, which despite its name holds
  every symbol a function's body resolved and is merged back over
  `resolved_symbols` at each inline call site. Moving a static in fewer than
  all three silently puts it back. `rewrite_frame_offsets` already had to keep
  the same three in step; the fuzzer caught both halves of getting it wrong.

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
`if`/`else`, counted `for` and condition-driven `while`, functions with one to
three parameters and a return value, self-recursion bounded by a decreasing
budget, and a local array, a `const` table and a two-field struct — all nested.
Every cell's final value is written out, so one program checks up to ten
results.

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

Twelve real bugs so far, most of them silent:

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
7. A call in another call's argument list clobbered the arguments already
   staged, three separate ways: frame colouring overlaid two functions that are
   siblings in the call graph but live at once during staging; the staging pool
   sits at a fixed address whose allocator resets per function, so a callee
   staged over its caller; and an inlined call stores straight into parameter
   slots, which for a self-nested call are the same bytes by construction.
   `f(0, v, f(12, v, v))` returned 12.
8. Loop unrolling was decided by the iteration count alone, so eight copies of
   a 16-bit division were emitted as readily as eight copies of two
   instructions — and nested loops multiplied it. A 76-line generated program
   overflowed the whole 16 KB CODE section, a build failure with no visible
   cause in the source. Unrolling is now bounded by the estimated size of the
   unrolled body; the example corpus is unchanged, so nothing that was worth
   unrolling stopped being unrolled.

9. A `const` array with a negative element was rejected as "not a compile-time
   constant" — but only once something read it, since an unused `const` is
   dropped before flattening. The identical `static` worked. Two
   `InitContext::integer` implementations had drifted: sema's evaluates the
   expression, codegen's pattern-matched a bare literal, and `-5` parses as
   `Unary(Neg, 5)`. Codegen now evaluates too, with the constant environment, so
   `[N - 1, 0]` resolves as well.
10. Repacking BSS to reclaim a dropped static's bytes moved a live one, and
    two of the three places an address lives were left behind — the per-use
    snapshots in `resolved_symbols`, and then `inline_param_symbols`, which is
    re-merged over those at every inline call site. Both showed up as a
    function pointer read out of the hole the dropped static left, on the very
    first run after the change. Caught before the change was ever committed,
    which is the case for having the fuzzer at all.

Regression tests: `tests/e2e/const_folding.rs`, `tests/e2e/consts.rs`, `tests/e2e/int_conversions.rs`,
`tests/e2e/short_circuit.rs`, `tests/e2e/nested_calls.rs`,
`tests/e2e/loop_sweep.rs`, `tests/e2e/bss_reclaim.rs`.

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
- **Mutual recursion, and calls through a function pointer.** Self-recursion is
  generated; a cycle of two functions is not, and neither is the indirect-call
  trampoline. The gap has already cost something: an indirect call contributes
  no call-graph edge, so frame colouring laid a driver's frame over its
  caller's locals, and `examples/device_drivers.wr` found it by hand. A
  generator that installs one of several same-signature functions in a vtable
  and dispatches through it would have found it first — the oracle only needs
  to know which function it installed. The area has since grown: an indirect
  call now stages pointer, string, enum and struct arguments as well as
  scalars, and the escape rule that guarded it was re-derived rather than
  merely relaxed. All of it is covered by hand-written tests and none of it by
  the generator.
- **Slices, pointers and enums.** Arrays and structs of scalars are generated;
  a slice or a `&T` gives two names for one piece of storage, which the oracle
  would have to alias-model, and enums are a separate lowering again.
- **Aggregates across a call.** A struct is a local today — never passed,
  returned, or pointed at — and an array field inside a struct is not
  generated. Both are shapes where a miscompile has already been found by hand.
- **Shift counts at or past the width**, and constant expressions standing alone
  (typed by their own literals, not by the program around them — see the
  specification). Both are defined; neither is in the oracle.

### Code-size benchmark

`tests/code_size.rs` compiles every `examples/*.wr` and checks two numbers per
program against `tests/code_size_baseline.txt`: bytes reserved per section, and
instructions in the emitted assembly. The second was added after the first
turned out to be blind to the peephole — section space is reserved at
placement, before those passes run, so a change that deletes instructions moved
nothing there. A change in either direction fails, with a per-program and
overall delta; `WRAITH_BLESS_SIZES=1 cargo test --test
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

### Argument staging holds one argument per nesting level

*Mostly lifted.* Arguments used to be evaluated into a fixed 11-byte zero-page
pool (`$F4-$FE`) as one contiguous block, so a call nested in another call's
argument list needed room for both lists at once and four 16-bit arguments
inside four more was a compile error.

A call whose whole list fits still stages there — it is the cheaper path, `LDA
temp; STA param` per byte, and nothing that used to fit changed by a byte. When
the block does not fit, each argument now moves to the software stack as soon
as it is evaluated, so the pool holds only that call's *widest single
argument* and the depth is bounded by the stack's 256 bytes. Because the frame
save shares that stack, a recursive callee's save happens before the arguments
go on rather than after, or it would bury them.

What is left is that a nesting level still costs its widest argument, so around
five levels of 16-bit nesting exhausts the pool. Removing even that means
pushing each argument straight from the registers it is produced in, without a
zero-page slot in between — which needs the per-argument staging in
`generate_call` restructured so the push has one place to happen, rather than
being reached through a dozen `continue`s.

The failure is still a compile error rather than a miscompile, and the fuzzer
budgets one argument per level to match, skipping and counting anything that
overruns anyway.

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

- **Shared exhaustive Stmt/Expr walker.** Hand-enumerated per-form walkers and
  merge lists that missed new variants were the dominant defect for a long
  time. A single recursion helper shared by all analysis walkers would close
  it; the import-merge half is already done, and `contains_call` /
  `expr_contains_call` are exhaustive.
- **Make codegen's per-form dispatch exhaustive, or make its fallback loud.**
  The same defect, one layer down, and now the more common one. Where a walker
  asks "is there a call in here", these matches ask "which strategy does this
  form need" — and their fallback is not `false`, it is *another strategy*,
  usually the scalar one. A form nobody enumerated does not fail; it gets
  loaded as though it were a `u8`.

  Six bugs in a row had this shape. Struct copy handled a literal and a call
  and fell through to the scalar path for every *place*, so `let q: P = PS[1]`
  bound one byte. Struct arguments matched a zero-page local and nothing else.
  `.low`/`.high` had no assignment arm. Indirect calls took scalars only.
  Frame colouring had no edge for an indirect call. Assignment evaluated its
  value generically *and* again per target, so `arr[i] = f()` called `f` twice.
  Each compiled silently and produced a wrong answer.

  Two shapes to attack. Resolvers returning `Option`/`bool` (
  `resolve_static_addr`, `yields_struct_pointer`, `emit_struct_place_address`)
  whose `None` a caller reads as "not applicable, use the ordinary path" —
  those want callers that distinguish "not this shape" from "unhandled", and
  error on the second. And genuine `_ =>` arms in strategy matches, which want
  the treatment `contains_call` got: enumerate every variant so a new one is a
  compile error. `aggregate.rs` alone has 24 catch-alls, 9 of which error.
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
