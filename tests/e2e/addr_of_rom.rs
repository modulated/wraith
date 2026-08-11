//! Taking the address of ROM data.
//!
//! An immutable `const` is emitted as data at an assembler label, and sema
//! leaves it at `SymbolLocation::Absolute(0)` because the address is the
//! linker's to choose. Every address-of site read that placeholder as a real
//! address and computed `0 + offset`, so `&A[1]` came out as `$0001` — the
//! *index*, pointing into system-reserved zero page. Reads through it returned
//! whatever happened to be there, and writes scribbled on it.
//!
//! Statics were always fine (they have a real BSS address) and locals were
//! always fine (the slot holds a runtime pointer); only ROM was wrong. These
//! read real values through the pointers so a wrong address shows up as a wrong
//! byte rather than as a plausible-looking `LDA`.

use crate::common::exec::run;

const DATA: &str = r#"
    const OUT: addr = 0x0900;
    const OUT1: addr = 0x0901;
    const OUT2: addr = 0x0902;
    const OUT3: addr = 0x0903;
    const CA: [u8; 6] = [10, 20, 30, 40, 50, 60];
    static SA: [u8; 6] = [11, 21, 31, 41, 51, 61];
"#;

#[test]
fn a_pointer_to_a_const_array_element_reads_that_element() {
    let mut e = run(&format!(
        "{DATA}
        #[reset]
        fn main() {{
            let p: &u8 = &CA[1];
            let q: &u8 = &CA[4];
            OUT = *p;
            OUT1 = *q;
            loop {{}}
        }}"
    ));
    assert_eq!(e.mem(0x0900), 20, "CA[1]");
    assert_eq!(e.mem(0x0901), 50, "CA[4]");
}

/// Offset zero is the case that looked right by accident before: `0 + 0` is
/// still 0, so it read zero page rather than the array and happened to return
/// a plausible byte.
#[test]
fn a_pointer_to_the_first_const_element_is_the_array_address() {
    let mut e = run(&format!(
        "{DATA}
        #[reset]
        fn main() {{
            let p: &u8 = &CA[0];
            OUT = *p;
            OUT1 = p[1];
            OUT2 = p[5];
            loop {{}}
        }}"
    ));
    assert_eq!(e.mem(0x0900), 10, "CA[0]");
    assert_eq!(e.mem(0x0901), 20, "indexing off the pointer reaches CA[1]");
    assert_eq!(e.mem(0x0902), 60, "and CA[5]");
}

/// The whole-array form (`&CA`) went through a different arm with the same bug.
#[test]
fn the_address_of_a_whole_const_array_is_its_label() {
    let mut e = run(&format!(
        "{DATA}
        #[reset]
        fn main() {{
            let p: &u8 = &CA[0];
            let i: u8 = 3;
            OUT = p[i];
            loop {{}}
        }}"
    ));
    assert_eq!(e.mem(0x0900), 40, "CA[3] through a runtime index");
}

/// A ROM pointer handed to a function must survive the call.
#[test]
fn a_const_element_pointer_can_be_passed_to_a_function() {
    let mut e = run(&format!(
        "{DATA}
        fn deref(p: &u8) -> u8 {{ return *p; }}
        fn second(p: &u8) -> u8 {{ return p[1]; }}
        #[reset]
        fn main() {{
            OUT = deref(&CA[2]);
            OUT1 = second(&CA[2]);
            loop {{}}
        }}"
    ));
    assert_eq!(e.mem(0x0900), 30, "CA[2]");
    assert_eq!(e.mem(0x0901), 40, "CA[3] via the pointer");
}

/// Statics kept their real address; the fix must not have redirected them
/// through a label.
#[test]
fn a_pointer_to_a_static_array_element_still_works() {
    let mut e = run(&format!(
        "{DATA}
        #[reset]
        fn main() {{
            let p: &u8 = &SA[1];
            OUT = *p;
            *p = 99;
            OUT1 = SA[1];
            loop {{}}
        }}"
    ));
    assert_eq!(e.mem(0x0900), 21, "SA[1]");
    assert_eq!(e.mem(0x0901), 99, "a static is RAM, so the write lands");
}

/// Locals compute their address at run time from the slot; unchanged.
#[test]
fn a_pointer_to_a_local_array_element_still_works() {
    let mut e = run(&format!(
        "{DATA}
        #[reset]
        fn main() {{
            let la: [u8; 6] = [12, 22, 32, 42, 52, 62];
            let p: &u8 = &la[1];
            OUT = *p;
            *p = 77;
            OUT1 = la[1];
            loop {{}}
        }}"
    ));
    assert_eq!(e.mem(0x0900), 22, "la[1]");
    assert_eq!(e.mem(0x0901), 77, "a local is RAM");
}

/// All three storage classes at once: a wrong base for any of them shows up as
/// one pointer reading another's data.
#[test]
fn pointers_into_different_storage_do_not_alias() {
    let mut e = run(&format!(
        "{DATA}
        #[reset]
        fn main() {{
            let la: [u8; 6] = [12, 22, 32, 42, 52, 62];
            let c: &u8 = &CA[1];
            let s: &u8 = &SA[1];
            let l: &u8 = &la[1];
            OUT = *c; OUT1 = *s; OUT2 = *l;
            loop {{}}
        }}"
    ));
    assert_eq!(
        (e.mem(0x0900), e.mem(0x0901), e.mem(0x0902)),
        (20, 21, 22),
        "const, static and local pointers each read their own array"
    );
}

/// The bug's real damage: a store through the bad pointer hit `$0001`, in the
/// system-reserved zero page. Nothing here should touch it.
#[test]
fn a_rom_pointer_does_not_point_into_zero_page() {
    let mut e = run(&format!(
        "{DATA}
        #[reset]
        fn main() {{
            let p: &u8 = &CA[1];
            OUT = *p;
            loop {{}}
        }}"
    ));
    assert_eq!(e.mem(0x0900), 20);
    // $0001 held the index under the old codegen. A ROM pointer must name the
    // DATA section, well clear of zero page.
    assert_eq!(e.mem(0x0001), 0, "system-reserved zero page is untouched");
}

/// Writing through a ROM pointer is now an ordinary ROM store: a silent no-op
/// on real hardware, which the emulator refuses outright so tests see it.
///
/// This is a real improvement rather than a remaining hole. Before the fix the
/// same write went to `$0001` and *succeeded*, corrupting zero page while
/// looking like it had worked. Rejecting it in the compiler needs pointer
/// provenance — knowing a `&u8` was derived from a `const` — which is the
/// analogue of the read-only rule slices already carry, and is not built yet.
#[test]
#[should_panic(expected = "store into ROM")]
fn a_write_through_a_rom_pointer_reaches_rom() {
    run(&format!(
        "{DATA}
        #[reset]
        fn main() {{
            let p: &u8 = &CA[1];
            *p = 99;
            OUT = *p;
            loop {{}}
        }}"
    ));
}
