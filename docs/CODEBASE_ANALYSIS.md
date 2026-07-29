# Wraith Codebase Analysis & Recommendations

_Date: 2026-07-28. Method: five parallel deep audits (sema, codegen/emitted-asm, front-end/perf, test infrastructure, repo/docs). Every CRITICAL and most HIGH findings were reproduced against the built compiler and, where relevant, assembled with `flatasm` and/or run on the mos6502 emulator. Suite at time of audit: 956 tests, all passing — every bug listed below slips through it._

## Executive summary

The project is in genuinely good shape architecturally: the emulator-backed e2e harness is exceptional for a project this size, the peephole's flag-liveness fixpoint is principled, frame coloring is well engineered, and the bug-log discipline in ROADMAP/TODO/test-file headers is excellent.

The audit nonetheless found **22 verified critical miscompiles/crashes on valid code** (6 remaining — the placement pair, math-JSR tracking, the temp-pool four, the import-merge three, the indirect-call spill, four of the five `$0000` constants bugs, and the match-pattern critical were fixed after the audit). They cluster into a small number of root causes, which matters more than the count:

1. **Hand-enumerated AST walkers and merge lists miss new variants.** New expression forms (`ForEach`, `CallIndirect`, the `SliceLen`/`U16Low`/`U16High` accessor nodes) and new analyzer state (`accessor_fields`, `const_env`, `local_arrays`, `static_inits`, `unreachable_stmts`) were each added to *some* of the places that must know about them. This one pattern accounts for roughly half the criticals.
2. **Zero-page scratch conventions are documented but not enforced.** The `$F0–$F3` "high pool" is both allocated through `TempAllocator` and written directly by string/struct/index paths; pool-exhaustion fallbacks silently reuse live slots (`unwrap_or(0xF2)`). Accounts for most of the rest. *(Substantially fixed: all demonstrated sites now allocate, spill, or error — see CG-C3/C5/C6.)*
3. **Function-size measurement and emission can drift apart.** Two independent overlap corruptions (jump tables, reset-handler static inits) existed because `placement.rs::measure()` didn't emit exactly what `generate_function` emits. *(Fixed: a shared prologue helper, word counting in the emitter, and a hard "emitted bytes > measured bytes" internal error now guard this class.)*
4. **Docs and implementation drift in both directions.** The spec documents features that don't work (shadowing, multidimensional arrays), misses features that do (struct-variant matching), and several verbatim spec examples don't compile.

**Recommended fix order** (details in each section):

| # | Action | Kills |
|---|--------|-------|
| 1 | Runtime enum storage redesign (frame slots, not scratch pointers) | CG-C8 |
| 2 | CI + sema-level fuzzer + doc-examples-compiled-in-tests | keeps 1 from regressing |

---

## CRITICAL — miscompile or crash on valid code

### Codegen (src/codegen/)

**~~CG-C1. Match jump tables invisible to size measurement; next function overwrites the match body.~~ FIXED.**
Was: `emit_word`/`emit_word_label` never bumped `byte_count`, so `placement::measure()` under-measured and the next function's `.ORG` landed inside the match. Fixed by counting both words in the emitter and adding a hard emitted-vs-measured assertion in `generate_function` (regression test: `e2e::placement::a_match_jump_table_is_counted_in_the_functions_size`). The same test surfaced a second, independent bug — the table was sized by the largest *armed* tag, so a variant with no arm and a higher tag read past the table; `determine_match_strategy` now uses the enum definition's maximum tag.

**~~CG-C2. Reset-handler static initializers not measured — same overlap corruption.~~ FIXED.**
Was: `placement::measure()` reproduced interrupt and function-pointer prologues but not the reset prologue (`item.rs:247-255` + `emit_static_inits`). Fixed by sharing one `emit_reset_prologue` helper between the measuring and real passes (regression test: `e2e::placement::reset_handler_static_initializers_are_counted_in_its_size`).

