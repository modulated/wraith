# Wraith Compiler — Development Roadmap

Open work only. For what the language already does, see the
[Language Specification](specification.md); for building and running the
compiler, see the [README](../README.md).

This is the language and compiler roadmap; it stays agnostic of any particular
program written in Wraith. Application work (device drivers, an OS, monitors)
lives in its own repository.

## Where this stands

Recently closed, each with its regression tests, so the open items below are
read against a current picture:

| Landed | Where it is pinned |
|---|---|
| Whole-struct copy from any place with an address — binding, assignment, and as an argument | `tests/e2e/struct_copy.rs` |
| An assignment evaluates its value once (`arr[i] = f()` called `f` twice) | `tests/e2e/assign_side_effects.rs` |
| `.low`/`.high` assignable, with a sema check behind them | `tests/e2e/word_halves.rs` |
| Pointers, strings, enums and structs across an indirect call; the escape rule re-derived | `tests/e2e/indirect_args.rs` |
| Frame colouring given an edge for indirect calls | `tests/e2e/vtable.rs` |
| Arguments spill to the software stack when the pool will not hold them | `tests/e2e/nested_calls.rs` |
| BSS repacked so a dropped `static` gives its bytes back | `tests/e2e/bss_reclaim.rs` |
| Two-branch comparisons (`>`, `<=`) fused into their branch; the V guard dropped | `tests/e2e/branch_fusion.rs` |
| The fuzzer dispatches through function pointers, two ways | `docs/fuzz-coverage.md` |
| The fuzzer binds, copies, passes and returns slices | `docs/fuzz-coverage.md` |
| The fuzzer builds a two-function call cycle, reaching the SCC path | `docs/fuzz-coverage.md` |
| The fuzzer mixes the two widths the language widens between | `docs/fuzz-coverage.md` |
| Implicit widening carries the source's sign at all seven sites | `tests/e2e/int_conversions.rs` |
| Code-size benchmark counts emitted instructions, not just reserved bytes | `tests/code_size.rs` |
| Binding, assignment and argument staging refuse an aggregate they cannot carry | `tests/e2e/aggregate_dispatch.rs` |
| Slice descriptors copy whole; a call through a function pointer returns like any call | `tests/e2e/aggregate_dispatch.rs` |
| The address resolvers error on a place they cannot handle, instead of saying `None` | `tests/e2e/aggregate_dispatch.rs` |
| Every parameter kind survives every call form — direct, inlined, recursive, indirect | `tests/e2e/aggregate_dispatch.rs` |
| Every struct field kind round-trips | `tests/e2e/aggregate_dispatch.rs` |
| Every return kind survives every call form — a negative result, kept | `tests/e2e/aggregate_dispatch.rs` |
| An interrupt arriving *mid-computation* leaves the interrupted work alone | `tests/e2e/interrupts_exec.rs` |
| A function-pointer argument is sized and staged like the two bytes it is | `tests/e2e/vtable.rs` |
| Re-pointing a `str` local writes both bytes, page crossing and all | `tests/e2e/strings_slices.rs` |
| A match expression's arm widens by the arm's sign, not by zero | `tests/e2e/int_conversions.rs` |
| A for-loop bound the counter cannot represent is refused, sign as well as width | `tests/e2e/control_flow.rs` |
| Two-byte `static`s and `static` struct fields store both bytes — a negative result, kept | `tests/e2e/aggregate_dispatch.rs` |
| The four call forms share one argument-staging routine and one width table | `src/codegen/expr/call.rs` |
| The fuzzer passes a struct across a call and makes its value observable | `docs/fuzz-coverage.md` |
| The fuzzer passes an enum across a call, tag and register convention both | `docs/fuzz-coverage.md` |
| The fuzzer passes a `str` across a call — the last of the four staging kinds | `docs/fuzz-coverage.md` |
| Operator precedence checked against the specification's table, metamorphically | `tests/fuzz_exec.rs` |
| Divide-by-zero defined as the all-ones sentinel at every width and sign | `tests/e2e/operators.rs` |
| A divisor the compiler can see is zero is refused | `tests/e2e/error_diagnostics.rs` |
| A shift count at or past the width shifts every bit out, and warns when constant | `tests/e2e/operators.rs` |
| A whole array is never assigned; the refusal names element-wise and `memcpy` | `tests/e2e/aggregate_dispatch.rs` |
| The fuzzer copies a run of bytes with `memcpy`, through `&arr[i]` and `&TBL[i]` | `docs/fuzz-coverage.md` |
| A struct returned by value comes back as its address, not its first byte | `tests/e2e/aggregate_dispatch.rs` |
| A constant struct lays out an array field inline, and folds its fields | `tests/e2e/aggregate_dispatch.rs` |
| `*p` on a pointer-to-pointer keeps its high byte; one indirect load serves both | `tests/e2e/aggregate_dispatch.rs` |
| The fuzzer aliases a variable through a pointer, and moves the alias | `docs/fuzz-coverage.md` |
| The fuzzer returns a struct from a call, binds it, assigns it, and pokes one through `&S` | `docs/fuzz-coverage.md` |
| The shrinker keeps a rejection's *reason*, so a reduction cannot drift to another | `tests/fuzz_exec.rs` |
| An aggregate is reachable through a pointer, a call's result or a run-time index | `tests/e2e/aggregate_dispatch.rs` |
| An enum field of a struct is stored inline, so a `match` on it picks the right arm | `tests/e2e/aggregate_dispatch.rs` |
| A `static` struct initialises every field kind, `str` and enum included | `tests/e2e/aggregate_dispatch.rs` |
| The fuzzer indexes an array field through the by-reference parameter | `docs/fuzz-coverage.md` |
| A failed declaration no longer hides the bodies; only its own name is suppressed | `tests/e2e/multi_error.rs` |
| An unknown type in a declaration is reported where it is written | `tests/e2e/multi_error.rs` |
| A broken module reports every error, once, however many paths reach it | `tests/e2e/import_diagnostics.rs` |
| Every `SemaError` variant is pinned by a golden test or excused with a reason | `tests/e2e/error_diagnostics.rs` |
| 137 of the specification's 230 code blocks compile on every run | `tests/e2e/spec_examples.rs` |
| A table generated from its index, `[\|i\| => i * i]`, folded once and emitted three ways | `tests/e2e/const_tables.rs` |
| An array of structs stored as columns under `#[soa]`, with every whole-element use refused | `tests/e2e/soa.rs` |
| Attributes on `enum`/`import`/`struct` refused rather than dropped; an SoA column at a constant index stored directly | `tests/e2e/soa.rs` |
| The fuzzer indexes a string, points at a struct field and an array element, nests a struct, and matches an enum payload | `docs/fuzz-coverage.md` |
| The fuzzer passes a pointer to a function (`bp(&v, x)`) and reaches storage through a pointer-to-pointer (`**pp`) | `docs/fuzz-coverage.md` |
| Multidimensional arrays (`[[T; N]; M]`) — a local initialises from a nested literal, matching `static`/`const` | `tests/e2e/local_arrays.rs`, `tests/e2e/types.rs` |
| `let mut x` names the absent `mut` instead of failing with "expected `:`" | `tests/e2e/error_diagnostics.rs` |
| A `%` and a `match` in one program no longer collide on an `mx_` label — a latent assembler-reject the payload fuzzer found | `tests/e2e/operators.rs` |
| An interrupt handler saves only the zero-page scratch its reachable code writes, not the whole region (a counter handler: 63 bytes → 0) | `tests/e2e/interrupts.rs` |
| A comparison collapses to a bare branch inside a standalone function, not only when inlined — a void `RTS` no longer looks like it reads A and the flags | `tests/e2e/branch_fusion.rs` |
| A constant or zero-extended 16-bit operand folds into the immediate that reads it (`CMP #$54` / `ADC #$00`) instead of staging through the `$20/$21` scratch pair | `tests/e2e/execution.rs` |
| An expression argument with no call in it is evaluated straight into the callee's frame slot, skipping the `$F4`-`$FE` pool round-trip and the byte-by-byte copy that followed it | `tests/e2e/nested_calls.rs` |

