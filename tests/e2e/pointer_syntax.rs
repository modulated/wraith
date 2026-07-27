//! Parsing of the pointer type and the `&` / `*` operators.
//!
//! Two things here are easy to get subtly wrong and impossible to notice later.
//!
//! The first is precedence. The existing `-` / `!` / `~` arms parse their
//! operand with `parse_prefix_expr` and let the caller apply postfix, so `-x.f`
//! actually parses as `(-x).f`. Copying that shape for `&` would make `&x[0]`
//! mean `(&x)[0]` — indexing a pointer to an array, which is meaningless. The
//! new arms take their postfix suffixes first.
//!
//! The second is that `&&` lexes as a single `AndAnd` token, so a pointer to a
//! pointer never reaches the single-`&` path at all.
//!
//! Sema and codegen still reject these, so the tests check the *parse* by
//! confirming the failure comes from the later stage rather than the parser.

use crate::common::exec::run;
use crate::common::harness::{CompileResult, compile, compile_to_ast};

/// Parse only. Returns the parse error, or `None` if it parsed.
fn parse_error(src: &str) -> Option<String> {
    compile_to_ast(src).err()
}

/// Assert `src` gets past the parser, whatever happens afterwards.
fn assert_parses(src: &str) {
    if let Some(e) = parse_error(src) {
        panic!("expected this to parse, got: {e}\n---\n{src}");
    }
}

/// Assert `src` parses *and* compiles. These forms are now implemented; the
/// point of testing them here is the shape of the tree, which
/// `tests/e2e/pointers.rs` then checks the behaviour of.
fn assert_compiles(src: &str) {
    assert_parses(src);
    match compile(src) {
        CompileResult::Success(..) => {}
        other => panic!("expected this to compile, got {other:?}"),
    }
}

// ============================================================================
// The type
// ============================================================================

#[test]
fn a_pointer_type_parses_in_every_position() {
    assert_parses("fn f(p: &u8) { } #[reset] fn main() { loop {} }");
    assert_parses("fn f() -> &u8 { } #[reset] fn main() { loop {} }");
    assert_parses("struct S { p: &u8 } #[reset] fn main() { loop {} }");
}

#[test]
fn a_pointer_type_is_distinct_from_a_slice_type() {
    // `&[T]` must still be a slice; only `&` followed by something other than
    // `[` is a pointer. Getting the peek wrong turns every slice into a
    // pointer-to-array.
    assert_parses("fn f(s: &[u8]) { } #[reset] fn main() { loop {} }");
    assert_parses("fn f(p: &u8) { } #[reset] fn main() { loop {} }");
}

#[test]
fn a_pointer_to_a_pointer_parses_despite_the_and_and_token() {
    // `&&u8` lexes as one `&&`, so without splitting it here this is a parse
    // error rather than a pointer to a pointer.
    assert_parses("fn f(q: &&u8) { } #[reset] fn main() { loop {} }");
}

#[test]
fn a_pointer_to_a_named_type_parses() {
    assert_parses(
        "struct Device { status: u8 } fn f(d: &Device) { } #[reset] fn main() { loop {} }",
    );
}

