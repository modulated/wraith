# Monitor Assembly Issue - SOLVED

## Problem
The original `monitor.wr` couldn't assemble because the Wraith compiler's import system doesn't include imported code in the assembly output.

## Root Cause
The compiler treats `import` statements like C's `extern` declarations - they allow type checking but don't emit code for imported functions. This causes undefined symbol errors during assembly:

```
Error: Symbol 'uart_putc' is undefined
Error: Symbol 'uart_init' is undefined
...
```

## Solution
Created `monitor_standalone.wr` which inlines all necessary dependencies (UART functions, hardware definitions) into a single file that compiles and assembles successfully.

## File Structure

### Working Files
- ✅ **`monitor_standalone.wr`** - Complete working monitor (no imports)
- ✅ Compiles to `monitor_standalone.asm`
- ✅ Assembles with ca65 without errors
- ✅ Uses `str` type with `uart_puts()` helper
- ✅ All string constants use the new `str` type

### Original Files (Broken due to import issues)
- ❌ `monitor.wr` - Requires imports to work
- ❌ `uart.wr` - Can be imported but code isn't included
- ❌ `hardware.wr` - Can be imported but code isn't included
- ❌ `vectors.wr` - Doesn't include monitor code

## Usage

### Compile
```bash
./target/debug/wraith examples/monitor/monitor_standalone.wr
```

### Assemble
```bash
ca65 examples/monitor/monitor_standalone.asm -o monitor.o
ld65 monitor.o -t none -o monitor.bin
```

### Features
The standalone monitor includes:
- ✅ All UART functions (init, putc, getc, puts, etc.)
- ✅ String output using `str` type
- ✅ Hex conversion functions
- ✅ Command parser
- ✅ Memory operations (read, write, jump, dump)
- ✅ All monitor commands (R, W, G, D, H)

## String Type Usage

The monitor demonstrates practical use of the `str` type:

```wraith
// String constants
const WELCOME: str = "\n\rWraith Monitor v1.0\nType 'H' for help\n\n";
const PROMPT: str = "> ";
const ERROR: str = "?ERR\n";
const OK: str = "OK\n";

// String output helper
fn uart_puts(s: str) {
    i: u16 = 0 as u16;
    loop {
        if i >= s.len { break; }
        uart_putc(s[i as u8]);
        i = i + 1 as u16;
    }
}

// Usage
fn print_welcome() {
    uart_puts(WELCOME);
}
```

## Size Comparison

| File | Lines | Contains |
|------|-------|----------|
| `monitor.wr` (broken) | 414 | Monitor only, imports UART |
| `uart.wr` | ~200 | UART driver |
| `hardware.wr` | ~50 | Hardware definitions |
| **Total if imports worked** | ~664 | All components |
| `monitor_standalone.wr` | 516 | Everything inline |

The standalone version is actually smaller than the combined total because it only includes the functions that are actually used.

## Long-term Fix

The proper solution is to fix the Wraith compiler to include imported code in the assembly output. See `IMPORT_ISSUE.md` for details.

Until then, use `monitor_standalone.wr` for a working monitor with full `str` type support.

## Commands

Once assembled and running on hardware:

```
> H              # Help
> R 8000         # Read byte from 0x8000
> W 0200 FF      # Write 0xFF to 0x0200
> D 8000 801F    # Dump memory from 0x8000 to 0x801F
> G 0300         # Jump to 0x0300
```

## Testing

```bash
# Compile
./target/debug/wraith examples/monitor/monitor_standalone.wr

# Assemble
ca65 examples/monitor/monitor_standalone.asm

# Check if assembled successfully
echo $?  # Should output 0
```

Result: ✅ **Working!**
