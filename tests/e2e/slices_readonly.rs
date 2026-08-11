//! Slices as read-only views over any storage.
//!
//! A slice descriptor is a real 16-bit base pointer plus a length, so nothing
//! about it requires the data to be in RAM. It used to be restricted to
//! zero-page locals ("slice source array must be a zero-page local") purely
//! because that was the only base the materializer knew how to load.
//!
//! Widening it is only safe because a slice is *read-only*: a `const` array
//! lives in ROM, where a store is a silent no-op on real hardware. That is the
//! same split `str` (ROM literal, read-only) and `str<N>` (RAM buffer,
//! writable) already draw, and the write path is rejected in sema with a
//! message that says so.

use crate::common::exec::run;
use crate::common::harness::{CompileResult, compile};

/// The three storage classes a slice can name, each with distinct values so a
/// slice reading the wrong one is visible rather than coincidentally right.
const SOURCES: &str = r#"
    const OUT: addr = 0x0900;
    const OUT1: addr = 0x0901;
    const OUT2: addr = 0x0902;
    const OUT3: addr = 0x0903;
    const CA: [u8; 6] = [10, 20, 30, 40, 50, 60];
    static SA: [u8; 6] = [11, 21, 31, 41, 51, 61];
"#;

#[test]
fn a_slice_of_a_const_array_reads_rom() {
    let mut e = run(&format!(
        "{SOURCES}
        #[reset]
        fn main() {{
            let s: &[u8] = CA[1..4];
            OUT = s[0]; OUT1 = s[1]; OUT2 = s[2];
            OUT3 = s.len as u8;
            loop {{}}
        }}"
    ));
    assert_eq!(
        (e.mem(0x0900), e.mem(0x0901), e.mem(0x0902)),
        (20, 30, 40),
        "CA[1..4] is 20, 30, 40"
    );
    assert_eq!(e.mem(0x0903), 3, "length");
}

#[test]
fn a_slice_of_a_static_array_reads_ram() {
    let mut e = run(&format!(
        "{SOURCES}
        #[reset]
        fn main() {{
            let s: &[u8] = SA[1..4];
            OUT = s[0]; OUT1 = s[1]; OUT2 = s[2];
            OUT3 = s.len as u8;
            loop {{}}
        }}"
    ));
    assert_eq!(
        (e.mem(0x0900), e.mem(0x0901), e.mem(0x0902)),
        (21, 31, 41),
        "SA[1..4] is 21, 31, 41"
    );
    assert_eq!(e.mem(0x0903), 3, "length");
}

#[test]
fn a_slice_of_a_local_array_still_works() {
    let mut e = run(&format!(
        "{SOURCES}
        #[reset]
        fn main() {{
            let la: [u8; 6] = [12, 22, 32, 42, 52, 62];
            let s: &[u8] = la[1..4];
            OUT = s[0]; OUT1 = s[1]; OUT2 = s[2];
            OUT3 = s.len as u8;
            loop {{}}
        }}"
    ));
    assert_eq!(
        (e.mem(0x0900), e.mem(0x0901), e.mem(0x0902)),
        (22, 32, 42),
        "la[1..4] is 22, 32, 42"
    );
    assert_eq!(e.mem(0x0903), 3, "length");
}

/// All three live at once, so a base loaded from the wrong storage class shows
/// up as one slice reading another's data.
#[test]
fn slices_over_different_storage_do_not_alias() {
    let mut e = run(&format!(
        "{SOURCES}
        #[reset]
        fn main() {{
            let la: [u8; 6] = [12, 22, 32, 42, 52, 62];
            let c: &[u8] = CA[1..4];
            let s: &[u8] = SA[1..4];
            let l: &[u8] = la[1..4];
            OUT = c[0]; OUT1 = s[0]; OUT2 = l[0];
            loop {{}}
        }}"
    ));
    assert_eq!(
        (e.mem(0x0900), e.mem(0x0901), e.mem(0x0902)),
        (20, 21, 22),
        "const, static and local slices each read their own array"
    );
}

