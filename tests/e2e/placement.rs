//! Two-phase function placement.
//!
//! `#[org]` used to bypass the section allocator entirely: it emitted at the
//! requested address and never told the allocator, which then handed the same
//! addresses out again. Pinning anything at a section's base made that section
//! unusable — `examples/uart.wr` had `main` at `$8000` and the next
//! auto-allocated function also at `$8000`.
//!
//! Placement now runs before any code is emitted: measure every function,
//! reserve the pinned ranges, allocate the rest into what is left. These tests
//! are mostly about the arithmetic at the edges of that — gaps exactly big
//! enough, gaps one byte too small, reservations at a section's first and last
//! address — plus the thing that actually matters, which is that programs using
//! `#[org]` run correctly.

use crate::common::exec::run;
use crate::common::harness::{CompileResult, compile, compile_success};

fn expect_error(src: &str) -> String {
    match compile(src) {
        CompileResult::CodegenError(e) => e,
        CompileResult::Success(..) => panic!("expected a compile error, but it compiled"),
        other => panic!("expected a codegen error, got {other:?}"),
    }
}

/// Every `.ORG $addr` in the output, in emission order.
fn orgs(asm: &str) -> Vec<u16> {
    asm.lines()
        .filter_map(|l| l.trim().strip_prefix(".ORG $"))
        .filter_map(|h| u16::from_str_radix(h.trim(), 16).ok())
        .collect()
}

/// The address a named function was placed at, read from the `.ORG` preceding
/// its label.
fn addr_of(asm: &str, name: &str) -> u16 {
    let label = format!("{name}:");
    let mut last_org = None;
    for line in asm.lines() {
        if let Some(hex) = line.trim().strip_prefix(".ORG $") {
            last_org = u16::from_str_radix(hex.trim(), 16).ok();
        }
        if line.trim() == label {
            return last_org.unwrap_or_else(|| panic!("no .ORG before `{name}`"));
        }
    }
    panic!("function `{name}` was not emitted");
}

/// Do two `[start, end)` ranges overlap?
fn overlaps(a: (u16, u16), b: (u16, u16)) -> bool {
    a.0 < b.1 && b.0 < a.1
}

// ============================================================================
// The case that motivated this
// ============================================================================

#[test]
fn auto_allocation_routes_around_an_org_at_the_section_base() {
    // CODE starts at $8000. Pinning `main` there used to hand $8000 straight
    // back out to the next function, so any program combining #[org] with
    // ordinary functions failed to compile.
    let src = r#"
        const OUT: addr = 0x0900;
        fn helper() -> u8 { return 40; }
        #[org(0x8000)]
        #[reset]
        fn main() { let _k: fn() -> u8 = helper; OUT = helper() + 2; loop {} }
    "#;
    let asm = compile_success(src);
    assert_eq!(addr_of(&asm, "main"), 0x8000, "the pin is honoured");
    assert!(
        addr_of(&asm, "helper") > 0x8000,
        "the auto-allocated function moves out of the way"
    );
    assert_eq!(run(src).mem(0x0900), 42, "and the program still runs");
}

#[test]
fn a_program_with_several_pinned_functions_runs() {
    // The real check on the arithmetic: every call has to land on the right
    // address, for pinned and auto-allocated functions alike.
    let src = r#"
        const OUT: addr = 0x0900;
        #[org(0x8000)]
        fn first() -> u8 { return 1; }
        fn middle() -> u8 { return 10; }
        #[org(0x9000)]
        fn last() -> u8 { return 100; }
        fn also_auto() -> u8 { return 1000 as u8; }
        #[reset]
        fn main() { OUT = first() + middle() + last(); loop {} }
    "#;
    assert_eq!(run(src).mem(0x0900), 111);
}

// ============================================================================
// Gap arithmetic
// ============================================================================

#[test]
fn functions_fill_the_gap_between_two_pinned_ones() {
    let src = r#"
        const OUT: addr = 0x0900;
        #[org(0x8000)]
        fn low() -> u8 { return 1; }
        #[org(0x8100)]
        fn high() -> u8 { return 2; }
        fn a() -> u8 { return 3; }
        fn b() -> u8 { return 4; }
        #[reset]
        fn main() {
            let _ka: fn() -> u8 = a;
            let _kb: fn() -> u8 = b;
            OUT = low() + high() + a() + b();
            loop {}
        }
    "#;
    let asm = compile_success(src);
    let (a, b) = (addr_of(&asm, "a"), addr_of(&asm, "b"));
    assert!(
        (0x8000..0x8100).contains(&a) && (0x8000..0x8100).contains(&b),
        "both should fit in the gap, got a=${a:04X} b=${b:04X}"
    );
    assert_eq!(run(src).mem(0x0900), 10);
}

