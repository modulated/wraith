//! `#[soa]` — an array of structs stored as one column per field.
//!
//! The reason is the addressing mode. Interleaved, `arr[i].x` has to multiply
//! the index by the element size before it can index at all: on a three-byte
//! record that is `STA tmp / CLC / ADC tmp / CLC / ADC tmp / TAY / LDA base,Y`,
//! seven instructions and nineteen cycles for one byte — and the multiply is
//! recomputed for every field read. In columns the index scales by the field's
//! own size, which for a byte field is not at all: `TAY / LDA col,Y`.
//!
//! The cost is that an element is no longer contiguous and so has no address.
//! That is why the layout is asked for by name: inference would make one added
//! `&arr[i]` flip the whole array back with nothing in the source to show for
//! it. Here the same line is a compile error — the refusals at the bottom.

use crate::common::exec::run;
use crate::common::harness::{CompileResult, compile, compile_success};

fn expect_error(src: &str) -> String {
    match compile(src) {
        CompileResult::SemaError(e) | CompileResult::CodegenError(e) => e,
        CompileResult::ParseError(e) => e,
        CompileResult::Success(..) => panic!("expected a compile error, but it compiled"),
        other => panic!("expected an error, got {other:?}"),
    }
}

fn warnings_of(src: &str) -> String {
    match compile(src) {
        CompileResult::Success(warnings, _) => warnings,
        other => panic!("expected the program to compile, got {other:?}"),
    }
}

/// The body of `main`, without the comments or the static-init preamble.
fn main_body(asm: &str) -> Vec<String> {
    asm.lines()
        .skip_while(|l| l.trim() != "main:")
        .take_while(|l| !l.trim_start().starts_with("lp_"))
        .map(|l| l.trim().to_string())
        .filter(|l| !l.starts_with(';') && !l.is_empty())
        .collect()
}

/// The `.BYTE` line under a named const array.
fn const_bytes(asm: &str, name: &str) -> Vec<u8> {
    let label = format!("{name}:");
    let line = asm
        .lines()
        .skip_while(|l| l.trim() != label)
        .nth(1)
        .unwrap_or_else(|| panic!("no data line under `{label}`:\n{asm}"));
    line.trim()
        .strip_prefix(".BYTE ")
        .unwrap_or_else(|| panic!("expected a `.BYTE` line under `{label}`, got `{line}`"))
        .split(", ")
        .map(|b| u8::from_str_radix(b.trim().trim_start_matches('$'), 16).unwrap())
        .collect()
}

// ============================================================================
// The addressing mode, which is the whole point
// ============================================================================

#[test]
fn a_column_read_indexes_without_multiplying() {
    let asm = compile_success(
        r#"
        struct Sprite { x: u8, y: u8, hp: u8 }
        #[soa]
        static S: [Sprite; 8] = [Sprite { x: 0, y: 0, hp: 0 }; 8];
        const OUT: addr = 0x0900;
        #[reset]
        fn main() {
            let i: u8 = 3;
            OUT = S[i].y;
            loop {}
        }
    "#,
    );
    let body = main_body(&asm);
    assert!(
        !body.iter().any(|l| l.starts_with("ADC")),
        "a column read should not multiply the index:\n{}",
        body.join("\n")
    );
    assert!(
        body.iter()
            .any(|l| l.starts_with("LDA $04") && l.ends_with(",Y")),
        "expected an absolute,Y load from a column:\n{}",
        body.join("\n")
    );
}

#[test]
fn the_interleaved_form_still_multiplies() {
    // The comparison the attribute exists to offer. Without this, a change that
    // stopped scaling *every* array-of-struct index would leave the test above
    // passing and the language broken.
    let asm = compile_success(
        r#"
        struct Sprite { x: u8, y: u8, hp: u8 }
        static S: [Sprite; 8] = [Sprite { x: 0, y: 0, hp: 0 }; 8];
        const OUT: addr = 0x0900;
        #[reset]
        fn main() {
            let i: u8 = 3;
            OUT = S[i].y;
            loop {}
        }
    "#,
    );
    let body = main_body(&asm);
    assert_eq!(
        body.iter().filter(|l| l.starts_with("ADC")).count(),
        2,
        "an interleaved read of a 3-byte record scales by repeated addition:\n{}",
        body.join("\n")
    );
}

#[test]
fn a_two_byte_field_scales_by_two_not_by_the_element() {
    // A column of `u16`s is indexed by doubling — one shift — where the
    // interleaved form would still multiply by the whole record.
    let asm = compile_success(
        r#"
        struct Ent { hp: u8, pos: u16 }
        #[soa]
        static E: [Ent; 8] = [Ent { hp: 0, pos: 0 }; 8];
        const OUT: addr = 0x0900;
        #[reset]
        fn main() {
            let i: u8 = 3;
            OUT = E[i].pos.low;
            loop {}
        }
    "#,
    );
    let body = main_body(&asm);
    assert!(
        body.contains(&"ASL A".to_string()),
        "a u16 column indexes by doubling:\n{}",
        body.join("\n")
    );
    assert!(
        !body.iter().any(|l| l.starts_with("ADC")),
        "doubling is a shift, not a multiply:\n{}",
        body.join("\n")
    );
}