**~~CG-C3. u16 left-save pool exhaustion silently reuses a live ancestor's slot.~~ FIXED.**
Was: `binary.rs` fell back to `unwrap_or(0xF2)` when the 4-byte high pool was full, evaluating `(a+b) + ((c+d) + ((e+f) + (g+h)))` as 291 instead of 255. The u16 left-save now falls back to the software-stack spill, and every arithmetic-helper fallback (u8/i8 mul/div/mod) is a hard `CodegenError::Internal` instead of a guessed address (regression tests: `e2e::temp_pools`).

**~~CG-C4. `JSR mul16/div16/mod16` doesn't invalidate register tracking.~~ FIXED.**
Was: the tracker kept believing `A == ZeroPage(x)` across the math JSR; `x * y + x` stored the product's low byte as the right operand (2408 instead of 2400). Fixed structurally: `emit_inst` now invalidates all register beliefs on any `JSR`, so no present or future call site can forget (regression tests: `e2e::math16::an_operand_survives_a_{mul16,div16,mod16}_call`).

**~~CG-C5. Hardcoded `$F0–$F2` scratch collides with allocator-managed uses (three shapes).~~ FIXED.**
Was: ForEach-over-string staged its pointer for the whole loop while a body's index assignment parked at `$F0`; `s[i * s.len as i]` had its staged pointer overwritten by `.len` and u8-multiply staging; `arr[s.len as u8] = 42` lost the parked value to `.len`'s staging. Now: ForEach re-stages at every loop head (the body may clobber `$F0–$F2` freely), string index and string `.len` stage through the allocator, and string equality spills the left pointer to the software stack across a call-bearing right operand (regression tests: `e2e::temp_pools`).

**~~CG-C6. Array-of-struct field write parks the value at `$F4/$F5` without allocating.~~ FIXED.**
Was: `ps[ident(1)].x = 42` stored 1, because the call's argument staging was handed the same `$F4`. The park is now allocated from the arg pool; exhaustion is a hard error (regression test: `e2e::temp_pools::array_of_struct_field_write_with_a_call_in_the_index`).

**~~CG-C7. `contains_call` misses `Expr::CallIndirect`.~~ FIXED.**
Was: a u16 op whose right side is a vtable call took the "$F0 is safe" fast path, and the callee's own codegen used the same pool — `(x + x) + ops.run(3 as u16)` restored the callee's product as the left operand. `contains_call` now covers `CallIndirect` (and `Match`, `StructInit`/`AnonStructInit`, `EnumVariant`, `Slice`) (regression test: `e2e::vtable::a_u16_left_operand_survives_an_indirect_call`).

**CG-C8. Runtime-constructed enums are pointers into shared scratch — any two live enum values alias.**
`aggregate.rs:1282-1298` builds enum bytes in the `$20–$3F` pool and stores the *pointer*; nothing copies the bytes out. `let e1 = mk(1); let e2 = mk(2);` — `match e1` extracts 2 (reproduced). Any later `$20` temp use destroys a live enum's payload.
_Fix:_ allocate runtime enum storage in the function's frame and copy on binding. This is a redesign, not a patch; until then the feature is only safe when matched immediately.

**CG-C9. ForEach over large/wide arrays silently miscounts or misindexes.**
`stmt.rs:1274` emits `CPX #$12C` for a 300-element array; `asm.rs:544` truncates to `#$2C` (44 iterations, confirmed in the binary). u16-element arrays of 128–255 elements wrap the scaled offset and re-read elements 0..n-129.
_Fix:_ reject `for x in arr` above 255 elements (and byte-size > 255 for scaled paths) at sema; make flatasm range-check immediates instead of truncating.

**CG-C10. Address-taken + tail-recursive function: loop restart jumps into the `$E0`-staging prologue — infinite loop.**
`item.rs:267-270` emits `{name}_loop_start` before the function-pointer prologue (`item.rs:299-315`). The tail update writes new args into frame params, jumps to the label, and the prologue immediately overwrites them with the stale `$E0` staging (reproduced: `count` loops forever on its original argument).
_Fix:_ emit the loop-start label after the staging prologue.

