# Wraith Language - Features Review & Roadmap

Updated: 2025-12-25

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

### 3.1 Warning System

**Status:** ✅ COMPLETE (2025-12-25)
**Location:** `src/sema/mod.rs`, `src/sema/analyze.rs`, `src/main.rs`
**Description:** Non-fatal diagnostics for code quality
**Priority:** MEDIUM

**Implemented warnings:**

-   ✅ Unused variables - Detects variables that are declared but never used
-   ✅ Unreachable code - Detects code after return/break/continue statements
-   ✅ Unused function parameters - Detects parameters that are never used (skips `_`-prefixed names)
-   ⬜ Unused imports
-   ⬜ Unnecessary zero page allocations
-   ⬜ Non-exhaustive match patterns

Warnings are displayed with full source context (file, line, column) similar to error messages. Parameters starting with `_` are excluded from unused parameter warnings (common convention for intentionally unused parameters).

### 3.2 Error Recovery

**Status:** Parser stops at first error
**Description:** Continue parsing to find multiple errors
**Priority:** MEDIUM


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

**Status:** ✅ COMPLETE (2025-12-25)
**Description:** Automatic interrupt vector table generation
**Priority:** MEDIUM

```wraith
#[nmi]
fn vblank_handler() { }

#[irq]
fn timer_handler() { }

#[reset]
fn start() { }
```

Generated vector table is automatically emitted at $FFFA-$FFFF.

---

## 5. Language Features


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

**Status:** ✅ COMPLETE (2025-12-25)
**Location:** `src/parser/stmt.rs:395-450`
**Description:** `+=`, `-=`, `*=`, `/=`, `%=`, `&=`, `|=`, `^=`, `<<=`, `>>=`
**Priority:** MEDIUM

```wraith
x += 5;   // Expands to: x = x + 5;
counter += 1;  // Optimized to INC when applicable
```

Parser expands compound assignments to binary operations, enabling optimization passes.

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

### 7.2 Advanced Register Allocation

**Status:** Basic implementation complete (X for loops, Y for temps, register state tracking)
**Description:** Full register allocation with liveness analysis
**Priority:** MEDIUM

**Current:** X used for loop counters, Y for nested expression temps
**Desired:** Allocate frequently-used variables to registers based on liveness

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

**Status:** ✅ COMPLETE (2025-12-25)
**Location:** `src/codegen/expr.rs:70-170`
**Description:** Full inline function expansion for `inline fn` functions
**Priority:** MEDIUM

Functions marked with `inline fn` are fully expanded at call sites with zero overhead. The function body is emitted inline instead of generating JSR/RTS. Used extensively in the CPU intrinsics library.

### 7.7 INC/DEC Optimization

**Status:** ✅ COMPLETE (2025-12-25)
**Location:** `src/codegen/stmt.rs:91-142`
**Description:** Pattern-based optimization for increment/decrement
**Priority:** MEDIUM

Automatically detects `x = x + 1`, `x += 1`, `x = x - 1`, and `x -= 1` patterns and generates optimized INC/DEC instructions instead of LDA/ADC/STA or LDA/SBC/STA sequences. Provides ~40-50% cycle reduction (5-6 cycles vs 8-11 cycles).

```wraith
counter += 1;  // Generates: INC $40
value = value - 1;  // Generates: DEC $41
```

---

## 8. Developer Experience

### 8.1 Expanded Test Coverage

**Status:** Good - 96 tests passing covering core features
**Location:** `tests/*.rs`
**Description:** Add more edge case and integration tests
**Priority:** MEDIUM

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

### 9.1 CPU Intrinsics

**Status:** ✅ COMPLETE (2025-12-25)
**Location:** `std/intrinsics.wr`
**Description:** Zero-overhead wrappers for 6502 CPU control instructions
**Priority:** MEDIUM

All functions are marked `inline` for zero overhead - they expand to single instructions at the call site.

```wraith
import { enable_interrupts, disable_interrupts } from "intrinsics.wr";

disable_interrupts();  // Inlined to: SEI
// Critical section
enable_interrupts();   // Inlined to: CLI
```

**Available intrinsics:**
- Interrupt control: `enable_interrupts()` (CLI), `disable_interrupts()` (SEI)
- Carry flag: `clear_carry()` (CLC), `set_carry()` (SEC)
- Decimal mode: `clear_decimal()` (CLD), `set_decimal()` (SED)
- Other: `clear_overflow()` (CLV), `nop()` (NOP), `brk()` (BRK), `wait_for_interrupt()`

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