// ============================================================================
// Where the bytes actually land
// ============================================================================

#[test]
fn a_const_array_is_emitted_as_columns() {
    let asm = compile_success(
        r#"
        struct Ent { hp: u8, pos: u16, tag: u8 }
        #[soa]
        const T: [Ent; 3] = [
            Ent { hp: 1, pos: 0x0201, tag: 3 },
            Ent { hp: 4, pos: 0x0605, tag: 6 },
            Ent { hp: 7, pos: 0x0A09, tag: 9 },
        ];
        const OUT: addr = 0x0900;
        #[reset]
        fn main() {
            let i: u8 = 1;
            OUT = T[i].hp;
            loop {}
        }
    "#,
    );
    assert_eq!(
        const_bytes(&asm, "T"),
        vec![
            1, 4, 7, // hp
            0x01, 0x02, 0x05, 0x06, 0x09, 0x0A, // pos, low byte first
            3, 6, 9, // tag
        ]
    );
}

#[test]
fn a_static_arrays_startup_image_is_columns_too() {
    // A `static` is written into RAM by the reset handler rather than sitting
    // in ROM: a second path to the same bytes, and the one that would silently
    // write records if it did not know the layout.
    let mut e = run(r#"
        struct Ent { hp: u8, tag: u8 }
        #[soa]
        static E: [Ent; 3] = [
            Ent { hp: 1, tag: 4 },
            Ent { hp: 2, tag: 5 },
            Ent { hp: 3, tag: 6 },
        ];
        const OUT: addr = 0x0900;
        #[reset]
        fn main() {
            let i: u8 = 2;
            OUT = E[i].hp + E[i].tag;
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0900), 3 + 6);
}

// ============================================================================
// Reading and writing through the columns
// ============================================================================

#[test]
fn a_column_round_trips_through_a_runtime_index() {
    let mut e = run(r#"
        struct Ent { hp: u8, tag: u8 }
        #[soa]
        static E: [Ent; 4] = [Ent { hp: 0, tag: 0 }; 4];
        const OUT: addr = 0x0900;
        #[reset]
        fn main() {
            let i: u8 = 0;
            while i < 4 {
                E[i].hp = i + 10;
                E[i].tag = i * 2;
                i = i + 1;
            }
            OUT = E[3].hp + E[2].tag;
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0900), 13 + 4);
}