**CG-C11. Static zero-fill writes past the array for sizes > 256 not a multiple of 256.**
`item.rs:377-392`: a 300-byte static zeroes 512 bytes (reproduced). Self-heals in the default layout but destroys anything a custom `wraith.toml` places after it.
_Fix:_ emit the partial page as a shorter final loop, mirroring `generate_local_array_init` (`stmt.rs:2963-2974`).

### Sema (src/sema/)

**~~SE-C1. `.low`/`.high` on a `const u16` reads `$0000`.~~ FIXED.**
The const evaluator now handles all three accessor nodes (`.low`/`.high` of a constant integer, `.len` of a constant string), so these fold at their use sites (regression test: `e2e::consts::low_and_high_of_a_const_u16_fold_to_its_bytes`).

**~~SE-C2. Using an imported `pub const` scalar reads `$0000`.~~ FIXED.**
`const_env` is now merged in `process_import`, so imported constants fold at their use sites and `const D: u8 = C + 1` in the importer works (regression test: `e2e::imports::an_imported_scalar_const_has_its_value`).

**~~SE-C3. A scalar `const` whose initializer doesn't fold is silently accepted; every use reads `$0000`.~~ FIXED.**
A failed const-eval of a scalar- or string-typed const is now a hard `SemaError` at the declaration; aggregates keep their ROM-data path (regression test: `e2e::consts::a_scalar_const_with_an_unevaluable_initializer_is_rejected`).

**~~SE-C4. A memory-mapped read hidden under a cast is folded to a compile-time constant.~~ FIXED.**
`contains_addr_reference` now recurses through `Cast`, `Index`, `Field`, `Slice`, and the accessor nodes, so `VIC as u16 + 1` reads the register at runtime like `VIC + 1` does (regression test: `e2e::consts::an_addr_read_under_a_cast_is_not_folded_to_its_address`).

**~~SE-C5. Local arrays in imported functions are emitted inline in ROM again.~~ FIXED.**
`local_arrays` and `array_block_sizes` are now merged, so imported functions' local arrays get RAM blocks like the root module's (regression test: `e2e::imports::a_local_array_in_an_imported_function_lives_in_ram`).

**~~SE-C6. Several analyzer maps don't survive import — including `accessor_fields` and all of mutable statics.~~ FIXED.**
The merge now covers `accessor_fields`, `resolved_struct_names`, `unreachable_stmts`, the child's full type registry (an imported function may use types the importer never names), `warnings`, `static_inits`, and mutable-static symbols. The BSS collision is solved by relocation: the child's allocations are shifted past the importer's high-water mark, so statics can be declared in any order across modules (regression tests: `e2e::imports::an_accessor_named_field_works_in_an_imported_function`, `mutable_statics_in_an_imported_module_are_initialized_and_dont_collide`, `a_static_declared_before_the_import_relocates_the_imported_ones`). A systematic merged-state struct remains a good idea but the list is currently complete.

**~~SE-C7. Match patterns are never checked against the scrutinee type.~~ FIXED.**
A new `check_pattern_type` runs in both match paths (statement and expression): enum patterns must name the scrutinee's enum (and the variant must exist, with the right binding count — a unit variant with bindings or a tuple variant with the wrong arity now errors cleanly instead of producing no bindings), and literal/range patterns must be representable in the scrutinee type (regression tests: five in `e2e::enums`).

**~~SE-C8. A bare `const` struct is accepted but never emitted; field reads hit `$0000`.~~ FIXED.**
A const of struct or enum type is now rejected at declaration with a pointer to `static` or a const array (regression test: `e2e::consts::a_const_of_struct_type_is_rejected`).

### Front end

**FE-C1. Compiler panic: constant string slice on a non-UTF-8 boundary.**
`const_eval.rs:135` — byte indices into a Rust `String`. `"héllo"[0..2]` panics: `byte index 2 is not a char boundary`.
_Fix:_ operate on `s.as_bytes()` (6502 strings are bytes anyway — bytes are arguably the *right* semantics) or validate char boundaries with a proper error.

