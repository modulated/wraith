//! RAM a dropped `static` used to keep.
//!
//! Mutable statics are given BSS addresses during registration, in declaration
//! order, long before liveness is known — an initializer's `&OTHER` has to
//! resolve to a number as it is flattened, and that number has to exist. So a
//! static the output dropped still reserved its bytes: codegen stopped
//! emitting its initializer, and everything after it stayed where it was.
//!
//! Rather than defer allocation, the layout is repacked once liveness is
//! known. What makes that delicate is that an address lives in three places by
//! then: the symbol table, the per-use snapshots in `resolved_symbols`, and
//! `inline_param_symbols` — which despite its name holds every symbol a
//! function's body resolved, and gets merged back over `resolved_symbols` at
//! an inline call site. Moving a static in fewer than all three puts it back
//! where it was. The fuzzer found exactly that, twice.

use crate::common::exec::run;

/// The address a program's first live static ends up at, read out of the
/// generated assembly's symbol line.
fn address_of(src: &str, name: &str) -> u16 {
    let asm = crate::common::compile_success(src);
    let line = asm
        .lines()
        .find(|l| l.trim_start().starts_with(&format!("{name} = $")))
        .unwrap_or_else(|| panic!("no address emitted for '{name}' in:\n{asm}"));
    let hex = line.split('$').nth(1).unwrap().trim();
    u16::from_str_radix(hex, 16).unwrap()
}

#[test]
fn a_dropped_static_gives_its_bytes_back() {
    // `WASTE` is never read, so nothing should be laid out over it — `KEPT`
    // takes the base of BSS rather than starting 16 bytes in.
    let with_waste = r#"
        const OUT: addr = 0x0900;
        static WASTE: [u8; 16] = [0; 16];
        static KEPT: u8 = 7;
        #[reset]
        fn main() { OUT = KEPT; loop {} }
    "#;
    let without = r#"
        const OUT: addr = 0x0900;
        static KEPT: u8 = 7;
        #[reset]
        fn main() { OUT = KEPT; loop {} }
    "#;
    assert_eq!(
        address_of(with_waste, "KEPT"),
        address_of(without, "KEPT"),
        "a dead static must not push a live one along"
    );
    assert_eq!(run(with_waste).mem(0x0900), 7, "and it still reads right");
}

#[test]
fn a_live_static_keeps_its_address_when_nothing_is_dropped() {
    // The other direction, and the one that matters for not disturbing
    // programs that were already fine: with every static live, the layout is
    // exactly what registration handed out.
    let src = r#"
        const OUT0: addr = 0x0900;
        const OUT1: addr = 0x0901;
        static A: u8 = 1;
        static B: u8 = 2;
        #[reset]
        fn main() { OUT0 = A; OUT1 = B; loop {} }
    "#;
    assert_eq!(address_of(src, "B"), address_of(src, "A") + 1);
    let mut e = run(src);
    assert_eq!((e.mem(0x0900), e.mem(0x0901)), (1, 2));
}

#[test]
fn a_pointer_into_a_moved_static_follows_it() {
    // `&TARGET` is flattened to a number during registration. If `TARGET`
    // moves, that number has to be recomputed — otherwise `P` points at the
    // hole the dropped static left.
    let mut e = run(r#"
        const OUT: addr = 0x0900;
        static WASTE: [u8; 8] = [0; 8];
        static TARGET: u8 = 99;
        static P: &u8 = &TARGET;
        #[reset]
        fn main() { OUT = *P; loop {} }
    "#);
    assert_eq!(e.mem(0x0900), 99, "the pointer followed its target");
}

#[test]
fn a_struct_static_read_through_an_inlined_function_follows_it() {
    // The shape the fuzzer reduced to. `body` is small enough to be inlined,
    // and the inline path merges the callee's symbol snapshot over the
    // caller's — so a static corrected everywhere else is put back here.
    let mut e = run(r#"
        const OUT: addr = 0x0900;
        struct Pair { a: u8, b: u8 }
        static WASTE: [u8; 6] = [0; 6];
        static S: Pair = Pair { a: 4, b: 38 };
        static SEEN: u8 = 0;
        fn body() { SEEN = S.a + S.b; }
        #[reset]
        fn main() { body(); OUT = SEEN; loop {} }
    "#);
    assert_eq!(e.mem(0x0900), 42);
}

#[test]
fn a_function_pointer_in_a_moved_static_still_dispatches() {
    // Function addresses are emitted as labels rather than numbers, so they
    // survive a move on their own — but the *static holding them* moves, and
    // the indirect call reads it through whichever address snapshot it found.
    let mut e = run(r#"
        const OUT: addr = 0x0900;
        struct Dev { call: fn(u8) -> u8 }
        fn twice(v: u8) -> u8 { return v + v; }
        static WASTE: [u8; 4] = [0; 4];
        static DEV: Dev = Dev { call: twice };
        #[reset]
        fn main() { OUT = DEV.call(21); loop {} }
    "#);
    assert_eq!(e.mem(0x0900), 42);
}

#[test]
fn several_dropped_statics_compact_in_declaration_order() {
    // Live statics keep their relative order; only the gaps close.
    let src = r#"
        const OUT0: addr = 0x0900;
        const OUT1: addr = 0x0901;
        static DEAD0: [u8; 3] = [0; 3];
        static A: u8 = 1;
        static DEAD1: [u8; 5] = [0; 5];
        static B: u16 = 0x0203;
        static DEAD2: u8 = 0;
        #[reset]
        fn main() { OUT0 = A; OUT1 = B.low; loop {} }
    "#;
    let a = address_of(src, "A");
    assert_eq!(address_of(src, "B"), a + 1, "B follows A with no gap");
    let mut e = run(src);
    assert_eq!((e.mem(0x0900), e.mem(0x0901)), (1, 3));
}
