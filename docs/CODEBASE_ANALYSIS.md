# Wraith Codebase Analysis & Recommendations

_Date: 2026-07-28. Method: five parallel deep audits (sema, codegen/emitted-asm, front-end/perf, test infrastructure, repo/docs). Every CRITICAL and most HIGH findings were reproduced against the built compiler and, where relevant, assembled with `flatasm` and/or run on the mos6502 emulator. Suite at time of audit: 956 tests, all passing — every bug listed below slips through it._

_Update: all 22 criticals and every HIGH item are now fixed (the struck-through entries), each with regression tests. MEDIUM and LOW items remain open._

## Executive summary

The project is in genuinely good shape architecturally: the emulator-backed e2e harness is exceptional for a project this size, the peephole's flag-liveness fixpoint is principled, frame coloring is well engineered, and the bug-log discipline in ROADMAP/TODO/test-file headers is excellent.

The audit nonetheless found **22 verified critical miscompiles/crashes on valid code** (**all 22 fixed after the audit** — see the struck-through entries; **all HIGH items are fixed as of 2026-07-30**). They cluster into a small number of root causes, which matters more than the count:

1. **Hand-enumerated AST walkers and merge lists miss new variants.** New expression forms (`ForEach`, `CallIndirect`, the `SliceLen`/`U16Low`/`U16High` accessor nodes) and new analyzer state (`accessor_fields`, `const_env`, `local_arrays`, `static_inits`, `unreachable_stmts`) were each added to *some* of the places that must know about them. This one pattern accounts for roughly half the criticals.
2. **Zero-page scratch conventions are documented but not enforced.** The `$F0–$F3` "high pool" is both allocated through `TempAllocator` and written directly by string/struct/index paths; pool-exhaustion fallbacks silently reuse live slots (`unwrap_or(0xF2)`). Accounts for most of the rest. *(Substantially fixed: all demonstrated sites now allocate, spill, or error — see CG-C3/C5/C6.)*
3. **Function-size measurement and emission can drift apart.** Two independent overlap corruptions (jump tables, reset-handler static inits) existed because `placement.rs::measure()` didn't emit exactly what `generate_function` emits. *(Fixed: a shared prologue helper, word counting in the emitter, and a hard "emitted bytes > measured bytes" internal error now guard this class.)*
4. **Docs and implementation drift in both directions.** The spec documents features that don't work (shadowing, multidimensional arrays), misses features that do (struct-variant matching), and several verbatim spec examples don't compile.

**Recommended fix order** (details in each section):

| # | Action | Kills |
|---|--------|-------|
| 1 | ~~All 22 audit criticals fixed~~ ~~CI + sema-level fuzzer~~ doc-examples-compiled-in-tests | prevents the next crop |
| 2 | Doc-examples-compiled-in-tests + error-message golden tests | keeps the fixed classes from regressing |

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

**~~CG-C8. Runtime-constructed enums are pointers into shared scratch — any two live enum values alias.~~ FIXED.**
An enum-typed local now gets a per-declaration data block in the same call-graph-colored RAM region local arrays use (sema `enum_blocks`, laid out by `finalize_frames`, merged across imports), and binding — both `let` and reassignment — copies the constructed bytes into it. The variable's slot points at its own block from then on, so `let e1 = mk(1); let e2 = mk(2); match e1` extracts 1, and `a = mk(3)` replaces only `a`. `SymbolInfo` carries a `decl_span` so a use site can find its declaration's block (regression tests: three in `e2e::enums`; two verified to fail with the fix reverted). Note: an enum *parameter* is still a bare pointer — passing `mk(1)` as an argument hands the callee scratch-backed storage; if that proves to matter in practice, the same copy needs to happen in argument marshaling.

**~~CG-C9. ForEach over large/wide arrays silently miscounts or misindexes.~~ FIXED.**
Sema rejects `for x in arr` past 255 elements (127 for u16 elements) with a message pointing at the workarounds, and the flat assembler now errors on any byte-mode operand that doesn't fit instead of truncating (`LDA #$1234`, a `CPX #$12C` from the next compiler bug of this shape) (regression tests: `e2e::control_flow::foreach_*`).