**FE-C2. Compound assignment evaluates the target multiple times.**
`parser/stmt.rs:483-496` desugars `x += y` by cloning the target. `arr[idx()] += 5;` calls `idx()` three times (reproduced in emitted asm) — a hardware-register read in an lvalue executes 3×.
_Fix:_ a real `Stmt::CompoundAssign` node with the address evaluated once, or restrict targets to side-effect-free lvalues in sema.

---

## HIGH — wrong diagnostics, missing checks, fragile invariants

### Compiler

- **~~SE-H1/H2. Escape analysis never descends into `for…in` bodies, and its expression walker misses several forms.~~ FIXED** — `walk_stmts` now covers `ForEach`; `walk_exprs_in_stmt` covers `For` bounds, `ForEach` iterables, and `Match` scrutinees; `walk_expr` covers `Match` arms, `StructInit`/`AnonStructInit` fields, `EnumVariant` data, `Slice`, `CallIndirect`, and the accessor nodes; `walk_calls` reports indirect calls so rule 4 sees their arguments (regression tests: `e2e::pointer_escape::{returning_a_local_pointer_from_a_for_each_body_is_rejected, address_of_a_local_in_a_for_bound_of_a_recursive_function_is_rejected}`).
- **~~SE-H3. Escape rule 2 misses stores through accessor-named fields.~~ FIXED** — `stores_beyond_the_frame` and `addr_of_target` peel the accessor nodes via `accessor_fields` (regression test: `e2e::pointer_escape::storing_a_local_pointer_through_an_accessor_named_field_is_rejected`).
- **SE-H4. Slice assignment to a `const` array bypasses the ROM-write check** (`stmt.rs:432-457`). `LUT[0..2] = [9, 9];` emits stores into ROM; `LUT[0] = 9;` is rejected. Peel `Expr::Slice` in `lvalue_root`.
- **SE-H5. Struct literals are not validated** (`expr.rs:122-137, 1033-1073`). `Point { x: true, z: 5 }` compiles; in the ROM path a 1-byte value in a 2-byte field shifts every later field — layout corruption. Unknown names and wrong types should error, and field values should get `expected_type` so literals adopt field width. Same gap in enum struct-variants.
- **SE-H6. `const` declarations never check the initializer against the declared type** (`register.rs:240-268`). `const C: u8 = "hello";` compiles; `const B: [u8; 2] = [300, 2];` silently truncates; `init.rs:168-174` truncates `[0; 5]` into `[u8; 2]` while `[0,0,0,0,0]` errors.
- **SE-H7. `for i: u8 in 0..300` compiles and runs 44 iterations** (`stmt.rs:468-546`). Bounds never checked against the counter type, constant or runtime.
- **SE-H8. `let x: i16 = -40000;` silently wraps to 25536** (`expr.rs:985-1009`) while `const X: i16 = -40000;` correctly errors.
- **SE-H9/H10.** False "unused import" for types used only in type position (`resolve_type` never records a use); enum tuple-variant payloads don't get `expected_type` (`E::V(5)` for `V(u16)` errors while `f(5)` works).
- **CG-H1. Interrupt save list omits `$E0–$EF`** (`item.rs:79-96`) — an NMI between indirect-arg staging and the callee's prologue copy destroys in-flight args if the handler itself calls indirectly.
- **CG-H2. Two peephole passes are flag-unsafe** (`peephole.rs:384-461`): `ORA #$00`/`AND #$FF`/`EOR #$00` and `TAX;TXA` removals don't consult flag liveness. Current codegen never emits the vulnerable sequences, but user `asm {}` flows through the same pipeline.
- **~~CG-H3. Silent `$20/$21` fallback in index assignment.~~ FIXED** — now a `CodegenError::Internal`; the `$20/$21` fallback was the very temp the adjacent comment explained could not hold the value.
- **CG-H4. Silent `$20-$23` hardcoding in slice materialization (runtime bounds)** (`stmt.rs:1757-1763`) — the comment admits complex bounds may clobber the parked `end`. Either restrict bounds to simple exprs with a hard error, or stage via the allocator.
- **CG-H5. Matches with ~15–20+ arms emit out-of-range branches** — fails at `flatasm` time (loud, good) but the compiler should invert-over-JMP like it does for `if`.
- **CG-H6. Tracking invalidation is per-callsite convention, not structural.** CG-C4 was the demonstrated instance and is fixed (`emit_inst` invalidates on `JSR`); raw `LDX`/`LDY` sites (`unary.rs:163,168`, `literal.rs:42-43`) have the same shape with no live instance demonstrated. Remaining work: invalidate by mnemonic for the load/store class too, or add tracked variants for every load/store and forbid raw ones outside the emitter.
- **FE-H1. Unary `-`/`!`/`~` don't bind postfix operators** (`parser/expr.rs:173-190`) — `-p.x` fails with "cannot apply '-' to type P". Self-acknowledged in a comment; `&`/`*` already do it right via `parse_postfix_with`.
- **FE-H2. Array sizes never range-checked** — `[u8; 4294967296]` hangs the compiler >2 min (static) or silently truncates to 0 bytes (local). The BSS-overflow machinery exists; the check must happen before materialization.
- **FE-H3. One statement error cascades into thousands.** Recovery exists only at item level; `synchronize()` stops *inside* the function body. Add statement-level recovery in `parse_block` and cap total errors.
- **FE-H4. Error carets miscount columns after multi-byte characters** (`span.rs:204-222` — char indices vs byte offsets).
- **DOCS-H1. The spec's shadowing section (`specification.md:333-354`) documents a feature the compiler rejects** (`duplicate symbol`). Also broken verbatim: `let LED: addr` at global scope (:283-306), `const PI_TIMES_100: u8 = 314;` (:209), uninitialized `let result: u8;` throughout the inline-asm chapter (:1987-2080), `array.length` (:1806), `fn process(data: [u8])` (:1709), and the string-comparison example (:1560-1565).
- **DOCS-H2. Parser bug found via doc verification: `if a == b { }` with an empty (or comment-only) block followed by another `if` fails to parse** — struct-literal-vs-block ambiguity, reproducible with `u8` variables. File and fix.
- **DOCS-H3. Struct-variant matching is documented as NOT IMPLEMENTED in three places but works and is tested** (spec :1004-1016, :1068, :1157 vs ROADMAP :192).
- **DOCS-H4. `std/README.md` documents pre-pointer signatures (`u16` instead of `&u8`) and a phantom `wait_for_interrupt()`**; omits 9 shipped functions.

