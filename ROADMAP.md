# Wraith Compiler - Improvement Roadmap

_Updated: January 2026_

---

## CURRENT FEATURES SUMMARY

### 1. LANGUAGE FEATURES ✅

#### Data Types

-   **Primitives**: u8, i8, u16, i16, bool, b8/b16 (BCD)
-   **Composite**: Pointers (*T, *mut T), Arrays ([T; N]), Slices (&[T]), Structs, Enums (Unit/Tuple/Struct variants)
-   **Special**: Length-prefixed strings

#### Variables & Constants

-   Local variables with zero-page hint (`zp`)
-   Static variables and compile-time constants
-   Memory-mapped I/O (`addr NAME = 0x6000;`)

#### Functions

-   Standard function declarations with parameters and return types
-   **Attributes**: `#[inline]`, `#[noreturn]`, `#[interrupt]`, `#[nmi]`, `#[irq]`, `#[reset]`, `#[org]`, `#[section]`
-   **Convention**: Zero-page parameter passing, A/Y register returns

#### Control Flow

-   `if/else if/else`, `while`, `loop`, `for..in` (ranges and arrays)
-   `match` with patterns (literals, ranges, wildcards, enums)
-   `return`, `break`, `continue`

#### Operators

-   **Arithmetic**: +, -, \*, /, %
-   **Bitwise**: &, |, ^, ~, <<, >>
-   **Comparison**: ==, !=, <, >, <=, >=
-   **Logical**: &&, ||, !
-   **Special**: Type casts (`as`), unary ops, pointer dereference

#### Inline Assembly

-   Raw 6502 assembly blocks with variable substitution

---

### 2. COMPILER FEATURES ✅

#### Type System

-   Explicit type declarations (no inference)
-   Strong type checking with no implicit conversions
-   BCD type restrictions for safety
-   Function arity validation

#### Error Reporting (14 error types)

-   UndefinedSymbol, TypeMismatch, InvalidBinaryOp, ArityMismatch
-   CircularImport, ReturnTypeMismatch, BreakOutsideLoop, DuplicateSymbol
-   FieldNotFound, OutOfZeroPage, InstructionConflict, ConstantOverflow
-   Source code context with line numbers and position markers

#### Warning System (6 warning types)

-   UnusedVariable, UnusedImport, UnusedParameter, UnusedFunction
-   UnreachableCode, NonExhaustiveMatch

#### Optimization Passes

-   **Constant Folding**: Compile-time expression evaluation
-   **Strength Reduction**: x\*2^n → x<<n (~10x faster), x/256 → x.high
-   **Peephole Optimization**: Pattern-based assembly improvements
-   **Inline Expansion**: Zero-cost abstractions via #[inline]
-   **Register Tracking**: Eliminates redundant loads/stores
-   **INC/DEC Optimization**: Single-byte instructions vs LDA/ADC

#### Memory Layout

-   **Zero Page ($00-$FF)**: System (32B), Temps (32B), Variables (64B), Params (64B)
-   **Sections**: STDLIB ($8000-$8FFF), CODE ($9000-$BFFF), DATA ($C000-$CFFF)
-   Configurable via wraith.toml

---

### 3. 6502-SPECIFIC FEATURES ✅

#### CPU Flags Access

-   `carry`, `zero`, `overflow`, `negative` - Read processor status as booleans

#### u16 Byte Access

-   `value.low`, `value.high` - 83% code reduction vs bit shifts

#### Binary Coded Decimal (BCD)

-   b8 (0-99), b16 (0-9999) types with automatic SED/CLD
-   Type-safe BCD operations (add/sub only)

#### Addressing Modes

-   Compiler generates optimal 6502 modes: Immediate, Absolute, Zero Page, Indexed, Indirect

#### Interrupt Handling

-   Automatic vector table generation ($FFFA-$FFFF)
-   Register save/restore in handlers
-   Proper RTI instruction emission

---

### 4. STANDARD LIBRARY ✅

#### Intrinsics (intrinsics.wr)

-   Interrupt control: `enable_interrupts()`, `disable_interrupts()`
-   Flag control: `clear_carry()`, `set_carry()`, `clear_decimal()`, `set_decimal()`
-   CPU ops: `nop()`, `brk()`, `wait_for_interrupt()`

#### Memory (mem.wr)

-   `memcpy()`, `memset()`, `memcmp()`

#### Math (math.wr)

-   Comparison: `min()`, `max()`, `clamp()`
-   Bit manipulation: `set_bit()`, `clear_bit()`, `test_bit()` (65C02 optimized)
-   Saturating arithmetic: `saturating_add()`, `saturating_sub()`
-   Advanced: `count_bits()`, `reverse_bits()`, `swap_nibbles()`

---

### 5. CODE GENERATION ✅

-   **Target**: 6502 assembly with optimization
-   **Comment Levels**: Minimal, Normal, Verbose
-   **Features**: Label generation, directive support, string deduplication
-   **Expression Compilation**: All expression types supported
-   **Statement Compilation**: All statement types supported

---

### 6. TESTING ✅

-   **198+ Tests**: E2E (29), Integration (24), Error (30), Feature tests
-   **Infrastructure**: Common harness, compilation utilities, pattern matching
-   **Coverage**: Comprehensive language feature validation