## What keeps going wrong

Nearly every silent miscompile found so far has had one of these shapes. They
are worth reading before adding a case to any of the matches involved — each
cost several bugs before it was named.

- **A fallback that is another strategy, not a failure.** A dispatch match
  whose default is the scalar path. A form nobody enumerated does not fail; it
  gets loaded as though it were a `u8`. Nine bugs: struct copy from a place,
  struct arguments, `.low`/`.high` assignment, indirect calls, frame colouring,
  double evaluation in `arr[i] = f()`, slice binding, `g(mk())`, and `Call`
  matched where `CallIndirect` was not. The three sites that decide an
  aggregate's fate now *refuse* what they cannot carry.

  Ten, now: `return s` from a `-> S` function fell through to the scalar return
  path, which loaded the struct's first byte into A. The slice return had been
  given its case; the struct one had not, and there was no `_ =>` to blame —
  just an `if` chain whose last arm was "an ordinary value".

- **One rule with several implementations.** The same question answered in more
  than one place drifts. A function pointer is two bytes with its high byte in
  Y — neither the number convention's reason nor the address convention's — so
  it fell off three separate hand-written lists of "the wide types", each
  omission a different wrong answer. The authoritative table already existed
  and already knew. Ask it; do not re-list the variants.

  Three more since: a *constant* struct decided constant-ness by matching
  `Expr::Literal` where sema decides it by folding, so `S { f: (-1) }` was
  constant to one and not the other; `*p` re-listed the two-byte types and left
  out the ones that are addresses, which the *store* side had already been
  fixed for; and the by-reference field load kept its own copy of the
  indirect-load sequence `emit_deref_load` exists to be — which is precisely
  how the two could disagree.

- **`None` meaning two different things.** "This form has no address, and never
  could" and "this form has one and nobody wrote the case" read identically to
  a caller. `Denotes` separates them and is exhaustive with no catch-all.

- **Generating a construct is not observing it.** A fuzzer that passes a struct
  and reads a field into a local the return never uses cannot tell a correct
  staging from a broken one. Every value a test introduces has to reach an
  output cell; with two wide parameters, *each* needs its own term.

- **The specification drifting behind the compiler.** Divide-by-zero and
  over-width shifts were both defined in code and called "undefined" in the
  reference — in the first case the reference contradicted its own stdlib
  section. When the hardware sequence already yields a sensible answer for
  free, document it rather than inventing a different one.

- **A tool that reduces has to preserve what it is reducing.** The shrinker
  kept only "wrong answer" versus "rejected", so reducing a rejection was free
  to walk to an unrelated one — and did, twice in one afternoon, to an `i8`/
  `i16` mismatch and to `% 0` folded out of two literals. Both times the
  reported program no longer contained the failure, and the time lost re-deriving
  it was more than the reduction saved. The rejection's *reason* is part of the
  kind now.

