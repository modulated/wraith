# Wraith Language - Features Review & Roadmap

Updated: 2025-12-20

This document lists remaining features and improvements for the Wraith programming language.

---

## 1. CRITICAL: Unimplemented AST Features

These features are already parsed by the compiler but have no code generation.

### 1.1 ForEach Loops

**Status:** AST complete, codegen missing
**Location:** `src/ast/stmt.rs:112-117`
**Description:** Iterate over arrays/slices
**Use Case:** Buffer processing, array operations
**Priority:** MEDIUM

```wraith
for u8 byte in buffer {
    process(byte);
}
```

### 1.2 Slice Type Support

**Status:** AST complete, minimal codegen
**Location:** `src/ast/types.rs:51-55`
**Description:** Slice types with length tracking
**Use Case:** Safe array access, string handling
**Priority:** MEDIUM

```wraith
fn process(slice: &[u8]) {
    for u8 i in 0..slice.len {
        // ...
    }
}
```

---

## 2. Type System Improvements

### 2.1 Fixed-Size Array Types

**Status:** AST exists, not used in semantic analysis
**Location:** `src/ast/types.rs:45-49`
**Description:** Support `[T; N]` array types in full semantic analysis
**Use Case:** Buffers, lookup tables, stack arrays
**Priority:** HIGH

```wraith
[u8; 256] screen_buffer;
```

### 2.2 Pointer Arithmetic

**Status:** Not implemented
**Description:** Add/subtract offsets from pointers
**Use Case:** Indirect indexed addressing, dynamic structures
**Priority:** MEDIUM

```wraith
*u8 ptr = 0xC000 as *u8;
*(ptr + 5) = 42;  // Write to 0xC005
```

### 2.3 Complete Named Type Resolution

**Status:** Incomplete (enums and structs work, but needs more comprehensive type checking)
**Description:** Full support for struct/enum types throughout semantic analysis
**Use Case:** Type checking, method resolution
**Priority:** HIGH

---

## 3. Error Handling & Diagnostics

### 3.1 Warning System

**Status:** Not implemented
**Description:** Non-fatal diagnostics for code quality
**Priority:** MEDIUM

**Suggested warnings:**

-   Unused variables
-   Unused imports
-   Unreachable code after return
-   Unnecessary zero page allocations
-   Non-exhaustive match patterns

### 3.2 Error Recovery

**Status:** Parser stops at first error
**Description:** Continue parsing to find multiple errors
**Priority:** MEDIUM

### 3.3 Source Context in Errors

**Status:** Only span positions shown
**Description:** Show source lines with error markers
**Priority:** HIGH

```
error: type mismatch
  --> test.wr:10:5
   |
10 |     LED = "hello";
   |           ^^^^^^^ expected u8, found string
```

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

### 4.2 NMI/IRQ/Reset Vectors

**Status:** `#[interrupt]` exists, no vector generation
**Description:** Automatic interrupt vector table
**Priority:** MEDIUM

```wraith
#[nmi]
fn vblank_handler() { }

#[irq]
fn timer_handler() { }

#[reset]
fn start() { }
```

---

## 5. Language Features

### 5.1 Constant Expressions

**Status:** Constant folding exists, but full const evaluation for addr calculations is incomplete
**Location:** `src/sema/analyze.rs:102-107`
**Description:** Compile-time expression evaluation for address declarations
**Priority:** HIGH

```wraith
const BASE = 0xC000;
const SIZE = 256;
addr SCREEN = BASE + SIZE;  // Currently not supported
```

### 5.2 Bitfield Access

**Status:** Not implemented
**Description:** Bit manipulation syntax
**Priority:** MEDIUM

```wraith
// Option 1: bit() method
if value.bit(7) { }

// Option 2: range syntax
u8 nibble = value.bits[7:4];
```

### 5.3 Module System

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

### 5.4 Public/Private Visibility

**Status:** Not implemented
**Description:** Control symbol visibility
**Priority:** MEDIUM

```wraith
pub fn public_api() { }
fn private_impl() { }
```

### 5.5 Compound Assignment Operators

**Status:** Tokens exist, not parsed
**Location:** `src/lexer/mod.rs:128-147`
**Description:** `+=`, `-=`, `*=`, etc.
**Priority:** MEDIUM

```wraith
x += 5;   // Instead of: x = x + 5;
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

### 7.1 Peephole Optimization

**Status:** Not implemented
**Description:** Local instruction pattern optimization
**Priority:** MEDIUM

**Examples:**

-   `LDA x; STA x` → remove
-   `LDA #0; CMP #0` → remove LDA, keep CMP
-   `LDA x; PHA; PLA` → remove if A not used

### 7.2 Register Allocation

**Status:** Not implemented (everything goes to memory)
**Description:** Keep hot variables in registers
**Priority:** HIGH

**Current:** All variables in zero page/absolute memory
**Desired:** Frequently-used values stay in A/X/Y

### 7.3 Dead Code Elimination

**Status:** Not implemented
**Description:** Remove unreachable/unused code
**Priority:** MEDIUM

### 7.4 Strength Reduction

**Status:** Not implemented
**Description:** Replace expensive ops with cheaper equivalents
**Priority:** MEDIUM

**Examples:**

-   `x * 2` → `x << 1`
-   `x / 256` → use high byte
-   `x % 256` → use low byte

### 7.5 Branch Optimization

**Status:** Not implemented
**Description:** Use optimal branching strategies
**Priority:** MEDIUM

**Examples:**

-   Use zero flag instead of comparison when possible
-   Reorder conditions for fewer branches
-   Convert `if !x` to optimal flag check