#[test]
fn a_function_too_large_for_a_gap_is_placed_after_it() {
    // The gap between the two pins is under 20 bytes, which a function
    // containing a loop does not fit in. It must be placed past the second
    // reservation rather than squeezed in and overrunning it.
    let src = r#"
        const OUT: addr = 0x0900;
        #[org(0x8000)]
        fn low() -> u8 { return 1; }
        #[org(0x8020)]
        fn high() -> u8 { return 2; }
        fn big(n: u8) -> u8 {
            let t: u8 = 0;
            for i in 0..20 { t = t + i + n; }
            return t;
        }
        #[reset]
        fn main() { let _k: fn(u8) -> u8 = big; OUT = low() + high() + big(1); loop {} }
    "#;
    let asm = compile_success(src);
    let low = addr_of(&asm, "low");
    let big = addr_of(&asm, "big");

    assert!(low < 0x8020, "low sits before the second pin");
    assert!(
        big > 0x8020,
        "`big` should not fit the gap after ${low:04X}, got ${big:04X}"
    );
}

#[test]
fn nothing_overlaps_anything_when_pins_and_auto_are_mixed() {
    // The property the whole exercise is for. Sizes vary so the packing is not
    // trivially spaced out.
    let src = r#"
        const OUT: addr = 0x0900;
        #[org(0x8000)]
        fn p1() -> u8 { return 1; }
        fn a1() -> u8 { let t: u8 = 0; for i in 0..5 { t = t + i; } return t; }
        #[org(0x8080)]
        fn p2() -> u8 { return 2; }
        fn a2() -> u8 { return 3; }
        fn a3() -> u8 { let t: u8 = 1; for i in 0..9 { t = t + i; } return t; }
        #[org(0x8200)]
        fn p3() -> u8 { return 4; }
        #[reset]
        fn main() {
            let _k1: fn() -> u8 = a1;
            let _k2: fn() -> u8 = a2;
            let _k3: fn() -> u8 = a3;
            OUT = p1() + p2() + p3() + a1() + a2() + a3();
            loop {}
        }
    "#;
    let asm = compile_success(src);

    // Reconstruct each function's extent from consecutive .ORG boundaries is
    // unreliable (emission order is not address order), so compare the pinned
    // addresses against the auto-allocated ones directly.
    let pinned = [
        (addr_of(&asm, "p1"), 0x8000u16),
        (addr_of(&asm, "p2"), 0x8080),
        (addr_of(&asm, "p3"), 0x8200),
    ];
    for (actual, expected) in pinned {
        assert_eq!(actual, expected, "a pin must not move");
    }

    for name in ["a1", "a2", "a3", "main"] {
        let a = addr_of(&asm, name);
        assert!(
            !pinned.iter().any(|(p, _)| a == *p),
            "`{name}` was placed on top of a pinned function at ${a:04X}"
        );
    }

    assert_eq!(run(src).mem(0x0900), 1 + 2 + 4 + 10 + 3 + 37);
}

// ============================================================================
// Section boundaries
// ============================================================================

#[test]
fn an_org_at_the_very_start_of_a_section_is_allowed() {
    let asm = compile_success(
        r#"
        const OUT: addr = 0x0900;
        #[org(0x8000)]
        fn at_base() -> u8 { return 7; }
        #[reset]
        fn main() { OUT = at_base(); loop {} }
    "#,
    );
    assert_eq!(addr_of(&asm, "at_base"), 0x8000);
}

#[test]
fn an_org_running_exactly_to_the_end_of_a_section_is_allowed() {
    // CODE ends at $BFFF. A function measured at N bytes placed so its last
    // byte is $BFFF must be accepted; an off-by-one in the containment check
    // would reject it.
    let src = r#"
        const OUT: addr = 0x0900;
        #[org(0xBF00)]
        fn near_end() -> u8 { return 7; }
        #[reset]
        fn main() { OUT = near_end(); loop {} }
    "#;
    assert_eq!(addr_of(&compile_success(src), "near_end"), 0xBF00);
    assert_eq!(run(src).mem(0x0900), 7);
}

