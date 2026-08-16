//! Aggregates reaching the scalar store, which writes one byte of them.
//!
//! Binding and assignment end in a store of `A` (plus `X` or `Y` for a
//! two-byte slot). Every strategy above that — the struct copy, the slice
//! materializer, the local-array fill — falls *through* to it rather than
//! failing, so a shape nobody enumerated is not a compile error. It is a
//! one-byte copy of an aggregate, which compiles and answers wrongly.
//!
//! Two findings here, both of that shape:
//!
//!   * `let b: &[u8] = a;` and `b = a;` had no case at all. A slice slot is a
//!     four-byte descriptor — base then length — and one `LDA`/`STA` gave `b`
//!     the low byte of the base, left the high byte at whatever the slot held
//!     before, and never wrote the length. `b[i]` then read through a
//!     half-written pointer.
//!   * The four sites that ask "did this expression return an aggregate in
//!     A:X" matched `Call` and not `CallIndirect`, though the two return
//!     identically. A struct or a slice from `DEV.get()` matched none of them.
//!
//! What keeps the next one from being silent is the guard: a slot wider than
//! the registers that reaches the store is now an error naming the type and
//! its width, because reaching there means every copy path declined it.

use crate::common::exec::run;

const PRE: &str = r#"
    const OUT0: addr = 0x0900;
    const OUT1: addr = 0x0901;
    const OUT2: addr = 0x0902;
    const T: [u8; 5] = [10, 20, 30, 40, 50];
"#;

/// Run `body` and report the three output cells.
fn outs(body: &str) -> (u8, u8, u8) {
    let mut e = run(&format!(
        "{PRE}#[reset]\nfn main() {{\n{body}\n    loop {{}}\n}}\n"
    ));
    (e.mem(0x0900), e.mem(0x0901), e.mem(0x0902))
}

// ---------------------------------------------------------------------------
// Slice descriptors copy whole.
// ---------------------------------------------------------------------------

#[test]
fn binding_a_slice_from_a_slice_copies_all_four_bytes() {
    // Base *and* length: reading an element proves the pointer arrived, and
    // `.len` proves the two bytes past it did.
    assert_eq!(
        outs(
            "    let a: &[u8] = T[1..4];\n\
              \x20   let b: &[u8] = a;\n\
              \x20   OUT0 = b[0]; OUT1 = b[2]; OUT2 = b.len as u8;"
        ),
        (20, 40, 3)
    );
}

#[test]
fn assigning_a_slice_from_a_slice_copies_all_four_bytes() {
    // The destination starts as a *different* slice, so a partial copy leaves
    // recognisable wreckage rather than an uninitialized slot that might
    // happen to read right.
    assert_eq!(
        outs(
            "    let a: &[u8] = T[0..2];\n\
              \x20   let b: &[u8] = T[3..5];\n\
              \x20   b = a;\n\
              \x20   OUT0 = b[0]; OUT1 = b[1]; OUT2 = b.len as u8;"
        ),
        (10, 20, 2)
    );
}

#[test]
fn a_copied_slice_is_independent_of_its_source() {
    // A copy, not an alias: re-pointing `a` must leave `b` where it was.
    assert_eq!(
        outs(
            "    let a: &[u8] = T[0..2];\n\
              \x20   let b: &[u8] = a;\n\
              \x20   a = T[3..5];\n\
              \x20   OUT0 = b[0]; OUT1 = a[0]; OUT2 = b.len as u8;"
        ),
        (10, 40, 2)
    );
}

#[test]
fn a_slice_copies_out_of_a_parameter() {
    // A slice parameter holds its descriptor inline — the caller copies all
    // four bytes into the slot — unlike a struct parameter, which holds a
    // pointer. So it is a source here, and the copy has to know that.
    let mut e = run(&format!(
        "{PRE}fn second(s: &[u8]) -> u8 {{ let t: &[u8] = s; return t[1]; }}\n\
         fn length(s: &[u8]) -> u8 {{ let t: &[u8] = s; return t.len as u8; }}\n\
         #[reset]\nfn main() {{\n\
         \x20   OUT0 = second(T[2..5]);\n\
         \x20   OUT1 = length(T[2..5]);\n    loop {{}}\n}}\n"
    ));
    assert_eq!((e.mem(0x0900), e.mem(0x0901)), (40, 3));
}

