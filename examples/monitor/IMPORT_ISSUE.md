# Import System Issue

## Problem

The Wraith compiler's `import` statement does not actually include the imported code in the assembly output. Imports only work for semantic analysis (type checking), but the codegen phase does not emit code for imported functions.

## Current Behavior

When compiling `monitor.wr`:

```wraith
import { uart_putc } from "./uart.wr";

fn main() {
    uart_putc(0x41);
}
```

The generated assembly contains:
```assembly
main:
    JSR uart_putc  ; ← uart_putc is NOT defined in this file
```

This causes assembly errors:
```
Error: Symbol 'uart_putc' is undefined
```

## Root Cause

The compiler treats imports like C's `extern` declarations - they declare that a symbol exists somewhere else, but don't include the actual implementation. However, Wraith doesn't have a linker that can resolve these external symbols.

## Solutions

### Option 1: Fix the Compiler (Recommended)

Modify the codegen phase to emit code for all imported functions and their transitive dependencies.

**Implementation**:
1. Track all imported modules during semantic analysis
2. During codegen, emit code for:
   - The main module's functions
   - All imported functions
   - All transitively imported functions
3. Deduplicate if the same function is imported multiple times

**Files to modify**:
- `src/codegen/mod.rs` - Add import resolution
- `src/sema/analyze.rs` - Track imported modules
- `src/sema/mod.rs` - Store import graph in `ProgramInfo`

### Option 2: Create Monolithic Files

Inline all dependencies into a single file. This works but defeats the purpose of having a module system.

**Example**: Create `monitor_standalone.wr` with all UART code inlined.

### Option 3: Multi-file Assembly + Linker

Generate separate `.asm` files and use a linker (ld65) to combine them.

**Challenges**:
- Need to handle name mangling
- Need to mark imports as `.import` directives in ca65 syntax
- Need linker configuration files

## Temporary Workaround

For now, the monitor can't use the `str` type's `uart_puts()` helper because it depends on the imported `uart_putc()` function.

**Revert to character-by-character output** until the import system is fixed.

## Impact on `str` Type

The `str` type itself works perfectly. The issue is specifically with the `uart_puts()` helper function that was added to the monitor, which calls the imported `uart_putc()`.

**The `str` type features that work**:
- ✅ String constants with `str` type
- ✅ `.len` property
- ✅ `[i]` indexing
- ✅ String iteration patterns

**What doesn't work in monitor**:
- ❌ `uart_puts()` helper (depends on imported `uart_putc`)

## Recommendation

**Short term**: Revert monitor.wr to not use `uart_puts()`, or inline UART code.

**Long term**: Implement proper import code inclusion in the compiler.
