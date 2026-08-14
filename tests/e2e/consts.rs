//! Constants that have no value anywhere must not silently read `$0000`.
//!
//! Immutable consts live at the `Absolute(0)` sentinel: they are referenced by
//! ROM label or folded into their use sites, never by computed address. Any
//! shape that misses both paths — an accessor the const evaluator didn't know,
//! a scalar initializer that can't fold, a type with no ROM emission — turns
//! every read into a load from `$0000` (or `$0000` plus a field offset). These
//! pin the four forms of it.

use crate::common::exec::run;
use crate::common::harness::{CompileResult, compile};

fn expect_error(src: &str) -> String {
    match compile(src) {
        CompileResult::SemaError(e) | CompileResult::CodegenError(e) => e,
        CompileResult::Success(..) => panic!("expected a compile error, but it compiled"),
        other => panic!("expected an error, got {other:?}"),
    }
}

#[test]
fn low_and_high_of_a_const_u16_fold_to_its_bytes() {
    // The const evaluator didn't know the accessor nodes, so `C.low` never
    // folded and codegen read the sentinel: 0 instead of 210.
    let mut e = run(r#"
        const C: u16 = 1234;
        const LO: addr = 0x0900;
        const HI: addr = 0x0901;
        #[reset]
        fn main() {
            LO = C.low;
            HI = C.high;
            loop {}
        }
    "#);
    assert_eq!((e.mem(0x0900), e.mem(0x0901)), (210, 4));
}

#[test]
fn len_of_a_const_string_folds() {
    let mut e = run(r#"
        const S: str = "hello";
        const OUT: addr = 0x0900;
        #[reset]
        fn main() {
            OUT = S.len.low;
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0900), 5);
}

#[test]
fn a_scalar_const_with_an_unevaluable_initializer_is_rejected() {
    // Arrays have a ROM-data path, so an unevaluated aggregate const is fine.
    // A scalar has no other path: before, this was accepted and every use of N
    // read $0000.
    let err = expect_error(
        r#"
        const A: [u8; 3] = [1, 2, 3];
        const N: u8 = A.len as u8;
        #[reset]
        fn main() { loop {} }
    "#,
    );
    assert!(err.contains("constant initializer"), "{err}");
}

#[test]
fn a_const_of_struct_type_is_rejected() {
    // It was accepted but never emitted: codegen writes ROM data only for
    // const arrays and strings, so TBL.b read $0000 + field offset.
    let err = expect_error(
        r#"
        struct S { a: u16, b: u8 }
        const TBL: S = S { a: 0x1234, b: 2 };
        #[reset]
        fn main() { loop {} }
    "#,
    );
    assert!(err.contains("struct or enum type"), "{err}");
}

#[test]
fn a_string_initializer_for_a_scalar_const_is_rejected() {
    // The value's kind was never checked against the declared type: this
    // compiled and folded a string into byte contexts.
    let err = expect_error(
        r#"
        const C: u8 = "hello";
        #[reset]
        fn main() { loop {} }
    "#,
    );
    assert!(err.contains("u8"), "{err}");
}

#[test]
fn an_out_of_range_array_element_is_rejected() {
    // The flattener truncated each element to the element width: [300, 2]
    // became [44, 2] with nothing said.
    let err = expect_error(
        r#"
        const B: [u8; 2] = [300, 2];
        const R0: addr = 0x0900;
        #[reset]
        fn main() { R0 = B[0]; loop {} }
    "#,
    );
    assert!(err.contains("u8"), "{err}");
}

#[test]
fn an_oversized_fill_initializer_is_rejected() {
    // `[0; 5]` into `[u8; 2]` was silently clamped to two repetitions while
    // the equivalent five-element list errored.
    let err = expect_error(
        r#"
        const B: [u8; 2] = [0; 5];
        const R0: addr = 0x0900;
        #[reset]
        fn main() { R0 = B[0]; loop {} }
    "#,
    );
    assert!(err.contains('5'), "{err}");
}

#[test]
fn an_addr_read_under_a_cast_is_not_folded_to_its_address() {
    // `V as u16 + 1` used to fold to the *address* 0x0902 and never read the
    // register at all. Runtime: 5 + 1 = 6.
    let mut e = run(r#"
        const V: addr = 0x0901;
        const OUT: addr = 0x0900;
        #[reset]
        fn main() {
            V = 5;
            let x: u16 = V as u16 + 1;
            OUT = x.low;
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0900), 6);
}

// ---------------------------------------------------------------------------
// Negative elements in a `const` array
// ---------------------------------------------------------------------------
//
// `const A: [i8; 2] = [-5, 1];` was rejected as "this initializer is not a
// compile-time constant" — but only once something read it, since an unused
// const is dropped before the flattening runs. The identical `static` worked.
//
// Two `InitContext::integer` implementations had drifted: sema's evaluates the
// expression, codegen's pattern-matched a bare `Expr::Literal`. `-5` parses as
// `Unary(Neg, 5)`, so codegen saw no literal, and a const array's elements are
// flattened rather than type-checked, so nothing had folded them either.
//
// Found by `tests/fuzz_exec.rs` once it started generating `const` tables.

#[test]
fn a_const_array_may_hold_negative_elements() {
    let mut e = run("const OUT0: addr = 0x0900;\nconst OUT1: addr = 0x0901;\n\
         const OUT2: addr = 0x0902;\nconst OUT3: addr = 0x0903;\n\
         const A: [i8; 3] = [-5, 1, -128];\n\
         const C: [i16; 2] = [-300, 1];\n\
         #[reset]\nfn main() {\n\
         \x20   let i: u8 = 0;\n\
         \x20   OUT0 = A[i] as u8;\n\
         \x20   OUT1 = A[2] as u8;\n\
         \x20   let c: u16 = C[i] as u16;\n\
         \x20   OUT2 = c.low;\n    OUT3 = c.high;\n    loop {}\n}\n");
    assert_eq!(e.mem(0x0900), 251, "-5 is 0xFB");
    assert_eq!(e.mem(0x0901), 128, "-128 is 0x80");
    assert_eq!(e.mem16(0x0902), 65236, "-300 is 0xFED4");
}

/// The value need not be a literal — any constant expression should flatten.
#[test]
fn a_const_array_element_may_be_a_constant_expression() {
    let mut e = run("const OUT0: addr = 0x0900;\nconst OUT1: addr = 0x0901;\n\
         const N: i8 = 7;\n\
         const A: [i8; 2] = [0 - 5, N - 10];\n\
         #[reset]\nfn main() {\n\
         \x20   let i: u8 = 0;\n\
         \x20   OUT0 = A[i] as u8;\n    OUT1 = A[1] as u8;\n    loop {}\n}\n");
    assert_eq!(e.mem(0x0900), 251, "0 - 5 is -5");
    assert_eq!(e.mem(0x0901), 253, "7 - 10 is -3");
}

/// The `static` form always worked, and has to keep working: the fix brought
/// codegen's flattening into line with sema's, not the other way round.
#[test]
fn a_static_array_still_holds_negative_elements() {
    let mut e = run("const OUT0: addr = 0x0900;\n\
         static B: [i8; 2] = [-5, 1];\n\
         #[reset]\nfn main() {\n\
         \x20   let i: u8 = 0;\n    OUT0 = B[i] as u8;\n    loop {}\n}\n");
    assert_eq!(e.mem(0x0900), 251);
}