---

## IMPROVEMENT SUGGESTIONS

### 🔴 COMPLETED

#### ✅ Constant Array Optimization

**Status**: ✅ COMPLETED (January 2026)
**Implementation**: Const arrays now emit to DATA section at $C000 with zero-fill optimization (.RES for arrays >= 16 bytes)
**Impact**: Enables lookup tables, character encodings, sprite data in ROM

#### ✅ Tail Call Optimization

**Status**: ✅ COMPLETED (January 2026)
**Implementation**:
- Semantic analysis detects tail recursive calls
- Tail recursive calls generate JMP to loop label instead of JSR
- Software stack ($0200-$02FF) preserves parameters across recursive calls
- Supports up to 32 levels of recursion (256 bytes / 8 bytes per level)

**Impact**:
- Prevents stack overflow in tail recursive functions
- Faster execution (JMP vs JSR+RTS overhead)
- Enables recursive algorithms like factorial, fibonacci with accumulator pattern

---

### 🟡 MEDIUM PRIORITY (High Impact)

#### 1. Module Visibility System

**Current**: All imports are public, no privacy control
**Improvement**: Add `pub` keyword for functions, structs, enums, constants
**Benefit**: Better encapsulation, clearer API boundaries
**Complexity**: Medium (semantic analysis changes)

**Syntax**:

```wraith
pub fn exported_function() { }
fn internal_helper() { }  // Not visible to importers
```

#### 2. Bitfield Access Syntax

**Current**: Manual bit manipulation with shifts and masks
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

#### 3. Compile-Time Array Bounds Checking

**Current**: Only runtime checks (if any)
**Improvement**: Error on `array[10]` when array size is 5
**Benefit**: Catch bugs at compile time
**Complexity**: Medium (constant propagation in semantic analysis)

#### 4. Standard Library Expansion

**Add These Functions**:

-   `mul16(a: u16, b: u16) -> u16` - 16-bit multiplication
-   `div16(a: u16, b: u16) -> u16` - 16-bit division
-   `strlen(s: *u8) -> u8` - Null-terminated string length
-   `strcmp(a: *u8, b: *u8) -> i8` - String comparison
-   `abs(x: i8) -> i8` and `abs16(x: i16) -> i16` - Absolute value

**Complexity**: Low-Medium (implementation work)

---

### 🟢 LOWER PRIORITY (Nice to Have)

#### 5. Disassembly Output Mode

**Current**: Only assembly source output
**Improvement**: Generate annotated listing with addresses and cycle counts
**Benefit**: Performance analysis and debugging
**Complexity**: Medium (need to track addresses during codegen)

**Example Output**:

```
9000: A9 00     LDA #$00        ; [2 cycles] Load zero into A
9002: 85 40     STA $40         ; [3 cycles] Store to variable x
```

#### 6. Branch Optimization Intelligence

**Current**: Status flags discarded after comparisons
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

#### 7. Inline Data Directive

**Current**: Data must be in static variables or string literals
**Improvement**: Allow inline data in functions
**Benefit**: Lookup tables, sprite data colocated with code
**Complexity**: Low (codegen addition)

**Syntax**:

```wraith
data lookup_table: [u8; 16] = [
    0x00, 0x01, 0x04, 0x09, 0x10, 0x19, ...  // Squares 0-15
];
```

#### 8. PRNG (Pseudo-Random Number Generator)

**Add to stdlib**:

-   `rand_init(seed: u16)` - Initialize generator
-   `rand_u8() -> u8` - Get random byte
-   `rand_range(min: u8, max: u8) -> u8` - Random in range

**Complexity**: Low (algorithm implementation)

---

## PRIORITIZED ROADMAP

### ✅ Phase 1: Performance Optimizations (COMPLETED)

**Status**: ✅ COMPLETED (January 2026)

1. ✅ **Constant array optimization** - Const arrays emit to DATA section with .RES optimization

**Impact Achieved**: Lookup tables, character encodings, and sprite data now work in ROM

---

### Phase 2: Advanced Optimizations

**Priority**: Sophisticated compiler improvements

1. ✅ **Tail call optimization** (COMPLETED) - Convert tail recursion to loops
2. **Branch optimization intelligence** - Track flag state across comparisons
3. **Standard library expansion** - Add mul16, div16, strlen, strcmp, abs

**Expected Impact**: Better generated code, more complete stdlib

---

### Phase 3: Language Ergonomics (Medium Effort, High Value)

**Priority**: Make the language more pleasant to use

1. **Module visibility system** - Add `pub` keyword for proper encapsulation
2. **Compile-time array bounds checking** - Catch out-of-bounds errors at compile time
3. **Bitfield access syntax** - Clean `.bit(n)` syntax for bit manipulation

**Expected Impact**: Fewer runtime bugs, cleaner code, better APIs

---

### Phase 4: Developer Tools (Medium Effort)

**Priority**: Professional tooling and debugging

1. **Disassembly output mode** - Annotated listing with addresses and cycle counts
2. **Inline data directives** - Lookup tables and sprite data in functions
3. **PRNG functions** - Random number generation in stdlib

**Expected Impact**: Better debugging, more flexibility, complete feature set
