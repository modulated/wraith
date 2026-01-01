# Wraith Language - Features Roadmap

Updated: 2025-12-30

This document lists remaining features and improvements for the Wraith programming language.

---

## 1. CRITICAL: Unimplemented AST Features

### 1.1 Slice Type Support

**Status:** AST complete, minimal codegen
**Location:** `src/ast/types.rs:51-55`
**Description:** Slice types with length tracking
**Use Case:** Safe array access, string handling
**Priority:** MEDIUM

```wraith
fn process(slice: &[u8]) {
    for i: u8 in 0..slice.len {
        // ...
    }
}
```

---

## 2. Type System Improvements

### 2.1 Type Inference

**Status:** Not implemented
**Description:** Infer types from context where possible
**Use Case:** Less verbose code
**Priority:** LOW

```wraith
x := 10;  // Infer u8 from literal
```

---

## 3. Error Handling & Diagnostics

### 3.1 Error Recovery

**Status:** Parser stops at first error
**Description:** Continue parsing to find multiple errors
**Priority:** MEDIUM

### 3.2 Unnecessary Zero Page Allocation Warnings

**Status:** Not implemented
**Description:** Warn when variables are allocated to zero page unnecessarily
**Priority:** LOW

---

## 4. 6502-Specific Features

### 4.1 CPU Flags Access

**Status:** ✅ COMPLETE (2025-12-30)
**Description:** Direct access to processor status flags
**Priority:** MEDIUM

**Implemented Flags:**
- ✅ `carry` - Carry flag (unsigned overflow, multi-byte arithmetic)
- ✅ `zero` - Zero flag (result is zero)
- ✅ `overflow` - Overflow flag (signed arithmetic overflow)
- ✅ `negative` - Negative flag (bit 7 set, sign bit)

```wraith
if carry { /* handle overflow */ }
if zero { /* value is zero */ }
x: u8 = carry as u8;  // Convert to 0 or 1
```

---

## 5. Language Features

### 5.1 Bitfield Access

**Status:** Not implemented
**Description:** Bit manipulation syntax
**Priority:** MEDIUM

```wraith
// Option 1: bit() method
if value.bit(7) { }

// Option 2: range syntax
u8 nibble = value.bits[7:4];
```

### 5.2 Module System

**Status:** Only file imports exist
**Description:** Proper module hierarchy and visibility
**Priority:** MEDIUM

```wraith
// graphics.wr
pub fn draw_sprite() { }
fn internal_helper() { }  // private

// main.wr
mod graphics;
graphics::draw_sprite();
```

### 5.3 Public/Private Visibility

**Status:** Not implemented
**Description:** Control symbol visibility
**Priority:** MEDIUM

```wraith
pub fn public_api() { }
fn private_impl() { }
```

---

## 6. Memory Management

### 6.1 Memory Section Control

**Status:** Only `#[org]` exists
**Description:** Control memory layout
**Priority:** MEDIUM

```wraith
#[section("DATA")]
static mut buffer: [u8; 256];

#[section("BSS")]
static mut scratch: u8;
```

---

## 7. Optimization

### 7.1 Advanced Register Allocation

**Status:** Basic implementation (X for loops, Y for temps)
**Description:** Full register allocation with liveness analysis
**Priority:** MEDIUM

**Current:** X used for loop counters, Y for nested expression temps
**Desired:** Allocate frequently-used variables to registers based on liveness

### 7.2 Dead Code Elimination

**Status:** Not implemented
**Description:** Remove unreachable/unused code
**Priority:** MEDIUM

### 7.3 Strength Reduction

**Status:** ✅ COMPLETE (2025-12-30)
**Description:** Replace expensive ops with cheaper equivalents
**Priority:** MEDIUM

**Implemented Optimizations:**
- ✅ `x * (power of 2)` → `x << n` (e.g., `x * 8` → `x << 3`)
- ✅ `x / 256` → `x.high` (for u16 only)
- ✅ `x % 256` → `x.low` (for u16 only)

