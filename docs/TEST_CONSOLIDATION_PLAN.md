# Test Suite Consolidation Plan

## Current State Analysis

### Test Distribution
- **lib tests (src/)**: 27 tests (unit tests in source files)
  - codegen::peephole: 3 tests
  - codegen::section_allocator: 3 tests
  - lexer: 7 tests
  - parser: 5 tests
  - sema::const_eval: 5 tests

- **Integration tests (tests/)**:
  - ast_tests.rs: 8 tests
  - codegen_tests.rs: 21 tests
  - error_tests.rs: 29 tests (+ 2 ignored)
  - feature_tests.rs: 33 tests (includes nested test_harness tests)
  - parse_demo.rs: 1 test
  - sema_tests.rs: 2 tests
  - test_harness.rs: 5 nested tests (helpers module)

- **Total**: ~126 tests

### Problems Identified

1. **Scattered Organization**
   - No clear separation between unit/integration/e2e tests
   - Overlapping test concerns (feature_tests vs codegen_tests)
   - Helper functions duplicated across files

2. **Incomplete Coverage**
   - No coverage metrics
   - Missing edge cases
   - No property-based testing
   - Limited error path testing

3. **Test File Overlap**
   - feature_tests.rs and codegen_tests.rs both test codegen
   - error_tests.rs mixes parse/sema/codegen errors
   - sema_tests.rs is minimal (only 2 tests)

4. **Maintenance Issues**
   - Duplicate helpers (appears_before, extract_instructions)
   - Inconsistent test naming
   - No test documentation

---

## Proposed Consolidation

### New Test Organization

```
tests/
├── common/              # Shared test infrastructure
│   ├── mod.rs          # Public exports
│   ├── harness.rs      # Compilation helpers
│   ├── assertions.rs   # Custom assertions
│   └── fixtures.rs     # Test data
│
├── unit/               # Unit tests (if not in src/)
│   └── ...
│
├── integration/        # Integration tests (full pipeline)
│   ├── lexer.rs       # Lexer integration tests
│   ├── parser.rs      # Parser integration tests
│   ├── sema.rs        # Semantic analysis tests
│   └── codegen.rs     # Code generation tests
│
├── e2e/               # End-to-end feature tests
│   ├── control_flow.rs
│   ├── functions.rs
│   ├── types.rs
│   ├── memory.rs
│   └── stdlib.rs
│
└── errors/            # Error message tests
    ├── lex_errors.rs
    ├── parse_errors.rs
    ├── sema_errors.rs
    └── codegen_errors.rs
```

### Migration Plan

#### Phase 1: Create Common Infrastructure
1. Create `tests/common/` module
2. Move all helpers from test_harness.rs
3. Add new assertion helpers
4. Create test fixture system