/// Runtime bounds take a different code path from constant ones (the offset is
/// computed into scratch and added), so each base kind needs covering there too.
#[test]
fn runtime_bounds_work_over_every_storage_class() {
    let mut e = run(&format!(
        "{SOURCES}
        #[reset]
        fn main() {{
            let la: [u8; 6] = [12, 22, 32, 42, 52, 62];
            let i: u8 = 1;
            let j: u8 = 4;
            let c: &[u8] = CA[i..j];
            let s: &[u8] = SA[i..j];
            let l: &[u8] = la[i..j];
            OUT = c[0]; OUT1 = s[0]; OUT2 = l[0];
            OUT3 = c.len as u8;
            loop {{}}
        }}"
    ));
    assert_eq!(
        (e.mem(0x0900), e.mem(0x0901), e.mem(0x0902)),
        (20, 21, 22),
        "runtime-bounded slices read their own array"
    );
    assert_eq!(e.mem(0x0903), 3);
}

/// A slice is a value; passing a *bound* one to a function must carry the base
/// across whatever storage it came from. (The expression form is covered
/// below.)
#[test]
fn a_rom_backed_slice_survives_being_passed_to_a_function() {
    let mut e = run(&format!(
        "{SOURCES}
        fn total(v: &[u8]) -> u8 {{
            let acc: u8 = 0;
            let n: u8 = v.len as u8;
            for i in 0..n {{ acc = acc + v[i]; }}
            return acc;
        }}
        #[reset]
        fn main() {{
            let c: &[u8] = CA[1..4];
            let s: &[u8] = SA[1..4];
            OUT = total(c);
            OUT1 = total(s);
            loop {{}}
        }}"
    ));
    assert_eq!(e.mem(0x0900), 90, "20 + 30 + 40");
    assert_eq!(e.mem(0x0901), 93, "21 + 31 + 41");
}

#[test]
fn a_rom_backed_slice_can_be_iterated_and_resliced() {
    let mut e = run(&format!(
        "{SOURCES}
        #[reset]
        fn main() {{
            let s: &[u8] = CA[1..5];
            let acc: u8 = 0;
            for x in s {{ acc = acc + x; }}
            OUT = acc;
            let t: &[u8] = s[1..3];
            OUT1 = t[0];
            OUT2 = t.len as u8;
            loop {{}}
        }}"
    ));
    assert_eq!(e.mem(0x0900), 140, "20 + 30 + 40 + 50");
    assert_eq!(e.mem(0x0901), 30, "re-slice offsets compose");
    assert_eq!(e.mem(0x0902), 2);
}

// ---------------------------------------------------------------------------
// Read-only is what makes the above safe
// ---------------------------------------------------------------------------

fn write_error(src: &str) -> String {
    match compile(src) {
        CompileResult::SemaError(e) | CompileResult::CodegenError(e) => e,
        other => panic!("expected the write to be rejected, got {other:?}"),
    }
}

/// Writing through a slice is rejected wherever the data lives — including the
/// RAM cases, where the store would in fact work. A slice does not carry the
/// mutability of what it points at, so allowing RAM-backed writes would make
/// the same expression legal or not depending on a declaration elsewhere.
#[test]
fn a_slice_cannot_be_written_through() {
    for (kind, decl, src_expr) in [
        ("const", "const A: [u8; 6] = [1,2,3,4,5,6];", "A[1..4]"),
        ("static", "static A: [u8; 6] = [1,2,3,4,5,6];", "A[1..4]"),
        ("local", "", "a[1..4]"),
    ] {
        let local = if kind == "local" {
            "let a: [u8; 6] = [1,2,3,4,5,6];"
        } else {
            ""
        };
        let e = write_error(&format!(
            "const OUT: addr = 0x0900;\n{decl}\n#[reset]\nfn main() {{ {local} let s: &[u8] = {src_expr}; s[0] = 9; OUT = s[0]; loop {{}} }}\n"
        ));
        assert!(
            e.contains("slice") && e.contains("read-only"),
            "the {kind} case should be rejected as a read-only slice, got:\n{e}"
        );
    }
}

