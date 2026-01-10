# Factorial Tail Recursion - Complete Bug Fix Report

## Problem

The `factorial_tail.wr` example produced incorrect results:
- **Expected**: `01 00 01 00 78 00 B0 13` (factorial 0,1,5,7 = 1,1,120,5040)
- **Initial**: `01 00 00 00 00 00 00 00` (only first result correct)
- **After partial fix**: `01 00 00 00 04 00 06 00` (still wrong!)

## Root Causes - THREE Separate Bugs

### Bug #1: Missing 16-bit Multiplication ✅ FIXED

**Problem**: The compiler only supported 8-bit multiplication using repeated addition.
- When multiplying `u16 * u16`, only the low bytes were multiplied
- Results > 255 would overflow and wrap around
- `acc * n` with large values produced garbage

**Fix**: Implemented shift-and-add algorithm for both u8 and u16
- `generate_multiply_u8`: 8 iterations for 8-bit multiplication
- `generate_multiply_u16`: 16 iterations for 16-bit multiplication
- Uses binary shift-and-add: `for each bit: if bit set, add; shift multiplicand left, shift multiplier right`

**Files Modified**:
- `src/codegen/expr/binary.rs` (lines 442-581)

---

### Bug #2: Temp Storage Conflict ✅ FIXED

**Problem**: Argument evaluation used the same temp storage ($20+) as expression evaluation
- First argument `n-1` stored to $20
- Second argument `acc * (n as u16)` evaluation uses TEMP ($20:$21) internally
- The multiplication stores right operand to $20:$21, **overwriting first argument**!

**Example**:
```
1. Evaluate n-1 → store to $20
2. Evaluate acc * (n as u16):
   - Binary operation stores right operand to TEMP ($20:$21)
   - This OVERWRITES the n-1 value!
3. Copy arguments to parameters:
   - Copy $20 to n parameter ← Gets wrong value!
```

**Fix**: Use separate temp storage area ($F4-$FE) for argument evaluation
- This area doesn't conflict with TEMP register ($20) used by expressions
- Both `generate_call` and `generate_tail_recursive_update` fixed

**Files Modified**:
- `src/codegen/expr/call.rs` (lines 90, 350)

---

### Bug #3: Register State Tracking Bug ✅ FIXED

**Problem**: After arithmetic modifies A, register tracker still thinks A contains the old value
- Load n from $80 → tracker records "A contains ZeroPage(0x80)"
- Compute n-1 with SBC → **tracker not updated**, still thinks "A contains ZeroPage(0x80)"
- Later, when evaluating `n as u16`, it calls `emit_lda_zp(0x80)`
- Optimizer sees "A already has 0x80" and **skips the load**!
- So A contains n-1 instead of n, and we multiply by wrong value!

**Example**:
```asm
LDA $80        ; Load n, tracker: A = ZeroPage(0x80)
SEC
SBC $20        ; A = n-1, but tracker still thinks A = ZeroPage(0x80)!
...
LDA $80        ; SKIPPED by optimizer! Still uses n-1
```

**Fix**: Invalidate register tracking after arithmetic operations
- Added `emitter.mark_a_unknown()` after ADD and SUB operations
- Forces subsequent loads to actually emit LDA instructions

**Files Modified**:
- `src/codegen/expr/binary.rs` (lines 242, 261)

---

## Verification

### Assembly Changes

**Before fixes** (tail call, lines 138-183):
```asm
; Bug: No LDA $80 to reload n!
LDA #$01
STA $20
LDA $80        ; Load n
SBC $20        ; A = n-1
STA $20        ; Store to temp (BUG: uses $20, conflicts with TEMP!)
; Cast uses A (which is n-1, not n!)
LDY #$00       ; BUG: Casts n-1 instead of n!
STA $20
STY $21
; Multiply acc * (n-1) instead of acc * n
...
```

**After all fixes** (tail call, lines 138-184):
```asm
; Fix #1: Shift-and-add multiplication (16-bit support)
; Fix #2: Uses $F4 temp storage (no conflict)
; Fix #3: Reloads n (register tracking fixed)

LDA #$01
STA $20
LDA $80        ; Load n
SEC
SBC $20        ; A = n-1
STA $F4        ; ✓ Store to $F4 (not $20!)
LDA $80        ; ✓ RELOAD n (tracker was invalidated!)
; Cast to U16
LDY #$00       ; ✓ Casts n, not n-1!
STA $20
STY $21
; Multiply acc * n using shift-and-add
LDA $81
LDY $82
STA $F0
STY $F1
LDA #$00
STA $22
STA $23
LDX #$10       ; ✓ 16 iterations for 16-bit
ml_5:
    LSR $21    ; ✓ Shift-and-add loop
    ROR $20
    BCC ms_6
    LDA $22
    CLC
    ADC $F0    ; ✓ 16-bit addition
    STA $22
    LDA $23
    ADC $F1
    STA $23
ms_6:
    ASL $F0    ; ✓ Shift multiplicand left
    ROL $F1
    DEX
    BNE ml_5
LDA $22
LDY $23
STA $F5        ; ✓ Store to $F5:$F6
STY $F6
LDA $F4        ; ✓ Load n-1 from correct location
STA $80
LDA $F5
STA $81
LDA $F6
STA $82
JMP factorial_loop_start
```

### Expected Results

```
Memory 0x6000-0x6007: 01 00 01 00 78 00 B0 13

Breakdown:
  0x6000-6001: factorial(0) = 1    = 0x0001 ✓
  0x6002-6003: factorial(1) = 1    = 0x0001 ✓
  0x6004-6005: factorial(5) = 120  = 0x0078 ✓
  0x6006-6007: factorial(7) = 5040 = 0x13B0 ✓
```

---

## Test Programs Created

1. **examples/multiply_test.wr**: Tests 16-bit multiplication
   - 2×3=6, 10×20=200, 100×50=5000, 255×2=510, 120×6=720
   - Expected results in `MULTIPLY_TEST_EXPECTED.md`

2. **examples/factorial_tail.wr**: Tests tail recursion with multiplication
   - factorial(0), factorial(1), factorial(5), factorial(7)

---

## Impact

These bugs affected:
- ✅ **All 16-bit multiplications** (Bug #1)
- ✅ **All function calls with complex arguments** (Bug #2)
- ✅ **All arithmetic followed by variable access** (Bug #3)

The fixes improve correctness across the entire compiler, not just tail recursion!