**~~CG-C10. Address-taken + tail-recursive function: loop restart jumps into the `$E0`-staging prologue — infinite loop.~~ FIXED.**
The tail-call loop label is now emitted *after* the function-pointer prologue, so an iteration no longer re-copies the stale `$E0` staging over the freshly updated parameters (regression test: `e2e::functions::a_tail_recursive_function_can_be_address_taken`, which looped forever before).

**~~CG-C11. Static zero-fill writes past the array for sizes > 256 not a multiple of 256.~~ FIXED.**
The fill is now one loop for full 256-byte pages plus a shorter loop for the trailing partial page, instead of one loop that ran the partial page's STA for a full 256 iterations (regression tests: `e2e::statics::a_zero_static_larger_than_a_page_does_not_overfill` and `..._of_exactly_one_page_keeps_its_single_loop`).

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

**~~FE-C1. Compiler panic: constant string slice on a non-UTF-8 boundary.~~ FIXED.**
Both constant-slice sites (`const_eval.rs`, `check_slice` in `analyze/expr.rs`) validate char boundaries and return a proper `SemaError`; a boundary-aligned slice of the same string compiles and reads back correctly (regression tests: `e2e::strings_slices::a_const_slice_*`, the first of which panicked the compiler before). The original entry follows.
`const_eval.rs:135` — byte indices into a Rust `String`. `"héllo"[0..2]` panics: `byte index 2 is not a char boundary`.
_Fix:_ operate on `s.as_bytes()` (6502 strings are bytes anyway — bytes are arguably the *right* semantics) or validate char boundaries with a proper error.

**~~FE-C2. Compound assignment evaluates the target multiple times.~~ FIXED (by rejection).**
The desugar still clones the target — semantically invisible for pure targets, but a call in the target ran its side effects three times. Compound-assignment targets containing a call are now a parse error pointing at the bind-first rewrite; pure targets (`arr[i] += 5`, `arr[i + 1] += 1`) keep working (regression tests: `e2e::operators::compound_assignment_*`). A real evaluate-once `Stmt::CompoundAssign` node remains the fuller fix if the restriction chafes.

---

## HIGH — wrong diagnostics, missing checks, fragile invariants

### Compiler