/// The diagnostic has to point at the write and name the way forward, not leave
/// the reader guessing which of several index forms was unsupported.
#[test]
fn the_slice_write_diagnostic_is_actionable() {
    let e = write_error(
        "const OUT: addr = 0x0900;\n#[reset]\nfn main() { let a: [u8; 6] = [1,2,3,4,5,6]; let s: &[u8] = a[1..4]; s[0] = 9; OUT = a[1]; loop {} }\n",
    );
    assert!(e.contains("^"), "expected a caret excerpt:\n{e}");
    assert!(
        e.contains("read-only"),
        "expected the rule to be stated:\n{e}"
    );
    assert!(
        e.contains("Write to the array"),
        "expected the workaround (index the array itself):\n{e}"
    );
}

/// Reading is unaffected by the write rule.
#[test]
fn a_slice_can_still_be_read_after_the_array_changes() {
    let mut e = run(r#"
        const OUT: addr = 0x0900;
        const OUT1: addr = 0x0901;
        #[reset]
        fn main() {
            let a: [u8; 6] = [1,2,3,4,5,6];
            let s: &[u8] = a[1..4];
            OUT = s[0];
            a[1] = 99;
            OUT1 = s[0];
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0900), 2, "before the array is written");
    assert_eq!(
        e.mem(0x0901),
        99,
        "a slice is a view, so it sees the array's new value"
    );
}

// ---------------------------------------------------------------------------
// Slice expressions as call arguments
//
// A slice argument is staged as its 4-byte descriptor in the argument temp
// pool, which is a zero-page address — and the materializer writes a descriptor
// to exactly such an address. So `total(a[1..4])` builds in place, with no
// intermediate to allocate and nothing to copy afterwards. It used to be
// rejected outright ("Slice expressions can only be used as assignment
// targets"), forcing a `let` binding at every call site.
// ---------------------------------------------------------------------------

#[test]
fn a_slice_expression_can_be_a_call_argument() {
    let mut e = run(&format!(
        "{SOURCES}
        fn total(v: &[u8]) -> u8 {{
            let acc: u8 = 0;
            let n: u8 = v.len as u8;
            for i in 0..n {{ acc = acc + v[i]; }}
            return acc;
        }}
        #[reset]
        fn main() {{
            let la: [u8; 6] = [12, 22, 32, 42, 52, 62];
            OUT = total(CA[1..4]);
            OUT1 = total(SA[1..4]);
            OUT2 = total(la[1..4]);
            loop {{}}
        }}"
    ));
    assert_eq!(e.mem(0x0900), 90, "const-backed: 20 + 30 + 40");
    assert_eq!(e.mem(0x0901), 93, "static-backed: 21 + 31 + 41");
    assert_eq!(e.mem(0x0902), 96, "local-backed: 22 + 32 + 42");
}

/// The bound is materialized into the staging slot, so a runtime bound has to
/// survive being computed there.
#[test]
fn a_slice_expression_argument_accepts_runtime_bounds() {
    let mut e = run(&format!(
        "{SOURCES}
        fn first(v: &[u8]) -> u8 {{ return v[0]; }}
        fn count(v: &[u8]) -> u8 {{ return v.len as u8; }}
        #[reset]
        fn main() {{
            let i: u8 = 2;
            let j: u8 = 5;
            OUT = first(CA[i..j]);
            OUT1 = count(CA[i..j]);
            loop {{}}
        }}"
    ));
    assert_eq!(e.mem(0x0900), 30, "CA[2] is 30");
    assert_eq!(e.mem(0x0901), 3, "2..5 is three elements");
}

/// Two slice arguments in one call get distinct staging slots; if they shared
/// one, both parameters would see the second descriptor.
#[test]
fn two_slice_expression_arguments_do_not_share_a_slot() {
    let mut e = run(&format!(
        "{SOURCES}
        fn firsts(a: &[u8], b: &[u8]) -> u8 {{ return a[0] + b[0]; }}
        fn lens(a: &[u8], b: &[u8]) -> u8 {{ return (a.len as u8) * 10 + (b.len as u8); }}
        #[reset]
        fn main() {{
            OUT = firsts(CA[1..3], SA[2..5]);
            OUT1 = lens(CA[1..3], SA[2..5]);
            loop {{}}
        }}"
    ));
    assert_eq!(e.mem(0x0900), 51, "CA[1] = 20 plus SA[2] = 31");
    assert_eq!(e.mem(0x0901), 23, "lengths 2 and 3, kept apart");
}