#[test]
fn a_pin_at_the_top_of_a_section_does_not_push_others_out_of_it() {
    // Reserving the last bytes of CODE must not make the allocator run past
    // $BFFF for everything else; the reservation is a hole, not a new floor.
    let src = r#"
        const OUT: addr = 0x0900;
        #[org(0xBFF0)]
        fn at_top() -> u8 { return 1; }
        fn a() -> u8 { return 2; }
        fn b() -> u8 { return 3; }
        #[reset]
        fn main() {
            let _ka: fn() -> u8 = a;
            let _kb: fn() -> u8 = b;
            OUT = at_top() + a() + b();
            loop {}
        }
    "#;
    let asm = compile_success(src);
    assert_eq!(addr_of(&asm, "at_top"), 0xBFF0);
    for name in ["a", "b", "main"] {
        let addr = addr_of(&asm, name);
        assert!(
            addr < 0xBFF0,
            "`{name}` at ${addr:04X} should have been placed below the pin"
        );
    }
    assert_eq!(run(src).mem(0x0900), 6);
}

// Section exhaustion, gaps that are one byte too small, and reservations that
// reach the top of memory are covered by unit tests on the allocator itself
// (`codegen::section_allocator::reservation_tests`), where a section small
// enough to fill can be constructed directly.

// ============================================================================
// Errors that must survive the rework
// ============================================================================

#[test]
fn two_pins_that_overlap_still_fail_and_name_both() {
    let err = expect_error(
        r#"
        const OUT: addr = 0x0900;
        #[org(0x8100)]
        fn alpha() -> u8 { let a: u8 = 1; return a + 2 + 3 + 4 + 5; }
        #[org(0x8105)]
        fn beta() -> u8 { return 2; }
        #[reset]
        fn main() { OUT = alpha() + beta(); loop {} }
    "#,
    );
    assert!(err.contains("alpha"), "{err}");
    assert!(err.contains("beta"), "{err}");
    assert!(
        err.contains("neither can be moved"),
        "the message should say why this is not fixable automatically: {err}"
    );
}

#[test]
fn a_pin_over_the_vector_table_still_fails() {
    let err = expect_error(
        r#"
        const OUT: addr = 0x0900;
        #[org(0xFFF8)]
        fn alpha() -> u8 { let a: u8 = 1; return a + 2 + 3; }
        #[reset]
        fn main() { OUT = alpha(); loop {} }
    "#,
    );
    assert!(err.contains("interrupt vector table"), "{err}");
}

#[test]
fn a_pin_outside_every_section_still_fails() {
    let err = expect_error(
        r#"
        const OUT: addr = 0x0900;
        #[org(0x0100)]
        fn alpha() -> u8 { return 7; }
        #[reset]
        fn main() { OUT = alpha(); loop {} }
    "#,
    );
    assert!(err.contains("not inside any configured section"), "{err}");
}

// ============================================================================
// Reproducibility
// ============================================================================

#[test]
fn the_same_source_always_produces_the_same_layout() {
    // Placement iterates maps in a few places; if any of that leaked into the
    // result, addresses would shuffle between builds of identical source.
    let src = r#"
        const OUT: addr = 0x0900;
        const TABLE: [u8; 4] = [1, 2, 3, 4];
        #[org(0x8400)]
        fn pinned() -> u8 { return 1; }
        fn a() -> u8 { return 2; }
        fn b() -> u8 { return 3; }
        fn c(i: u8) -> u8 { return TABLE[i]; }
        #[reset]
        fn main() { OUT = pinned() + a() + b() + c(0); loop {} }
    "#;
    let first = compile_success(src);
    for _ in 0..4 {
        assert_eq!(orgs(&compile_success(src)), orgs(&first));
    }
}

#[test]
fn declaration_order_decides_the_layout_of_auto_allocated_functions() {
    // Not a promise to users so much as a check that placement follows source
    // order rather than hash order.
    let asm = compile_success(
        r#"
        const OUT: addr = 0x0900;
        fn first() -> u8 { return 1; }
        fn second() -> u8 { return 2; }
        fn third() -> u8 { return 3; }
        #[reset]
        fn main() {
            let _k1: fn() -> u8 = first;
            let _k2: fn() -> u8 = second;
            let _k3: fn() -> u8 = third;
            OUT = first() + second() + third();
            loop {}
        }
    "#,
    );
    let (a, b, c) = (
        addr_of(&asm, "first"),
        addr_of(&asm, "second"),
        addr_of(&asm, "third"),
    );
    assert!(a < b && b < c, "got ${a:04X}, ${b:04X}, ${c:04X}");
}

