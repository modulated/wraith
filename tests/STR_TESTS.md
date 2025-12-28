# String (`str`) Type Test Suite

This directory contains comprehensive tests for the `str` primitive type added to the Wraith compiler.

## Test Files

### 1. `str_basic.wr` - Basic Type Support ✅
**Purpose**: Verify that the `str` type is recognized and can be used in basic scenarios.

**Tests**:
- Constant string declarations with `str` type
- Function parameters with `str` type
- Function return values with `str` type
- Local variable declarations with `str` type
- String literals as function arguments

**Example**:
```wraith
const GREETING: str = "Hello, World!";

fn test(msg: str) -> str {
    return msg;
}

fn main() {
    s: str = "Test";
    result: str = test(GREETING);
}
```

---

### 2. `str_len.wr` - String Length Access ✅
**Purpose**: Test the `.len` property for getting string length.

**Tests**:
- Accessing `.len` on local string variables
- Accessing `.len` on string constants
- Passing `.len` result to functions
- Using `.len` in expressions and assignments

**Expected Behavior**:
- `.len` returns `u16` (the length in bytes)
- Length is stored as the first 2 bytes of the string data
- Works with string literals, variables, and constants

**Example**:
```wraith
fn test() -> u16 {
    s: str = "Test";
    return s.len;  // Returns 4
}
```

---

### 3. `str_indexing.wr` - String Indexing ✅
**Purpose**: Test indexing strings with `s[i]` syntax.

**Tests**:
- Direct indexing on string variables
- Indexing on string constants
- Indexing with different index values
- Passing indexed characters to functions

**Expected Behavior**:
- Indexing returns `u8` (single byte)
- Index must be `u8` type
- No bounds checking (programmer's responsibility)
- Accesses bytes starting after the 2-byte length prefix

**Example**:
```wraith
fn main() {
    test_str: str = "Hello";
    ch1: u8 = test_str[0];  // 'H' = 0x48
    ch2: u8 = test_str[4];  // 'o' = 0x6F
}
```

---

### 4. `str_iteration.wr` - String Iteration Patterns ✅
**Purpose**: Demonstrate common string iteration use cases and patterns.

**Tests**:
- `sum_bytes()` - Iterate through all characters and sum byte values
- `find_char()` - Find first occurrence of a character (returns index or 0xFFFF)
- `count_char()` - Count occurrences of a character
- `str_eq()` - Compare two strings for equality
- `starts_with_char()` - Check if string starts with a character
- `ends_with_char()` - Check if string ends with a character

**Patterns Demonstrated**:
1. **Basic iteration loop**:
   ```wraith
   i: u16 = 0 as u16;
   loop {
       if i >= s.len { break; }
       // Process s[i as u8]
       i = i + 1 as u16;
   }
   ```

2. **Early exit on condition**:
   ```wraith
   loop {
       if i >= s.len { return not_found_value; }
       if s[i as u8] == target { return i; }
       i = i + 1 as u16;
   }
   ```

3. **Comparing lengths first**:
   ```wraith
   if a.len != b.len { return false; }
   // Then compare character by character
   ```

---

### 5. `str_edge_cases.wr` - Edge Cases and Special Characters ✅
**Purpose**: Test boundary conditions and special character handling.

**Tests**:
- **Empty strings**: `""` with length 0
- **Single character strings**: `"A"`
- **Escape sequences**: `\n`, `\t`, `\r`, `\0`, `\"`, `\\`
- **Long strings**: 89+ characters
- **Boundary access**: First and last character access
- **Zero-length iteration**: Loop that doesn't execute for empty strings
- **Special characters in literals**: Quotes, backslashes, etc.

**Edge Cases Covered**:
1. Empty string has `.len == 0`
2. Accessing first character when string might be empty (requires length check)
3. Accessing last character (requires `last_idx = s.len - 1`)
4. Escape sequences are stored as single bytes
5. Iteration over empty string should not execute loop body

**Example Special Cases**:
```wraith
const EMPTY: str = "";                    // len = 0
const NEWLINE: str = "Line1\nLine2";      // \n is one byte (0x0A)
const TAB: str = "Col1\tCol2";            // \t is one byte (0x09)
const QUOTE: str = "Say \"Hello\"";       // Escaped quotes
const BACKSLASH: str = "Path\\File";      // Escaped backslash
```

---

## Implementation Details

### String Memory Layout
Strings are stored in the DATA section with the following format:
```
[u16 length (little-endian)][byte0][byte1][byte2]...
```

For example, `"ABC"` is stored as:
```
0x03 0x00 0x41 0x42 0x43
```

### Type System
- **Type**: `str` (primitive type)
- **Internal representation**: `Type::String` in semantic analyzer
- **Runtime value**: 2-byte pointer to length-prefixed data
- **Lifetime**: Static (stored in DATA section)
- **Mutability**: Immutable

### Operations
| Operation | Syntax | Returns | Notes |
|-----------|--------|---------|-------|
| Length | `s.len` | `u16` | Number of bytes in string |
| Indexing | `s[i]` | `u8` | Byte at index `i`, no bounds check |

### Assembly Generation

**`.len` operation**:
```assembly
; Get string address in A:X
LDA string_ptr
STA $F0
STX $F1
; Load length (first 2 bytes)
LDY #$00
LDA ($F0),Y  ; Low byte
TAX
INY
LDA ($F0),Y  ; High byte
; Result: A=low, X=high
```

**`[i]` operation**:
```assembly
; Get string address
LDA string_ptr
STA $F0
STX $F1
; Skip length prefix (add 2)
LDA $F0
CLC
ADC #$02
STA $F0
LDA $F1
ADC #$00
STA $F1
; Get index in Y and load byte
LDA index
TAY
LDA ($F0),Y  ; Result in A
```

---

## Running the Tests

Compile any test file with:
```bash
./target/debug/wraith tests/str_<test_name>.wr
```

All tests should compile successfully with no errors. Warnings about unused variables are expected since these are test files demonstrating patterns.

**Quick test all files**:
```bash
for f in tests/str_*.wr; do
    echo "Testing $f"
    ./target/debug/wraith "$f" || echo "FAILED: $f"
done
```

---

## Test Coverage Summary

✅ Basic type declarations and usage
✅ String length access (`.len`)
✅ String indexing (`[i]`)
✅ Iteration patterns
✅ String comparison
✅ Character search and counting
✅ Empty string handling
✅ Escape sequences
✅ Boundary conditions
✅ Long strings
✅ Special characters

---

## Known Limitations

1. **No bounds checking**: Indexing beyond string length is undefined behavior
2. **No slicing**: Cannot extract substrings
3. **No concatenation**: Cannot combine strings at compile or runtime
4. **No mutation**: Strings are immutable
5. **Static lifetime only**: String literals only, no runtime string construction
6. **ASCII/bytes only**: No UTF-8 support, treats strings as byte arrays

These limitations are by design for a 6502 target with limited resources.