/// Mixing a slice expression with scalar arguments has to keep every argument's
/// staging slot straight — the slice takes four bytes where a u8 takes one.
#[test]
fn a_slice_expression_argument_mixes_with_scalars() {
    let mut e = run(&format!(
        "{SOURCES}
        fn pick(before: u8, v: &[u8], after: u8) -> u8 {{
            return before + v[0] + after + (v.len as u8);
        }}
        #[reset]
        fn main() {{
            OUT = pick(1, CA[1..4], 2);
            loop {{}}
        }}"
    ));
    assert_eq!(e.mem(0x0900), 26, "1 + 20 + 2 + 3");
}

/// A bound slice variable and a slice expression must reach the callee the same
/// way, so the two forms are interchangeable at a call site.
#[test]
fn the_bound_and_expression_forms_agree() {
    let mut e = run(&format!(
        "{SOURCES}
        fn total(v: &[u8]) -> u8 {{
            let acc: u8 = 0;
            let n: u8 = v.len as u8;
            for i in 0..n {{ acc = acc + v[i]; }}
            return acc;
        }}
        #[reset]
        fn main() {{
            let bound: &[u8] = CA[1..4];
            OUT = total(bound);
            OUT1 = total(CA[1..4]);
            loop {{}}
        }}"
    ));
    assert_eq!(
        e.mem(0x0900),
        e.mem(0x0901),
        "both forms give the same slice"
    );
    assert_eq!(e.mem(0x0900), 90);
}

/// Re-slicing an already-bound slice, in argument position.
#[test]
fn a_reslice_expression_can_be_a_call_argument() {
    let mut e = run(&format!(
        "{SOURCES}
        fn first(v: &[u8]) -> u8 {{ return v[0]; }}
        #[reset]
        fn main() {{
            let s: &[u8] = CA[1..5];
            OUT = first(s[1..3]);
            loop {{}}
        }}"
    ));
    assert_eq!(
        e.mem(0x0900),
        30,
        "CA[1..5] re-sliced from 1 starts at CA[2]"
    );
}

// ---------------------------------------------------------------------------
// Returning a slice expression
//
// A returned slice hands the caller a pointer to its 4-byte descriptor, so the
// descriptor needs storage that outlives the expression. A bound variable has a
// frame slot already; sema now reserves the equivalent for the expression form,
// keyed by its span and colored with the call graph like any local.
// ---------------------------------------------------------------------------

#[test]
fn a_slice_expression_can_be_returned() {
    let mut e = run(&format!(
        "{SOURCES}
        fn middle(v: &[u8]) -> &[u8] {{ return v[1..3]; }}
        #[reset]
        fn main() {{
            let s: &[u8] = CA[0..6];
            let m: &[u8] = middle(s);
            OUT = m[0];
            OUT1 = m[1];
            OUT2 = m.len as u8;
            loop {{}}
        }}"
    ));
    assert_eq!(
        e.mem(0x0900),
        20,
        "CA[1] — offsets compose through the call"
    );
    assert_eq!(e.mem(0x0901), 30);
    assert_eq!(e.mem(0x0902), 2);
}

/// Returning a slice of a `const` array directly, with no parameter involved.
#[test]
fn a_returned_slice_expression_may_name_rom_directly() {
    let mut e = run(&format!(
        "{SOURCES}
        fn table() -> &[u8] {{ return CA[2..5]; }}
        #[reset]
        fn main() {{
            let t: &[u8] = table();
            OUT = t[0];
            OUT1 = t[2];
            OUT2 = t.len as u8;
            loop {{}}
        }}"
    ));
    assert_eq!(e.mem(0x0900), 30, "CA[2]");
    assert_eq!(e.mem(0x0901), 50, "CA[4]");
    assert_eq!(e.mem(0x0902), 3);
}