#[test]
fn writing_one_column_leaves_the_others_alone() {
    // The failure this catches is a column base worked out from the wrong
    // field, which lands a write in a neighbour's run rather than out of
    // bounds — silent, and invisible to a test that only reads back what it
    // just wrote.
    let mut e = run(r#"
        struct Ent { a: u8, b: u8, c: u8 }
        #[soa]
        static E: [Ent; 4] = [Ent { a: 1, b: 2, c: 3 }; 4];
        const OUT: addr = 0x0900;
        #[reset]
        fn main() {
            let i: u8 = 2;
            E[i].b = 99;
            OUT = E[0].a + E[0].b + E[0].c
                + E[1].a + E[1].b + E[1].c
                + E[2].a + E[2].c
                + E[3].a + E[3].b + E[3].c;
            loop {}
        }
    "#);
    // Every untouched byte still holds its initial value: 1+2+3 three times
    // over, less the b that was overwritten.
    assert_eq!(e.mem(0x0900), (1 + 2 + 3) * 4 - 2);
}

#[test]
fn a_two_byte_column_keeps_both_bytes() {
    let mut e = run(r#"
        struct Ent { tag: u8, pos: u16 }
        #[soa]
        static E: [Ent; 4] = [Ent { tag: 0, pos: 0 }; 4];
        const LO: addr = 0x0900;
        const HI: addr = 0x0901;
        #[reset]
        fn main() {
            let i: u8 = 2;
            E[i].pos = 0xBEEF;
            E[1].pos = 0x1234;
            LO = E[2].pos.low;
            HI = E[2].pos.high;
            loop {}
        }
    "#);
    assert_eq!((e.mem(0x0900), e.mem(0x0901)), (0xEF, 0xBE));
}

#[test]
fn a_constant_index_resolves_to_one_absolute_address() {
    // The shape matters: for many combinations of length, field count and
    // index the two layouts name the *same* byte by coincidence — with four
    // elements of three bytes, `E[3].c` is base + 11 either way. Five elements
    // pulls them apart, so this actually distinguishes a column address from a
    // composed record address.
    let asm = compile_success(
        r#"
        struct Ent { a: u8, b: u8, c: u8 }
        #[soa]
        static E: [Ent; 5] = [Ent { a: 0, b: 0, c: 0 }; 5];
        const OUT: addr = 0x0900;
        #[reset]
        fn main() {
            OUT = E[3].c;
            loop {}
        }
    "#,
    );
    let body = main_body(&asm);
    // Column c starts after five `a`s and five `b`s, so E[3].c is base + 13.
    assert!(
        body.contains(&"LDA $040D".to_string()),
        "expected a direct load from the c column at base + 13, not the \
         interleaved base + 11:\n{}",
        body.join("\n")
    );
}

#[test]
fn a_pointer_to_one_field_is_still_allowed() {
    // A *field* has an address even when the element does not — it is one byte
    // in one column — so the refusal must not overreach.
    let mut e = run(r#"
        struct Ent { hp: u8, tag: u8 }
        #[soa]
        static E: [Ent; 4] = [Ent { hp: 7, tag: 0 }; 4];
        const OUT: addr = 0x0900;
        #[reset]
        fn main() {
            let p: &u8 = &E[1].hp;
            *p = *p + 1;
            OUT = E[1].hp;
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0900), 8);
}

// ============================================================================
// What it refuses, and why
// ============================================================================

fn refusal(body: &str) -> String {
    expect_error(&format!(
        r#"
        struct Ent {{ hp: u8, tag: u8 }}
        #[soa]
        static E: [Ent; 4] = [Ent {{ hp: 1, tag: 2 }}; 4];
        const OUT: addr = 0x0900;
        fn take(e: Ent) -> u8 {{ return e.hp; }}
        fn point(p: &Ent) -> u8 {{ return p.hp; }}
        fn span(s: &[Ent]) -> u8 {{ return s.len.low; }}
        #[reset]
        fn main() {{ {body} loop {{}} }}
    "#
    ))
}

#[test]
fn binding_a_whole_element_is_refused() {
    let err = refusal("let e: Ent = E[1]; OUT = e.hp;");
    assert!(
        err.contains("no address of its own"),
        "expected the unaddressable-element refusal, got:\n{err}"
    );
}

#[test]
fn taking_the_address_of_an_element_is_refused() {
    let err = refusal("OUT = point(&E[1]);");
    assert!(err.contains("no address of its own"), "got:\n{err}");
}

#[test]
fn passing_a_whole_element_is_refused() {
    let err = refusal("OUT = take(E[1]);");
    assert!(err.contains("no address of its own"), "got:\n{err}");
}

#[test]
fn assigning_a_whole_element_is_refused() {
    let err = refusal("E[1] = Ent { hp: 5, tag: 6 }; OUT = E[1].hp;");
    assert!(err.contains("no address of its own"), "got:\n{err}");
}

#[test]
fn slicing_an_soa_array_is_refused() {
    // A slice is a base and a length over contiguous elements, which is exactly
    // what columns are not. It reaches elements without ever forming an index
    // for one, so it needs its own refusal.
    let err = refusal("OUT = span(E[0..2]);");
    assert!(err.contains("no address of its own"), "got:\n{err}");
}

#[test]
fn the_refusal_names_the_array_and_a_way_forward() {
    let err = refusal("let e: Ent = E[1]; OUT = e.hp;");
    assert!(
        err.contains('E'),
        "the refusal should name the array:\n{err}"
    );
    assert!(
        err.contains("E[i].hp") && err.contains("remove `#[soa]`"),
        "the refusal should offer both ways out:\n{err}"
    );
}

// ============================================================================
// What the attribute itself refuses
// ============================================================================

#[test]
fn soa_on_something_that_is_not_an_array_is_refused() {
    let err = expect_error(
        r#"
        #[soa]
        static X: u8 = 1;
        #[reset]
        fn main() { loop {} }
    "#,
    );
    assert!(err.contains("needs an array type"), "got:\n{err}");
}

#[test]
fn soa_on_an_array_of_scalars_is_refused() {
    let err = expect_error(
        r#"
        #[soa]
        static X: [u8; 4] = [0; 4];
        #[reset]
        fn main() { loop {} }
    "#,
    );
    assert!(err.contains("no fields to make columns of"), "got:\n{err}");
}

#[test]
fn soa_over_an_aggregate_field_is_refused() {
    // A nested struct field would need its own columns — a different feature,
    // and one that would quietly not happen if the size check alone let a
    // two-byte struct through.
    let err = expect_error(
        r#"
        struct Inner { a: u8, b: u8 }
        struct Outer { i: Inner, k: u8 }
        #[soa]
        static X: [Outer; 4] = [Outer { i: Inner { a: 0, b: 0 }, k: 0 }; 4];
        #[reset]
        fn main() { loop {} }
    "#,
    );
    assert!(
        err.contains("scalar of one or two bytes") && err.contains("Outer.i"),
        "got:\n{err}"
    );
}

#[test]
fn an_attribute_a_static_cannot_take_is_refused() {
    // Statics used to drop every attribute on the floor, so this compiled and
    // did nothing.
    let err = expect_error(
        r#"
        #[inline]
        static X: u8 = 1;
        #[reset]
        fn main() { loop {} }
    "#,
    );
    assert!(err.contains("cannot take #[inline]"), "got:\n{err}");
}

#[test]
fn soa_on_an_addr_declaration_is_refused() {
    let err = expect_error(
        r#"
        #[soa]
        const PORT: addr = 0x0900;
        #[reset]
        fn main() { loop {} }
    "#,
    );
    assert!(err.contains("names a fixed location"), "got:\n{err}");
}

// ============================================================================
// The suggestion
// ============================================================================

const FIELD_ONLY: &str = r#"
    struct Ent { hp: u8, tag: u8 }
    static A: [Ent; 4] = [Ent { hp: 1, tag: 2 }; 4];
    const OUT: addr = 0x0900;
    #[reset]
    fn main() {
        let i: u8 = 2;
        A[i].hp = 5;
        OUT = A[i].hp + A[1].tag;
        loop {}
    }
"#;

#[test]
fn an_array_read_only_by_field_is_suggested_for_soa() {
    let w = warnings_of(FIELD_ONLY);
    assert!(
        w.contains("`A`") && w.contains("#[soa]"),
        "expected the suggestion, got:\n{w}"
    );
    assert!(
        w.contains("multiplies the index by 2"),
        "the suggestion should say what the access costs now:\n{w}"
    );
}

#[test]
fn the_suggestion_names_the_cost_as_well_as_the_saving() {
    // A suggestion that only sells the upside is one the reader has to go and
    // discover the downside of.
    let w = warnings_of(FIELD_ONLY);
    assert!(
        w.contains("no longer have an address"),
        "the suggestion should say what columns cost:\n{w}"
    );
}

#[test]
fn an_array_used_whole_anywhere_is_not_suggested() {
    let w = warnings_of(
        r#"
        struct Ent { hp: u8, tag: u8 }
        static A: [Ent; 4] = [Ent { hp: 1, tag: 2 }; 4];
        const OUT: addr = 0x0900;
        fn point(p: &Ent) -> u8 { return p.hp; }
        #[reset]
        fn main() {
            let i: u8 = 2;
            OUT = A[i].hp + point(&A[1]);
            loop {}
        }
    "#,
    );
    assert!(
        !w.contains("#[soa]"),
        "an element used whole cannot take columns:\n{w}"
    );
}

#[test]
fn an_array_named_only_from_a_static_initialiser_is_not_suggested() {
    // A use that never reaches the expression checker still has to count, or
    // the suggestion would be made about an array a pointer already aliases.
    let w = warnings_of(
        r#"
        struct Ent { hp: u8, tag: u8 }
        static A: [Ent; 4] = [Ent { hp: 1, tag: 2 }; 4];
        static P: &Ent = &A;
        const OUT: addr = 0x0900;
        #[reset]
        fn main() {
            let i: u8 = 2;
            OUT = A[i].hp + P.hp;
            loop {}
        }
    "#,
    );
    assert!(!w.contains("#[soa]"), "`A` is aliased by `P`:\n{w}");
}

#[test]
fn an_array_that_already_has_the_attribute_is_not_suggested() {
    let w = warnings_of(
        r#"
        struct Ent { hp: u8, tag: u8 }
        #[soa]
        static A: [Ent; 4] = [Ent { hp: 1, tag: 2 }; 4];
        const OUT: addr = 0x0900;
        #[reset]
        fn main() {
            let i: u8 = 2;
            OUT = A[i].hp;
            loop {}
        }
    "#,
    );
    assert!(!w.contains("#[soa]"), "already has it:\n{w}");
}

#[test]
fn an_array_whose_fields_would_not_take_columns_is_not_suggested() {
    // Suggesting a layout the compiler would then refuse wastes the reader's
    // time twice.
    let w = warnings_of(
        r#"
        struct Inner { a: u8, b: u8 }
        struct Outer { i: Inner, k: u8 }
        static A: [Outer; 4] = [Outer { i: Inner { a: 0, b: 0 }, k: 0 }; 4];
        const OUT: addr = 0x0900;
        #[reset]
        fn main() {
            let n: u8 = 2;
            OUT = A[n].k;
            loop {}
        }
    "#,
    );
    assert!(!w.contains("#[soa]"), "`Outer.i` is an aggregate:\n{w}");
}