- **A latent collision hides until a rare construct becomes common.** Two label
  namespaces both spelled their exits `mx_N` — the u8 modulo off the general
  counter, `match` off its own — and since labels are file-global, a program
  with both could emit the same name twice. It sat unhit for as long as `match`
  was rare; the enum-payload fuzzer made matches common, and the two counters
  met within 3000 seeds. A shared prefix across two independent counters is a
  duplicate waiting for the traffic to find it — give each namespace its own.

- **Verify against the bug, not against the fix.** Every correctness change
  here is checked by putting the defect back and watching a named test fail.
  For a table copied from the specification, break the *reference* instead —
  agreeing with a reference you derived from the code proves nothing. And
  beware a test that passes by luck of layout: the `str` page-crossing bug was
  invisible until two literals landed in different pages.

---

## Language features

### Bit-range slice

`flags.bits[7:4]` — extract and insert a contiguous field. Single-bit access is
complete; this is the multi-bit generalization.

### Compile-time functions (`const fn`)

Deferred, deliberately. A generated table already covers the common case — a
table whose entries are a function of their index — and it does so with a
constant evaluator that already existed. `const fn` is the general version: a
named body, callable from a table's body and from any other constant, with
recursion and its own termination question.

The reason to wait is that the general feature is only worth its cost once
there is a table the specific feature cannot express. Two candidates are
plausible — a sine table wanting real arithmetic, and a CRC table wanting a
loop per entry — and neither is a *shape* the current body can state, so this
is the item to reach for when one turns up.

### Patterns the 6502 has and the language does not

These came out of a survey of what assembly programmers did on this machine
that Wraith cannot say efficiently today. They are listed in the order their
cost/benefit looked best; none is started. (Structure-of-arrays layout was the
first of them and is now `#[soa]`; the count of the spec's compiling examples
above is the other half of the same survey being worked through.)

- **Page alignment.** `LDA tbl,X` crosses a page boundary and costs an extra
  cycle; a table the programmer wants aligned has no way to say so.
  `#[align(256)]` on a `const` or `static`, honoured by the section allocator.

- **`critical { }` blocks.** Disabling interrupts around a multi-byte update is
  `SEI` / body / `CLI` today — written by hand in `asm!`, and wrong if the
  caller already had them disabled. A block form can save and restore the flag
  instead, and the compiler knows how long the body is.

- **Fixed-point arithmetic.** A `q8.8` type with the shifts folded in. Every
  6502 program that draws anything reinvents this, usually as a pair of `u8`s
  and a comment.

- **Carry chaining / wider integers.** `u32` addition is four `ADC`s and no
  `CLC` between them. The language stops at 16 bits, so wider arithmetic is
  written out by hand at every use.

- **Unroll control.** `#[unroll]` on a loop with a constant bound. The
  trade — code size for the index arithmetic — is the programmer's to make,
  and there is no way to make it.

- **Calling-convention control.** A leaf function that wants its argument in X
  rather than through the staging pool has no way to ask.

### Columns for a local array, and for a nested field

`#[soa]` applies to a top-level `static` or `const`, which is where an entity
pool lives on this machine. Two shapes are outside it:

- A local `let` array. `let` takes no attributes today, so this is a parser
  change before it is a layout one. Frame arrays are small and short-lived, so
  the multiply matters less; worth doing when something asks for it.
- A field that is itself a struct or an array. Its parts would each want a
  column, which is a nested scheme rather than the flat one — and the flat rule
  ("every field is a scalar of one or two bytes") is what makes indexing a
  column cost no multiply. Refused with a message that says so.

Two smaller things the work exposed:

- A constant index into a column still goes through the runtime-index path on
  the *write* side: `E[1].hp = v` emits `LDA #1 / TAY / STA col,Y` where
  `STA col+1` would do. Correct, three bytes larger.
- `enum` and `import` declarations still drop attributes silently, the way
  `static` did before this change. Nothing valid can be written there yet, so
  nothing is currently mis-accepted, but the same silent-drop is waiting.

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
redundancy.* It proposed tracking status flags across statements; that case
barely occurs, because an `if` ends in a label and usually contains a call, and
both invalidate flags.

The waste was elsewhere. A comparison feeding an `if` built a 0/1 in `A` and
then compared *that* against zero — eleven instructions for one branch.
`collapse_boolean_compares` matched only a one-branch comparison tail, and
unsigned `<=` and `>` have no single 6502 branch so they end in two. Both
shapes are matched now, and the guard no longer requires V dead, which it never
affected: together that removes 115 instructions across the example corpus.

Finding it took an instrumented run reporting every guard's value per site.
Two earlier guesses were measured and both were wrong — worth remembering
before optimising against intuition.

*Then a second cause.* The guard refused a collapse whenever the boolean
looked live in `A` or the flags after the branch, and at a void function's
`RTS` both did: the liveness treated a return as reading the accumulator and
every flag, as if a caller took them. A value returns in a register and is
loaded there immediately before the `RTS`; a flag is never a return channel and
a void function returns nothing. So `RTS` reads neither — and with that, a
condition inside a *standalone* function collapses the same as one inlined into
`main`, which it did not before. `tests/e2e/branch_fusion.rs`. (The earlier note
here — that the survivors fed an assignment or an argument, where the 0/1 was
wanted — was wrong: every one fed a branch, and this is what blocked them.
Instrumenting the guard, once more, is what showed it.)

### Smaller code

