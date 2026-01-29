# String Improvements Implementation Plan

## Overview

This plan implements ergonomic and performance improvements for string handling in Wraith, optimized for the 6502's limitations.

**Key Design Decision:** Strings are limited to 256 bytes maximum. This allows:
- Single-byte length prefix (saves 1 byte per string)
- Faster length operations (no 16-bit arithmetic)
- Simpler indexing (8-bit indices only)

---

## Implementation Tasks

### 1. Compile-Time String Concatenation

**Goal:** Allow string literals to be concatenated at compile time.

**Syntax:**
```rust
const MSG: str = "Hello, " + "World!";
const PATH: str = "data/" + "level" + ".txt";
```

**Implementation:**
- **Phase 1: Lexer** - Add `+` operator support between string literals
- **Phase 2: Parser** - Parse string concatenation expressions
- **Phase 3: Const Eval** - Evaluate concatenation at compile time
- **Phase 4: Validation** - Enforce 256-byte limit

**Files to modify:**
- `src/sema/const_eval.rs` - Add string concatenation evaluation
- `src/sema/analyze/expr.rs` - Type checking for string concat
- `src/codegen/` - Ensure concatenated strings are deduplicated

**Test cases:**
```rust
const A: str = "Hello";
const B: str = "World";
const C: str = A + " " + B;  // "Hello World"

// Error: exceeds 256 bytes
const TOO_LONG: str = "a".repeat(300);  // Compile error
```

---

### 2. Zero-Page String Pointer Caching

**Goal:** Cache frequently accessed string pointers in zero page for faster operations.

**Design:**
- Reserve 2-4 zero-page locations for "hot" string pointers
- Compiler tracks which strings are accessed most frequently
- First access loads pointer to cache, subsequent accesses use cache

**Implementation:**
- **Phase 1:** Add string access frequency analysis in sema
- **Phase 2:** Allocate cache slots in zero page
- **Phase 3:** Modify codegen to use cached pointers when beneficial

**Files to modify:**
- `src/sema/analyze/mod.rs` - Track string access patterns
- `src/codegen/memory_layout.rs` - Reserve cache slots
- `src/codegen/expr/` - Use cached pointers

**Example codegen:**
```assembly
; Without caching (current):
LDA #<str_label    ; 2 bytes
LDX #>str_label    ; 2 bytes
STA $F0            ; 2 bytes
STX $F1            ; 2 bytes
LDY index          ; 2 bytes
LDA ($F0),Y        ; 2 bytes (5 cycles)

; With caching (after):
; First access:
LDA #<str_label    ; Setup cache
LDX #>str_label
STA str_cache
STX str_cache+1

; Subsequent accesses:
LDY index          ; 2 bytes
LDA (str_cache),Y  ; 2 bytes (5 cycles, but no setup)
```

---

### 3. String Comparison Operators

**Goal:** Add `==` and `!=` operators for strings.

**Syntax:**
```rust
if name == "exit" { ... }
if input != "" { ... }
```

**Implementation:**
- **Phase 1:** Type system - Allow string comparisons in `==` and `!=`
- **Phase 2:** Codegen - Generate efficient comparison routine
- **Phase 3:** Optimization - Early exit on length mismatch

**Algorithm:**
```rust
fn string_compare(s1: str, s2: str) -> bool {
    if s1.len != s2.len {  // Quick reject
        return false;
    }
    for i in 0..s1.len {
        if s1[i] != s2[i] {
            return false;
        }
    }
    return true;
}
```

**Files to modify:**
- `src/sema/analyze/expr.rs` - Allow string types in comparison
- `src/codegen/expr/compare.rs` - Generate string comparison code

**Runtime routine:** (emitted once per compilation)
```assembly
strcmp:
    ; Compare lengths first (fast reject)
    LDY #$00
    LDA ($F0),Y
    CMP ($F2),Y
    BNE strcmp_ne
    
    ; Compare each byte
strcmp_loop:
    INY
    LDA ($F0),Y
    CMP ($F2),Y
    BNE strcmp_ne
    CPY length
    BNE strcmp_loop
    
strcmp_eq:
    LDA #$01
    RTS
    
strcmp_ne:
    LDA #$00
    RTS
```

---

### 4. String Slicing/Substring

**Goal:** Allow extracting substrings at compile time.

**Syntax:**
```rust
const FULL: str = "Hello, World!";
const PREFIX: str = FULL[0..5];     // "Hello"
const SUFFIX: str = FULL[7..];      // "World!" (to end)
const MIDDLE: str = FULL[3..8];     // "lo, W"
```

**Constraints:**
- Must be compile-time only (no runtime substring allocation)
- Indices must be constant expressions
- Result must fit in 256 bytes

**Implementation:**
- **Phase 1:** Parser - Add slice syntax for string literals
- **Phase 2:** Const eval - Validate indices and extract substring
- **Phase 3:** Error handling - Bounds checking at compile time

**Files to modify:**
- `src/parser/expr.rs` - Parse string slice syntax
- `src/sema/const_eval.rs` - Evaluate string slices
- `src/sema/analyze/expr.rs` - Type checking and validation

**Error cases:**
```rust
const S: str = "Hello";
const BAD1: str = S[0..10];    // Error: end exceeds length
const BAD2: str = S[3..2];     // Error: start > end
const BAD3: str = S[5..5];     // Error: empty string (maybe allow?)
```

