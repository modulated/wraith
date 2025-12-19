# Wraith Language - Features Review & Roadmap

Generated: 2025-12-19

This document lists potential features, improvements, and missing implementations for the Wraith programming language. Review and remove items that are not desired for the language's design goals.

---

## 1. CRITICAL: Unimplemented AST Features

These features are already parsed by the compiler but have no code generation. They likely exist from initial design but were never completed.

### 1.1 Match Statements ✅

**Status:** ✅ IMPLEMENTED
**Location:** `src/codegen/stmt.rs:250-344`
**Description:** Pattern matching with literals, ranges, wildcards, and enum variants
**Use Case:** State machines, input handling, protocol parsing
**Priority:** HIGH

```wraith
match input {
    0..=9 => handle_digit(),
    'a'..='z' => handle_letter(),
    _ => handle_other(),
}
```

### 1.2 ForEach Loops

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

### 1.3 Struct Operations 🔶

**Status:** 🔶 PARTIAL (field access done, initialization needs type info)
**Location:** `src/codegen/expr.rs:718-789`
**Description:** Struct initialization and field access
**Use Case:** Organizing related data, hardware register access
**Priority:** HIGH

```wraith
struct Point { u8 x, u8 y }
Point p = Point { x: 10, y: 20 };
u8 px = p.x;
```

### 1.4 Enum Operations ⬜

**Status:** ⬜ NOT IMPLEMENTED (needs type infrastructure)
**Location:** `src/ast/expr.rs:142-147`
**Description:** Enum variant construction and matching
**Use Case:** Type-safe state machines, error handling
**Priority:** HIGH

```wraith
enum State {
    Idle,
    Running { u8 speed },
    Error { u8 code },
}
State s = State::Running { speed: 5 };
```

### 1.5 Pointer Operations ✅

**Status:** ✅ IMPLEMENTED
**Location:** `src/codegen/expr.rs:76-127`
**Description:** Dereference (`*ptr`) and address-of (`&var`, `&mut var`)
**Use Case:** Indirect addressing, pointer manipulation, dynamic data structures
**Priority:** HIGH

```wraith
*mut u8 ptr = &mut data;
*ptr = 42;
```

### 1.6 Array Literals ✅

**Status:** ✅ IMPLEMENTED
**Location:** `src/codegen/expr.rs:310-377`
**Description:** Array initialization syntax: `[1, 2, 3]` and `[0; 10]`
**Use Case:** Lookup tables, sine tables, tile maps
**Priority:** HIGH

```wraith
[u8; 8] lookup = [0, 1, 4, 9, 16, 25, 36, 49];
[u8; 256] buffer = [0; 256];
```

### 1.7 Type Casts ✅

**Status:** ✅ IMPLEMENTED
**Location:** `src/codegen/expr.rs:912-998`
**Description:** Explicit type conversions between primitives
**Use Case:** u8↔u16 conversions, pointer casts
**Priority:** HIGH

```wraith
u16 addr = 0xC000;
u8 high = (addr >> 8) as u8;
```

### 1.8 Slice Type Support

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
**Description:** Support `[T; N]` array types
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

### 2.3 Named Type Resolution

**Status:** Incomplete
**Description:** Full support for struct/enum types in semantic analysis
**Use Case:** Type checking, method resolution
**Priority:** HIGH

---

## 3. Error Handling & Diagnostics

### 3.1 Better Error Messages ✅

**Status:** ✅ IMPLEMENTED
**Location:** `src/sema/mod.rs:16-220`, `src/sema/types.rs:50-79`, `src/sema/analyze.rs`
**Description:** Add detailed error types with context
**Priority:** HIGH

**Implemented error types:**

-   ✅ `UndefinedSymbol { name, span }`
-   ✅ `TypeMismatch { expected, found, span }`
-   ✅ `InvalidBinaryOp { op, left_ty, right_ty, span }`
-   ✅ `InvalidUnaryOp { op, operand_ty, span }`
-   ✅ `ArityMismatch { expected, found, span }`
-   ✅ `ImmutableAssignment { symbol, span }`
-   ✅ `CircularImport { path, chain }`
-   ✅ `ReturnTypeMismatch { expected, found, span }`
-   ✅ `ReturnOutsideFunction { span }`
-   ✅ `BreakOutsideLoop { span }`
-   ✅ `DuplicateSymbol { name, span, previous_span }`
-   ✅ `FieldNotFound { struct_name, field_name, span }`
-   ✅ `ImportError { path, reason, span }`
-   ✅ `OutOfZeroPage { span }`
-   ✅ `Custom { message, span }`

**Additional improvements:**
-   Added `Type::display_name()` method for user-friendly type formatting
-   Error messages now show types as "u8", "bool", etc. instead of "Primitive(U8)"
-   All error creation sites updated to use detailed error types with proper context

### 3.2 Warning System

**Status:** Not implemented
**Description:** Non-fatal diagnostics for code quality
**Priority:** MEDIUM

**Suggested warnings:**

-   Unused variables
-   Unused imports
-   Unreachable code after return
-   Unnecessary zero page allocations
-   Non-exhaustive match patterns

### 3.3 Error Recovery

**Status:** Parser stops at first error
**Description:** Continue parsing to find multiple errors
**Priority:** MEDIUM

### 3.4 Source Context in Errors

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

### 4.7 NMI/IRQ/Reset Vectors

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

