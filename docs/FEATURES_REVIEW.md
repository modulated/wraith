# Wraith Language - Features Roadmap

Updated: 2025-12-25

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

**Status:** Not implemented
**Description:** Direct access to processor status flags
**Priority:** MEDIUM

```wraith
if carry { /* handle overflow */ }
if zero { /* value is zero */ }
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

**Status:** Not implemented
**Description:** Replace expensive ops with cheaper equivalents
**Priority:** MEDIUM

**Examples:**
- `x * 2` → `x << 1`
- `x / 256` → use high byte
- `x % 256` → use low byte

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

**Status:** Known issues
**Description:**
- Enum patterns in match statements don't parse (expects single colon instead of double colon)
- Named type variable declarations require lookahead disambiguation

**Priority:** MEDIUM

---

## Priority Matrix

### HIGH Priority (Implement Soon)

1. ✅ ~~Language reference documentation~~ → In progress
2. ✅ ~~Semantic validation~~ → COMPLETE (2025-12-25)
3. ⬜ Error recovery in parser
4. ⬜ Parser bug fixes (enum patterns, lookahead)

### MEDIUM Priority (Nice to Have)

5. ⬜ Module system with visibility
6. ⬜ Memory section control
7. ⬜ Slice type support
8. ⬜ CPU flags access
9. ⬜ Bitfield access helpers
10. ⬜ Advanced register allocation
11. ⬜ Dead code elimination
12. ⬜ Strength reduction
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

## Recently Completed (2025-12-25)

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
- Focus areas: optimization, developer experience, standard library
- Target use cases: games, embedded systems, retro computing
- Test coverage is excellent and well-organized
- Semantic analysis is comprehensive with good error/warning reporting

---