/// The bound form must keep working, and agree with the expression form.
#[test]
fn the_returned_bound_and_expression_forms_agree() {
    let mut e = run(&format!(
        "{SOURCES}
        fn expr_form(v: &[u8]) -> &[u8] {{ return v[1..4]; }}
        fn bound_form(v: &[u8]) -> &[u8] {{ let m: &[u8] = v[1..4]; return m; }}
        #[reset]
        fn main() {{
            let s: &[u8] = CA[0..6];
            let a: &[u8] = expr_form(s);
            let b: &[u8] = bound_form(s);
            OUT = a[0];
            OUT1 = b[0];
            OUT2 = a.len as u8;
            OUT3 = b.len as u8;
            loop {{}}
        }}"
    ));
    assert_eq!(e.mem(0x0900), e.mem(0x0901), "same base");
    assert_eq!(e.mem(0x0902), e.mem(0x0903), "same length");
    assert_eq!(e.mem(0x0900), 20);
}

/// Two functions each returning a slice expression get their own descriptor
/// slots; sharing one would have the second overwrite the first.
#[test]
fn two_returned_slice_expressions_do_not_share_a_descriptor() {
    let mut e = run(&format!(
        "{SOURCES}
        fn head(v: &[u8]) -> &[u8] {{ return v[0..2]; }}
        fn tail(v: &[u8]) -> &[u8] {{ return v[4..6]; }}
        #[reset]
        fn main() {{
            let s: &[u8] = CA[0..6];
            let h: &[u8] = head(s);
            let t: &[u8] = tail(s);
            OUT = h[0];
            OUT1 = t[0];
            loop {{}}
        }}"
    ));
    assert_eq!(e.mem(0x0900), 10, "CA[0]");
    assert_eq!(e.mem(0x0901), 50, "CA[4]");
}

/// Runtime bounds in a returned slice expression.
#[test]
fn a_returned_slice_expression_accepts_runtime_bounds() {
    let mut e = run(&format!(
        "{SOURCES}
        fn window(v: &[u8], i: u8, j: u8) -> &[u8] {{ return v[i..j]; }}
        #[reset]
        fn main() {{
            let s: &[u8] = CA[0..6];
            let w: &[u8] = window(s, 2, 5);
            OUT = w[0];
            OUT1 = w.len as u8;
            loop {{}}
        }}"
    ));
    assert_eq!(e.mem(0x0900), 30, "CA[2]");
    assert_eq!(e.mem(0x0901), 3);
}

// ---------------------------------------------------------------------------
// Indexing with a 16-bit value
// ---------------------------------------------------------------------------

/// `.len` is a `u16`, so `for i in 0..s.len` types `i` as one and `s[i]` then
/// fails the 8-bit index gate. That is a real constraint — indexed addressing
/// goes through an 8-bit register — so the diagnostic has to name the cast
/// rather than leave the reader comparing two operands.
#[test]
fn a_sixteen_bit_index_explains_the_cast() {
    let e = write_error(
        "const OUT: addr = 0x0900;\n#[reset]\nfn main() { let a: [u8; 6] = [1,2,3,4,5,6]; let s: &[u8] = a[1..4]; let t: u8 = 0; for i in 0..s.len { t = t + s[i]; } OUT = t; loop {} }\n",
    );
    assert!(e.contains("index must be `u8` or `i8`"), "{e}");
    assert!(
        e.contains("[i as u8]"),
        "expected the cast spelled out:\n{e}"
    );
    assert!(
        e.contains("`.len` is a `u16`"),
        "expected the cause named:\n{e}"
    );
}

/// Both spellings the diagnostic suggests must actually work.
#[test]
fn the_suggested_index_casts_work() {
    let mut cast_at_use = run(&format!(
        "{SOURCES}
        #[reset]
        fn main() {{
            let s: &[u8] = CA[1..4];
            let t: u8 = 0;
            for i in 0..s.len {{ t = t + s[i as u8]; }}
            OUT = t;
            loop {{}}
        }}"
    ));
    assert_eq!(cast_at_use.mem(0x0900), 90, "20 + 30 + 40");

    let mut bound_first = run(&format!(
        "{SOURCES}
        #[reset]
        fn main() {{
            let s: &[u8] = CA[1..4];
            let n: u8 = s.len as u8;
            let t: u8 = 0;
            for i in 0..n {{ t = t + s[i]; }}
            OUT = t;
            loop {{}}
        }}"
    ));
    assert_eq!(bound_first.mem(0x0900), 90);
}