---

### 5. String Iteration Sugar

**Goal:** Provide ergonomic syntax for iterating over string characters.

**Syntax:**
```rust
// Current verbose way:
for i in 0..msg.len {
    let c: u8 = msg[i];
    process(c);
}

// Suggested sugar:
for c in msg {  // c is u8
    process(c);
}

// With index:
for (i, c) in msg.enumerate() {
    buffer[i] = c;
}
```

**Implementation:**
- **Phase 1:** Parser - Recognize `for <var> in <string>` syntax
- **Phase 2:** Desugar - Convert to indexed loop during semantic analysis
- **Phase 3:** Codegen - Generate standard indexed loop

**Files to modify:**
- `src/parser/stmt.rs` - Parse string iteration
- `src/sema/analyze/stmt.rs` - Desugar to indexed loop

**Desugaring:**
```rust
// Input:
for c in msg {
    process(c);
}

// Desugared:
for i in 0..msg.len {
    let c: u8 = msg[i];
    process(c);
}
```

**Optimization:** Use X register as counter for 0-255 range instead of memory:
```assembly
    LDX #$00
loop:
    CPX msg_len
    BEQ done
    LDY #$02      ; Skip length byte
    TXA
    TAY
    LDA (msg_ptr),Y
    JSR process
    INX
    JMP loop
done:
```

---

### 6. String Literal Pooling Across Modules

**Goal:** Deduplicate string literals across all imported modules, not just within a single file.

**Current behavior:** Each file deduplicates its own strings
**Desired behavior:** Global string pool across all modules

**Implementation:**
- **Phase 1:** Track all strings during semantic analysis of imports
- **Phase 2:** Build global string table in `ProgramInfo`
- **Phase 3:** Modify codegen to emit global pool once

**Files to modify:**
- `src/sema/mod.rs` - Add global string table to `ProgramInfo`
- `src/sema/analyze/mod.rs` - Collect strings from imports
- `src/codegen/mod.rs` - Use global string pool

**Example:**
```rust
// file1.wr
pub const MSG: str = "Hello";

// file2.wr  
import { MSG } from "file1.wr";
const GREETING: str = "Hello";  // Same as MSG, should share storage

// Result: Only one "Hello" in final binary
```

**Memory savings:** Could save 10-20% on string data in multi-module projects.

---

### 7. String Format Change (256-byte limit)

**Goal:** Change string format from `[u16 length][data]` to `[u8 length][data]`.

**Rationale:**
- 6502 is an 8-bit processor - 16-bit operations are slow
- Strings > 256 bytes are rare on 6502 systems
- Saves 1 byte per string
- Simplifies all string operations

**Implementation:**
- **Phase 1:** Update `Type::String` documentation and size (still 2 bytes - pointer)
- **Phase 2:** Modify codegen to emit single-byte length
- **Phase 3:** Update all string operations (len, index, iteration)
- **Phase 4:** Add compile-time validation for 256-byte limit

**Files to modify:**
- `src/sema/types.rs` - Document new format
- `src/codegen/mod.rs` - Emit single-byte length
- `src/codegen/expr/` - Update len, index operations
- `src/sema/analyze/expr.rs` - Validate string size

**Format change:**
```assembly
; Old format:
str_0:
    .BYTE $14, $00   ; 2 bytes: u16 length (20)
    .BYTE $48, $65, $6C, $6C, $6F  ; "Hello"
    ; Total: 7 bytes

; New format:
str_0:
    .BYTE $14        ; 1 byte: u8 length (20)
    .BYTE $48, $65, $6C, $6C, $6F  ; "Hello"
    ; Total: 6 bytes
```

**Operations impact:**
```rust
// Length (was 16-bit, now 8-bit):
// Old: LDA ($F0),Y; TAX; INY; LDA ($F0),Y  (6 instructions)
// New: LDA ($F0),Y  (2 instructions)

// Indexing:
// Old: Add 2 to skip u16 length
// New: Add 1 to skip u8 length
```

---

## Implementation Order

1. **String format change (#7)** - Foundation for all other changes
2. **Compile-time concatenation (#1)** - Builds on new format
3. **String comparison (#3)** - Common operation
4. **String iteration sugar (#5)** - High ergonomic value
5. **String slicing (#4)** - Requires const eval
6. **Zero-page caching (#2)** - Performance optimization
7. **Cross-module pooling (#6)** - Global optimization

## Testing Strategy

For each feature:
1. Unit tests in `tests/` directory
2. Integration tests with `compile_success!` macro
3. End-to-end tests where applicable
4. Error case tests for validation

## Estimated Timeline

- String format change: 1-2 hours
- Compile-time concatenation: 2-3 hours
- String comparison: 2-3 hours
- String iteration: 1-2 hours
- String slicing: 2-3 hours
- Zero-page caching: 3-4 hours
- Cross-module pooling: 3-4 hours

**Total: ~14-21 hours of focused work**

---

## Success Criteria

- [ ] All existing tests pass
- [ ] New tests for each feature pass
- [ ] No regressions in code generation
- [ ] Documentation updated (language_spec.md)
- [ ] Examples demonstrating new features