- **~~SE-H1/H2. Escape analysis never descends into `for…in` bodies, and its expression walker misses several forms.~~ FIXED** — `walk_stmts` now covers `ForEach`; `walk_exprs_in_stmt` covers `For` bounds, `ForEach` iterables, and `Match` scrutinees; `walk_expr` covers `Match` arms, `StructInit`/`AnonStructInit` fields, `EnumVariant` data, `Slice`, `CallIndirect`, and the accessor nodes; `walk_calls` reports indirect calls so rule 4 sees their arguments (regression tests: `e2e::pointer_escape::{returning_a_local_pointer_from_a_for_each_body_is_rejected, address_of_a_local_in_a_for_bound_of_a_recursive_function_is_rejected}`).
- **~~SE-H3. Escape rule 2 misses stores through accessor-named fields.~~ FIXED** — `stores_beyond_the_frame` and `addr_of_target` peel the accessor nodes via `accessor_fields` (regression test: `e2e::pointer_escape::storing_a_local_pointer_through_an_accessor_named_field_is_rejected`).
- **~~SE-H4. Slice assignment to a `const` array bypasses the ROM-write check~~ FIXED** — `lvalue_root` now peels `Expr::Slice` with the same reference check as `Index`/`Field`, so `LUT[0..2] = [9, 9];` is rejected like `LUT[0] = 9;` (regression test: `e2e::aggregate_init::assigning_to_a_const_array_slice_is_rejected`).
- **~~SE-H5. Struct literals are not validated~~ FIXED** — a shared `check_struct_init_fields` runs for named literals, anonymous literals and enum struct-variants: unknown field names and wrong types error, and each value is checked with the field's declared type as `expected_type` so literals adopt field width. The const/static flattener got the same checks for the path that never type-checks its initializer (regression tests: four in `e2e::aggregate_init`).
- **~~SE-H6. `const` declarations never check the initializer against the declared type~~ FIXED** — a string under a scalar name is a `TypeMismatch` at declaration; the flattener range-checks every integer element against its type, rejects an oversized fill count (`[0; 5]` into `[u8; 2]`) instead of clamping, and rejects a bool literal for a non-bool/non-u8 type (regression tests: three in `e2e::consts`).
- **~~SE-H7. `for i: u8 in 0..300` compiles and runs 44 iterations~~ FIXED** — constant bounds are checked by value and runtime bounds by type against the counter type (regression tests: `e2e::control_flow::for_loop_*`).
- **~~SE-H8. `let x: i16 = -40000;` silently wraps to 25536~~ FIXED** — a negated literal that fits no signed type is an error, matching the `const` form (regression test: `e2e::types::a_negative_let_initializer_beyond_i16_is_rejected`).
- **~~SE-H9/H10.~~ FIXED** — `resolve_type` records a use (`all_used_symbols`), so a type named only in type position is no longer a false "unused import"; enum tuple-variant payloads get `expected_type`, so `E::V(5)` for `V(u16)` works like `f(5)` (regression tests: `e2e::imports::a_type_used_only_in_type_position_is_not_an_unused_import`, `e2e::enums::tuple_variant_small_literal_adopts_the_payload_width`).
- **~~CG-H1. Interrupt save list omits `$E0–$EF`~~ FIXED** — the indirect-arg staging block is saved/restored under `save_scratch` (regression test: `e2e::interrupts::interrupt_handler_preserves_the_indirect_arg_staging_block`).
- **~~CG-H2. Two peephole passes are flag-unsafe~~ FIXED** — `ORA #$00`/`AND #$FF`/`EOR #$00` and the transfer-pair removals now consult the flag-liveness fixpoint (unit tests: four in `codegen::peephole::tests`).
- **~~CG-H3. Silent `$20/$21` fallback in index assignment.~~ FIXED** — now a `CodegenError::Internal`; the `$20/$21` fallback was the very temp the adjacent comment explained could not hold the value.
- **~~CG-H4. Silent `$20-$23` hardcoding in slice materialization (runtime bounds)~~ FIXED** — both bounds are spilled to the software stack immediately after evaluation, so a complex bound can't clobber the parked one; `$20-$23` only appear in the straight-line arithmetic after both are final (regression test: `e2e::strings_slices::slice_with_complex_runtime_bounds_keeps_both`, verified to fail with the fix reverted: length wrapped to 253).
- **~~CG-H5. Matches with ~15–20+ arms emit out-of-range branches~~ FIXED** — arm-ward branches in sequential matches invert over a JMP, the same shape `if` uses (regression test: `e2e::control_flow::a_large_match_assembles_and_picks_the_right_arm`, verified to fail pre-fix: "branch out of range (delta 129)").
- **~~CG-H6. Tracking invalidation is per-callsite convention, not structural.~~ FIXED** — `emit_inst` now mirrors every instruction's effect in the register tracker (loads set the parsed belief, stores invalidate the location, transfers transfer, arithmetic marks A unknown), so a raw `emit_inst("LDX", ...)` can no longer leave a stale belief; the tracked wrappers only refine it (unit tests: four in `codegen::emitter::tests`).
- **~~FE-H1. Unary `-`/`!`/`~` don't bind postfix operators~~ FIXED** — the operand now takes postfix suffixes via `parse_postfix_with`, like `&`/`*`; `as` still binds looser (`-x as i16` stays `(-x) as i16`) (regression tests: `e2e::operators::unary_*`).
- **~~FE-H2. Array sizes never range-checked~~ FIXED** — the parser caps element count at 65535 (the address space is 64 KiB), `resolve_type` checks total bytes, and an inline local larger than a frame slot errors instead of being clamped (regression tests: three in `e2e::types`).
- **~~FE-H3. One statement error cascades into thousands.~~ FIXED** — `parse_block` recovers per statement (record, synchronize, guaranteed progress) and total errors are capped at 50 (unit tests: two in `parser::tests`).
- **~~FE-H4. Error carets miscount columns after multi-byte characters~~ FIXED** — `offset_to_line_col` walks `char_indices()` (byte offsets) instead of counting chars against a byte offset (unit test: `ast::span::tests::carets_count_columns_past_multibyte_characters`).
- **~~DOCS-H1. The spec's shadowing section documents a feature the compiler rejects~~ FIXED** — the shadowing section now documents the duplicate-symbol rejection and why; the verbatim broken examples are fixed (`const LED: addr`, `PI_TIMES_100: u16`, `fn process(data: &[u8])`, `array.len`, initialized `let result: u8 = 0`). The string-comparison example compiles as of the CG-C5 fix.
- **~~DOCS-H2. Parser bug: `if a == b { }` with an empty block followed by another `if` fails to parse~~ FIXED** — a `no_empty_struct_literal` flag marks condition/scrutinee position (`if`/`while`/`for`/`match`), where a trailing `{` can only open the body; fresh delimiters (parens, call args, brackets) restore the literal reading (regression tests: three in `e2e::control_flow`).
- **~~DOCS-H3. Struct-variant matching is documented as NOT IMPLEMENTED but works and is tested~~ FIXED** — spec updated, including the stale "experimental" note on tuple-variant matching.
- **~~DOCS-H4. `std/README.md` documents pre-pointer signatures and a phantom `wait_for_interrupt()`~~ FIXED** — signatures corrected to `&u8`, the phantom removed, and the 13 omitted shipped functions documented.

