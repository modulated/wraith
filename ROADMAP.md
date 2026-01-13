# Wraith Compiler - Development Roadmap

_Updated: January 2026_

This roadmap contains **unimplemented features only**. For a complete list of current features, see the [Language Specification](specification.md).

---

## 🟡 HIGH PRIORITY

### 1. Module Visibility System

**Current State**: All imports are public, no privacy control
**Improvement**: Add `pub` keyword for functions, structs, enums, constants
**Benefit**: Better encapsulation, clearer API boundaries
**Complexity**: Medium (semantic analysis changes)

**Syntax**:
```wraith
pub fn exported_function() { }
fn internal_helper() { }  // Not visible to importers
```

---

### 2. Standard Library Expansion

**Missing Functions**:
- `mul16(a: u16, b: u16) -> u16` - 16-bit multiplication
- `div16(a: u16, b: u16) -> u16` - 16-bit division
- `strlen(s: *u8) -> u8` - Null-terminated string length
- `strcmp(a: *u8, b: *u8) -> i8` - String comparison
- `abs(x: i8) -> i8` and `abs16(x: i16) -> i16` - Absolute value

**Complexity**: Low-Medium (implementation work)

---

### 3. Compile-Time Array Bounds Checking

**Current State**: Only runtime checks (if any)
**Improvement**: Error on `array[10]` when array size is 5
**Benefit**: Catch bugs at compile time
**Complexity**: Medium (constant propagation in semantic analysis)

---

### 4. BCD Enhancements

#### 4.1 Peephole Optimization for SED/CLD

**Current State**: Each BCD operation generates individual SED/CLD pairs
**Improvement**: Combine consecutive BCD operations into one SED...CLD block
**Benefit**: Reduced code size and faster execution
**Complexity**: Low (add peephole pattern to optimizer)

**Example**:
```asm
; Current:
SED
ADC ...
CLD
SED        ; Redundant!
ADC ...
CLD

; After optimization:
SED
ADC ...
ADC ...
CLD
```

#### 4.2 Compile-Time BCD Literal Validation

**Current State**: Can cast any u8 to b8 without validation (e.g., `0xFF as b8`)
**Improvement**: Validate that BCD literals contain only valid decimal digits (0-9 per nibble)
**Benefit**: Catch invalid BCD values at compile time
**Complexity**: Low (add validation in cast analysis)

#### 4.3 BCD String Conversion Helpers

**Missing Functions**:
- `bcd_to_string(value: b8) -> str` - Convert BCD to decimal string
- `string_to_bcd(s: *u8) -> b8` - Parse decimal string to BCD
- `bcd16_to_string(value: b16) -> str` - 16-bit version

**Benefit**: Display and parse BCD numbers for user interfaces
**Complexity**: Medium (string manipulation in 6502)

---

### 5. Additional Compiler Warnings

#### 5.1 Address Overlap Warning

**Improvement**: Warn when `addr` location overlaps with compiler-generated memory sections
**Benefit**: Prevent memory corruption from overlapping allocations
**Complexity**: Low (add validation in semantic analysis)

**Example**:
```wraith
// wraith.toml specifies CODE section at $9000-$BFFF
addr MY_VAR = 0x9500;  // WARNING: Overlaps with CODE section
```

---

## 🟢 MEDIUM PRIORITY

### 6. Bitfield Access Syntax

**Current State**: Manual bit manipulation with shifts and masks
**Improvement**: Add `.bit(n)` accessor and bitfield syntax
**Benefit**: Cleaner code, fewer errors, better optimization
**Complexity**: Medium (parser + codegen)

**Syntax**:
```wraith
status.bit(7)              // Read bit 7
status.set_bit(7)          // Set bit 7
status.clear_bit(7)        // Clear bit 7
flags.bits[7:4]            // Access bits 7-4 (nibble)
```

---

### 7. Branch Optimization Intelligence

**Current State**: Status flags discarded after comparisons
**Improvement**: Track flag state and reuse for multiple conditionals
**Benefit**: Eliminate redundant CMP instructions
**Complexity**: High (complex dataflow analysis)

**Example**:
```wraith
if x > 5 {           // CMP x, #5
    foo();
}
if x > 5 {           // Could skip second CMP if x unchanged
    bar();
}
```

---

### 8. Disassembly Output Mode

**Current State**: Only assembly source output
**Improvement**: Generate annotated listing with addresses and cycle counts
**Benefit**: Performance analysis and debugging
**Complexity**: Medium (need to track addresses during codegen)

**Example Output**:
```
9000: A9 00     LDA #$00        ; [2 cycles] Load zero into A
9002: 85 40     STA $40         ; [3 cycles] Store to variable x
```

---

## 🔵 LOWER PRIORITY

### 9. Inline Data Directive

**Current State**: Data must be in static variables or string literals
**Improvement**: Allow inline data in functions
**Benefit**: Lookup tables, sprite data colocated with code
**Complexity**: Low (codegen addition)

**Syntax**:
```wraith
data lookup_table: [u8; 16] = [
    0x00, 0x01, 0x04, 0x09, 0x10, 0x19, ...  // Squares 0-15
];
```

---

### 10. PRNG (Pseudo-Random Number Generator)

**Add to stdlib**:
- `rand_init(seed: u16)` - Initialize generator
- `rand_u8() -> u8` - Get random byte
- `rand_range(min: u8, max: u8) -> u8` - Random in range

**Complexity**: Low (algorithm implementation)

---

## PRIORITIZED PHASES

### Phase 1: Core Language & Safety
**Focus**: Essential language features and compile-time safety

1. Module visibility system (`pub` keyword)
2. Compile-time array bounds checking
3. BCD literal validation
4. Address overlap warning

**Expected Impact**: Fewer runtime bugs, better APIs, safer code

---

### Phase 2: Performance & Optimization
**Focus**: Code generation improvements

1. BCD SED/CLD peephole optimization
2. Branch optimization intelligence
3. Standard library expansion (mul16, div16, abs, string functions)

**Expected Impact**: Faster code, more complete stdlib, better optimization

---

### Phase 3: Developer Experience
**Focus**: Ergonomics and tooling

1. Bitfield access syntax
2. Disassembly output mode
3. BCD string conversion helpers
4. Inline data directives
5. PRNG functions

**Expected Impact**: Cleaner code, better debugging, complete feature set

---

## Recently Completed ✅

For reference, these major features were completed in January 2026:
- Constant array optimization (const arrays emit to DATA section with .RES)
- Tail call optimization (recursive functions use JMP instead of JSR)
- Multi-dimensional array indexing (full support for `arr[i][j]` syntax)
- Comprehensive warning system (8 warning types implemented)

See [Language Specification](specification.md) for complete documentation of all implemented features.
