# Wraith Compiler - Optimization Backlog

_Updated: July 2026_

Code-generation work that does not change the language. For language features
and the OS milestones driving them, see [ROADMAP.md](ROADMAP.md); for what the
language already does, see [specification.md](specification.md); for building
and running the compiler, see [README.md](README.md).

Nothing here is required for correctness — every item is about producing smaller
or faster code for programs that already compile.

---

## 65C02 Target Support

The 65C02 has instructions the NMOS 6502 lacks. Emitting them unconditionally
would break the classic part, so they need a target flag:

```
(no flag)  # Classic 6502 (NMOS) — the default
--cmos     # WDC 65C02
```

**The flag does not exist yet**; it gates everything in this section. The
generic path is what the compiler emits today, in every case below.

### Addressing modes

- [ ] `JMP (addr,X)` — indexed indirect jump

    Jump-table match dispatch (`src/codegen/stmt.rs`,
    `generate_match_jump_table`) currently loads the target through a zero-page
    vector:

    ```asm
    LDA match_0_jt,X
    STA $34
    LDA match_0_jt+1,X
    STA $35
    JMP ($34)
    ```

    On a 65C02 this is one instruction, `JMP (match_0_jt,X)`, and needs no
    zero-page pair at all — which also removes the collision that made the
    vector share an address with the enum payload pointer.

### New instructions

- [ ] `STZ addr` — store zero without disturbing A

    Currently `LDA #$00; STA addr`. Common in the reset handler, which zeroes
    every mutable `static` at startup.

- [ ] `BRA rel` — unconditional relative branch

    2 bytes against 3 for `JMP`, when the target is within -128..+127. Every
    loop back-edge and match arm exit is a candidate.

- [ ] `PHX`/`PLX`, `PHY`/`PLY` — push and pull X and Y directly

    Currently `TXA; PHA` and `PLA; TAX`, which also destroys A. The interrupt
    prologue and epilogue in `src/codegen/item.rs` do exactly this.

- [ ] `INC A`, `DEC A` — increment and decrement the accumulator

    Currently `CLC; ADC #$01` / `SEC; SBC #$01`.

- [ ] `TSB`/`TRB addr` — test and set/reset bits without a mask in A

- [ ] `SMB`/`RMB addr` — set/reset a single memory bit

- [ ] `BBR`/`BBS addr,rel` — branch on a memory bit

    These three are the natural lowering for the bitfield syntax proposed in
    [ROADMAP.md](ROADMAP.md); worth doing together with it.

---

## Code Size

- [ ] **Consolidate duplicate enum variant data**

    Two constructions of the same variant with the same payload emit the same
    bytes twice.

- [ ] **Move inline data out of the instruction stream**

    Const enum payloads are emitted inline with a `JMP` over them. Placing them
    in `DATA` removes the jump. Const arrays and string literals already
    allocate from `DATA` through `SectionAllocator`.

- [ ] **Automatic inlining of small functions**

    `#[inline]` is explicit only. A leaf function whose body is smaller than its
    call sequence is always worth inlining; the size is already measured in the
    first pass of `generate_function`.

- [ ] **Reclaim BSS from dropped statics**

    Dead statics are no longer emitted, but sema assigns their RAM addresses
    before liveness is known, so the space stays reserved. Ordering BSS
    allocation after the liveness walk would recover it.

---

## Future Considerations

- [ ] 65816 target support (16-bit mode)
- [ ] Optimization level flags (`-O0`, `-O1`, `-O2`)
- [ ] Size vs speed trade-off (`-Os`)

---

## Fixed Bugs

Kept for the record. Each was a silent miscompile or a hard error, and each now
has a regression test.

| Bug | Where | Fix |
| --- | --- | --- |
| Array parameter reserved the full array size in zero page while the caller passed a 2-byte pointer | `sema/analyze/mod.rs`, `codegen/item.rs` | Treat array parameters as pointers |
| Inline asm `{var}` resolved to an outer scope's variable when names collided | `sema/table.rs`, `codegen/stmt.rs` | Added `containing_function` to `SymbolInfo` and filtered lookups by it |
| Inline asm `{var}` resolved to the parameter-passing area instead of the variable | `codegen/` | Same fix as above |
| `str` arguments lost the high byte of their pointer | `codegen/expr/call.rs` | Count `Type::String` as 2 bytes and pass in A:X |
| Enum variants could only be built from constants, not variables | `codegen/expr/aggregate.rs` | Split into inline (ROM) and runtime (temp storage) construction |
| `a > b` was true when `a == b` | `codegen/expr/compare.rs` | `BEQ` now targets a false branch instead of falling through with the operand in A |
| u16 `x = x + 1` compiled to a bare `INC`, dropping the carry | `codegen/` | Restricted the optimization to single-byte types |
| Register tracking survived labels, so a value was assumed live across a branch target | `codegen/emitter.rs` | `emit_label` invalidates all register state |
| `div16` returned `$FFFF` for every input | `std/math.wr` | Rewritten as proper restoring division |
| `a[i + 2] = v` stored the index instead of the value | `codegen/stmt.rs` | Dedicated temp slot for the value |
| `let a: [u16; 3] = [0, 0, 0]` was rejected | `sema/analyze/expr.rs` | Element literals adopt the declared element type |
| Jump-table match dispatch overwrote the enum payload pointer | `codegen/memory_layout.rs` | Moved the jump vector clear of the pointer triple |
| Inline asm `{param}` never resolved inside `#[inline]` functions | `codegen/expr/call.rs` | Set the emitter's current function to the callee during expansion |
| Every function was allocated and `.ORG`-ed twice | `codegen/item.rs` | Removed a duplicated block whose warning had been silenced |
| Const arrays were emitted at a hardcoded `$C000`, outside the configured DATA section | `codegen/item.rs` | Allocate from `DATA` through `SectionAllocator` |