#[test]
fn assigning_a_slice_to_itself_changes_nothing() {
    // The copy would otherwise read bytes it has already overwritten. It
    // cannot here — the four moves go low to high through disjoint slots — but
    // the same-slot case is skipped outright rather than relying on that.
    assert_eq!(
        outs(
            "    let a: &[u8] = T[1..4];\n\
              \x20   a = a;\n\
              \x20   OUT0 = a[0]; OUT1 = a[2]; OUT2 = a.len as u8;"
        ),
        (20, 40, 3)
    );
}

#[test]
fn a_slice_still_binds_from_a_range_and_from_a_call() {
    // The two shapes that already worked, so the new case in front of them
    // does not shadow either.
    let mut e = run(&format!(
        "{PRE}fn mk() -> &[u8] {{ return T[1..3]; }}\n\
         #[reset]\nfn main() {{\n\
         \x20   let a: &[u8] = T[0..2];\n\
         \x20   let b: &[u8] = mk();\n\
         \x20   OUT0 = a[0]; OUT1 = b[0]; OUT2 = b.len as u8;\n    loop {{}}\n}}\n"
    ));
    assert_eq!((e.mem(0x0900), e.mem(0x0901), e.mem(0x0902)), (10, 20, 2));
}

// ---------------------------------------------------------------------------
// A call through a function pointer returns like any other call.
// ---------------------------------------------------------------------------

#[test]
fn a_slice_returned_through_a_function_pointer_binds() {
    let mut e = run(&format!(
        "{PRE}struct D {{ get: fn() -> &[u8] }}\n\
         fn mk() -> &[u8] {{ return T[1..4]; }}\n\
         static DEV: D = D {{ get: mk }};\n\
         #[reset]\nfn main() {{\n\
         \x20   let s: &[u8] = DEV.get();\n\
         \x20   OUT0 = s[0]; OUT1 = s[2]; OUT2 = s.len as u8;\n    loop {{}}\n}}\n"
    ));
    assert_eq!((e.mem(0x0900), e.mem(0x0901), e.mem(0x0902)), (20, 40, 3));
}

