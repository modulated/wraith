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

// ---------------------------------------------------------------------------
// The resolvers themselves, not just the sites that call them.
// ---------------------------------------------------------------------------

/// A struct's address is reachable three ways, and all three still work.
///
/// The guards above are a backstop at the three *call sites* that decide an
/// aggregate's fate. `emit_struct_place_address` now answers in three ways of
/// its own — an address, "not a place", or an error saying it is a place with
/// no case — so a fourth caller added later inherits the check instead of
/// needing its own. What that error costs is a `Denotes` classification that
/// enumerates every expression form, so a construct added to the language has
/// to be placed on one side or the other rather than defaulting to "value".
///
/// This pins the three shapes that must keep resolving. Removing any one of
/// them turns this from a silent one-byte copy into a compiler error, which is
/// the whole point; the test is here so the shapes themselves cannot quietly
/// stop working.
#[test]
fn every_struct_place_still_yields_an_address() {
    let mut e = run(r#"
        const OUT0: addr = 0x0900;
        const OUT1: addr = 0x0901;
        const OUT2: addr = 0x0902;
        struct P { x: u8, y: u8 }
        static PS: [P; 3] = [
            P { x: 1, y: 2 },
            P { x: 4, y: 38 },
            P { x: 7, y: 8 },
        ];
        fn sum(p: P) -> u8 { return p.x + p.y; }
        // A by-reference parameter: the slot holds a pointer to the caller's
        // storage, so the address is a load rather than a constant.
        fn relay(p: P) -> u8 { return sum(p); }
        #[reset]
        fn main() {
            // Inline storage the assembler knows the address of.
            let local: P = P { x: 5, y: 37 };
            OUT0 = sum(local);
            OUT1 = relay(local);
            // An array element whose offset exists only at run time.
            let i: u8 = 1;
            OUT2 = sum(PS[i]);
            loop {}
        }
    "#);
    assert_eq!(
        (e.mem(0x0900), e.mem(0x0901), e.mem(0x0902)),
        (42, 42, 42),
        "inline storage, a by-reference parameter, and a runtime-indexed element"
    );
}

/// Every kind of two-byte parameter survives a tail-recursive call.
///
/// The loop that replaces the `JSR` has to know two things about each
/// parameter: how wide it is, and which register its high byte arrives in. It
/// got the second wrong for the two kinds whose convention the type alone does
/// not settle.
///
/// A `str` is a two-byte address and loads as `LDA #<label / LDX #>label`, but
/// `is_two_byte_value` did not list it at all and `high_byte_in_x` did not
/// either — so it was rebound as one byte, from the wrong register. An enum's
/// value is a two-byte pointer to its data block, also in A:X, but its type is
/// spelled `Named`, which covers structs as well; a struct *field* is stored
/// inline rather than as a pointer, so only the registry can separate them.
///
/// Both came back as zero after one iteration, and at depth 0 — where the loop
/// is never taken — both were fine, which is why this runs each at both depths.
#[test]
fn every_kind_of_parameter_survives_a_tail_call() {
    let cases: [(&str, &str, u8); 6] = [
        (
            "a pointer",
            r#"
            const O0: addr = 0x0900;
            const O1: addr = 0x0901;
            static V: u8 = 77;
            fn rec(d: u8, p: &u8, n: u8) -> u8 { if d == 0 { return *p + n; } return rec(d - 1, p, n); }
            #[reset]
            fn main() { O0 = rec(0, &V, 1); O1 = rec(3, &V, 1); loop {} }
        "#,
            78,
        ),
        (
            "a struct, by reference",
            r#"
            const O0: addr = 0x0900;
            const O1: addr = 0x0901;
            struct P { x: u8, y: u8 }
            fn rec(d: u8, s: P, n: u8) -> u8 { if d == 0 { return s.x + s.y + n; } return rec(d - 1, s, n); }
            #[reset]
            fn main() { let q: P = P { x: 4, y: 38 }; O0 = rec(0, q, 0); O1 = rec(3, q, 0); loop {} }
        "#,
            42,
        ),
        (
            "a str",
            r#"
            const O0: addr = 0x0900;
            const O1: addr = 0x0901;
            fn rec(d: u8, s: str, n: u8) -> u8 { if d == 0 { return (s.len as u8) + n; } return rec(d - 1, s, n); }
            #[reset]
            fn main() { O0 = rec(0, "abcd", 1); O1 = rec(3, "abcd", 1); loop {} }
        "#,
            5,
        ),
        (
            "an enum",
            r#"
            const O0: addr = 0x0900;
            const O1: addr = 0x0901;
            enum C { R, G, B }
            fn rec(d: u8, c: C, n: u8) -> u8 { if d == 0 { return (c as u8) + n; } return rec(d - 1, c, n); }
            #[reset]
            fn main() { O0 = rec(0, C::B, 1); O1 = rec(3, C::B, 1); loop {} }
        "#,
            3,
        ),
        (
            "an array",
            r#"
            const O0: addr = 0x0900;
            const O1: addr = 0x0901;
            fn rec(d: u8, a: [u8; 3], n: u8) -> u8 { if d == 0 { return a[1] + n; } return rec(d - 1, a, n); }
            #[reset]
            fn main() { let arr: [u8; 3] = [5, 6, 7]; O0 = rec(0, arr, 1); O1 = rec(3, arr, 1); loop {} }
        "#,
            7,
        ),
        (
            // Two bytes, high in Y, and neither a number nor an address —
            // the shape most easily left off a hand-written list of the
            // wide types. Rebinding it also has to *read* it back out of
            // the parameter slot rather than treat the name as a label.
            "a function pointer",
            r#"
            const O0: addr = 0x0900;
            const O1: addr = 0x0901;
            fn bump(a: u8) -> u8 { return a + 1; }
            fn rec(d: u8, f: fn(u8) -> u8, n: u8) -> u8 { if d == 0 { return f(6) + n; } return rec(d - 1, f, n); }
            #[reset]
            fn main() { O0 = rec(0, bump, 1); O1 = rec(3, bump, 1); loop {} }
        "#,
            8,
        ),
    ];
    for (what, src, want) in cases {
        let mut e = run(src);
        assert_eq!(
            (e.mem(0x0900), e.mem(0x0901)),
            (want, want),
            "{what} passed through a tail call, at depth 0 and depth 3"
        );
    }
}

/// A `str` is a two-byte address wherever it is stored, not just in a slot.
///
/// `is_two_byte_value` — "the one authoritative answer", so a load or store
/// site cannot drift by re-listing the variants — omitted `Type::String`. A
/// struct literal with a `str` field therefore stored one byte of the pointer
/// and took the other from `Y`, so `d.name.len` read through a half-formed
/// address. Reassigning the same field afterwards worked, which is what kept
/// it hidden: the two paths disagreed.
#[test]
fn a_str_field_is_initialised_with_both_its_bytes() {
    let mut e = run(r#"
        const OUT0: addr = 0x0900;
        const OUT1: addr = 0x0901;
        struct D { name: str, tag: u8 }
        #[reset]
        fn main() {
            let d: D = D { name: "abcd", tag: 1 };
            OUT0 = (d.name.len as u8) + d.tag;
            // The assignment path, which already worked, so the two stay level.
            d.name = "abcdefg";
            OUT1 = (d.name.len as u8) + d.tag;
            loop {}
        }
    "#);
    assert_eq!((e.mem(0x0900), e.mem(0x0901)), (5, 8));
}

/// Every parameter kind, through every way a call is emitted.
///
/// The conventions meet here, and each call form stages its arguments in its
/// own code: a direct `JSR`, an inlined body, a recursive call that saves the
/// callee's frame, and an indirect call through a fixed staging block. Four
/// implementations of one rule is three chances to differ, and they did.
///
/// A struct is passed by *address*. An inlined call copied two bytes out of the
/// argument variable's slot instead — which is right for an array, whose slot
/// holds a pointer to its data, and for a struct *parameter*, whose slot holds
/// the caller's address, but wrong for a struct *local*, whose slot holds the
/// struct itself. `p.x` dereferenced the first two bytes of the contents: with
/// `P { x: 4, y: 38 }` that is the address `$2604`.
#[test]
fn every_parameter_kind_survives_every_call_form() {
    // (name, type, body, argument, expected)
    let kinds: [(&str, &str, &str, &str, u8); 8] = [
        ("a u16", "u16", "(p as u8) + 1", "300", 45),
        ("an i8 widened to i16", "i16", "(p as u8) + 0", "neg", 197),
        ("a pointer", "&u8", "*p + 1", "&V", 78),
        ("a struct", "P", "p.x + p.y", "q", 42),
        ("a str", "str", "(p.len as u8) + 1", "\"abcd\"", 5),
        ("an enum", "C", "(p as u8) + 1", "C::B", 3),
        ("an array", "[u8; 3]", "p[1] + 1", "arr", 7),
        // A function pointer is two bytes with the high byte in Y, like a
        // `u16` — the one two-byte kind that is neither a number nor reached
        // by address, and the one two call forms sized as a single byte.
        ("a function pointer", "fn(u8) -> u8", "p(6) + 1", "bump", 8),
    ];
    for (what, ty, body, arg, want) in kinds {
        // Each form declares `f` its own way and calls it its own way.
        let forms: [(&str, String, String); 4] = [
            (
                "a direct call",
                format!("fn f(p: {ty}) -> u8 {{ return {body}; }}"),
                format!("f({arg})"),
            ),
            (
                "an inlined call",
                format!("#[inline]\nfn f(p: {ty}) -> u8 {{ return {body}; }}"),
                format!("f({arg})"),
            ),
            (
                "a recursive call",
                format!(
                    "fn f(d: u8, p: {ty}) -> u8 {{ if d == 0 {{ return {body}; }} \
                     return f(d - 1, p) + 0; }}"
                ),
                format!("f(3, {arg})"),
            ),
            (
                "a call through a function pointer",
                format!(
                    "fn f(p: {ty}) -> u8 {{ return {body}; }}\n\
                     struct DV {{ call: fn({ty}) -> u8 }}\n\
                     static DEV: DV = DV {{ call: f }};"
                ),
                "DEV.call(ARG)".replace("ARG", arg),
            ),
        ];
        for (form, decl, call) in forms {
            // An array through an indirect call is refused on purpose: the
            // argument is staged at a fixed address for a callee not known
            // until run time, and the diagnostic says to pass a `&T`. Pinned
            // below rather than skipped silently.
            if ty.starts_with('[') && form.contains("function pointer") {
                continue;
            }
            let mut e = run(&format!(
                "const OUT: addr = 0x0900;\n\
                 static V: u8 = 77;\n\
                 struct P {{ x: u8, y: u8 }}\n\
                 enum C {{ R, G, B }}\n\
                 fn bump(a: u8) -> u8 {{ return a + 1; }}\n\
                 {decl}\n\
                 #[reset]\nfn main() {{\n\
                 \x20   let q: P = P {{ x: 4, y: 38 }};\n\
                 \x20   let arr: [u8; 3] = [5, 6, 7];\n\
                 \x20   let neg: i8 = (-59);\n\
                 \x20   OUT = {call};\n    loop {{}}\n}}\n"
            ));
            assert_eq!(e.mem(0x0900), want, "{what} through {form}");
        }
    }
}

/// The one combination above that is refused, and why it is not a gap.
///
/// An indirect call stages its arguments at a fixed block, because the callee
/// is not known until run time and every candidate has to read them from the
/// same place. That suits anything that fits the registers or is reached by
/// address; a whole array does not, and the diagnostic says what to pass
/// instead rather than staging something wrong.
#[test]
fn an_array_through_a_function_pointer_is_refused_with_a_way_out() {
    crate::common::assert_error_contains(
        r#"
        const OUT: addr = 0x0900;
        fn f(p: [u8; 3]) -> u8 { return p[1]; }
        struct DV { call: fn([u8; 3]) -> u8 }
        static DEV: DV = DV { call: f };
        #[reset]
        fn main() { let arr: [u8; 3] = [5, 6, 7]; OUT = DEV.call(arr); loop {} }
    "#,
        "Pass a `&T` to it instead",
    );
}

/// A whole array is never assigned, at any storage class.
///
/// The rule used to depend on where the array lived. A `static` array and a
/// struct field *are* their elements at a fixed address, so there was no
/// pointer to repoint and the store put two bytes of the literal's ROM address
/// over the first two elements; both were refused. A *local* array does have a
/// slot holding a pointer, so repointing it compiled — and meant two things
/// nobody wrote:
///
///   * `a = b` **aliased** `b`'s elements instead of copying them, so a later
///     `b[1] = 99` was visible through `a`.
///   * `a = [1, 2, 3]` pointed `a` at the literal **in ROM**, where every
///     subsequent `a[i] = v` is a silent no-op on real hardware. The emulator
///     harness catches the store; a program on a machine would not.
///
/// Binding already refuses everything but a literal, so refusing assignment
/// outright leaves one rule at every storage class: an array is initialised
/// once and copied explicitly thereafter. That is where the other native 6502
/// languages ended up too — Prog8 forbids it and points at `sys.memcopy`, C
/// forbids it and arrays decay to pointers, and Mad-Pascal dropped Pascal's
/// value semantics to do the same.
#[test]
fn a_whole_array_is_never_assigned_at_any_storage_class() {
    for (what, src) in [
        (
            "a local, from a literal",
            "fn main() { let a: [u8; 3] = [1, 2, 3]; a = [4, 5, 6]; OUT = a[1]; loop {} }",
        ),
        (
            "a local, from another local",
            "fn main() { let a: [u8; 3] = [1, 2, 3]; let b: [u8; 3] = [7, 8, 9]; a = b; \
             OUT = a[1]; loop {} }",
        ),
    ] {
        crate::common::assert_error_contains(
            &format!("const OUT: addr = 0x0900;\n#[reset]\n{src}\n"),
            "an array is initialised once and never assigned as a whole",
        );
        // The way out is named, not just the refusal.
        crate::common::assert_error_contains(
            &format!("const OUT: addr = 0x0900;\n#[reset]\n{src}\n"),
            "memcpy",
        );
        let _ = what;
    }

    crate::common::assert_error_contains(
        r#"
        const OUT: addr = 0x0900;
        static A: [u8; 3] = [1, 2, 3];
        #[reset]
        fn main() { A = [4, 5, 6]; OUT = A[1]; loop {} }
    "#,
        "an array is initialised once and never assigned as a whole",
    );

    crate::common::assert_error_contains(
        r#"
        const OUT: addr = 0x0900;
        struct D { f: [u8; 3] }
        #[reset]
        fn main() { let d: D = D { f: [1, 2, 3] }; d.f = [4, 5, 6]; OUT = d.f[1]; loop {} }
    "#,
        "an array is initialised once and never assigned as a whole",
    );
}

/// The field case names element-wise and *not* `memcpy`, because `&d.f[0]` is
/// not something the compiler can emit — and it says which, rather than
/// reporting an internal error for source it simply does not handle.
#[test]
fn the_address_of_an_element_of_a_field_is_refused_by_name() {
    crate::common::assert_error_contains(
        r#"
        import { memcpy } from "std/mem.wr";
        const OUT: addr = 0x0900;
        struct D { f: [u8; 3] }
        #[reset]
        fn main() {
            let d: D = D { f: [1, 2, 3] };
            let s: [u8; 3] = [7, 8, 9];
            memcpy(&d.f[0], &s[0], 3);
            OUT = d.f[1];
            loop {}
        }
    "#,
        "can only be taken through the array's name",
    );
}

/// What replaces it: element-wise, or an explicit `memcpy`.
///
/// The copy has to be a real one — independent of its source afterwards —
/// which is exactly what the repointing form was not.
#[test]
fn an_array_is_copied_element_wise_or_with_memcpy() {
    let mut e = run(r#"
        import { memcpy } from "std/mem.wr";
        const OUT0: addr = 0x0900;
        const OUT1: addr = 0x0901;
        const OUT2: addr = 0x0902;
        #[reset]
        fn main() {
            let a: [u8; 3] = [1, 2, 3];
            let b: [u8; 3] = [4, 5, 6];
            memcpy(&a[0], &b[0], 3);
            OUT0 = a[0];
            // Independent of its source: changing `b` must not move `a`.
            b[1] = 99;
            OUT1 = a[1];
            // And the destination is still writable, which the ROM repoint
            // silently was not.
            a[2] = 42;
            OUT2 = a[2];
            loop {}
        }
    "#);
    assert_eq!(
        (e.mem(0x0900), e.mem(0x0901), e.mem(0x0902)),
        (4, 5, 42),
        "memcpy copies, the copy is independent, and it stays writable"
    );
}

/// Every kind of struct field, initialised then reassigned then read.
///
/// A field's type decides both how wide the store is and which register the
/// high byte comes from, and the two predicates that answer those had drifted
/// — `Type::String` was missing from both, so a `str` field was a one-byte
/// store from the wrong register.
#[test]
fn every_kind_of_struct_field_round_trips() {
    // (field type, initial value, reassigned value, how to read it, expected)
    let cases: [(&str, &str, &str, &str, u8); 5] = [
        ("u16", "300", "301", "(d.f as u8)", 45),
        ("&u8", "&V", "&W", "*d.f", 88),
        ("str", "\"ab\"", "\"abcd\"", "(d.f.len as u8)", 4),
        ("C", "C::R", "C::B", "(d.f as u8)", 2),
        ("fn(u8) -> u8", "bump", "dbl", "d.f(21)", 42),
    ];
    for (ty, init, reassigned, read, want) in cases {
        let mut e = run(&format!(
            "const OUT0: addr = 0x0900;\n\
             const OUT1: addr = 0x0901;\n\
             static V: u8 = 77;\n\
             static W: u8 = 88;\n\
             enum C {{ R, G, B }}\n\
             fn bump(a: u8) -> u8 {{ return a + 1; }}\n\
             fn dbl(a: u8) -> u8 {{ return a + a; }}\n\
             struct D {{ f: {ty}, tag: u8 }}\n\
             #[reset]\nfn main() {{\n\
             \x20   let d: D = D {{ f: {init}, tag: 9 }};\n\
             \x20   OUT0 = d.tag;\n\
             \x20   d.f = {reassigned};\n\
             \x20   OUT1 = {read};\n    loop {{}}\n}}\n"
        ));
        assert_eq!(
            (e.mem(0x0900), e.mem(0x0901)),
            (9, want),
            "a `{ty}` field: the tag beside it must survive the store, and the field must read back"
        );
    }
}

/// The same question where the struct is a `static` rather than a local.
///
/// A local struct sits in the frame and its fields are reached from a
/// zero-page base; a `static` is at a fixed address and resolves through a
/// different path entirely. The field matrix above exercises only the first.
///
/// Two of the five field kinds cannot get here: a `str` field and an enum
/// field are both refused at the initialiser as *not a compile-time constant*,
/// and a `&u8` field with them. That is a `const`-evaluation gap rather than a
/// codegen one — a `fn` field is accepted, which is the same two bytes of
/// address — and it is recorded on the roadmap rather than worked around here.
/// The two that do get here round-trip, tag intact on both sides of the store.
#[test]
fn a_static_structs_two_byte_fields_round_trip() {
    let cases: [(&str, &str, &str, &str, u8); 2] = [
        ("u16", "300", "301", "(G.f as u8)", 45),
        ("fn(u8) -> u8", "bump", "dbl", "G.f(21)", 42),
    ];
    for (ty, init, reassigned, read, want) in cases {
        let mut e = run(&format!(
            "const OUT0: addr = 0x0900;\n\
             const OUT1: addr = 0x0901;\n\
             const OUT2: addr = 0x0902;\n\
             fn bump(a: u8) -> u8 {{ return a + 1; }}\n\
             fn dbl(a: u8) -> u8 {{ return a + a; }}\n\
             struct D {{ f: {ty}, tag: u8 }}\n\
             static G: D = D {{ f: {init}, tag: 9 }};\n\
             #[reset]\nfn main() {{\n\
             \x20   OUT0 = G.tag;\n\
             \x20   G.f = {reassigned};\n\
             \x20   OUT1 = {read};\n\
             \x20   OUT2 = G.tag;\n    loop {{}}\n}}\n"
        ));
        assert_eq!(
            (e.mem(0x0900), e.mem(0x0901), e.mem(0x0902)),
            (9, want, 9),
            "a `{ty}` field of a static struct: the tag beside it must survive \
             the store, and the field must read back"
        );
    }
}

/// Every kind of return value, out of every form a function is called by.
///
/// The mirror of `every_parameter_kind_survives_every_call_form`. Arguments go
/// in through four separate staging routines and had drifted; returns come back
/// through the register conventions — `A` for a byte, `A:Y` for a 16-bit
/// number, `A:X` for anything reached by address — and these were found to
/// agree. Kept because that is worth knowing, and because the next convention
/// added has somewhere to prove itself.
#[test]
fn every_kind_of_return_value_survives_every_call_form() {
    // (name, type, expression producing one, how to consume it, expected)
    let kinds: [(&str, &str, &str, &str, u8); 6] = [
        ("a u16", "u16", "300", "(r as u8) + 1", 45),
        ("a pointer", "&u8", "&V", "*r + 1", 78),
        ("a struct", "P", "P { x: 4, y: 38 }", "r.x + r.y", 42),
        ("a str", "str", "\"abcd\"", "(r.len as u8) + 1", 5),
        ("an enum", "C", "C::B", "(r as u8) + 1", 3),
        // 20 from T[1], plus a length of 3.
        ("a slice", "&[u8]", "T[1..4]", "r[0] + (r.len as u8)", 23),
    ];
    for (what, ty, produce, consume, want) in kinds {
        let forms: [(&str, String, &str); 4] = [
            (
                "a direct call",
                format!("fn g() -> {ty} {{ return {produce}; }}"),
                "g()",
            ),
            (
                "an inlined call",
                format!("#[inline]\nfn g() -> {ty} {{ return {produce}; }}"),
                "g()",
            ),
            (
                "a recursive call",
                format!(
                    "fn g(d: u8) -> {ty} {{ if d == 0 {{ return {produce}; }} return g(d - 1); }}"
                ),
                "g(2)",
            ),
            (
                "a call through a function pointer",
                format!(
                    "fn g() -> {ty} {{ return {produce}; }}\n\
                     struct DV {{ call: fn() -> {ty} }}\n\
                     static DEV: DV = DV {{ call: g }};"
                ),
                "DEV.call()",
            ),
        ];
        for (form, decl, call) in forms {
            let mut e = run(&format!(
                "const OUT: addr = 0x0900;\n\
                 static V: u8 = 77;\n\
                 const T: [u8; 5] = [10, 20, 30, 40, 50];\n\
                 struct P {{ x: u8, y: u8 }}\n\
                 enum C {{ R, G, B }}\n\
                 {decl}\n\
                 #[reset]\nfn main() {{\n\
                 \x20   let r: {ty} = {call};\n\
                 \x20   OUT = {consume};\n    loop {{}}\n}}\n"
            ));
            assert_eq!(e.mem(0x0900), want, "{what} returned from {form}");
        }
    }
}

/// Field chains, which is what the address resolvers recurse through.
///
/// `resolve_static_addr` and `resolve_static_struct_lvalue` walk a chain a
/// level at a time, and both were changed to answer three ways rather than
/// two. Nothing about that should move a nested access, and this is how that
/// stays true — read and write, a scalar field and a wide one, through a local
/// and through a `static`, and passing an inner struct on by address.
#[test]
fn a_nested_field_chain_resolves_through_every_form() {
    let cases: [(&str, &str, u8); 5] = [
        (
            "reading through a local",
            r#"
            const OUT: addr = 0x0900;
            struct In { a: u8, b: u16 }
            struct Out { tag: u8, inner: In }
            #[reset]
            fn main() {
                let o: Out = Out { tag: 1, inner: In { a: 41, b: 300 } };
                OUT = o.inner.a + o.tag;
                loop {}
            }
        "#,
            42,
        ),
        (
            "writing through a local",
            r#"
            const OUT: addr = 0x0900;
            struct In { a: u8, b: u16 }
            struct Out { tag: u8, inner: In }
            #[reset]
            fn main() {
                let o: Out = Out { tag: 1, inner: In { a: 0, b: 300 } };
                o.inner.a = 41;
                OUT = o.inner.a + o.tag;
                loop {}
            }
        "#,
            42,
        ),
        (
            "a wide nested field, which stores two bytes",
            r#"
            const OUT: addr = 0x0900;
            struct In { a: u8, b: u16 }
            struct Out { tag: u8, inner: In }
            #[reset]
            fn main() {
                let o: Out = Out { tag: 1, inner: In { a: 0, b: 300 } };
                o.inner.b = 301;
                OUT = (o.inner.b as u8) + 1;
                loop {}
            }
        "#,
            46,
        ),
        (
            "through a static, whose base is an address not a slot",
            r#"
            const OUT: addr = 0x0900;
            struct In { a: u8, b: u16 }
            struct Out { tag: u8, inner: In }
            static S: Out = Out { tag: 1, inner: In { a: 41, b: 300 } };
            #[reset]
            fn main() { OUT = S.inner.a + S.tag; loop {} }
        "#,
            42,
        ),
        (
            "passing the inner struct on by address",
            r#"
            const OUT: addr = 0x0900;
            struct In { a: u8, b: u8 }
            struct Out { tag: u8, inner: In }
            fn sum(i: In) -> u8 { return i.a + i.b; }
            #[reset]
            fn main() {
                let o: Out = Out { tag: 1, inner: In { a: 4, b: 38 } };
                OUT = sum(o.inner);
                loop {}
            }
        "#,
            42,
        ),
    ];
    for (what, src, want) in cases {
        let mut e = run(src);
        assert_eq!(e.mem(0x0900), want, "a nested chain, {what}");
    }
}

/// A struct returned by value comes back as its **address** in A:X.
///
/// `return s` used to fall through to the scalar return path: it loaded the
/// struct's first byte into A and left X holding whatever the last instruction
/// happened to leave there. The caller does the right thing — it dereferences
/// A:X and copies the struct's bytes out — so a two-field struct came back as
/// two bytes read from wherever that pair pointed. `mk(7)` returning
/// `S { f0: 7, f1: 8 }` produced `(0, 0)`, read from zero page `$0007`.
///
/// Nothing diagnosed it: the program compiled, ran, and answered wrongly. The
/// slice return had been given this treatment already; the struct one was
/// simply missing, which is the "one rule, several implementations" shape.
///
/// Every way of naming the returned struct is here, because the address comes
/// from a different place in each: a local's frame slot, a by-reference
/// parameter's pointer, a `static`'s fixed address, and a literal that built
/// itself into a temporary and already holds a pointer.
#[test]
fn a_struct_returned_by_value_comes_back_as_its_address() {
    let cases: [(&str, &str, (u8, u8)); 5] = [
        (
            "a local, assigned field by field",
            r#"
            struct S { f0: u8, f1: u8 }
            fn mk(x: u8) -> S {
                let s: S = S { f0: 0, f1: 0 };
                s.f0 = x;
                s.f1 = x + 1;
                return s;
            }
            #[reset]
            fn main() { let r: S = mk(7); O0 = r.f0; O1 = r.f1; loop {} }
            "#,
            (7, 8),
        ),
        (
            "a by-reference parameter, passed straight back",
            r#"
            struct S { f0: u8, f1: u8 }
            fn id(p: S) -> S { return p; }
            #[reset]
            fn main() {
                let s: S = S { f0: 3, f1: 4 };
                let r: S = id(s);
                O0 = r.f0; O1 = r.f1; loop {}
            }
            "#,
            (3, 4),
        ),
        (
            "a literal, which already holds a pointer",
            r#"
            struct S { f0: u8, f1: u8 }
            fn mk(x: u8) -> S { return S { f0: x, f1: 9 }; }
            #[reset]
            fn main() { let r: S = mk(5); O0 = r.f0; O1 = r.f1; loop {} }
            "#,
            (5, 9),
        ),
        (
            "a `static`, at a fixed address and mutable",
            r#"
            struct S { f0: u8, f1: u8 }
            static G: S = S { f0: 1, f1: 2 };
            fn get() -> S { return G; }
            #[reset]
            fn main() { G.f1 = 8; let r: S = get(); O0 = r.f0; O1 = r.f1; loop {} }
            "#,
            (1, 8),
        ),
        (
            "one with an array field, whose elements travel with it",
            r#"
            struct S { f0: u8, a: [u8; 3] }
            fn mk(x: u8) -> S {
                let s: S = S { f0: x, a: [1, 2, 3] };
                s.a[1] = x + 1;
                return s;
            }
            #[reset]
            fn main() { let r: S = mk(4); O0 = r.f0; O1 = r.a[1]; loop {} }
            "#,
            (4, 5),
        ),
    ];

    for (what, body, want) in cases {
        let mut e = run(&format!(
            "const O0: addr = 0x0900;\nconst O1: addr = 0x0901;\n{body}"
        ));
        assert_eq!((e.mem(0x0900), e.mem(0x0901)), want, "returning {what}");
    }
}

/// An array field is stored inline, and is reached the same way wherever the
/// struct lives.
///
/// The field's elements are part of the struct's bytes, not a pointer to them,
/// so the address arithmetic is the struct's base plus the field's offset plus
/// the scaled index — three terms, and a 16-bit element scales the last one.
#[test]
fn an_array_field_is_stored_inline_at_either_width() {
    // A local, both indices, with a scalar field on each side of the array so a
    // wrong offset lands somewhere visible.
    let mut e = run(r#"
        const O0: addr = 0x0900;
        const O1: addr = 0x0901;
        const O2: addr = 0x0902;
        const O3: addr = 0x0903;
        struct S { f0: u8, a: [u8; 4], f1: u8 }
        #[reset]
        fn main() {
            let s: S = S { f0: 11, a: [1, 2, 3, 4], f1: 22 };
            s.a[2] = 99;
            let i: u8 = 3;
            s.a[i] = 77;
            O0 = s.f0;
            O1 = s.a[2];
            O2 = s.a[i];
            O3 = s.f1;
            loop {}
        }
    "#);
    assert_eq!(
        (e.mem(0x0900), e.mem(0x0901), e.mem(0x0902), e.mem(0x0903)),
        (11, 99, 77, 22),
        "a local struct's array field, constant and run-time index"
    );

    // A `static`, which is a fixed address rather than a frame slot.
    let mut e = run(r#"
        const O0: addr = 0x0900;
        const O1: addr = 0x0901;
        struct S { f0: u8, a: [u8; 4] }
        static G: S = S { f0: 11, a: [1, 2, 3, 4] };
        #[reset]
        fn main() { G.a[2] = 99; O0 = G.a[2]; O1 = G.a[1]; loop {} }
    "#);
    assert_eq!(
        (e.mem(0x0900), e.mem(0x0901)),
        (99, 2),
        "a static struct's array field"
    );

    // 16-bit elements: the index scales by two, and both bytes move.
    let mut e = run(r#"
        const O0: addr = 0x0900;
        const O2: addr = 0x0902;
        struct S { f0: u16, a: [u16; 4] }
        #[reset]
        fn main() {
            let s: S = S { f0: 1111, a: [1000, 2000, 3000, 4000] };
            let i: u8 = 3;
            s.a[i] = 40000;
            let x: u16 = s.a[i];
            O0 = x.low; O2 = x.high;
            loop {}
        }
    "#);
    assert_eq!(
        (e.mem(0x0900), e.mem(0x0902)),
        ((40000u16 & 0xFF) as u8, (40000u16 >> 8) as u8),
        "a 16-bit array field carries both bytes"
    );
}

/// A struct behind a pointer is *one* piece of storage, not a copy.
///
/// The callee writes a field through `&S` and the caller sees it, which is the
/// whole reason the parameter is by reference.
#[test]
fn a_struct_behind_a_pointer_is_one_piece_of_storage() {
    let mut e = run(r#"
        const O0: addr = 0x0900;
        const O1: addr = 0x0901;
        struct S { f0: u8, f1: u8 }
        fn bump(p: &S) -> u8 {
            p.f0 = p.f0 + 1;
            return p.f1;
        }
        #[reset]
        fn main() {
            let s: S = S { f0: 5, f1: 9 };
            O1 = bump(&s);
            O0 = s.f0;
            loop {}
        }
    "#);
    assert_eq!(
        (e.mem(0x0900), e.mem(0x0901)),
        (6, 9),
        "a write through &S is visible to the caller"
    );
}

/// The same for a `&T` to a scalar: two names, one byte, in both directions.
#[test]
fn a_pointer_is_a_second_name_for_one_byte() {
    let mut e = run(r#"
        const O0: addr = 0x0900;
        const O1: addr = 0x0901;
        #[reset]
        fn main() {
            let x: u8 = 5;
            let p: &u8 = &x;
            *p = *p + 1;
            O0 = x;          // the write through `p` is visible as `x`
            x = 20;
            O1 = *p;         // and the write to `x` is visible through `p`
            loop {}
        }
    "#);
    assert_eq!((e.mem(0x0900), e.mem(0x0901)), (6, 20));
}

/// Two shapes in this family are *refused*, and the tests say so rather than
/// leaving it to be discovered.
///
/// Both are ordinary-looking source with no way to spell the same thing, so
/// they are on the roadmap. What matters here is that they are refused rather
/// than miscompiled — the failure mode this whole file exists for.
#[test]
fn a_field_of_a_call_and_an_indexed_field_of_a_pointer_are_refused() {
    crate::common::assert_error_contains(
        r#"
        const O0: addr = 0x0900;
        struct S { f0: u8, f1: u8 }
        fn mk(x: u8) -> S { return S { f0: x, f1: 9 }; }
        #[reset]
        fn main() { O0 = mk(6).f1; loop {} }
        "#,
        "Field access only supported on variables",
    );

    crate::common::assert_error_contains(
        r#"
        const O0: addr = 0x0900;
        struct S { f0: u8, a: [u8; 3] }
        fn peek(p: &S, i: u8) -> u8 { return p.a[i]; }
        #[reset]
        fn main() {
            let s: S = S { f0: 0, a: [7, 8, 9] };
            O0 = peek(&s, 2);
            loop {}
        }
        "#,
        "only variable array indexing is currently supported",
    );
}

/// A constant struct with an array field becomes ROM data, elements and all.
///
/// The constant path handled integer and bool fields and refused everything
/// else with "only integer and bool literals supported" — which named the
/// wrong thing, since an array literal *is* a literal. The effect was that
/// `return S { a: [1, 2, 3] }` failed where the same struct bound to a local
/// compiled, so whether an array field was allowed depended on where the
/// literal stood.
#[test]
fn a_constant_struct_lays_out_its_array_field_inline() {
    let mut e = run(r#"
        const O0: addr = 0x0900;
        const O1: addr = 0x0901;
        const O2: addr = 0x0902;
        const O3: addr = 0x0903;
        struct S { f0: u8, a: [u8; 3], f1: u8 }
        fn mks(k: u8) -> S {
            // A constant literal: ROM data, reached by label.
            if k == 0 { return S { f0: 1, a: [2, 3, 4], f1: 5 }; }
            // A local: frame storage, reached by its address.
            let t: S = S { f0: 6, a: [7, 8, 9], f1: 10 };
            return t;
        }
        #[reset]
        fn main() {
            let s: S = S { f0: 0, a: [0, 0, 0], f1: 0 };
            s = mks(0);
            O0 = s.f1; O1 = s.a[2];
            s = mks(1);
            O2 = s.f1; O3 = s.a[2];
            loop {}
        }
    "#);
    assert_eq!(
        (e.mem(0x0900), e.mem(0x0901), e.mem(0x0902), e.mem(0x0903)),
        (5, 4, 10, 9),
        "the scalar after the array field is at the right offset, both ways in"
    );

    // The fill form, a 16-bit element, and a field the literal omits — which
    // is zeroed for its whole length rather than for one element.
    let mut e = run(r#"
        const O0: addr = 0x0900;
        const O2: addr = 0x0902;
        const O4: addr = 0x0904;
        struct S { a: [u16; 3], f: u16 }
        fn mks() -> S { return S { a: [513; 3], f: 40000 }; }
        #[reset]
        fn main() {
            let s: S = S { a: [0; 3], f: 0 };
            s = mks();
            let x: u16 = s.a[2];
            O0 = x.low; O2 = x.high;
            let y: u16 = s.f;
            O4 = y.low;
            loop {}
        }
    "#);
    assert_eq!(
        (e.mem(0x0900), e.mem(0x0902), e.mem(0x0904)),
        (1, 2, (40000u16 & 0xFF) as u8),
        "a 16-bit array field fills two bytes per element, so the field after it lands right"
    );
}

/// Dereferencing a pointer *to a pointer* loads both bytes.
///
/// `*pp` decided "two bytes" by re-listing `u16`/`i16`/`b16`, which leaves out
/// the two-byte values that are addresses — a `&T` and a function pointer. So
/// `let q: &u8 = *pp;` loaded one byte into A and the binding stored whatever
/// X happened to hold as the high half: `q` came out as `$0000` where `p`
/// named `$0400`, and both the read and the write through it landed in zero
/// page instead. The store side had already been fixed this way; this is the
/// other half.
#[test]
fn a_pointer_read_through_a_pointer_keeps_its_high_byte() {
    let mut e = run(r#"
        const O0: addr = 0x0900;
        const O1: addr = 0x0901;
        // A `static`, so the address has a non-zero high byte — the half that
        // was being dropped. A zero-page local would hide the bug.
        static G: u8 = 7;
        #[reset]
        fn main() {
            let p: &u8 = &G;
            let pp: &&u8 = &p;
            let q: &u8 = *pp;
            O0 = *q;      // reads G through two levels
            *q = 9;       // and writes it
            O1 = G;
            loop {}
        }
    "#);
    assert_eq!((e.mem(0x0900), e.mem(0x0901)), (7, 9));
}
