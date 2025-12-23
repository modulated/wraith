# Wraith Compiler Test Suite

This document describes the testing infrastructure for the Wraith compiler.

## Test Organization

Tests are organized into several categories:

### 1. **Unit Tests** (`src/**/*.rs`)
- Test individual modules in isolation
- Located alongside source code with `#[cfg(test)]`
- Run with: `cargo test --lib`

### 2. **Integration Tests** (`tests/*.rs`)
- Test compiler end-to-end
- Located in `tests/` directory
- Categories:
  - **test_harness.rs**: Reusable test utilities and helpers
  - **feature_tests.rs**: Test each language feature works correctly
  - **error_tests.rs**: Verify error handling and messages
  - **codegen_tests.rs**: Test assembly output correctness
  - **ast_tests.rs**: Test AST construction
  - **sema_tests.rs**: Test semantic analysis

### 3. **Example Programs** (`tests/*.wr`)
- Real Wraith programs for manual testing
- Compiled to `.asm` files for inspection
- Used for regression testing and demonstrations

## Test Harness

The `test_harness` module provides utilities for writing tests:

### Compilation Helpers

```rust
// Compile source and get assembly
let asm = assert_compiles("fn main() {}");

// Assert compilation fails at specific phase
assert_fails_at(source, "parse");  // "lex", "parse", "sema", "codegen"

// Assert error contains specific text
assert_error_contains(source, "type mismatch");
```

### Assembly Verification

```rust
// Assert assembly contains instruction
assert_asm_contains(&asm, "LDA #$2A");

// Assert assembly doesn't contain instruction
assert_asm_not_contains(&asm, "ADC");

// Assert instruction ordering
assert_asm_order(&asm, "LDA #$2A", "STA $0400");

// Count pattern occurrences
let count = count_pattern(&asm, ".byte $00");
```

## Running Tests

```bash
# Run all tests
cargo test

# Run specific test file
cargo test --test feature_tests

# Run specific test
cargo test test_variable_declaration_u8

# Run with output
cargo test -- --nocapture

# Run only integration tests
cargo test --tests
```

## Test Coverage

### Current Status

**Feature Tests**: 26/29 passing (90%)
- ✅ Variables and types
- ✅ Hex/binary literals
- ✅ Operators (arithmetic, bitwise, shifts)
- ✅ Control flow (if/else, while)
- ⚠️  For loops (type annotation not supported)
- ⚠️  Mutable variables (scoping issue)
- ✅ Functions (calls, returns, parameters)
- ✅ Arrays (literals, fill)
- ✅ Structs (definition, initialization)
- ✅ Enums (definition)
- ✅ Memory operations (addr declarations)

**Error Tests**: 15/23 passing (65%)
- ✅ Parse errors (missing semicolons, braces)
- ✅ Type mismatches
- ✅ Function arity errors
- ✅ Immutable assignment errors
- ⚠️  Error message wording (minor differences)

**Legacy Tests**: 31 tests in existing files
- codegen_tests.rs: ~15 tests
- ast_tests.rs: ~8 tests
- sema_tests.rs: ~8 tests

**Total**: ~70 automated tests

## Writing New Tests

### Feature Test Example

```rust
#[test]
fn test_my_feature() {
    let asm = assert_compiles(
        r#"
        addr OUT = 0x400;
        fn main() {
            x: u8 = 42;
            OUT = x;
        }
        "#,
    );

    assert_asm_contains(&asm, "LDA #$2A");
    assert_asm_contains(&asm, "STA $0400");
}
```

### Error Test Example

```rust
#[test]
fn test_my_error() {
    assert_error_contains(
        r#"
        fn main() {
            x: u8 = undefined;
        }
        "#,
        "undefined symbol",
    );
}
```

## Test-Driven Development

When adding new features:

1. **Write the test first** (it should fail)
2. **Implement the feature** (make the test pass)
3. **Add error tests** (verify error handling)
4. **Add edge cases** (boundary conditions)
5. **Update documentation** (examples in tests/)

## Continuous Integration

Tests run automatically on:
- Every commit
- Pull requests
- Before releases

## Known Issues

Current test failures indicate areas needing work:

1. **For loop type annotations** - Parser doesn't support `for i: u8 in range`
2. **Mutable variable scoping** - Symbol table issue with mut variables
3. **Error message consistency** - Some error texts differ from expectations

These are tracked as issues and will be fixed in future releases.

## Adding Test Categories

To add a new test category:

1. Create `tests/category_name_tests.rs`
2. Add `mod test_harness;` at the top
3. Use `use test_harness::*;` for helpers
4. Write tests with `#[test]` attribute
5. Document the category in this file

## Benchmarking

For performance testing:

```bash
cargo bench  # (when benchmarks are added)
```

## Test Data

Example programs in `tests/*.wr`:
- test_arrays.wr - Array type support
- test_hex_literals.wr - Hex and binary literals
- test_memory_functions.wr - Standard library functions
- test_named_types.wr - Struct and enum types
- test_new_syntax.wr - Variable declaration syntax

These serve as both tests and documentation examples.