### Tests

- **T-H1. Multidimensional arrays: spec claims support (`specification.md:1291-1311`), compiler rejects them, zero tests notice.** Either the spec or the implementation is wrong and the suite has no opinion.
- **T-H2. No CI whatsoever.** 956 tests and the fuzzer run only when someone remembers.
- **T-H3. Fuzzing covers only lex+parse** — the phases least likely to panic. A sema fuzzer (drive `wraith::sema::analyze` on parser-accepted input) is the highest-value addition; FE-C1 is exactly the class it would find.
- **T-H4. Error-message assertions too loose to catch regressions** (`"type"`, `"expected"`). `import_diagnostics.rs` (exact file:line:col) is the model; the rest should follow it or use golden snapshots.

---

## MEDIUM — quality, structure, efficiency

### Generated-assembly efficiency (verified in emitted asm)

- **CG-M1. Comparisons materialize a boolean, then conditions re-test it** — `if (x > 3)` emits 13 instructions where 7 suffice. Hottest pattern in the language; branch on flags when a compare feeds a condition directly.
- **CG-M2. Literal right operands round-trip through `$20`** — `CMP $20` after `STA $20` should be `CMP #$03` (4 bytes, 6 cycles per compare-with-constant).
- **CG-M3. `JMP` to the next instruction** after an `if` without `else` — trivial peephole.
- **CG-M4. Argument staging double-store for simple args** (`STA $F4; STA $42`) — skip staging for literal/variable args to non-recursive callees.
- **CG-M5. Interrupt prologue saves 60 zero-page bytes unconditionally** (~780 cycles/interrupt) — `frames.rs:199-202` hardcodes `save_scratch/save_math = true`; compute from the handler's reachable call graph. (Also note: nothing checks total interrupt stack usage against the 256-byte hardware stack — a handler preempting a deep main chain can overflow silently; sema M4.)
- **CG-M6/M7/M8.** stdlib math ROM over-allocation (~49 bytes across mul16/div16/mod16); copy loops unrolled per byte (6 code bytes per byte copied — a DEX/BNE loop pays off beyond ~3 bytes); `LDA #$00; LDY #$00` → `LDA #$00; TAY`.

