//! Assigning to `.low` and `.high` of a 16-bit value.
//!
//! Reading them always worked — `w.low` is one `LDA` at the value's address,
//! `w.high` one at the next byte — but there was no store path, so `w.low = v`
//! failed with the generic "Only variable, index, field, and slice assignment
//! supported". Nothing was behind the gap: the destination of a 16-bit
//! accessor is known before the value is evaluated, so the store is a single
//! `STA` and needs none of the staging an array element or a struct field
//! does.
//!
//! Sema had a matching hole. `lvalue_root` peels a chain to the thing that has
//! to be mutable, and it only peeled `.low`/`.high` when sema had re-resolved
//! them as a *struct field* of that name. A genuine accessor stopped the walk,
//! so the mutability and ROM checks never ran — invisible while codegen
//! rejected everything, and a store into ROM the moment it did not.

use crate::common::exec::run;

fn halves(decl: &str, body: &str) -> (u8, u8) {
    let mut e = run(&format!(
        "const OUT0: addr = 0x0900;\n\
         const OUT1: addr = 0x0901;\n\
         {decl}\
         #[reset]\nfn main() {{\n{body}\n\
         \x20   OUT0 = w.low;\n    OUT1 = w.high;\n    loop {{}}\n}}\n"
    ));
    (e.mem(0x0900), e.mem(0x0901))
}

#[test]
fn writing_the_low_byte_of_a_local() {
    assert_eq!(
        halves("", "    let w: u16 = 0x1234;\n    w.low = 0x78;"),
        (0x78, 0x12),
        "only the low byte changed"
    );
}

#[test]
fn writing_the_high_byte_of_a_local() {
    assert_eq!(
        halves("", "    let w: u16 = 0x1234;\n    w.high = 0x56;"),
        (0x34, 0x56),
        "only the high byte changed"
    );
}

#[test]
fn writing_both_halves_of_a_static() {
    // A `static` lives at an absolute address rather than in the zero-page
    // frame, so the store is the four-digit form.
    assert_eq!(
        halves(
            "static w: u16 = 0;\n",
            "    w.low = 0xCD;\n    w.high = 0xAB;"
        ),
        (0xCD, 0xAB)
    );
}

#[test]
fn writing_a_half_of_a_signed_word() {
    assert_eq!(
        halves("", "    let w: i16 = 0;\n    w.high = 0xFF;"),
        (0x00, 0xFF)
    );
}

#[test]
fn the_halves_reassemble_into_the_whole() {
    // The point of writing a half: the 16-bit value really is those two bytes,
    // so arithmetic on it sees the assignment.
    let mut e = run(r#"
        const OUT0: addr = 0x0900;
        const OUT1: addr = 0x0901;
        #[reset]
        fn main() {
            let w: u16 = 0;
            w.low = 0x01;
            w.high = 0x01;
            let sum: u16 = w + 1;      // 0x0101 + 1
            OUT0 = sum.low;
            OUT1 = sum.high;
            loop {}
        }
    "#);
    assert_eq!((e.mem(0x0900), e.mem(0x0901)), (0x02, 0x01));
}

#[test]
fn a_computed_value_is_evaluated_once() {
    // The double-evaluation bug this file's sibling covers: `.low` reaches its
    // target arm after the value is generated, so it must not generate it
    // again.
    let mut e = run(r#"
        const OUT0: addr = 0x0900;
        const OUT1: addr = 0x0901;
        static CALLS: u8 = 0;
        fn bump() -> u8 { CALLS = CALLS + 1; return CALLS; }
        #[reset]
        fn main() {
            let w: u16 = 0;
            w.low = bump();
            OUT0 = CALLS;
            OUT1 = w.low;
            loop {}
        }
    "#);
    assert_eq!((e.mem(0x0900), e.mem(0x0901)), (1, 1));
}

#[test]
fn writing_a_half_of_a_const_is_rejected() {
    // The sema hole: a `const` is ROM, so this store would silently do nothing
    // on real hardware. Before `lvalue_root` peeled a genuine accessor, this
    // check never ran.
    crate::common::assert_error_contains(
        "const W: u16 = 0x1234;\n#[reset]\nfn main() { W.low = 1; loop {} }\n",
        "a const lives in ROM",
    );
}

#[test]
fn writing_a_half_of_a_parameter_is_rejected() {
    // Parameters are immutable, and the accessor does not launder that.
    crate::common::assert_error_contains(
        "fn f(w: u16) { w.high = 1; }\n\
         #[reset]\nfn main() { f(1); loop {} }\n",
        "immutable",
    );
}

#[test]
fn writing_a_half_of_something_with_no_address_is_rejected() {
    // A call's result lives wherever the return convention left it, which is
    // not a place to store into. The message says what to do instead.
    crate::common::assert_error_contains(
        "fn make() -> u16 { return 1; }\n\
         #[reset]\nfn main() { make().low = 1; loop {} }\n",
        "no fixed address",
    );
}