#### Phase 2: Consolidate Existing Tests
1. Merge codegen_tests.rs + feature_tests.rs → integration/codegen.rs + e2e/*
2. Split error_tests.rs by phase → errors/*
3. Move ast_tests.rs → integration/parser.rs
4. Expand sema_tests.rs → integration/sema.rs
5. Remove parse_demo.rs (fold into integration tests)

#### Phase 3: Add Coverage Tooling
1. Set up `cargo-llvm-cov` or `tarpaulin`
2. Add coverage CI checks
3. Target 80%+ coverage

#### Phase 4: Fill Coverage Gaps
1. Add edge case tests
2. Add error path tests
3. Add property-based tests (quickcheck)
4. Add fuzzing harness

---

## Detailed Actions

### 1. Common Test Infrastructure

**tests/common/mod.rs**:
```rust
pub mod harness;
pub mod assertions;
pub mod fixtures;

pub use harness::*;
pub use assertions::*;
pub use fixtures::*;
```

**tests/common/harness.rs**:
- `compile()` - Full compilation pipeline
- `compile_to_ast()` - Stop at AST
- `compile_to_sema()` - Stop at sema
- `lex_only()` - Lex only
- `parse_only()` - Parse only

**tests/common/assertions.rs**:
- `assert_compiles()` - Success assertion
- `assert_lex_error()` - Lex error
- `assert_parse_error()` - Parse error
- `assert_sema_error()` - Sema error
- `assert_codegen_error()` - Codegen error
- `assert_asm_contains()` - Assembly contains
- `assert_asm_not_contains()` - Assembly doesn't contain
- `assert_asm_order()` - Instruction ordering
- `assert_asm_pattern()` - Regex pattern matching
- `assert_optimization()` - Optimization applied

**tests/common/fixtures.rs**:
- Common test programs
- Expected outputs
- Error message templates

### 2. Test Categories

#### Integration Tests (tests/integration/)
Test individual compiler phases in isolation:
- Lexer correctness
- Parser correctness
- Semantic analysis correctness
- Codegen correctness

#### E2E Tests (tests/e2e/)
Test complete language features:
- Control flow (if/while/for/match)
- Functions (calls, inline, recursion)
- Types (primitives, structs, enums, arrays)
- Memory (addresses, pointers, zero page)
- Standard library

#### Error Tests (tests/errors/)
Test error messages and diagnostics:
- Lex errors (invalid tokens)
- Parse errors (syntax errors)
- Sema errors (type errors, undefined symbols, duplicates)
- Codegen errors (out of memory, etc.)

### 3. Coverage Targets

#### Minimum Coverage Goals
- **Lexer**: 95%+ (simple, deterministic)
- **Parser**: 90%+ (grammar coverage)
- **Sema**: 85%+ (all error paths)
- **Codegen**: 80%+ (all instructions, optimizations)
- **Overall**: 85%+

#### Uncovered Areas to Address
- Error recovery paths
- Edge cases (empty programs, single statements)
- Boundary conditions (u8 overflow, zero page exhaustion)
- Invalid combinations (inline recursive, etc.)
- Optimization corner cases

### 4. Advanced Testing

#### Property-Based Testing
Use `quickcheck` for:
- Expression evaluation (eval(parse(expr)) == eval_direct(expr))
- Type safety (well-typed programs don't crash)
- Optimization correctness (optimized == unoptimized semantically)

#### Fuzzing
Use `cargo-fuzz` for:
- Lexer (arbitrary input)
- Parser (arbitrary tokens)
- Sema (arbitrary AST)

#### Snapshot Testing
Use `insta` for:
- AST snapshots
- Assembly output snapshots
- Error message snapshots

### 5. Test Documentation

Add module documentation:
```rust
//! # Semantic Analysis Integration Tests
//!
//! Tests the semantic analysis phase in isolation.
//!
//! ## Coverage
//! - Symbol resolution
//! - Type checking
//! - Duplicate detection
//! - Warning generation
//!
//! ## Test Organization
//! - Symbol tests: test_symbol_*
//! - Type tests: test_type_*
//! - Duplicate tests: test_duplicate_*
//! - Warning tests: test_warning_*
```

---

## Implementation Checklist

### Phase 1: Infrastructure (1-2 hours)
- [ ] Create tests/common/ module structure
- [ ] Move test_harness.rs → common/harness.rs
- [ ] Extract assertions to common/assertions.rs
- [ ] Create fixtures system
- [ ] Update all test imports

### Phase 2: Consolidation (2-3 hours)
- [ ] Create integration/ directory
- [ ] Merge codegen tests
- [ ] Split error tests by phase
- [ ] Move parser tests
- [ ] Expand sema tests
- [ ] Remove redundant files

### Phase 3: Coverage (1-2 hours)
- [ ] Add cargo-llvm-cov to dev dependencies
- [ ] Create coverage script
- [ ] Run coverage analysis
- [ ] Document coverage gaps

### Phase 4: Fill Gaps (3-4 hours)
- [ ] Add missing edge case tests
- [ ] Add error path tests
- [ ] Add boundary condition tests
- [ ] Add optimization verification tests

### Phase 5: Advanced (Optional, 4+ hours)
- [ ] Set up quickcheck
- [ ] Add property tests
- [ ] Set up cargo-fuzz
- [ ] Add snapshot testing

---

## Benefits

1. **Maintainability**
   - Clear organization
   - No duplication
   - Easy to find tests

2. **Confidence**
   - High coverage
   - Edge cases covered
   - Regressions caught early

3. **Documentation**
   - Tests serve as examples
   - Clear test names
   - Well-documented modules

4. **Development Speed**
   - Fast to add new tests
   - Easy to debug failures
   - Quick to run specific suites

---

## Migration Example

### Before (feature_tests.rs)
```rust
#[test]
fn test_if_statement() {
    let asm = assert_compiles(
        r#"
        fn main() {
            if true {
                x: u8 = 10;
            }
        }
        "#,
    );
    assert_asm_contains(&asm, "BEQ");
}
```

### After (e2e/control_flow.rs)
```rust
use crate::common::*;

#[test]
fn if_statement_basic() {
    let asm = compile_success(r#"
        fn main() {
            if true {
                x: u8 = 10;
            }
        }
    "#);

    assert_asm_contains(&asm, "BEQ");
}

#[test]
fn if_statement_with_else() {
    // ...
}

#[test]
fn if_statement_nested() {
    // ...
}

#[test]
fn if_statement_complex_condition() {
    // ...
}
```

---

## Next Steps

1. Review and approve this plan
2. Decide on phases to implement
3. Set coverage targets
4. Begin Phase 1 (infrastructure)
