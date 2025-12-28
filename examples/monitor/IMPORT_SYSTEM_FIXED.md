# Import System - FIXED

## Problem (Previously)
The Wraith compiler's import system only performed semantic analysis (type checking) but did not include imported code in the final assembly output. This caused undefined symbol errors during assembly:

```
Error: Symbol 'uart_putc' is undefined
Error: Symbol 'uart_init' is undefined
```

## Solution Implemented
Modified the compiler to collect and emit all code from imported modules in the generated assembly.

### Changes Made

#### 1. ProgramInfo Structure (`src/sema/mod.rs`)
Added `imported_items` field to track AST items from imported modules:

```rust
pub struct ProgramInfo {
    // ... existing fields ...
    /// Items from imported modules that need to be emitted in codegen
    pub imported_items: Vec<Spanned<crate::ast::Item>>,
}
```

#### 2. Semantic Analyzer (`src/sema/analyze.rs`)
Modified `process_import()` to:
- Collect ALL items from imported modules (not just explicitly imported symbols)
- Merge resolved_symbols from imported modules (so codegen can reference them)
- Merge folded_constants for constant expression evaluation
- Merge resolved_types for type information
- Merge function_metadata for inline functions

```rust
// Collect all items from the imported file for codegen
self.imported_items.extend(ast.items.clone());

// Merge ALL resolved_symbols from the imported module
for (span, symbol) in &imported_info.resolved_symbols {
    self.resolved_symbols.insert(*span, symbol.clone());
    // ... also add constants/addresses to symbol table ...
}
```

#### 3. Code Generator (`src/codegen/mod.rs`)
Updated to emit code in phases:
1. **Address declarations** (from both main and imported modules)
2. **Code from imported modules** (with deduplication)
3. **Code from main module** (with deduplication)
4. **String literals** (from all modules)
5. **Interrupt vector table**

Deduplication prevents emitting the same function multiple times when it's imported by multiple modules.

#### 4. Function Generation (`src/codegen/item.rs`)
Added check to skip inline functions (they're expanded at call sites):

```rust
// Skip code generation for inline functions
if let Some(metadata) = info.function_metadata.get(name) {
    if metadata.is_inline {
        return Ok(());
    }
}
```

This prevents assembly errors from functions like `nop` and `brk` that conflict with instruction mnemonics.

## How It Works Now

### Example: monitor.wr imports uart.wr

**monitor.wr:**
```wraith
import { uart_init, uart_putc, uart_newline } from "./uart.wr";

#[reset]
fn main() {
    uart_init();
    uart_putc(0x48);  // 'H'
    uart_newline();
    loop {}
}
```

**Generated Assembly (monitor.asm):**
```assembly
.SETCPU "65C02"

; Address declarations from imported modules
UART_LCR = $E003
UART_LSR = $E005
; ...

; ============================================================
; Code from imported modules
; ============================================================
.ORG $8000
uart_init:
    LDA #$80
    STA UART_LCR
    ; ... full function implementation ...
    RTS

uart_putc:
    ; ... full function implementation ...
    RTS

uart_newline:
    ; ... full function implementation ...
    RTS

; ============================================================
; Code from main module
; ============================================================
.ORG $80C0
main:
    JSR uart_init
    LDA #$48
    JSR uart_putc
    JSR uart_newline
    ; ... rest of main ...
```

## Usage

### Compile with Imports
```bash
./target/debug/wraith examples/monitor/monitor.wr
```

**Output:** `monitor.asm` includes all code from `uart.wr` and `intrinsics.wr`

### Assemble
```bash
ca65 examples/monitor/monitor.asm -o monitor.o
ld65 monitor.o -t none -o monitor.bin
```

**Result:** ✅ Assembles successfully with all symbols defined

## Benefits

1. **Modular Code** - Split code into logical modules (uart.wr, via.wr, etc.)
2. **Code Reuse** - Import shared functionality across multiple programs
3. **Type Safety** - Imports are type-checked during compilation
4. **Single Assembly File** - All required code in one .asm file for easy assembly
5. **Automatic Deduplication** - Transitive imports handled correctly

## Files Affected

- `src/sema/mod.rs` - Added imported_items field
- `src/sema/analyze.rs` - Modified process_import() to collect items and merge metadata
- `src/codegen/mod.rs` - Emit imported items before main module
- `src/codegen/item.rs` - Skip inline functions during emission
- `src/codegen/expr.rs` - Include imported_items when cloning ProgramInfo

## Test Cases

All test cases now pass:

✅ **test_import.wr** - Import address declarations from via.wr
✅ **test_import_uart.wr** - Import functions from uart.wr
✅ **monitor.wr** - Full monitor program with uart.wr + intrinsics.wr imports

## Breaking Changes

None - this is purely additive functionality. Existing code continues to work.

## Obsolete Workarounds

The following files are no longer needed:
- ❌ `monitor_standalone.wr` - Can now use imports instead of inlining everything
- ❌ `SOLUTION.md` - Import system now works correctly
- ❌ `IMPORT_ISSUE.md` - Issue is resolved

These files are kept for reference but monitor.wr with imports is now the recommended approach.

## Future Enhancements

Potential improvements:
- Explicit export lists (control what can be imported)
- Import aliases (`import { foo as bar }`)
- Wildcard imports (`import * from "module.wr"`)
- Circular import detection (partially implemented)
- Symbol visibility control (public/private)

---

**Status:** ✅ **WORKING** (as of 2025-12-28)

The Wraith import system now correctly includes all required code from imported modules in the final assembly output.
