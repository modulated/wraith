# Wraith Compiler - TODO List

Updated: 2025-12-26

This document tracks all pending tasks, improvements, and fixes for the Wraith compiler.

---

## Language Features

- [ ] **Slice Type Support**
  - Status: AST complete, minimal codegen
  - Priority: MEDIUM
  - Description: Slice types with length tracking for safe array access

- [ ] **Type Inference**
  - Status: Not implemented
  - Priority: LOW
  - Description: Infer types from context (e.g., `x := 10`)

- [ ] **Error Recovery in Parser**
  - Status: Parser stops at first error
  - Priority: MEDIUM
  - Description: Continue parsing to find multiple errors

- [ ] **CPU Flags Access**
  - Status: Not implemented
  - Priority: MEDIUM
  - Description: Direct access to processor status flags

- [ ] **Bitfield Access**
  - Status: Not implemented
  - Priority: MEDIUM
  - Description: Bit manipulation syntax

- [ ] **Module System with Visibility**
  - Status: Only file imports exist
  - Priority: MEDIUM
  - Description: Proper module hierarchy and pub/private

- [ ] **Memory Section Control**
  - Status: Only #[org] exists
  - Priority: MEDIUM
  - Description: #[section("DATA")] attribute

---

## Optimization

- [ ] **Advanced Register Allocation**
  - Status: Basic (X for loops, Y for temps)
  - Priority: MEDIUM
  - Description: Full register allocation with liveness analysis

- [ ] **Dead Code Elimination**
  - Status: Not implemented
  - Priority: MEDIUM
  - Description: Remove unreachable/unused code

- [ ] **Strength Reduction**
  - Status: Not implemented
  - Priority: MEDIUM
  - Description: Replace expensive ops with cheaper ones

- [ ] **Branch Optimization**
  - Status: Not implemented
  - Priority: MEDIUM
  - Description: Use optimal branching strategies

---

## Standard Library

- [ ] **Math Functions**
  - Status: Not implemented
  - Priority: MEDIUM
  - Description: min, max, abs, clamp, mul16, div16

- [ ] **String Functions**
  - Status: Not implemented
  - Priority: MEDIUM
  - Description: str_cmp, str_copy, str_concat

- [ ] **Bit Manipulation**
  - Status: Not implemented
  - Priority: MEDIUM
  - Description: set_bit, clear_bit, test_bit, reverse_bits

- [ ] **Random Number Generation**
  - Status: Not implemented
  - Priority: LOW
  - Description: PRNG for games

---

## Documentation

- [x] **Language Reference Manual** ✓
  - Status: Complete
  - Location: `docs/language_spec.md`
  - Includes: syntax grammar, type system rules, memory model, calling convention, import system
  - Updated: 2025-12-26 with latest language features (mut, const, addr, imports)

---

## Notes

- All high priority compiler issues are complete!
- Medium priority items improve functionality but aren't blocking
- Low priority items are enhancements and future improvements
- Run tests after each fix to ensure no regressions
- Memory model: All variables allocated in zero page (0x00-0xFF) or absolute memory (0x0200+)
- When zero page is exhausted, compilation fails with OutOfZeroPage error