### 5.1 Constant Expressions ⬜

**Status:** ⬜ NOT IMPLEMENTED (only integer literals)
**Location:** `src/sema/analyze.rs:102-107`
**Description:** Compile-time expression evaluation
**Priority:** HIGH

```wraith
const BASE = 0xC000;
const SIZE = 256;
addr SCREEN = BASE + SIZE;
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

### 5.7 Module System

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

### 5.8 Public/Private Visibility

**Status:** Not implemented
**Description:** Control symbol visibility
**Priority:** MEDIUM

```wraith
pub fn public_api() { }
fn private_impl() { }
```

### 5.9 Compound Assignment Operators

**Status:** Tokens exist, not parsed
**Location:** `src/lexer/mod.rs:128-147`
**Description:** `+=`, `-=`, `*=`, etc.
**Priority:** MEDIUM

```wraith
x += 5;   // Instead of: x = x + 5;
```

---

## 6. Memory Management

### 6.1 Zero Page Allocation Tracking ✅

**Status:** ✅ IMPLEMENTED
**Location:** `src/sema/analyze.rs:25-94` (ZeroPageAllocator)
**Description:** Track and manage zero page usage
**Priority:** HIGH

**Issues:**

-   No conflict detection
-   Manual address assignment
-   No usage reporting

### 6.2 Memory Section Control

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

### 7.2 Register Allocation ⬜

**Status:** ⬜ NOT IMPLEMENTED (everything goes to memory)
**Description:** Keep hot variables in registers
**Priority:** HIGH

**Current:** All variables in zero page/absolute memory
**Desired:** Frequently-used values stay in A/X/Y

### 7.3 Dead Code Elimination

**Status:** Not implemented
**Description:** Remove unreachable/unused code
**Priority:** MEDIUM

### 7.4 Constant Folding ✅

**Status:** ✅ IMPLEMENTED
**Location:** `src/sema/const_eval.rs`, `src/sema/analyze.rs:489-492`, `src/codegen/expr.rs:18-36`
**Description:** Evaluate constant expressions at compile time
**Priority:** HIGH

**Implemented features:**
- Evaluates arithmetic operations: `+`, `-`, `*`, `/`, `%`
- Evaluates bitwise operations: `&`, `|`, `^`, `<<`, `>>`
- Evaluates comparison operations: `==`, `!=`, `<`, `<=`, `>`, `>=`
- Evaluates logical operations: `&&`, `||`
- Evaluates unary operations: `-`, `~`, `!`
- Handles nested expressions
- Integration with codegen for optimized output

```wraith
u8 x = 2 + 3;  // Generates: LDA #5 (folded at compile time)
u16 addr = 0xC000 + 256;  // Generates: LDA #$00; LDX #$C1 (folded)
```

### 7.5 Strength Reduction

**Status:** Not implemented
**Description:** Replace expensive ops with cheaper equivalents
**Priority:** MEDIUM

**Examples:**

-   `x * 2` → `x << 1`
-   `x / 256` → use high byte
-   `x % 256` → use low byte

### 7.6 Branch Optimization

**Status:** Not implemented
**Description:** Use optimal branching strategies
**Priority:** MEDIUM

**Examples:**

-   Use zero flag instead of comparison when possible
-   Reorder conditions for fewer branches
-   Convert `if !x` to optimal flag check

### 7.7 Inline Expansion

**Status:** `#[inline]` exists but not enforced
**Location:** `src/ast/item.rs:18`
**Description:** Actually inline functions marked `#[inline]`
**Priority:** MEDIUM

---

## 8. Developer Experience

### 8.1 Comprehensive Test Suite

**Status:** Only 3 small tests
**Location:** `tests/*.wr`
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

### 11.3 Integration Tests

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

---

## Priority Matrix

### HIGH Priority (Implement Soon)

1. ✅ Match statements - critical for state machines
2. 🔶 Struct/enum codegen - language feels incomplete (struct field access done, init/enums partial)
3. ✅ Better error messages - detailed error types with user-friendly formatting
4. ✅ Pointer operations - essential for 6502
5. ✅ Array literals - lookup tables everywhere
6. ⬜ Constant expressions - needed for addr calculations
7. ✅ Type casts - u8↔u16 conversions common
8. ✅ Zero page allocation tracking - prevent conflicts
9. ⬜ Register allocation - performance critical
10. ✅ Constant folding - compile-time expression evaluation

### MEDIUM Priority (Nice to Have)

11. ⬜ ForEach loops
12. ⬜ Module system with visibility
13. ⬜ Volatile memory access
14. ⬜ Compound assignment operators
15. ⬜ Peephole optimization
16. ⬜ Warning system
17. ⬜ Memory section control
18. ⬜ Standard library (memcpy, etc.)
19. ⬜ NMI/IRQ vector generation
20. ⬜ Build system

### LOW Priority (Future)

21. ⬜ Type inference
22. ⬜ LSP support
23. ⬜ Simulator integration
24. ⬜ Fixed-point arithmetic
25. ⬜ Heap allocator

---

## Notes

-   The language has a solid foundation with lexer, parser, AST, semantic analysis, and basic codegen
-   Many features are "80% done" - they parse but don't generate code
-   Focus on completing existing AST features before adding new ones
-   Error messages need significant improvement for good developer experience
-   6502-specific optimizations are critical for performance
-   Consider which features align with the target use cases (games, embedded systems, retro computing)

---