- **Reclaim BSS from dropped statics.** *Done.* The layout is repacked between
  `reachable_symbols` and `finalize_frames`, so live statics keep their order
  and the gaps close.

  The lesson worth keeping is where an address lives by that point: there are
  *three* copies — the symbol table, the per-use snapshots in
  `resolved_symbols`, and `inline_param_symbols`, which despite its name holds
  every symbol a function's body resolved. Moving a static in fewer than all
  three silently puts it back. The fuzzer caught both halves of getting it
  wrong.

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
budget, a local array, a `const` table and a struct with an array field, and a
pointer that names one of the program's own variables and can be moved to
another — all nested. Every cell's final value is written out, so one program
checks a dozen or more results.

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

Seventeen real bugs so far, most of them silent:

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
11. `s = mk();` had no codegen path where `let s: &[u8] = mk();` did — the
    binding and assignment forms of a slice-returning call had drifted apart.
    Found while teaching the generator to pass slices around, by the guard
    under the store rather than by a wrong answer, since that guard had landed
    the day before.
12. The widening the language performs *without* an `as` zero-extended, at five
    of the six places it happens: a binding, an assignment, an argument
    (direct, inlined and indirect), an array element and a struct field. Only
    the explicit cast and `return` were right. So `let b: i16 = a;` on a
    negative `i8` dropped the sign — and four of the five did not recognise a
    signed source at all, emitting no high byte, so the slot took whatever `Y`
    held from an unrelated instruction and the same program could answer
    differently depending on the statement before it. All six now share one
    `emit_widen_a_into_y`.
13. A struct returned by value came back as its *first byte* in A, with X
    holding whatever the last instruction left. The caller was right — it
    dereferences A:X and copies the bytes out — so `mk(7)` returning
    `S { f0: 7, f1: 8 }` answered `(0, 0)`, read from zero page `$0007`. The
    slice return had this case and the struct one was simply missing.
14. A *constant* struct refused an array field, and decided "constant" by
    matching `Expr::Literal` where sema decides it by *folding* — so a struct
    compiled only when every field happened to be a non-negative number, and
    `return S { a: [1, 2] }` failed where the same struct bound to a local
    compiled.
15. `*p` on a `&&u8` loaded **one byte**: it decided "two bytes" by re-listing
    `u16`/`i16`/`b16`, which leaves out the two-byte values that are addresses.
    The binding then stored whatever X held as the high half, so the pointer
    came out `$0000` where it should have been `$0400` and both the read and
    the write through it landed in zero page. The store side had already been
    fixed the same way.
16. The by-reference field load kept its own copy of the indirect-load
    sequence that `emit_deref_load` exists to be — which is how 15 could
    happen on one side and not the other. Both go through it now.
17. An **enum field of a struct** was stored as a pointer's low byte and read
    back by dereferencing that byte as an address, though the struct's layout
    gives the field `size_of(enum)` bytes inline. A `match` on such a field
    took whichever arm the garbage selected. The field matrix passed the whole
    time: the store and the load were wrong in the same direction, so the
    round trip agreed with itself. Found by asking why a `static` struct could
    not name an enum variant — the answer was that the value it would have
    written was not what the reader expected.

Regression tests: `tests/e2e/const_folding.rs`, `tests/e2e/consts.rs`, `tests/e2e/int_conversions.rs`,
`tests/e2e/short_circuit.rs`, `tests/e2e/nested_calls.rs`,
`tests/e2e/loop_sweep.rs`, `tests/e2e/bss_reclaim.rs`,
`tests/e2e/aggregate_dispatch.rs`.

**To widen**, in rough order of value — each needs the oracle extended to match,
and an oracle that is merely *probably* right is worse than no oracle:

- **Division by zero.** *Done.* The specification said undefined; the compiler
  had not been undefined for some time, and its own stdlib section documented
  `div16` returning `0xFFFF`. `x / 0` and `x % 0` are now specified as the
  all-ones value at every width and sign — what shift-and-subtract already
  produces, and what RISC-V defines for the same reason. The signed paths were
  fixed to preserve it (they gave `+1` for a negative dividend), and a divisor
  the compiler can see is zero is a compile error. The generator takes
  arbitrary divisors: 670 of 2000 programs divide by zero at run time.
- **Precedence.** *Done.* Expressions from the main generator stay fully
  parenthesised on purpose — a precedence disagreement there would masquerade
  as a codegen bug. A separate generator writes one operator chain twice, flat
  and as the tree the specification's table says it means, and requires both to
  agree; no oracle is involved. Six levels mix freely (`* / %`, `+ -`, `<< >>`,
  `&`, `^`, `|`); the relational and logical levels cannot, since `a & b == c`
  groups as `a & (b == c)` and does not type-check.