#[test]
fn a_self_referential_struct_parses_and_sizes() {
    // The reason all four size tables stop at the pointer: following the
    // pointee here would run away. The size has to be 3 — one byte for the
    // value and two for the address.
    assert_parses("struct Node { value: u8, next: &Node } #[reset] fn main() { loop {} }");

    // Checked by behaviour rather than by reading a comment: two Node locals
    // sit side by side in the frame, so if the pointer field were sized 0 or 1
    // the second would overlap the first's `next`, and writing `a.next` would
    // land on top of `b.value`.
    let mut e = run(r#"
        const R0: addr = 0x0900;
        const R1: addr = 0x0901;
        struct Node { value: u8, next: &Node }
        #[reset]
        fn main() {
            let a: Node = Node { value: 1, next: 0 as &Node };
            let b: Node = Node { value: 2, next: 0 as &Node };
            a.next = &b;
            R0 = a.value;
            R1 = b.value;
            loop {}
        }
    "#);
    assert_eq!(
        (e.mem(0x0900), e.mem(0x0901)),
        (1, 2),
        "the locals do not overlap"
    );
}

// ============================================================================
// The operators
// ============================================================================

#[test]
fn address_of_and_deref_parse() {
    assert_compiles("#[reset] fn main() { let x: u8 = 1; let p: &u8 = &x; loop {} }");
    assert_compiles(
        "#[reset] fn main() { let x: u8 = 1; let p: &u8 = &x; let v: u8 = *p; loop {} }",
    );
}

#[test]
fn a_deref_assignment_parses() {
    // `*p = v;` reaches the statement parser through the ordinary
    // expression-or-assignment path, so it needs no separate handling.
    assert_compiles("#[reset] fn main() { let x: u8 = 1; let p: &u8 = &x; *p = 9; loop {} }");
}

#[test]
fn address_of_binds_looser_than_indexing() {
    // `&x[0]` must be the address of the element, not `(&x)[0]`. If it parsed
    // the other way this would be an index applied to a pointer-to-array, and
    // the error would come from somewhere else entirely.
    assert_compiles("#[reset] fn main() { let a: [u8; 4] = [0; 4]; let p: &u8 = &a[0]; loop {} }");
}

#[test]
fn address_of_binds_looser_than_field_access() {
    assert_compiles(
        "struct S { x: u8 } \
         #[reset] fn main() { let s: S = S { x: 1 }; let p: &u8 = &s.x; loop {} }",
    );
}

#[test]
fn deref_binds_looser_than_field_access() {
    // `*p.next` is `*(p.next)`, matching C and Rust.
    assert_parses(
        "struct Node { next: &u8 } \
         fn f(p: &Node) -> u8 { return *p.next; } \
         #[reset] fn main() { loop {} }",
    );
}

// ============================================================================
// Binary `&` and `*` are untouched
// ============================================================================

#[test]
fn binary_and_and_multiply_still_parse() {
    // `&` and `*` were only ever infix before. Adding prefix arms must not
    // disturb the binary readings.
    assert_parses("#[reset] fn main() { let a: u8 = 6 & 3; let b: u8 = 6 * 3; loop {} }");
    assert_parses(
        "#[reset] fn main() { let a: u8 = 1; let b: u8 = 2; let c: u8 = a & b; loop {} }",
    );
    assert_parses(
        "#[reset] fn main() { let a: u8 = 1; let b: u8 = 2; let c: u8 = a * b; loop {} }",
    );
}

#[test]
fn logical_and_still_parses() {
    // `&&` is a real operator and the new prefix arm must not eat it.
    //
    // Written as a `let` rather than an `if` condition on purpose: `if a && b {`
    // does not parse today, because `b {` is taken for a struct literal. That
    // is a pre-existing ambiguity, unrelated to pointers, and reproducible on
    // the commit before this one.
    assert_parses(
        "#[reset] fn main() { let a: bool = true; let b: bool = false; \
         let c: bool = a && b; loop {} }",
    );
}

#[test]
fn address_of_binds_tighter_than_as() {
    // `&x as u16` is `(&x) as u16` — the address as a number — matching C and
    // Rust. `as` is the one postfix suffix the `&` arm leaves to its caller;
    // taking it would give the address of a cast, which is a temporary and
    // therefore has no address at all. `specification.md` has documented this
    // spelling since before it parsed.
    assert_compiles(
        "static SLOT: u8 = 0; \
         #[reset] fn main() { let n: u16 = &SLOT as u16; let p: &u8 = n as &u8; *p = 1; loop {} }",
    );
}

#[test]
fn deref_binds_tighter_than_as() {
    assert_compiles(
        "#[reset] fn main() { let x: u8 = 1; let p: &u8 = &x; let n: u16 = *p as u16; loop {} }",
    );
}