**Performance Impact:**
- Multiplication by power of 2: ~10x faster (shift vs multiply loop)
- Division/modulo by 256: Instant (single byte extraction)

### 7.4 Branch Optimization

**Status:** Not implemented
**Description:** Use optimal branching strategies
**Priority:** MEDIUM

**Examples:**
- Use zero flag instead of comparison when possible
- Reorder conditions for fewer branches
- Convert `if !x` to optimal flag check

---

## 8. Developer Experience

### 8.1 Test Coverage

**Status:** ✅ Excellent - 198 tests, organized by concern
**Location:** `tests/lib.rs`, `tests/{errors,integration,e2e}/`
**Description:** Comprehensive test suite with good organization
**Priority:** MEDIUM (continue expanding)

**Coverage:**
- Error tests (30 tests)
- Integration tests (24 tests)
- E2E tests (29 tests)
- Plus 126 legacy tests (to be consolidated)

### 8.2 Disassembly Output

**Status:** Not implemented
**Description:** Generate annotated assembly listing
**Priority:** MEDIUM

```asm
; main() at test.wr:5
$8000   LDA #$0A    ; u8 x = 10
$8002   STA $40     ; [2 cycles]
```

---

## 9. Standard Library

### 9.1 Math Functions

**Status:** Not implemented
**Description:** Common math operations
**Priority:** MEDIUM

```wraith
min(a, b), max(a, b), abs(x), clamp(x, lo, hi)
mul16(a, b) -> u16  // Optimized 16-bit multiply
div16(a, b) -> u16  // Optimized 16-bit divide
```

### 9.2 String Functions

**Status:** Length-prefixed strings exist, no functions
**Description:** String manipulation
**Priority:** MEDIUM

```wraith
str_cmp(a, b) -> bool
str_copy(dest, src)
str_concat(dest, src)
```

### 9.3 Bit Manipulation

**Status:** Not implemented
**Description:** Common bit operations
**Priority:** MEDIUM

```wraith
set_bit(value, bit) -> u8
clear_bit(value, bit) -> u8
test_bit(value, bit) -> bool
reverse_bits(value) -> u8
```

### 9.4 Random Numbers

**Status:** Not implemented
**Description:** PRNG for games
**Priority:** LOW

```wraith
u8 random();
u8 random_range(min, max);
```

---

## 10. Documentation

### 10.1 Language Reference Manual

**Status:** Missing
**Description:** Complete language specification
**Priority:** HIGH

**Should include:**
- Syntax grammar
- Type system rules
- Memory model
- Calling convention
- Standard library reference

---

## 11. Code Quality Issues

### 11.1 Parser Issues

**Status:** Mostly resolved
**Description:**
- ✅ ~~Enum patterns in match statements don't parse~~ → FIXED (2025-12-30)
- ✅ ~~Match arms now support simple expressions~~ → FIXED (2025-12-30)
- ✅ Named type variable declarations with lookahead → Working correctly
- ⬜ Error recovery (parser stops at first error) → Not implemented

**Priority:** MEDIUM (error recovery remaining)

---

## Priority Matrix

### HIGH Priority (Implement Soon)

1. ✅ ~~Language reference documentation~~ → In progress
2. ✅ ~~Semantic validation~~ → COMPLETE (2025-12-25)
3. ✅ ~~Parser bug fixes (match expressions)~~ → COMPLETE (2025-12-30)
4. ⬜ Error recovery in parser

### MEDIUM Priority (Nice to Have)

5. ⬜ Module system with visibility
6. ⬜ Memory section control
7. ⬜ Slice type support
8. ✅ ~~CPU flags access~~ → COMPLETE (2025-12-30)
9. ⬜ Bitfield access helpers
10. ⬜ Advanced register allocation
11. ⬜ Dead code elimination
12. ✅ ~~Strength reduction~~ → COMPLETE (2025-12-30)
13. ⬜ Branch optimization
14. ⬜ Standard library functions (math, string, bit manipulation)
15. ⬜ Disassembly output

