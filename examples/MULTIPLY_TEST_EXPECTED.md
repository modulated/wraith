# Multiplication Test - Expected Results

## Test Cases

### Test 1: 2 × 3 = 6
- **Calculation**: 2 × 3 = 6
- **Hexadecimal**: 0x0006
- **Expected Memory**:
  - `0x6000` (RESULT_2x3_LO): `0x06`
  - `0x6001` (RESULT_2x3_HI): `0x00`

### Test 2: 10 × 20 = 200
- **Calculation**: 10 × 20 = 200
- **Hexadecimal**: 0x00C8
- **Expected Memory**:
  - `0x6002` (RESULT_10x20_LO): `0xC8`
  - `0x6003` (RESULT_10x20_HI): `0x00`

### Test 3: 100 × 50 = 5000
- **Calculation**: 100 × 50 = 5000
- **Hexadecimal**: 0x1388
- **Expected Memory**:
  - `0x6004` (RESULT_100x50_LO): `0x88`
  - `0x6005` (RESULT_100x50_HI): `0x13`

### Test 4: 255 × 2 = 510
- **Calculation**: 255 × 2 = 510
- **Hexadecimal**: 0x01FE
- **Expected Memory**:
  - `0x6006` (RESULT_255x2_LO): `0xFE`
  - `0x6007` (RESULT_255x2_HI): `0x01`

### Test 5: 120 × 6 = 720 (Factorial Case)
- **Calculation**: 120 × 6 = 720
- **Hexadecimal**: 0x02D0
- **Expected Memory**:
  - `0x6008` (RESULT_120x6_LO): `0xD0`
  - `0x6009` (RESULT_120x6_HI): `0x02`

## Expected Memory Dump

When the program completes, memory should contain:

```
Address | Value | Description
--------|-------|---------------------------
0x6000  | 0x06  | 2×3 = 6 (low byte)
0x6001  | 0x00  | 2×3 = 6 (high byte)
0x6002  | 0xC8  | 10×20 = 200 (low byte)
0x6003  | 0x00  | 10×20 = 200 (high byte)
0x6004  | 0x88  | 100×50 = 5000 (low byte)
0x6005  | 0x13  | 100×50 = 5000 (high byte)
0x6006  | 0xFE  | 255×2 = 510 (low byte)
0x6007  | 0x01  | 255×2 = 510 (high byte)
0x6008  | 0xD0  | 120×6 = 720 (low byte)
0x6009  | 0x02  | 120×6 = 720 (high byte)
```

**Summary**: `06 00 C8 00 88 13 FE 01 D0 02`

## Algorithm: Shift-and-Add Multiplication

The implementation uses the binary shift-and-add algorithm, which is efficient for any bit width.

### How It Works

For multiplying `A × B`:

1. Initialize `result = 0`
2. For each bit in `B` (from LSB to MSB):
   - If the current bit is 1: `result += A`
   - Shift `A` left by 1 (double it)
   - Shift `B` right by 1 (move to next bit)
3. Return `result`

### Example: 6 × 3 using 8-bit

```
Initial:
  A = 6 (0b00000110)
  B = 3 (0b00000011)
  result = 0

Iteration 1: B's bit 0 = 1
  result += A    → result = 0 + 6 = 6
  A <<= 1        → A = 12
  B >>= 1        → B = 1

Iteration 2: B's bit 1 = 1
  result += A    → result = 6 + 12 = 18
  A <<= 1        → A = 24
  B >>= 1        → B = 0

B is now 0, remaining iterations do nothing.
Final result = 18 ✓
```

### 16-bit Implementation

For 16-bit multiplication, the algorithm is the same but operates on 16-bit values:
- 16-bit addition with carry
- 16-bit left shift (ASL low byte, ROL high byte)
- 16-bit right shift (LSR high byte, ROR low byte)

### Assembly Implementation

Key 6502 instructions used:
- `ASL` / `ROL`: 16-bit left shift
- `LSR` / `ROR`: 16-bit right shift
- `ADC`: Addition with carry for 16-bit math
- `BCC`: Branch if carry clear (test if bit was 0)

The loop runs 16 times for u16×u16, checking each bit of the multiplier.

## Factorial Test Results

With the fixed multiplication, `factorial_tail.wr` should now produce:

```
Address | Value | Description
--------|-------|---------------------------
0x6000  | 0x01  | factorial(0) = 1 (low)
0x6001  | 0x00  | factorial(0) = 1 (high)
0x6002  | 0x01  | factorial(1) = 1 (low)
0x6003  | 0x00  | factorial(1) = 1 (high)
0x6004  | 0x78  | factorial(5) = 120 (low)
0x6005  | 0x00  | factorial(5) = 120 (high)
0x6006  | 0xB0  | factorial(7) = 5040 (low)
0x6007  | 0x13  | factorial(7) = 5040 (high)
```

**Summary**: `01 00 01 00 78 00 B0 13`

Note: 5040 = 0x13B0 (low byte = 0xB0, high byte = 0x13)
