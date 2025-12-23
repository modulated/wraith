# Removal of `mut` Keyword from Wraith

## Summary

The `mut` keyword has been completely removed from the Wraith language. Variables are **mutable by default**, and the `const` keyword is used to make values immutable and compile-time constant.

## Changes Made

### 1. Lexer (src/lexer/mod.rs)
- ✅ Removed `Token::Mut` - `mut` is now treated as a regular identifier

### 2. Parser (src/parser/)
- ✅ **item.rs**: Removed `Token::Mut` from static/const parsing
  - `const NAME: type = value;` → immutable, compile-time constant
  - `NAME: type = value;` → mutable, runtime storage
  - `zp NAME: type = value;` → mutable, zero-page storage

- ✅ **expr.rs**: Removed `mut` from type syntax
  - `*T` → mutable pointer (all pointers are mutable)
  - `&[T]` → mutable slice (all slices are mutable)
  - `&var` → mutable reference (all references are mutable)

- ✅ **stmt.rs**: Removed `Token::Mut` from statement parsing

- ✅ **error.rs**: Removed `Token::Mut` from error formatting

### 3. Test Suite (tests/error_tests.rs)
- ✅ Added `test_const_cannot_be_reassigned` (ignored - TODO: implement checking)
- ✅ Added `test_const_cannot_be_modified` (ignored - TODO: implement checking)
- ✅ Added `test_regular_variable_can_be_reassigned` (passing)

## Current Behavior

### ✅ `mut` as Identifier
```wraith
fn main() {
    mut: u8 = 42;  // 'mut' is a valid variable name
    LED = mut;      // Works perfectly
}
```

### ✅ Variables are Mutable by Default
```wraith
fn main() {
    counter: u8 = 0;
    counter = counter + 1;  // ✅ Works - variables are mutable
}
```

### ✅ `const` for Immutable Values
```wraith
const MAX_BRIGHTNESS: u8 = 255;  // Compile-time constant
const BAUD_RATE: u8 = 9600;       // Folded to immediate values

fn main() {
    LED = MAX_BRIGHTNESS;  // Generates: LDA #$FF
}
```

## Examples Updated

- ✅ `examples/test_mut_as_identifier.wr` - Demonstrates `mut` as identifier
- ✅ `examples/const_demo.wr` - Shows idiomatic const usage
- ✅ `examples/uart_tl16c550b.wr` - Updated to use `const` properly

## TODO: Immutability Checking

The tests for const immutability are currently **ignored** because the semantic analyzer doesn't yet check if you try to assign to a const. This needs to be implemented:

```rust
const MAX: u8 = 100;
fn main() {
    MAX = 50;  // TODO: Should error - cannot assign to const
}
```

## Verification

Run the test to verify `mut` is treated as an identifier:
```bash
cargo build
./target/debug/wraith examples/test_mut_as_identifier.wr
```

Run tests to verify mutable variables work:
```bash
cargo test test_regular_variable_can_be_reassigned
```

All tests pass! ✅