### 7.6 Inline Expansion

**Status:** `#[inline]` exists but not enforced
**Location:** `src/ast/item.rs:18`
**Description:** Actually inline functions marked `#[inline]`
**Priority:** MEDIUM

---

## 8. Developer Experience

### 8.1 Comprehensive Test Suite

**Status:** Partial - 51 tests exist but need more coverage
**Location:** `tests/*.rs`
**Description:** Full test coverage of language features
**Priority:** HIGH

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

## 9. Standard Library / Prelude

### 9.1 Memory Functions

**Status:** Not implemented
**Description:** Common memory operations
**Priority:** HIGH

```wraith
memcpy(dest, src, len);
memset(dest, value, len);
memcmp(a, b, len) -> bool;
```

### 9.2 Math Functions

**Status:** Not implemented
**Description:** Common math operations
**Priority:** MEDIUM

```wraith
min(a, b), max(a, b), abs(x), clamp(x, lo, hi)
mul16(a, b) -> u16  // Optimized 16-bit multiply
div16(a, b) -> u16  // Optimized 16-bit divide
```

### 9.3 String Functions

**Status:** Length-prefixed strings exist, no functions
**Location:** String literal support in parser
**Description:** String manipulation
**Priority:** MEDIUM

```wraith
str_cmp(a, b) -> bool
str_copy(dest, src)
str_concat(dest, src)
```

### 9.4 Bit Manipulation

**Status:** Not implemented
**Description:** Common bit operations
**Priority:** MEDIUM

```wraith
set_bit(value, bit) -> u8
clear_bit(value, bit) -> u8
test_bit(value, bit) -> bool
reverse_bits(value) -> u8
```

### 9.5 Random Numbers

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

-   Syntax grammar
-   Type system rules
-   Memory model
-   Calling convention
-   Standard library reference

---

## 11. Testing & Quality

### 11.1 Integration Tests

**Status:** Minimal
**Description:** Full programs that compile and run
**Priority:** HIGH

---

## 12. Code Quality Issues

### 12.1 Unsafe Static Mutables

**Status:** Existing code smell
**Location:** `src/sema/analyze.rs:139` - `static mut PARAM_COUNTER`
**Description:** Using static mut for allocation counter
**Priority:** HIGH

**Fix:** Use proper allocator struct passed through context

### 12.2 TODO Comments

**Status:** Several in codebase
**Locations:**

-   `src/sema/mod.rs:21` - "Add specific errors"
-   `src/sema/analyze.rs:106` - "Support constant expressions"
-   `src/sema/analyze.rs:137` - "Implement proper calling convention"

**Priority:** Review and resolve each TODO

### 12.3 Magic Numbers

**Status:** Hardcoded addresses throughout
**Examples:**

-   `0x50` - Parameter base address
-   `0x10` - Loop counter address
-   `0x40` - Temporary variable base

**Priority:** HIGH - Use named constants
**Fix:** Have a config somewhere that allows setting these

### 12.4 Error Handling

**Status:** Many `.map_err(|_| SemaError::SymbolNotFound)`
**Description:** Swallowing detailed error information
**Priority:** MEDIUM

### 12.5 Parser Issues

**Status:** Known issues
**Description:**
-   Enum patterns in match statements don't parse (expects single colon instead of double colon)
-   Named type variable declarations require lookahead disambiguation

**Priority:** MEDIUM

---

## Priority Matrix

### HIGH Priority (Implement Soon)

1. ⬜ Constant expressions - needed for addr calculations
2. ⬜ Register allocation - performance critical
3. ⬜ Fixed-size array types - complete semantic support
4. ⬜ Complete named type resolution
5. ⬜ Source context in error messages
6. ⬜ Memory functions (memcpy, etc.)
7. ⬜ Comprehensive test suite
8. ⬜ Language reference documentation
9. ⬜ Fix code quality issues (static mut, magic numbers)

### MEDIUM Priority (Nice to Have)

10. ⬜ ForEach loops
11. ⬜ Module system with visibility
12. ⬜ Compound assignment operators
13. ⬜ Peephole optimization
14. ⬜ Warning system
15. ⬜ Memory section control
16. ⬜ NMI/IRQ vector generation
17. ⬜ Parser improvements (enum patterns, lookahead)
18. ⬜ Slice type support
19. ⬜ Pointer arithmetic
20. ⬜ CPU flags access

### LOW Priority (Future)

21. ⬜ Type inference
22. ⬜ LSP support
23. ⬜ Simulator integration
24. ⬜ Fixed-point arithmetic
25. ⬜ Heap allocator

---

## Recently Completed (2025-12-20)

✅ **Match Statements** - Pattern matching with literals, ranges, wildcards, and enum variants
✅ **Struct Operations** - Complete with initialization, field access, and memory layout
✅ **Enum Operations** - Type definitions, variant construction, and match statement codegen
✅ **Pointer Operations** - Dereference and address-of operators
✅ **Array Literals** - Array initialization with `[1, 2, 3]` and `[0; 10]` syntax
✅ **Type Casts** - Explicit conversions between primitive types
✅ **Better Error Messages** - Detailed error types with user-friendly formatting
✅ **Zero Page Allocation Tracking** - Conflict detection and management
✅ **Constant Folding** - Compile-time expression evaluation and optimization

---

## Notes

-   The language has a solid foundation with lexer, parser, AST, semantic analysis, and codegen
-   Core features are now complete: structs, enums, match statements, constant folding
-   Focus should shift to optimization (register allocation), developer experience (error messages), and standard library
-   Consider which features align with the target use cases (games, embedded systems, retro computing)

---