### LOW Priority (Future)

16. ⬜ Type inference
17. ⬜ LSP support
18. ⬜ Simulator integration
19. ⬜ Fixed-point arithmetic
20. ⬜ Heap allocator
21. ⬜ Random number generation

---

## Recently Completed (2025-12-30)

### Type System & Code Generation
✅ **CPU Flags Access** - Direct access to 6502 processor status flags
  - `carry`, `zero`, `overflow`, `negative` keywords
  - Used as boolean expressions in conditionals
  - Convertible to u8 (0 or 1) for storage
  - Essential for overflow detection and multi-byte arithmetic

✅ **Strength Reduction Optimization** - Transform expensive operations into cheaper equivalents
  - `x * 2^n` → `x << n` (~10x faster than multiply loop)
  - `x / 256` → `x.high` (instant u16 byte extraction)
  - `x % 256` → `x.low` (instant u16 byte extraction)

✅ **Built-in `.low` / `.high` Accessors** - Efficient u16/i16 byte extraction (83% code reduction vs shifts)
```wraith
value: u16 = 0x1234;
low: u8 = value.low;   // Single LDA instruction
high: u8 = value.high; // Single LDA instruction
```

✅ **Automatic Type Promotion** - u8→u16, i8→i16, bool→u8 implicit conversions
✅ **16-bit Arithmetic** - Full multi-byte add/subtract with proper carry propagation
✅ **Register Conventions** - Standardized A=low, Y=high for u16 values
✅ **Assembly Output** - Trailing newlines for Unix convention

### Parser Improvements
✅ **Match Expression Bodies** - Match arms now accept simple expressions without requiring blocks
```wraith
match color {
    Color::Red => 1,        // Simple expression (new)
    Color::Green => { 2 },  // Block (still works)
}
```

✅ **Enum Pattern Parsing** - Full `Enum::Variant` syntax support verified working
✅ **Variable Declaration Lookahead** - Proper disambiguation already implemented

### Bug Fixes
✅ **Parameter Passing** - Fixed $80+ region allocation
✅ **Register State Tracking** - Invalidate A register after comparisons
✅ **Memory Layout** - Resolved temp storage collision (loop_end_temp moved to $22)
✅ **For-Loop Register Usage** - X for counters, Y for u16 high bytes

---

## Previously Completed (2025-12-25)

### Compiler Features
✅ **Break/Continue Statements** - Full implementation verified
✅ **Warning System** - Unused variables, parameters, imports, unreachable code, non-exhaustive matches
✅ **ForEach Loops** - Array iteration with `for item: u8 in array`
✅ **Peephole Optimization** - 6 optimization passes eliminating redundant operations
✅ **Semantic Validation** - Comprehensive duplicate detection (functions, structs, enums, fields, variants, parameters)

### Infrastructure
✅ **Test Suite Consolidation** - Organized by concern (errors, integration, e2e)
✅ **Common Test Infrastructure** - Shared harness, assertions, fixtures
✅ **Test Coverage** - Expanded from 126 to 198 tests with better organization

### Previously Completed (2025-12-20 - 2025-12-24)
✅ CPU Intrinsics Library
✅ Inline Function Expansion
✅ Compound Assignment Operators
✅ INC/DEC Optimization
✅ Interrupt Vector Generation
✅ Match Statements
✅ Struct & Enum Operations
✅ Pointer Operations
✅ Array Literals
✅ Type Casts
✅ Constant Folding
✅ Source Context in Errors
✅ Register State Tracking & Optimizations

---

## Notes

- Core language features are complete and solid
- Parser is robust with expression/block flexibility in match statements
- u16 handling is highly optimized with `.low`/`.high` accessors
- Type promotion eliminates unnecessary casts
- Focus areas: optimization, developer experience, standard library
- Target use cases: games, embedded systems, retro computing
- Test coverage is excellent and well-organized (271 tests, all passing)
- Semantic analysis is comprehensive with good error/warning reporting

---
