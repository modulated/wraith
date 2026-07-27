//! Constant initializers for aggregates — arrays of structs above all.
//!
//! Two flatteners used to exist, one for a mutable `static`'s startup image and
//! one for a `const` array's ROM data, and both were shallow: an integer, an
//! array of integers, a struct of integers, and nothing nested. An array *of*
//! structs matched no case in either.
//!
//! The `const` path at least said so. The `static` path fell through to its
//! zero-fill fallback and produced a table of zeros with no diagnostic — the
//! worst possible outcome, because the program runs and every lookup returns
//! nothing. That is what most of these tests are here to hold down.
//!
//! The other half is reading the table back. A `const` array's data is placed
//! during codegen and named by label; its symbol carries the `Absolute(0)`
//! sentinel rather than an address, and adding a field offset to that sentinel
//! produced loads from `$0002`.

use crate::common::exec::run;
use crate::common::harness::{CompileResult, compile, compile_success};

fn expect_error(src: &str) -> String {
    match compile(src) {
        CompileResult::SemaError(e) | CompileResult::CodegenError(e) => e,
        CompileResult::Success(..) => panic!("expected a compile error, but it compiled"),
        other => panic!("expected an error, got {other:?}"),
    }
}

// ============================================================================
// A `static` table in RAM
// ============================================================================

