# Wraith Compiler - Development Roadmap

_Updated: January 2026_

This roadmap contains **unimplemented features only**. For a complete list of current features, see the [Language Specification](specification.md).

---

## 🟡 HIGH PRIORITY

### 1. Standard Library Expansion

**Missing Functions**:
- `mul16(a: u16, b: u16) -> u16` - 16-bit multiplication
- `div16(a: u16, b: u16) -> u16` - 16-bit division
- `strlen(s: *u8) -> u8` - Null-terminated string length
- `strcmp(a: *u8, b: *u8) -> i8` - String comparison
- `abs(x: i8) -> i8` and `abs16(x: i16) -> i16` - Absolute value

**Complexity**: Low-Medium (implementation work)

---

### 2. Tagged Enum Pattern Matching

**Current State**:
- Tuple variant creation works: `Option::Some(42)`
- Pattern matching code exists but is **minimally tested**
- Struct variant pattern matching explicitly **not implemented**

**Improvements Needed**:

#### 4.1 Complete Testing for Tuple Variant Pattern Matching

**What Works** (but needs testing):
```rust
enum Option {
    None,
    Some(u8),
}

match opt {
    Option::Some(value) => {  // Code exists, needs tests
        // Extract 'value' from enum
    }
    Option::None => { }
}
```

**Status**: Code generation exists in `src/codegen/stmt.rs:1188-1226` and semantic analysis in `src/sema/analyze/stmt.rs:524-572`, but **zero tests verify this works correctly**. High risk of bugs.

**Action Items**:
- Write comprehensive tests for single-field tuple variants
- Test multi-field tuple variants (e.g., `RGB(u8, u8, u8)`)
- Test with u16 fields (multi-byte data)
- Test nested pattern matching
- Test edge cases (zero fields, mismatched binding counts)

**Complexity**: Low (testing only, implementation exists)

#### 4.2 Implement Struct Variant Pattern Matching

**Currently Fails**:
```rust
enum Message {
    Move { x: u8, y: u8 },
}

match msg {
    Message::Move { x, y } => {  // ❌ Error: not implemented
        // Would extract x and y
    }
}
```

**Action Items**:
- Implement codegen for struct variant field extraction
- Add semantic analysis for named field bindings
- Handle field order independence
- Write comprehensive tests

**Complexity**: Medium (requires new codegen logic)

---

### 3. BCD Enhancements

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

#### 4.2 BCD String Conversion Helpers

**Missing Functions**:
- `bcd_to_string(value: b8) -> str` - Convert BCD to decimal string
- `string_to_bcd(s: *u8) -> b8` - Parse decimal string to BCD
- `bcd16_to_string(value: b16) -> str` - 16-bit version

**Benefit**: Display and parse BCD numbers for user interfaces
**Complexity**: Medium (string manipulation in 6502)

---

## 🟢 MEDIUM PRIORITY

### 4. Bitfield Access Syntax

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

### 5. Branch Optimization Intelligence

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

### 6. Disassembly Output Mode

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

### 7. Inline Data Directive

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

### 8. PRNG (Pseudo-Random Number Generator)

**Add to stdlib**:
- `rand_init(seed: u16)` - Initialize generator
- `rand_u8() -> u8` - Get random byte
- `rand_range(min: u8, max: u8) -> u8` - Random in range

**Complexity**: Low (algorithm implementation)

---

## PRIORITIZED PHASES

### Phase 1: Core Language & Safety
**Focus**: Essential language features and compile-time safety

1. **Tagged enum pattern matching** (tuple variant testing + struct variant implementation)

**Expected Impact**: Fewer runtime bugs, complete enum functionality

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
- Comprehensive warning system (9 warning types implemented)
- **Compile-time array bounds checking** (errors on constant out-of-bounds access)
- **BCD literal validation** (compile-time range checking for b8/b16 casts)
- **Address overlap warning** (warns when addr overlaps CODE/DATA sections)
- **Module visibility system** (pub keyword for explicit exports, private by default)

See [Language Specification](specification.md) for complete documentation of all implemented features.