### Structure & maintainability

- **The dominant defect pattern is per-form walkers and merge lists.** Fix structurally: one exhaustive recursion helper over `Stmt`/`Expr` used by all analysis walkers (a catch-all that recurses rather than returns `false`), and one `ImportedState` struct merged in a single statement so a new field can't be forgotten. Also make `SemanticAnalyzer::with_base_path` call `Self::new()` (two 45-field initializer copies today — sema M1).
- **Test-suite dedup:** `tests/error_tests.rs`, `tests/feature_tests.rs`, and `tests/codegen_tests.rs` substantially duplicate the consolidated `tests/lib.rs` suites (weaker, string-matching forms of the same tests). 20 of 21 tracked `.wr` fixtures in `tests/integration/` are unreferenced. `tests/common/fixtures.rs` is entirely dead.
- **String-matching pockets inside e2e** (`cpu_flags.rs`, much of `frames.rs`/`types.rs`/`control_flow.rs`/`memory.rs`) should become behavioral assertions where behavior is assertable.
- **Emulator fidelity:** `TestBus` is 64 KB of writable RAM — a wild store into ROM succeeds silently on the emulator; consider a ROM-protection option. The no-halt assertion is waived when devices are attached (`exec.rs:279-283`) — a hanging driver test passes unless it checks `Exec.halted`.
- **std coverage gaps:** `set_bit`/`clear_bit`/`test_bit`, `saturating_*`, `count_bits`, `reverse_bits`, `mem_read`/`mem_write`/`mem_jump` (the riskiest function in std), most of `intrinsics.wr` — untested.
- **FE-M1–M6:** `#[org(0x10000)]` silently truncates to `$0000`; oversized integer literals lex as "unexpected character"; flatasm silently truncates `.BYTE 300` and `LDA #$1234`, silently overwrites duplicate labels, wraps emission past `$FFFF`; lexer errors render as `LexError { ... }` debug output instead of source-context carets; sema reports only the first error while the parser reports unbounded thousands; user config mistakes (`default_section` missing, `end < start`) panic in `config/mod.rs`.
- **SE-M3/M4.** Dead/wrong helpers: `StructDef::calculate_size`/`EnumDef::calculate_size` unused and under-count nested structs; `SemanticAnalyzer.errors` is never pushed to; interrupt-save docs contradict the hardcoded behavior.
- **DOCS-M10–M20:** internal spec contradictions (glob imports, string comparison, "no implicit conversions", struct passing, ca65 output, keyword count); `#[interrupt]` gets a full handler prologue but is never installed in any vector — a silent trap that should error until wired; `WRAITH_STD_PATH` undocumented; `docs/TESTING.md` and `docs/FEATURES_REVIEW.md` from an earlier era (delete or refresh); `axon.cfg` is a stale cc65 config referenced by nothing — delete; `syntax_extension/` highlights nonexistent keywords and omits shipped ones; `examples/monitor_standalone.wr` (move in progress) doesn't compile; `std/math.wr` bit helpers emit 65C02-only instructions on an NMOS target and clobber zp `$20` inside the compiler's scratch pool.