### Tests

- **~~T-H1. Multidimensional arrays: spec claims support, compiler rejects, zero tests notice.~~ FIXED (by documentation + pinning)** — the spec now marks arrays of arrays NOT IMPLEMENTED with the flattened-indexing workaround, and `e2e::types::multidimensional_arrays_are_rejected_loudly` keeps the rejection loud. Implementing them remains a feature, not a fix.
- **~~T-H2. No CI whatsoever.~~ FIXED** — `.github/workflows/ci.yml` runs `cargo test --all-targets`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, and a compile check of the fuzz targets (they link only under `cargo afl`, so a real campaign stays local).
- **~~T-H3. Fuzzing covers only lex+parse~~ FIXED** — `fuzz/fuzz_targets/fuzz_sema.rs` drives `sema::analyze` on parser-accepted input, the phase where the panics actually live.
- **~~T-H4. Error-message assertions too loose to catch regressions~~ FIXED (for the weakest cases)** — the `"undefined"`/phase-only assertions in `error_tests.rs` now pin the exact message and line:col position, following `import_diagnostics.rs`. A full golden-snapshot migration for the top ~20 diagnostics remains a good follow-up.

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

1. ~~CI (highest-value process change).~~ **Done** — `.github/workflows/ci.yml` runs `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, and a fuzz-target compile check on every push and PR.
2. **Compile the spec's examples as tests.** A doc-test-style harness that extracts ```rust blocks from specification.md and asserts they compile would have caught 8 of the HIGH doc bugs automatically, permanently.
3. ~~Sema-level fuzzing.~~ **Done** — `fuzz/fuzz_targets/fuzz_sema.rs` points the existing AFL harness at `sema::analyze` (const-eval included, string slicing in particular).
4. ~~Add a size-integrity assertion in codegen.~~ **Done** — `generate_function` now hard-errors when the real pass emits more than the measuring pass reserved.
5. **Error-message golden tests** following `import_diagnostics.rs`'s exact-position style for the top ~20 diagnostics.
