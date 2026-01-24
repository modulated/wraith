# Wraith Compiler Optimization Roadmap

## 65C02 Target Support

The 65C02 processor has additional instructions that can improve code density and performance. These optimizations require a CLI flag to enable.

### CLI Flag

```
(no flag)  # Default: Classic 6502 (NMOS)
--cmos     # WDC 65C02 (CMOS)
```

### 65C02-Specific Optimizations

#### Addressing Modes

- [ ] `JMP (addr,X)` - Indexed indirect jump
    - Eliminates need for temporary storage in jump tables
    - Current: `LDA table,X; STA $30; LDA table+1,X; STA $31; JMP ($30)`
    - 65C02: `JMP (table,X)`

#### New Instructions

- [ ] `STZ addr` - Store zero directly
    - Current: `LDA #$00; STA addr`
    - 65C02: `STZ addr`

- [ ] `BRA rel` - Branch always (unconditional relative branch)
    - Saves 1 byte vs `JMP` for short distances (-128 to +127)
    - Current: `JMP label` (3 bytes)
    - 65C02: `BRA label` (2 bytes, if in range)

- [ ] `PHX/PLX`, `PHY/PLY` - Push/pull X and Y directly
    - Current: `TXA; PHA` / `PLA; TAX`
    - 65C02: `PHX` / `PLX`

- [ ] `INC A`, `DEC A` - Increment/decrement accumulator
    - Current: `CLC; ADC #$01` or `SEC; SBC #$01`
    - 65C02: `INC A` or `DEC A`

- [ ] `TSB/TRB addr` - Test and set/reset bits
    - Useful for bit manipulation without affecting other bits

- [ ] `SMB/RMB` - Set/reset memory bit (65C02 variants only)
- [ ] `BBR/BBS` - Branch on bit reset/set (65C02 variants only)

## Code Size Optimizations

- [ ] Consolidate duplicate enum variant data
- [ ] Move inline data to a data section (avoid JMP over data)
- [ ] Dead function elimination
- [ ] Inline small functions automatically

## Future Considerations

- [ ] 65816 target support (16-bit mode)
- [ ] Optimization level flags (`-O0`, `-O1`, `-O2`)
- [ ] Size vs speed trade-off options (`-Os` for size)

---

# Compiler Bugs

## 1. Array Parameter Offset Bug - FIXED

**Location:** `src/sema/analyze/mod.rs` and `src/codegen/item.rs`

**Description:**
When a function takes an array parameter, the callee reserved the full array size in zero-page for parameter offsets, but the caller only passes a 2-byte pointer.

**Fix applied:**

- Added `is_array_param` check in `src/sema/analyze/mod.rs` to treat array parameters as 2-byte pointers
- Updated `src/codegen/item.rs` to handle array types when calculating parameter offsets

---

## 2. Inline Asm Variable Scoping Bug - FIXED

**Location:** `src/sema/table.rs`, `src/sema/analyze/`, `src/codegen/stmt.rs`, `src/codegen/item.rs`

**Description:**
Variable references in inline asm `{varname}` resolved to variables from outer/caller scope instead of the current function's local scope when names collide.

**Fix applied:**

- Added `containing_function: Option<String>` field to `SymbolInfo`
- Set `containing_function` during semantic analysis for all local variables and parameters
- Updated `substitute_asm_vars` to filter symbols by current function
- Fixed temp emitter in `generate_function` to also set `current_function` for size calculation pass

---

## 3. Inline Asm Variable Substitution Bug - FIXED

**Location:** `src/codegen/` (inline asm variable resolution)

**Description:**
Variable references in inline asm sometimes resolve to the parameter passing area ($80+) instead of the actual variable storage location.

**Example:**

```wraith
fn main() {
    let buf_ptr: u16 = 0;  // Allocated at $44/$45
    asm {
        "LDA {buffer}",
        "STA {buf_ptr}",   // Resolves to $80 instead of $44
    }
}
```

**Fix applied:**
This bug was fixed by the same changes applied for Bug #2. The `containing_function` tracking in `SymbolInfo` ensures that:
- Local variables are correctly identified by their containing function
- When looking up `{varname}` in inline asm, the lookup filters by the current function
- Variables/parameters from other functions with the same name are not incorrectly matched

---

## 4. String Parameter Passing Bug - FIXED

**Location:** `src/codegen/expr/call.rs` (function call argument handling)

**Description:**
When passing a `str` type to a function, the high byte of the string pointer is not stored correctly.

**Example:**

```wraith
fn process_string(s: str) { ... }

fn main() {
    const MY_STRING: str = "Hello";
    process_string(MY_STRING);  // High byte not passed correctly
}
```

**Fix applied:**
- Added `Type::String` handling in `generate_call` function
- String parameters are now correctly counted as 2-byte types in the total_bytes calculation
- Added string-specific handling to store both low (A) and high (X) bytes to temp and param locations
- Strings use the same A:X register convention as enums for the 2-byte pointer

---

## 5. Enum Variant Construction with Variables - FIXED

**Location:** `src/codegen/expr/aggregate.rs` (enum variant expression handling)

**Description:**
Enum variants with data can only be constructed with constant values, not variables.

**Example:**

```wraith
enum Result {
    Some { value: u8 },
    None,
}

fn example(i: u8) -> Result {
    return Result::Some { value: i };  // ERROR: cannot use variable
    // return Result::Some { value: 5 };  // OK: constant works
}
```

**Fix applied:**
- Refactored `generate_enum_variant` into two paths:
  - `generate_enum_variant_inline`: For constant values, uses efficient inline data (data in ROM)
  - `generate_enum_variant_runtime`: For runtime values, allocates temp storage and generates code to write tag/data at runtime
- Added helper functions `is_variant_data_constant` and `is_expr_constant` to detect constant expressions
- Runtime path uses temp allocator's primary pool ($20-$3F) for enum data storage
- Returns pointer to temp storage (zero page, so high byte is always 0)