// ============================================================================
// Interaction with data placement
// ============================================================================

#[test]
fn const_arrays_are_allocated_around_a_pin_in_data() {
    // DATA starts at $D000. A function pinned there must not have the table
    // written over it.
    let src = r#"
        const OUT: addr = 0x0900;
        const TABLE: [u8; 8] = [1, 2, 3, 4, 5, 6, 7, 8];
        #[org(0xD000)]
        fn pinned() -> u8 { return 9; }
        #[reset]
        fn main() { let i: u8 = 3; OUT = pinned() + TABLE[i]; loop {} }
    "#;
    let asm = compile_success(src);
    let pinned = addr_of(&asm, "pinned");
    let table = addr_of(&asm, "TABLE");
    assert_eq!(pinned, 0xD000);
    assert!(
        !overlaps((table, table + 8), (pinned, pinned + 1)),
        "TABLE at ${table:04X} collides with the pinned function at ${pinned:04X}"
    );
    assert_eq!(run(src).mem(0x0900), 9 + 4);
}

#[test]
fn an_interrupt_handler_can_be_pinned() {
    // Handlers carry a prologue and epilogue that the measuring pass has to
    // reproduce, or the reserved range is the wrong size.
    let src = r#"
        const OUT: addr = 0x0900;
        static COUNT: u8 = 0;
        #[org(0x8300)]
        #[irq]
        fn on_irq() { COUNT = COUNT + 1; }
        #[reset]
        fn main() { OUT = COUNT; loop {} }
    "#;
    let asm = compile_success(src);
    assert_eq!(addr_of(&asm, "on_irq"), 0x8300);
    // The vector must point at the pinned address, not wherever the allocator
    // would have put it.
    assert!(asm.contains(".WORD on_irq"));
}

// ============================================================================
// Measurement integrity
//
// The allocator reserves each function's *measured* size, so anything the
// measuring pass under-counts is bytes the next function's .ORG lands on top
// of. Two omissions lived here: jump-table .WORDs added nothing to the count
// (the emitter never counted them), and the reset handler's static
// initializers were not reproduced at all. Both corrupted the binary
// silently; `generate_function` now also hard-errors if the real pass ever
// emits more than was measured.
// ============================================================================

#[test]
fn a_match_jump_table_is_counted_in_the_functions_size() {
    // Eight arms is a 16-byte table — past the 10-byte measurement slack — so
    // before the fix, `filler` was placed inside `pick`'s match body and the
    // top two arms jumped into filler/leftover bytes.
    let src = r#"
        const R0: addr = 0x0900;
        const R1: addr = 0x0901;
        const R2: addr = 0x0902;
        const R3: addr = 0x0903;
        enum E { A, B, C, D, F, G, H, I }
        fn pick(x: E) -> u8 {
            match x {
                E::A => { return 1; }
                E::B => { return 2; }
                E::C => { return 3; }
                E::D => { return 4; }
                E::F => { return 5; }
                E::G => { return 6; }
                E::H => { return 7; }
                _ => { return 9; }
            }
        }
        fn filler() -> u8 { return 0xAA; }
        #[reset]
        fn main() {
            R0 = pick(E::G);
            R1 = pick(E::H);
            R2 = filler();
            R3 = pick(E::I);   // no arm: the wildcard table entry, the last one
            loop {}
        }
    "#;
    let mut e = run(src);
    assert_eq!(
        (e.mem(0x0900), e.mem(0x0901), e.mem(0x0902), e.mem(0x0903)),
        (6, 7, 0xAA, 9),
        "every table entry, the wildcard included, and the following function survive placement"
    );
}

#[test]
fn reset_handler_static_initializers_are_counted_in_its_size() {
    // main is declared first, so filler is placed right after it. Before the
    // fix, main's measured size omitted the static-init stream, filler was
    // .ORG'd into the middle of it, and reset never finished initializing
    // `tab` or reached the body.
    let mut e = run(r#"
        const R0: addr = 0x0900;
        const R1: addr = 0x0901;
        static tab: [u8; 20] = [
            1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20,
        ];
        #[reset]
        fn main() {
            R0 = tab[19];
            R1 = filler();
            loop {}
        }
        fn filler() -> u8 { return 0xAB; }
    "#);
    assert_eq!((e.mem(0x0900), e.mem(0x0901)), (20, 0xAB));
}