- **Mixing across signedness families**, and narrowing. *The widening half is
  done.* A program now picks one family and mixes its two widths — `u8`/`u16`
  or `i8`/`i16` — because those are the two the language widens between without
  an `as`. Variables, parameters, locals and return types each take either
  half; the aggregates take one.

  The oracle needed less than expected, because arithmetic is never mixed:
  operands of an operator have to agree, so the only place the two widths meet
  is a *boundary* — a binding, an assignment, an argument, a `return`. A
  boundary operand carries the type it was generated at, and the rule is that
  the arithmetic happens at the narrow type and the widening after it, so
  `200u8 + 100` bound to a `u16` is 44 and not 300.

  It found the widening was zero-extending everywhere the language did it
  implicitly — five of the six sites. See the note under
  [Correctness & diagnostics](#correctness--diagnostics).
- **Cycles of three or more.** *Done for two.* A pair that call each other is
  the case frame colouring solves with Tarjan; the generator builds one, each
  member calling the other at `d - 1` with a base case at `d == 0`.

  Two constraints make that safe and would apply to a longer cycle: a pair
  member may not reach its partner by an *ordinary* call (a fresh literal
  budget resets the cycle's depth every second edge and never terminates), and
  a pair member takes no wide parameter, since its partner would have to pass
  one on every mutual edge.

  **The disjoint frame layout inside an SCC is not what carries this** —
  deliberately overlapping two members' frames survives 600 seeds, because the
  per-edge save covers it. What does carry it is the recursive-edge set:
  counting only self-edges fails at seed 38.
- **Pointers and enums.** *The pointer half is done.* `main` declares
  `let p: &T = &v{k};` over one of its own variables; `*p` reads, `*p = e`
  writes, and `p = &v{j}` moves the alias, so which storage a read reaches is
  program state rather than a fact about the declaration.

  The alias modelling turned out to be cheap, because the *generator* chooses
  the targets: every one has the pointer's own type, so a store through it is
  never a widening, and the oracle carries one index. A store is followed
  immediately by a read of a variable it may have landed in — the alias has to
  hold at the very next statement, whatever the compiler believes about its
  registers in between.

  Checked against reintroduced defects: reading a 16-bit pointee as one byte,
  and storing one as one byte, each fail inside the default 120 seeds. Dropping
  the register invalidation after an indirect store does *not* — it survives
  120 and fails 1 of 2000, because the register cache rarely spans the store
  even when nothing tells it not to. That thinness is recorded rather than
  papered over.

  Still open here: a pointer to an element, a field or another pointer, and a
  pointer passed to a function. The pointer-to-pointer case is where the live
  bug was — see the `emit_deref_load` entry below — and it is covered by
  `tests/e2e/aggregate_dispatch.rs` rather than by the generator.

  *The slice half of this item is done*, and needed no alias modelling: a slice
  of the `const` table views ROM, nothing writes it, and the table is the
  generator's own, so the oracle carries `(start, len)` and reads the element
  out of the table it already knows. Programs that declare the table declare
  two slices over it, and a descriptor reaches one four ways — a range
  expression, a copy from another slice, `f0`'s parameter, and a call to `mk` —
  which are four codegen paths rather than one. Each slice reports its first
  element *and* its length as output cells, so a copy that moves one half and
  not the other is visible even when no expression read it.

  The generator was checked against the bugs rather than assumed to cover them:
  truncating the descriptor copy fails seed 0, staging a slice argument as two
  bytes fails seed 119, and copying two bytes of a returned descriptor fails
  three of the first hundred and twenty. It also found a fourth on its way in:
  `s = mk();` had no codegen path where `let s: &[u8] = mk();` did.
- **Aggregates across a call.** *The argument half is done, all four kinds.*
  `f0` may take a struct, an enum and a `str` beside its slice — at most two at
  once, since a descriptor and three addresses are ten of the argument pool's
  eleven bytes before a single value parameter. Each is folded into the
  callee's result so that passing it can be told from staging it wrongly; see
  [What keeps going wrong](#what-keeps-going-wrong) for why that is not
  optional. Each is verified by giving it the wrong convention and watching the
  default 120 seeds fail.

  A `str`'s `.len` stages its pointer through the four-byte high pool, so it is
  read once per call rather than freely: at 12% of expressions it put 790 of
  6000 seeds over that pool.

  *The return, the array field and the pointer are done too.* `mks(k) -> S`
  returns a constant literal on one branch and a local on the other — ROM data
  reached by label, frame storage reached by its address — and `main` binds it,
  assigns it and passes it on, because binding and assignment are separate
  copies in the compiler and only generating one leaves the other's length free
  to be wrong. The struct gained an array field between its two scalars, three
  elements where the local array has four, so an offset computed from the wrong
  array lands on a cell the program reports. And `pk(pp: &S, w)` writes one
  field through a pointer and returns another, so the callee's storage and the
  caller's are the same bytes.

  Four defects came out of it: a returned struct came back as its *first byte*
  in A rather than its address in A:X; a constant struct refused an array field
  and decided "constant" by matching `Expr::Literal` where sema decides it by
  folding; the by-reference field load kept its own copy of the indirect-load
  sequence; and `*p` on a `&&u8` read one byte.

  *And an array field indexed through the by-reference parameter* — `xp.a[i]`,
  which the compiler refused until an element's address became a run-time
  computation. The struct term folded into that function's result is now either
  kind of field, because a read left in a dead local proves nothing and the
  index arithmetic has no other way into an output cell. Dropping the field
  offset, and dropping the index add, each fail inside the default 120 seeds.

  Two thin spots recorded rather than papered over. Failing to *scale* the
  index needs a 16-bit aggregate type and a struct-taking function at once, so
  it survives 120 seeds and fails at 2000; the e2e matrix covers that case
  directly. And no program declares a mutable `static` *struct*, so
  `emit_static_field_load`'s offset is unverified by the generator — dropping it
  survives any number of seeds.

  Since closed: enum payloads, indexing a string rather than measuring it, a
  struct nested inside a struct, a pointer to a struct field and an array
  element, a pointer passed to a function (`bp(&v, x)`), and a pointer to a
  pointer (`**pp`). What is left is narrow: a struct returned *through a
  function pointer* in the generator (that one has an e2e test), a nested
  field reached *through the by-reference parameter*, and a `u16` enum
  payload — each recorded in the fuzzer's own caveats.
- **Copying a run of bytes.** *Done.* `memcpy(&arr[d], &TBL[s], n)` is
  generated wherever the program's type is `u8`, which is what a whole-array
  assignment turned into once that statement was refused. It puts three things
  in reach that nothing else did: an `import`, the address of a *local* array
  element in zero page and of a `const` one reached by label — two different
  computations with their own bug history — and a three-argument call to a
  library function rather than a generated one. The oracle copies the same run,
  so a copy that moves the wrong length or the wrong base shows up in the
  array's output cells.

  The gate is `u8` and not "eight bits wide": `memcpy` takes `&u8`, so an `i8`
  array would need its address converted, and the generator produced exactly
  that invalid program at seed 4 before the gate was narrowed.
- **Constant expressions standing alone** — typed by their own literals, not by
  the program around them. Defined in the specification, not in the oracle.
- **Shift counts at or past the width.** *Done.* A 6502 has no barrel shifter,
  so a variable shift is a loop and the count is simply performed: every bit
  leaves, zeros arrive, and an arithmetic right shift saturates to the sign.
  Specified as such, deliberately *not* masked — masking would make `x << 8` on
  a `u8` mean `x << 0` and would cost an `AND` on every variable shift. A
  constant count at or past the width is a warning, since clearing a value by
  shifting it out is a real if unusual idiom.

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
5. **Branch/flag tracking last** — *partly done, and it went the way this
   entry predicted.* It is the biggest single efficiency prize and the one
   whose predecessor already produced several silent miscompiles, so it waited
   for the fuzzer. What has landed is peephole work: the two remaining
   comparison shapes fused, and a guard relaxed over a flag the rewrite cannot
   touch. What is left is the version that does not pattern-match at all —
   `generate_condition_branch`, described under
   [Branch optimization intelligence](#branch-optimization-intelligence).

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

A call whose whole list fits still stages there when it has to — but the
common case no longer pays for the pool at all. When no argument contains a
call, the callee is not address-taken and the edge is not a recursion one,
each argument is evaluated straight into the callee's frame slot, skipping the
`$F4`-`$FE` pool and the `LDA temp; STA param` copy that used to follow it. The
frame colouring already guarantees those slots do not alias the caller's live
frame outside a recursion SCC, and the stdlib helpers an argument's evaluation
may `JSR` work in `$D0`-`$D8`, clear of the `$40`-`$CF` frame region, so a slot
written early cannot be clobbered while a later argument is built. This is
where the byte savings in the code-size baseline came from. The pool is still
the path for a recursive edge, an address-taken callee, or a list with a call
nested in it. When even the pool does not fit, each argument moves to the
software stack as soon as it is evaluated, so the pool holds only that call's
*widest single argument* and the depth is bounded by the stack's 256 bytes.
Because the frame save shares that stack, a recursive callee's save happens
before the arguments go on rather than after, or it would bury them.

What is left is that a nesting level still costs its widest argument, so around
five levels of 16-bit nesting exhausts the pool. Removing even that means
pushing each argument straight from the registers it is produced in, without a
zero-page slot in between — which needs the per-argument staging in
`generate_call` restructured so the push has one place to happen, rather than
being reached through a dozen `continue`s.

The failure is still a compile error rather than a miscompile, and the fuzzer
budgets one argument per level to match, skipping and counting anything that
overruns anyway.

### The expression temp pool runs out under combined pressure

A second fixed zero-page pool, separate from the argument one above: the
four-byte high pool (`$F0-$F3`) that a `u8` multiply, a pointer compare and a
string index allocate their working storage from.

It became reachable when the fuzzer started passing structs. A function taking
a slice *and* a struct, recursing, with a multiply in the recursive call's
argument holds four bytes of descriptor and two of address while the multiply
asks for its own — and the compiler stops with *temporary storage exhausted in
u8 multiply*. Seed 270 of the execution fuzzer is the repro; no shorter
hand-written program reproduces it, which is itself the point: the trigger is
cumulative pressure, not any one construct.

Like its sibling it is a **compile error and never a wrong answer** — 6000
seeds produced no miscompile — so the harness skips it under `is_known_limit`
and it is recorded here rather than reported as a bug on every run.

The skip rate is a measure worth watching: it was 0.65% of 6000 seeds before
`xp.a[i]` was generated and 2.4% after, because an element address at a
*computed* index parks a byte while the index runs, inside a function that may
already hold a descriptor and an address. Giving a *constant* index its own
path — the offset folds into the base, with no temp and no run-time add —
brought it back to about 1.7%, and made the emitted code smaller besides. The
harness caps the rate at 5% so this cannot quietly degrade into a suite that
mostly tests the pool limit. Removing it
means the same restructuring the argument-pool item describes: values pushed
straight from the registers they are produced in, rather than through a
zero-page slot with a fixed budget.

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

Multi-error reporting covers declarations, bodies, statements, independent
subexpressions, and — since the two boundaries below were crossed — everything
after a failed declaration and everything inside a failed module.

- **Recover from a failed declaration into the bodies.** *Done.* A declaration
  that failed to register left its symbol missing, so every use of it reported
  "cannot find …" on top of the real error; analysis stopped before the bodies
  to avoid that, and one broken declaration hid every mistake below it.

  The suppression is per *name*: what a failed declaration would have defined is
  remembered, and exactly those `UndefinedSymbol`s are dropped in `record` — the
  one place every collected error passes through. The body loop records rather
  than propagates, because propagating discarded the causes to report a symptom.

  It also turned up a diagnostic that did not exist. An unknown named type in a
  declaration was not reported at all: resolution accepts any name in type
  position, as it must — `struct A { next: &B }` may name a `B` below it — and
  nothing checked afterwards, so the first sign was a mismatch at the *use* site
  against a type that does not exist. A pass after registration reports each one
  where it is written.
- **Report several errors from one imported module.** *Done.* The child's whole
  set is rendered against its own source and carried up under one trail, which
  followed from the recovery above: before it, a module's declaration errors hid
  its body's.

  The remaining half was the diamond. Only *successful* analyses were cached, so
  a second import of a broken module re-analyzed it and rendered everything
  again — two modules importing one broken third turned three mistakes into six.
  Failed modules are remembered too, and a later import of one says the import
  failed and points at the report instead of repeating it.

Also open:

- **The widening the language performs by itself is now one code path.** *Done,
  and worth keeping in view.* Only lossless widening is implicit — `u8` to
  `u16`, `i8` to `i16` — and which extension applies is a property of the
  *source*: a signed one carries its sign into the new high byte. Six places
  perform it, and five zero-extended. Four of those five did not recognise a
  signed source at all, so they emitted no high byte and the destination took
  whatever `Y` held from an unrelated instruction — the same program could
  answer differently depending on the statement before it.

  The lesson is the one the aggregate guards taught a layer up: six sites each
  deciding the same rule for themselves is five chances to get it wrong. They
  share `emit_widen_a_into_y` and `implicit_widening` now, so a seventh site
  asks rather than re-derives.

- **Widen the spec-example harness further.** *133 of the spec's 225 code
  blocks* are compiled on every run (`tests/e2e/spec_examples.rs`), up from 94,
  via ` ```rust,compile ` for whole programs and ` ```rust,compile,fragment `
  for statement runs wrapped in a generated `main`.

  Most of the gain was the stdlib reference, whose examples each called a
  library function without importing it — so none of them compiled, and a
  reader copying one got an undefined name. Compiling them also found four
  examples that were simply wrong: two declared a `#[reset]` function *inside*
  `main` and called `main()` from it.

  The fix for a block that needs a helper is to put the helper *in the block*,
  not to teach the harness a third shape: an example that cannot stand on its
  own is one a reader cannot use either.

  What is left is capped by a test, with each category written down beside it:
  ~29 truncated `{ ... }` signatures where the ellipsis is the point, ~20
  deliberate error examples, 9 imports of illustrative modules, and features
  the spec marks as not implemented.
- **Every diagnostic is pinned, or says why not.** *Done, and self-checking.*
  The `SemaError` variant list is read from the source at test time, in the
  style of the fuzzer's AST coverage, and each name must be pinned by a golden
  test in `tests/e2e/error_diagnostics.rs` or listed with a reason.

  Two variants turned out to have no construction site anywhere —
  `OutOfZeroPage`, which this item asked for a test of, and
  `ReturnOutsideFunction`, which cannot be built because a `return` outside a
  function does not parse. Both are deleted: a variant nobody raises is a
  message nobody reads and a case every `match` still has to carry.

  Writing the exemption list caught two invented reasons on the first pass, for
  variants that were already pinned forty lines above. An exemption list is only
  worth as much as the willingness to check it.

---

## An aggregate reached through something that is not its name

*Done.* Four refusals with one cause, and one miscompile behind them.

`&x.f[0]` and `&m[i][j]`, `p.a[i]` on either side, `p.inner.v`, `mk(6).f1` and
`&buf[i]` were each refused with their own message, and the reason in every
case was the same: the compiler could resolve an address only at *compile*
time. A pointer's target is not known then; nor is a call's result; nor is an
element at a run-time index.

`emit_aggregate_base` is the run-time counterpart of `resolve_static_addr` —
the address of what an expression denotes, in A:X, with a pointer dereferenced
on the way through because that is what `p.field` means. It tries the constant
answer first, so a place that has one still gets the two immediate loads it
always did, and the code-size benchmark did not move. Its three answers follow
the contract the address resolvers already use: `Some` is the address, `None`
is "no storage at all", `Err` is "storage nobody can address".

It subsumed `generate_addr_of_element` and `generate_addr_of_field`, which are
deleted rather than left as a second implementation.

Two things worth keeping from doing it:

- **The store path had to compute the address before the value.** It used to
  evaluate the index into `Y` first, so reaching a run-time base after that
  would have evaluated the index a second time — invisible for arithmetic, two
  calls for `p.a[f()] = v`.
- **An enum field was a miscompile, not a gap** — see the `static` struct entry
  below. Pursuing "which field kinds can a `static` initialise" is what found
  it, which is the argument for closing feature gaps rather than working around
  them: the workaround (assign in `main`) hit the same broken path.

Still open in this family: a struct nested inside another struct is laid out
inline and reachable, but the fuzzer does not generate one, and enum *payloads*
are neither generated nor exercised beyond the tag.

---

## Structure & maintainability

- **Shared exhaustive Stmt/Expr walker.** Hand-enumerated per-form walkers and
  merge lists that missed new variants were the dominant defect for a long
  time. A single recursion helper shared by all analysis walkers would close
  it; the import-merge half is already done, and `contains_call` /
  `expr_contains_call` are exhaustive.
- **Make codegen's per-form dispatch exhaustive, or make its fallback loud.**
  *Done for the sites and for the resolvers under them.* See
  [What keeps going wrong](#what-keeps-going-wrong) for the shape and the count.
  Binding, assignment and argument staging classify what they are about to
  store and refuse a slot wider than the registers carry; the loop-unrolling
  estimate enumerates every expression form rather than charging a flat `_`;
  and `Denotes` separates "no address" from "nobody wrote the case".

  What is left is smaller and known: `array_field_base` and
  `array_of_struct_base` are narrow enough that their `None` has one meaning,
  and the `Option`-returning helpers outside `aggregate.rs` have not been
  audited against this distinction.

  One more turned up while writing the array-copy refusal: `&x.f[0]` and
  `&m[i][j]` reached `generate_addr_of_element` with no resolved symbol, and it
  reported an *internal compiler error* — a compiler bug — for source the
  compiler simply did not handle. *Both are emitted now*, along with `p.a[i]`,
  `p.inner.v`, `mk(6).f1` and `&buf[i]`: they were all the same refusal, which
  was that an address could only be resolved at *compile* time. See
  [An aggregate reached through something that is not its
  name](#an-aggregate-reached-through-something-that-is-not-its-name).
- **Whole-array assignment meant "repoint", and only a local could be
  repointed.** *Done — the statement is refused at every storage class.* A
  local array's slot held a pointer to its data, so `a = [4, 5, 6]` rebound the
  slot and `a = b` left the two *aliased*; a `static` array and a struct field
  *are* the data, so the same statement stored the literal's ROM address over
  the elements. Three storage classes, three meanings, none of them a copy.

  The language question behind it — should assigning an array copy? — is
  answered the other way: it should not be a statement at all. A copy on the
  6502 is a loop whose length is the array's, and an assignment that looks like
  a register move must not emit one silently. The refusal names both ways out,
  element-wise and `memcpy`, and the specification now says so under
  [Array Assignment and Copying](specification.md#array-assignment-and-copying).
  A slice is untouched: `sl = TBL[1..4]` moves two numbers and stays legal.

- **Interrupts were only ever tested from the idle loop.** *Done.* A handler's
  zero-page frame may *share addresses* with `main`'s — a handler is not
  `main`'s callee, so colouring cannot separate them — and the whole design
  rests on the handler saving that span. Every existing test fired through
  `pulse_irq`, which waits for the idle loop first, where nothing is live.

  `run_interrupted` asserts the line every *n* instructions while `main` works,
  so the handler is entered between the halves of a 16-bit add and across a
  `JSR` with the argument pool staged. **The saves hold** — a negative result,
  kept because dropping the math working storage from the save list (keeping
  the push/pop balance, so nothing else breaks) fails the new test and no
  existing one.

  *Narrowed.* The save set was the whole shared scratch region — 63 zero-page
  bytes, ~1000 cycles per interrupt — whatever the handler touched. It is now
  exactly the scratch bytes the handler's reachable code *writes*: an address
  the handler never writes it cannot corrupt, and codegen knows the addresses
  where the sema AST scan does not, so a pre-pass emits each reachable function
  into a throwaway emitter and unions the zero-page stores
  (`narrow_interrupt_scratch`). A `DATA_PORT = DATA_PORT + 1` counter handler
  now saves nothing; `interrupt_counter` shrank 824 → 68 bytes. It falls back to
  the full region whenever the graph is opaque — an indirect call, inline `asm`,
  or a 16-bit math routine whose own scratch use is not scanned — so the
  conservative save is still there where it must be. The `run_interrupted` guard
  now also drives a *non-opaque* scratch-touching handler (a 16-bit compare
  through `$20/$21`, the same bytes `main`'s arithmetic stages), which is where
  a save narrowed one byte too far would surface.

  The storm has to be bounded: an interrupt pulls the CPU out of `loop {}` as
  readily as out of anything else, and the harness detects termination by the
  program counter standing still, so an unbounded storm never halts.

- **One rule, four implementations.** *Done.* A call's arguments were staged in
  four separate pieces of code, each deciding a parameter's width and register
  convention for itself, and every chance to differ had been taken.
  `ParamClass` answers the classification once — exhaustive over `Type`, no
  catch-all — and `stage_argument` is the one routine all four call forms use.
  What is left per site is only where the bytes wait and how the site names
  itself in a diagnostic. 214 lines shorter, byte-identical output, verified by
  putting eight known miscompiles back one at a time.

  The matrices that found them are kept: `every_parameter_kind_survives_every_
  call_form` and its tail-call twin. Widening one by a single row is what found
  the last two.
- **A `static` struct can only initialise some of its field kinds.** *Done —
  every kind is accepted now.* A `u16` and a `fn` field already were; a `&T`
  turned out to be too, which the entry had wrong. A `str` field is the same
  shape as a `fn` field — two bytes the assembler fills in — and differs only in
  *who* names the label: the string collector does, at codegen, and
  deduplicates identical literals, so the content travels out of sema and is
  resolved before anything is emitted.

  The enum field was not a missing case but a **miscompile**, and it was live
  for a local struct as much as a `static` one. An enum's layout says it is
  stored inline — the tag and its payload, `size_of` bytes — but constructing a
  variant yields a *pointer* to those bytes, and the struct paths stored that
  pointer's low byte into the field and dereferenced it again on the way out. A
  `match` on such a field took whichever arm the garbage selected;
  `every_kind_of_struct_field_round_trips` passed the whole time, because the
  store and the load were wrong in the same direction. Initialising, assigning
  and reading now all treat an enum field as inline bytes, which is what the
  layout always said.

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