#[test]
fn a_struct_returned_through_a_function_pointer_binds() {
    let mut e = run(r#"
        const OUT0: addr = 0x0900;
        const OUT1: addr = 0x0901;
        struct P { x: u8, y: u8 }
        struct D { get: fn() -> P }
        fn mk() -> P { return P { x: 3, y: 39 }; }
        static DEV: D = D { get: mk };
        #[reset]
        fn main() {
            let p: P = DEV.get();
            OUT0 = p.x; OUT1 = p.y;
            loop {}
        }
    "#);
    assert_eq!((e.mem(0x0900), e.mem(0x0901)), (3, 39));
}

#[test]
fn a_struct_returned_through_a_function_pointer_passes_as_an_argument() {
    // The argument site had the same missing case, and there the dropped half
    // is the pointer's *high* byte — the callee then dereferences an address
    // in whatever page the staging slot happened to name.
    let mut e = run(r#"
        const OUT: addr = 0x0900;
        struct P { x: u8, y: u8 }
        struct D { get: fn() -> P }
        fn mk() -> P { return P { x: 4, y: 38 }; }
        fn sum(p: P) -> u8 { return p.x + p.y; }
        static DEV: D = D { get: mk };
        #[reset]
        fn main() { OUT = sum(DEV.get()); loop {} }
    "#);
    assert_eq!(e.mem(0x0900), 42);
}

#[test]
fn a_struct_assigned_from_a_function_pointer_call_copies() {
    let mut e = run(r#"
        const OUT0: addr = 0x0900;
        const OUT1: addr = 0x0901;
        struct P { x: u8, y: u8 }
        struct D { get: fn() -> P }
        fn mk() -> P { return P { x: 5, y: 37 }; }
        static DEV: D = D { get: mk };
        #[reset]
        fn main() {
            let p: P = P { x: 0, y: 0 };
            p = DEV.get();
            OUT0 = p.x; OUT1 = p.y;
            loop {}
        }
    "#);
    assert_eq!((e.mem(0x0900), e.mem(0x0901)), (5, 37));
}

// ---------------------------------------------------------------------------
// Arguments: the same question, at the staging site.
// ---------------------------------------------------------------------------

#[test]
fn a_slice_from_a_call_stages_as_a_descriptor() {
    // `g(mk())`. A returning function leaves a *pointer* to its descriptor in
    // A:X, so the four bytes have to be copied through it. This staged A
    // alone, handing `g` one byte of that pointer as though it were the base
    // of the slice — `s[1]` then read from wherever the low byte pointed.
    let mut e = run(&format!(
        "{PRE}fn mk() -> &[u8] {{ return T[1..4]; }}\n\
         fn second(s: &[u8]) -> u8 {{ return s[1]; }}\n\
         fn length(s: &[u8]) -> u8 {{ return s.len as u8; }}\n\
         #[reset]\nfn main() {{\n\
         \x20   OUT0 = second(mk());\n\
         \x20   OUT1 = length(mk());\n    loop {{}}\n}}\n"
    ));
    assert_eq!((e.mem(0x0900), e.mem(0x0901)), (30, 3));
}

#[test]
fn a_slice_from_a_function_pointer_call_stages_as_a_descriptor() {
    let mut e = run(&format!(
        "{PRE}struct D {{ get: fn() -> &[u8] }}\n\
         fn mk() -> &[u8] {{ return T[2..5]; }}\n\
         fn second(s: &[u8]) -> u8 {{ return s[1]; }}\n\
         static DEV: D = D {{ get: mk }};\n\
         #[reset]\nfn main() {{ OUT0 = second(DEV.get()); loop {{}} }}\n"
    ));
    assert_eq!(e.mem(0x0900), 40);
}

#[test]
fn an_argument_no_staging_path_claims_is_an_error_not_a_byte() {
    // The argument site's version of the guard. A struct is staged as an
    // address and a slice as four bytes; neither is what the register staging
    // below them writes, so an argument that reached it had been declined by
    // every path that knows those conventions.
    crate::common::assert_error_contains(
        &format!(
            "{PRE}fn second(s: &[u8]) -> u8 {{ return s[1]; }}\n\
             #[reset]\nfn main() {{\n\
             \x20   let a: &[u8] = T[0..4];\n\
             \x20   let k: u8 = 1;\n\
             \x20   OUT0 = second(match k {{ 1 => a, _ => a }});\n    loop {{}}\n}}\n"
        ),
        "passed by address or by descriptor",
    );
}

// ---------------------------------------------------------------------------
// The guard itself.
// ---------------------------------------------------------------------------

#[test]
fn an_aggregate_no_copy_path_claims_is_an_error_not_a_byte() {
    // A `match` yielding a slice type checks — the arms agree and the result
    // is a `&[u8]` — and no copy path handles it, because each arm would have
    // to leave a descriptor somewhere the binding can find. What matters is
    // not that this is rejected; it may well be implemented later. It is that
    // reaching the store says so, instead of copying one byte and running.
    crate::common::assert_error_contains(
        &format!(
            "{PRE}#[reset]\nfn main() {{\n\
             \x20   let a: &[u8] = T[0..4];\n\
             \x20   let k: u8 = 1;\n\
             \x20   let b: &[u8] = match k {{ 1 => a, _ => a }};\n\
             \x20   OUT0 = b[0];\n    loop {{}}\n}}\n"
        ),
        "it is 4 bytes",
    );
}

// ---------------------------------------------------------------------------
// Returns: the third site with the same conventions.
// ---------------------------------------------------------------------------

#[test]
fn an_aggregate_relayed_through_a_return_survives() {
    // A return leaves a pointer to the aggregate in A:X, so relaying one — the
    // value arriving as a parameter or from another call, and going straight
    // back out — puts the binding, the argument and the return conventions in
    // a line. Each of the four has a different source shape.
    let mut e = run(r#"
        const OUT0: addr = 0x0900;
        const OUT1: addr = 0x0901;
        const OUT2: addr = 0x0902;
        const T: [u8; 4] = [11, 22, 33, 44];
        struct P { x: u8, y: u8 }
        struct D { get: fn() -> &[u8] }
        fn mk() -> &[u8] { return T[1..4]; }
        static DEV: D = D { get: mk };
        fn relay_indirect() -> &[u8] { return DEV.get(); }
        fn relay_param(s: &[u8]) -> &[u8] { return s; }
        fn relay_struct(p: P) -> P { return p; }
        #[reset]
        fn main() {
            let a: &[u8] = relay_indirect();
            let b: &[u8] = relay_param(T[0..2]);
            let q: P = relay_struct(P { x: 6, y: 7 });
            OUT0 = a[0]; OUT1 = b.len as u8; OUT2 = q.y;
            loop {}
        }
    "#);
    assert_eq!(
        (e.mem(0x0900), e.mem(0x0901), e.mem(0x0902)),
        (22, 2, 7),
        "a slice from an indirect call, a slice from a parameter, a struct from a parameter"
    );
}

#[test]
fn a_slice_assigned_from_a_call_copies_the_descriptor() {
    // The assignment counterpart of the binding above, and the one the two
    // forms had drifted apart on: `let s: &[u8] = mk();` handled a call and
    // `s = mk();` did not. The guard reported it rather than storing one byte
    // of the returned pointer.
    let mut e = run(&format!(
        "{PRE}fn mk(k: u8) -> &[{}] {{\n\
         \x20   if k == 0 {{ return T[0..2]; }}\n\
         \x20   return T[2..5];\n}}\n\
         #[reset]\nfn main() {{\n\
         \x20   let s: &[u8] = T[0..5];\n\
         \x20   s = mk(1);\n\
         \x20   OUT0 = s[0]; OUT1 = s[2]; OUT2 = s.len as u8;\n    loop {{}}\n}}\n",
        "u8"
    ));
    assert_eq!((e.mem(0x0900), e.mem(0x0901), e.mem(0x0902)), (30, 50, 3));
}

/// A tail-recursive call rebinds its own parameters, at their own widths.
///
/// The loop that replaces the `JSR` sized every argument by its *own* type and
/// recognised only the 16-bit primitives, so a slice parameter — four bytes —
/// counted as one. It rebound a quarter of the descriptor, and, because the
/// destination offsets assumed one byte each, wrote the parameter after it
/// *inside* the descriptor. A `u16` parameter followed by anything had the
/// same shape.
///
/// The recursion has to run at least once for any of that to show: at depth 0
/// the loop is never taken and the original parameters are still intact, which
/// is why this checks both.
#[test]
fn a_tail_recursive_call_rebinds_wide_parameters_whole() {
    let mut e = run(r#"
        const OUT0: addr = 0x0900;
        const OUT1: addr = 0x0901;
        const OUT2: addr = 0x0902;
        const OUT3: addr = 0x0903;
        const T: [u8; 4] = [11, 22, 33, 44];
        fn rec(d: u8, sp: &[u8], tail: u8) -> u8 {
            if d == 0 { return sp[0] + tail + (sp.len as u8); }
            return rec(d - 1, sp, tail);
        }
        fn wide(d: u8, w: u16, tail: u8) -> u8 {
            if d == 0 { return (w as u8) + tail; }
            return wide(d - 1, w, tail);
        }
        #[reset]
        fn main() {
            let s: &[u8] = T[1..4];
            OUT0 = rec(0, s, 5);
            OUT1 = rec(3, s, 5);
            OUT2 = wide(0, 300, 7);
            OUT3 = wide(3, 300, 7);
            loop {}
        }
    "#);
    assert_eq!(
        (e.mem(0x0900), e.mem(0x0901)),
        (30, 30),
        "22 + 5 + 3: the descriptor and the parameter after it survive the loop"
    );
    assert_eq!(
        (e.mem(0x0902), e.mem(0x0903)),
        (51, 51),
        "300 truncates to 44, plus 7: a wide parameter does not shift the next one"
    );
}