### 12.1 Parser Issues

**Status:** Known issues
**Description:**
-   Enum patterns in match statements don't parse (expects single colon instead of double colon)
-   Named type variable declarations require lookahead disambiguation

**Priority:** MEDIUM

---

## Priority Matrix

### HIGH Priority (Implement Soon)

1. ⬜ Language reference documentation
2. ⬜ Semantic validation (duplicate functions, undefined variables at sema phase)

### MEDIUM Priority (Nice to Have)

10. ⬜ ForEach loops
11. ⬜ Module system with visibility
12. ⬜ Peephole optimization
13. ⬜ Memory section control
15. ⬜ Parser improvements (enum patterns, lookahead)
16. ⬜ Slice type support
17. ⬜ CPU flags access
18. ⬜ Bitfield access helpers

### LOW Priority (Future)

21. ⬜ Type inference
22. ⬜ LSP support
23. ⬜ Simulator integration
24. ⬜ Fixed-point arithmetic
25. ⬜ Heap allocator

---

## Recently Completed

### 2025-12-20
✅ **Match Statements** - Pattern matching with literals, ranges, wildcards, and enum variants
✅ **Struct Operations** - Complete with initialization, field access, and memory layout
✅ **Enum Operations** - Type definitions, variant construction, and match statement codegen
✅ **Pointer Operations** - Dereference and address-of operators
✅ **Array Literals** - Array initialization with `[1, 2, 3]` and `[0; 10]` syntax
✅ **Type Casts** - Explicit conversions between primitive types
✅ **Better Error Messages** - Detailed error types with user-friendly formatting
✅ **Zero Page Allocation Tracking** - Conflict detection and management
✅ **Constant Folding** - Compile-time expression evaluation and optimization

### 2025-12-22
✅ **Source Context in Errors** - Error messages now show source code with --> and | markers
✅ **Constant Expressions for addr** - Full compile-time evaluation for address calculations
✅ **Code Quality Fixes** - Removed static mut, added memory layout abstraction, no magic numbers
✅ **Pointer Arithmetic** - Full support for pointer offset calculations
✅ **Memory Functions** - memcpy, memset, memcmp in standard library
✅ **Std Library Path** - Proper search path and portable imports
✅ **Fixed-Size Array Types** - Complete semantic analysis support for [T; N]
✅ **Named Type Resolution** - Full support for struct/enum types throughout compiler
✅ **Comprehensive Test Suite** - 96 tests covering all major features
✅ **Basic Register Allocation** - X for loop counters, Y for temps, register state tracking
✅ **Register Optimizations** - Store-load elimination, smart evaluation order, optimized loads
✅ **Variable Scoping Fixes** - Proper span-based symbol resolution for local variables

### 2025-12-25
✅ **CPU Intrinsics Library** - Zero-overhead inline wrappers for 6502 control instructions (CLI, SEI, CLC, SEC, CLD, SED, CLV, NOP, BRK)
✅ **Inline Function Expansion** - Full implementation of `inline fn` with zero overhead, body expanded at call sites
✅ **Compound Assignment Operators** - Support for +=, -=, *=, /=, %=, &=, |=, ^=, <<=, >>=
✅ **INC/DEC Optimization** - Pattern-based optimization detecting x += 1 and x -= 1, generating INC/DEC (~40-50% cycle reduction)
✅ **Interrupt Vector Generation** - Automatic generation of 6502 vector table at $FFFA-$FFFF for #[nmi], #[irq], #[reset]
✅ **Import Function Metadata Fix** - Inline functions from imported modules now work correctly
✅ **Compiler Output Improvements** - Cargo-style colored output with timing information
✅ **Register State Tracking Fix** - Fixed binary operation bug that caused incorrect optimization
✅ **Break/Continue Statements** - Verified full implementation with loop context tracking (parser, sema, codegen)
✅ **Warning System** - Non-fatal diagnostics for unused variables, unused parameters, and unreachable code with source context

---

## Notes

-   The language has a solid foundation with lexer, parser, AST, semantic analysis, and codegen
-   Core features are now complete: structs, enums, match statements, constant folding
-   Focus should shift to optimization (register allocation), developer experience (error messages), and standard library
-   Consider which features align with the target use cases (games, embedded systems, retro computing)

---