#[test]
fn a_static_array_of_structs_keeps_its_initialiser() {
    // The headline case. This compiled before and gave zeros.
    let mut e = run(r#"
        const R0: addr = 0x0900;
        const R1: addr = 0x0901;
        const R2: addr = 0x0902;
        struct Entry { start: u16, size: u16, flags: u8 }
        static REG: [Entry; 3] = [
            Entry { start: 0x1234, size: 0x0100, flags: 7 },
            Entry { start: 0x5678, size: 0x0200, flags: 8 },
            Entry { start: 0x9ABC, size: 0x0300, flags: 9 },
        ];
        #[reset]
        fn main() {
            let i: u8 = 1;
            R0 = REG[i].flags;
            R1 = REG[i].start.high;
            R2 = REG[2].size.low;
            loop {}
        }
    "#);
    assert_eq!(
        (e.mem(0x0900), e.mem(0x0901), e.mem(0x0902)),
        (8, 0x56, 0x00)
    );
}

#[test]
fn a_static_table_can_be_written_at_runtime_over_its_initial_values() {
    // A registry is initialised once and then edited, so both halves have to
    // agree about the layout.
    let mut e = run(r#"
        const R0: addr = 0x0900;
        const R1: addr = 0x0901;
        struct Entry { start: u16, flags: u8 }
        static REG: [Entry; 3] = [
            Entry { start: 0x1111, flags: 1 },
            Entry { start: 0x2222, flags: 2 },
            Entry { start: 0x3333, flags: 3 },
        ];
        #[reset]
        fn main() {
            let i: u8 = 1;
            REG[i].start = 0xBEEF;
            R0 = REG[i].start.high;
            R1 = REG[2].start.high;
            loop {}
        }
    "#);
    assert_eq!(
        (e.mem(0x0900), e.mem(0x0901)),
        (0xBE, 0x33),
        "the neighbour is untouched"
    );
}

#[test]
fn struct_fields_land_at_their_declared_offsets_whatever_order_they_are_written() {
    // The walk follows the struct definition, not the initializer, so writing
    // the fields out of order cannot silently transpose them.
    let mut e = run(r#"
        const R0: addr = 0x0900;
        const R1: addr = 0x0901;
        struct Entry { a: u8, b: u16, c: u8 }
        static T: [Entry; 1] = [ Entry { c: 0x33, a: 0x11, b: 0x2222 } ];
        #[reset]
        fn main() { R0 = T[0].a; R1 = T[0].c; loop {} }
    "#);
    assert_eq!((e.mem(0x0900), e.mem(0x0901)), (0x11, 0x33));
}

#[test]
fn an_omitted_struct_field_is_zero_and_does_not_shift_the_others() {
    let mut e = run(r#"
        const R0: addr = 0x0900;
        const R1: addr = 0x0901;
        struct Entry { a: u8, b: u16, c: u8 }
        static T: [Entry; 2] = [
            Entry { a: 0x11, c: 0x33 },
            Entry { a: 0x44, b: 0x5566, c: 0x77 },
        ];
        #[reset]
        fn main() { R0 = T[0].b.low; R1 = T[1].c; loop {} }
    "#);
    assert_eq!(
        (e.mem(0x0900), e.mem(0x0901)),
        (0x00, 0x77),
        "the omitted field is zero and the next element still starts on time"
    );
}

#[test]
fn a_short_array_initialiser_pads_with_zeros() {
    let mut e = run(r#"
        const R0: addr = 0x0900;
        const R1: addr = 0x0901;
        struct Entry { start: u16, flags: u8 }
        static T: [Entry; 4] = [ Entry { start: 0x1234, flags: 5 } ];
        #[reset]
        fn main() { R0 = T[0].flags; R1 = T[3].flags; loop {} }
    "#);
    assert_eq!((e.mem(0x0900), e.mem(0x0901)), (5, 0));
}

#[test]
fn a_struct_holding_an_array_flattens() {
    // Nesting the other way round: the walk is recursive in both directions.
    let mut e = run(r#"
        const R0: addr = 0x0900;
        const R1: addr = 0x0901;
        struct Row { tag: u8, cells: [u8; 3] }
        static T: [Row; 2] = [
            Row { tag: 1, cells: [10, 20, 30] },
            Row { tag: 2, cells: [40, 50, 60] },
        ];
        #[reset]
        fn main() { R0 = T[0].tag; R1 = T[1].tag; loop {} }
    "#);
    assert_eq!((e.mem(0x0900), e.mem(0x0901)), (1, 2));
}

#[test]
fn a_fill_initialiser_of_structs_repeats_the_element() {
    let mut e = run(r#"
        const R0: addr = 0x0900;
        const R1: addr = 0x0901;
        struct Entry { start: u16, flags: u8 }
        static T: [Entry; 4] = [Entry { start: 0xABCD, flags: 6 }; 4];
        #[reset]
        fn main() { R0 = T[3].flags; R1 = T[3].start.high; loop {} }
    "#);
    assert_eq!((e.mem(0x0900), e.mem(0x0901)), (6, 0xAB));
}

// ============================================================================
// A `const` table in ROM
// ============================================================================

#[test]
fn a_const_array_of_structs_reads_back_correctly() {
    let mut e = run(r#"
        const R0: addr = 0x0900;
        const R1: addr = 0x0901;
        const R2: addr = 0x0902;
        const R3: addr = 0x0903;
        struct Entry { start: u16, flags: u8 }
        const ROM: [Entry; 3] = [
            Entry { start: 0x1234, flags: 11 },
            Entry { start: 0x5678, flags: 22 },
            Entry { start: 0x9ABC, flags: 33 },
        ];
        #[reset]
        fn main() {
            let i: u8 = 2;
            R0 = ROM[i].flags;          // runtime index
            R1 = ROM[1].flags;          // constant index
            R2 = ROM[i].start.high;
            R3 = ROM[0].start.low;
            loop {}
        }
    "#);
    assert_eq!(
        (e.mem(0x0900), e.mem(0x0901), e.mem(0x0902), e.mem(0x0903)),
        (33, 22, 0x9A, 0x34)
    );
}

#[test]
fn a_const_table_reserves_its_real_size() {
    // The old reading looked for a primitive element type and fell back to one
    // byte, so `[Entry; 3]` reserved three bytes for nine bytes of data — the
    // table ran off its own allocation and into whatever DATA handed out next.
    let asm = compile_success(
        r#"
        const R0: addr = 0x0900;
        struct Entry { start: u16, flags: u8 }
        const ROM: [Entry; 3] = [
            Entry { start: 0x1234, flags: 11 },
            Entry { start: 0x5678, flags: 22 },
            Entry { start: 0x9ABC, flags: 33 },
        ];
        #[reset]
        fn main() { R0 = ROM[0].flags; loop {} }
    "#,
    );
    assert!(asm.contains("ROM (9 bytes)"), "{asm}");
    assert!(
        asm.contains("$34, $12, $0B, $78, $56, $16, $BC, $9A, $21"),
        "little-endian, in declaration order:\n{asm}"
    );
}

#[test]
fn a_const_table_is_addressed_by_label_not_by_the_sentinel() {
    // A const's symbol carries `Absolute(0)`, so an indexed field read used to
    // come out as `LDA $0002,Y` — the field offset added to nothing.
    let asm = compile_success(
        r#"
        const R0: addr = 0x0900;
        struct Entry { start: u16, flags: u8 }
        const ROM: [Entry; 2] = [
            Entry { start: 0x1234, flags: 11 },
            Entry { start: 0x5678, flags: 22 },
        ];
        #[reset]
        fn main() { let i: u8 = 1; R0 = ROM[i].flags; loop {} }
    "#,
    );
    assert!(
        asm.contains("ROM+2,Y"),
        "expected a label-relative load:\n{asm}"
    );
}

#[test]
fn a_const_table_of_function_pointers_can_be_called_through() {
    // A ROM dispatch table: the addresses are only known to the assembler, so
    // each one has to come back out as a `.WORD label`.
    let mut e = run(r#"
        const R0: addr = 0x0900;
        struct Cmd { code: u8, run: fn() -> u8 }
        fn peek() -> u8 { return 0x41; }
        fn poke() -> u8 { return 0x42; }
        const TABLE: [Cmd; 2] = [
            Cmd { code: 1, run: peek },
            Cmd { code: 2, run: poke },
        ];
        #[reset]
        fn main() {
            let i: u8 = 1;
            let f: fn() -> u8 = TABLE[i].run;
            R0 = f();
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0900), 0x42);
}

// ============================================================================
// Constant data is read-only
// ============================================================================

#[test]
fn assigning_to_a_const_array_element_is_rejected() {
    // This compiled and emitted a store into ROM — a no-op on real hardware,
    // and invisible under the emulator's flat memory. The same shape as local
    // arrays living in the CODE section.
    let err = expect_error(
        r#"
        const R0: addr = 0x0900;
        const LUT: [u8; 4] = [1, 2, 3, 4];
        #[reset]
        fn main() { let i: u8 = 0; LUT[i] = 9; R0 = 0; loop {} }
    "#,
    );
    assert!(err.contains("ROM"), "the reason should be given: {err}");
}

#[test]
fn assigning_to_a_const_struct_field_is_rejected() {
    let err = expect_error(
        r#"
        const R0: addr = 0x0900;
        struct Entry { flags: u8 }
        const ROM: [Entry; 2] = [Entry { flags: 1 }, Entry { flags: 2 }];
        #[reset]
        fn main() { let i: u8 = 0; ROM[i].flags = 9; R0 = 0; loop {} }
    "#,
    );
    assert!(err.contains("ROM"), "{err}");
}

#[test]
fn writing_through_a_pointer_is_still_allowed() {
    // The mutability check walks to the root of an lvalue chain, but must stop
    // at a reference: `p[i] = v` and `p.field = v` write to the pointee, and
    // the pointer's own mutability says nothing about that. Nor does a
    // parameter's, since aggregates have always been passed by address.
    let mut e = run(r#"
        const R0: addr = 0x0900;
        const R1: addr = 0x0901;
        struct Dev { status: u8 }
        fn fill(buf: &u8) { buf[0] = 0x5A; }
        fn bump(d: &Dev) { d.status = d.status + 1; }
        fn viastruct(d: Dev) { d.status = d.status + 10; }
        #[reset]
        fn main() {
            let b: [u8; 2] = [0; 2];
            fill(&b);
            let d: Dev = Dev { status: 1 };
            bump(&d);
            viastruct(d);
            R0 = b[0];
            R1 = d.status;
            loop {}
        }
    "#);
    assert_eq!((e.mem(0x0900), e.mem(0x0901)), (0x5A, 12));
}

// ============================================================================
// Diagnostics
// ============================================================================

#[test]
fn a_non_constant_field_in_a_table_is_an_error_not_zeros() {
    // The whole point. Before, an initializer the compiler could not evaluate
    // silently became zeros; a table full of them looks exactly like a table
    // that was never filled in.
    let err = expect_error(
        r#"
        const R0: addr = 0x0900;
        struct Entry { flags: u8 }
        fn pick() -> u8 { return 3; }
        static T: [Entry; 1] = [ Entry { flags: pick() } ];
        #[reset]
        fn main() { R0 = T[0].flags; loop {} }
    "#,
    );
    assert!(err.contains("compile-time constant"), "{err}");
}

#[test]
fn too_many_elements_for_the_declared_length_is_an_error() {
    let err = expect_error(
        r#"
        const R0: addr = 0x0900;
        struct Entry { flags: u8 }
        static T: [Entry; 2] = [
            Entry { flags: 1 }, Entry { flags: 2 }, Entry { flags: 3 },
        ];
        #[reset]
        fn main() { R0 = T[0].flags; loop {} }
    "#,
    );
    assert!(err.contains("3 elements"), "{err}");
}

#[test]
fn a_scalar_static_with_no_constant_value_still_starts_at_zero() {
    // The tolerance that has to survive: RAM is undefined at power-on, so a
    // scalar the compiler cannot fold is zero rather than an error. That is
    // what BSS means, and why `= 0` needs no special case.
    let mut e = run(r#"
        const R0: addr = 0x0900;
        static COUNT: u8 = 0;
        #[reset]
        fn main() { R0 = COUNT; loop {} }
    "#);
    assert_eq!(e.mem(0x0900), 0);
}

// ============================================================================
// Regressions
// ============================================================================

#[test]
fn a_long_run_of_zeros_is_still_reserved_rather_than_spelled_out() {
    // 256 zero bytes as `.BYTE` lines is 16 lines of nothing. `.RES` only
    // advances the assembler's address and the image starts zeroed.
    let asm = compile_success(
        r#"
        const R0: addr = 0x0900;
        const BUFFER: [u8; 256] = [0; 256];
        #[reset]
        fn main() { R0 = BUFFER[0]; loop {} }
    "#,
    );
    assert!(asm.contains(".RES 256"), "{asm}");
}

#[test]
fn zero_padding_after_a_short_table_is_reserved_too() {
    // The same collapse now applies to the tail of a partly-written table,
    // which is where most of the zeros in a real program are.
    let asm = compile_success(
        r#"
        const R0: addr = 0x0900;
        struct Entry { start: u16, flags: u8 }
        const ROM: [Entry; 16] = [ Entry { start: 0x1234, flags: 1 } ];
        #[reset]
        fn main() { R0 = ROM[0].flags; loop {} }
    "#,
    );
    assert!(
        asm.contains(".RES 45"),
        "expected the tail reserved:\n{asm}"
    );
}

#[test]
fn a_function_pointer_field_is_read_as_two_bytes_at_a_constant_index() {
    // The constant-index path had the same one-byte reading as the runtime one.
    let mut e = run(r#"
        const R0: addr = 0x0900;
        struct Cmd { code: u8, run: fn() -> u8 }
        fn poke() -> u8 { return 0x42; }
        const TABLE: [Cmd; 2] = [
            Cmd { code: 1, run: poke },
            Cmd { code: 2, run: poke },
        ];
        #[reset]
        fn main() { let f: fn() -> u8 = TABLE[1].run; R0 = f(); loop {} }
    "#);
    assert_eq!(e.mem(0x0900), 0x42);
}

#[test]
fn a_pointer_field_in_a_table_keeps_both_of_its_bytes() {
    // `&T` is an address like a function pointer, and was missing from the same
    // width test. A truncated high byte reads as a plausible zero-page address.
    let mut e = run(r#"
        const R0: addr = 0x0900;
        struct Slot { tag: u8, at: &u8 }
        static TARGET: u8 = 0;
        static SLOTS: [Slot; 2] = [
            Slot { tag: 1, at: &TARGET },
            Slot { tag: 2, at: &TARGET },
        ];
        #[reset]
        fn main() {
            let i: u8 = 1;
            let p: &u8 = SLOTS[i].at;
            *p = 0x77;
            R0 = TARGET;
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0900), 0x77);
}

#[test]
fn a_pointer_field_can_be_written_as_well_as_read() {
    // Storing into a `&T` field has to take the high byte from X, not Y —
    // the mirror of the load. Both directions, three storage shapes.
    let mut e = run(r#"
        const R0: addr = 0x0900;
        const R1: addr = 0x0901;
        const R2: addr = 0x0902;
        struct Slot { tag: u8, at: &u8 }
        static A: u8 = 0;
        static B: u8 = 0;
        static C: u8 = 0;
        static SLOTS: [Slot; 2] = [
            Slot { tag: 1, at: &A },
            Slot { tag: 2, at: &A },
        ];
        fn retarget(s: &Slot, to: &u8) { s.at = to; }
        #[reset]
        fn main() {
            let i: u8 = 1;
            SLOTS[i].at = &B;                 // indexed write
            let p: &u8 = SLOTS[i].at;
            *p = 0x11;

            let local: Slot = Slot { tag: 3, at: &A };
            local.at = &C;                    // local struct write
            let q: &u8 = local.at;
            *q = 0x22;

            retarget(&SLOTS[0], &A);          // through a pointer parameter
            let r: &u8 = SLOTS[0].at;
            *r = 0x33;

            R0 = B; R1 = C; R2 = A;
            loop {}
        }
    "#);
    assert_eq!(
        (e.mem(0x0900), e.mem(0x0901), e.mem(0x0902)),
        (0x11, 0x22, 0x33)
    );
}