---

## LOW — polish

- `address_allocator.rs` (188 lines) is dead; `TempAllocator::reset` is never called (and the "reset at function boundaries" comment at `aggregate.rs:1397` is false — runtime enum construction leaks pool bytes program-wide); `is_primary_free`, `TempAllocStats`, `ParseErrorKind::InvalidInteger`/`InvalidType` dead.
- u8 div/mod by zero loads a never-initialized zp byte while the comment says "leave A as-is".
- `-x as i16` parses as `(-x) as i16` (Rust/C parse `-(x as i16)`) — document or align.
- String limit error says 256; the limit is 255. `-true` type-checks as bool. `1_000` lexes as `1` + `_000`. CLI: `-v` is `--version`; `--help` writes to stderr.
- Match-arm pattern bindings allocate fresh frame slots per arm; siblings could share (cf. `loop_bound_free`).
- `tests/visibility_errors.rs` writes fixed filenames into the shared temp dir (parallel-run flakiness).
- Repo: committed `.DS_Store`; `fuzz/pGNi` empty AFL artifact committed; "Reclaim BSS" listed in both ROADMAP and TODO; gitignored local debris (`tests/str_*.asm`, `tests/integration/*.asm`/`*.o`) can be deleted; README typo "abstraciton"; spec revision history never updated.
- Duplication: six near-identical unsigned compare routines; ZeroPage/Absolute arms of `generate_index_assignment`; `generate_divide_i16`/`generate_modulo_i16`; the `emit_signed_lt` closure copied verbatim into two files. `stmt.rs` is 3,953 lines and wants a `store_value_to_slot(ty, loc)` helper.

---

## What's done well

- **The execution harness** (`tests/common/exec.rs` + device models): compile → assemble → emulate from the reset vector, asserting on post-execution memory, with a UART and VIA that have real side effects. Well beyond typical compiler test infra.
- **`emit_label` invalidating all register beliefs** is the right conservative default and silently prevents a whole class of tracking bugs; the flag-liveness fixpoint guarding the CMP peephole is principled and well unit-tested.
- **MMIO volatility is modeled properly** — every store/load-folding peephole pass consults it.
- **`src/sema/init.rs`** — the single type-driven flattener with an explicit fatal/non-fatal distinction is a model fix.
- **Frame coloring** (Tarjan SCCs, software-stack save/restore, deep-recursion warnings, interrupt-recursion rejection) and **two-phase placement** (upfront `#[org]` reservation, conflict diagnostics with spans, vector-table reservation) are solid engineering; the criticals there are measurement omissions, not design flaws.
- **`const_eval`** uses checked arithmetic throughout — the most overflow-resistant part of the tree.
- **README accuracy** (flag-for-flag CLI match, the honest flatasm-vs-ca65 explanation), completions embedded via `include_str!` (can't drift), and ROADMAP/TODO's bug-log discipline — every fixed bug has a regression test and a story.
- **Honest comments**: nearly every hardcoded scratch use has a comment explaining the convention, which made the audit tractable. The failures are where conventions aren't *enforced*, not where they're undocumented.

---

## Appendix: process recommendations

1. **CI (highest-value process change).** A single GitHub Actions workflow running `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, and a `cargo fuzz run` smoke pass would gate everything above.
2. **Compile the spec's examples as tests.** A doc-test-style harness that extracts ```rust blocks from specification.md and asserts they compile would have caught 8 of the HIGH doc bugs automatically, permanently.
3. **Sema-level fuzzing.** Point the existing cargo-fuzz harness at `sema::analyze` and at const-eval (string slicing in particular).
4. ~~Add a size-integrity assertion in codegen.~~ **Done** — `generate_function` now hard-errors when the real pass emits more than the measuring pass reserved.
5. **Error-message golden tests** following `import_diagnostics.rs`'s exact-position style for the top ~20 diagnostics.
