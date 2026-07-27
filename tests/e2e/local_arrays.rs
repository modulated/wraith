//! Local arrays must live in RAM.
//!
//! A local array's *data* used to be emitted inline in the instruction stream —
//! `.BYTE`/`.RES` between a `JMP` and its landing label — with only a pointer to
//! it in the frame slot. On a real board the CODE section is ROM, so `buf[i] = v`
//! on a local array silently did nothing. Every test passed because the
//! emulator's bus is flat RAM for all 64 KB, which is exactly the sort of thing
//! a test harness hides rather than reveals.
//!
//! So most of these assert on the *emitted assembly* rather than on runtime
//! behaviour: the question is not "did the write land" — it always did, in the
//! emulator — but "where does the array live".

use crate::common::exec::run;
use crate::common::harness::compile_success;

/// The BSS section from wraith.toml, where mutable globals and now local arrays
/// live.
const BSS_START: u16 = 0x0400;
const BSS_END: u16 = 0x07FF;

/// Pull every `NAME @ $ADDR` annotation the compiler emits for RAM-backed
/// storage out of the listing.
fn ram_addr_of(asm: &str, name: &str) -> Option<u16> {
    let needle = format!("{name} @ $");
    asm.lines().find_map(|l| {
        let i = l.find(&needle)? + needle.len();
        u16::from_str_radix(l.get(i..i + 4)?, 16).ok()
    })
}

// ============================================================================
// Where the data lives
// ============================================================================

#[test]
fn a_local_array_is_allocated_in_ram() {
    let asm = compile_success(
        r#"
        const OUT: addr = 0x0900;
        #[reset]
        fn main() {
            let buf: [u8; 8] = [0; 8];
            buf[3] = 42;
            OUT = buf[3];
            loop {}
        }
    "#,
    );
    let addr = ram_addr_of(&asm, "buf")
        .unwrap_or_else(|| panic!("no RAM address recorded for `buf`:\n{asm}"));
    assert!(
        (BSS_START..=BSS_END).contains(&addr),
        "`buf` should live in BSS, got ${addr:04X}"
    );
}

#[test]
fn a_local_arrays_data_is_not_emitted_into_the_code_stream() {
    // The old shape was `JMP af_2 / af_1: .BYTE … / af_2:` — data parked in the
    // instruction stream and jumped over.
    let asm = compile_success(
        r#"
        const OUT: addr = 0x0900;
        #[reset]
        fn main() {
            let buf: [u8; 8] = [0; 8];
            buf[0] = 1;
            OUT = buf[0];
            loop {}
        }
    "#,
    );
    assert!(
        !asm.contains("JMP af_"),
        "local array data should not be jumped over in the code stream:\n{asm}"
    );
}

#[test]
fn a_const_array_still_lives_in_rom() {
    // The fix must not move read-only tables out of ROM — that is where they
    // belong, and DATA is where the section allocator puts them.
    let asm = compile_success(
        r#"
        const TABLE: [u8; 4] = [10, 20, 30, 40];
        const OUT: addr = 0x0900;
        #[reset]
        fn main() { let i: u8 = 2; OUT = TABLE[i]; loop {} }
    "#,
    );
    assert!(asm.contains("TABLE:"), "the table keeps its own label");
    assert!(
        asm.contains(".BYTE $0A, $14, $1E, $28"),
        "and its bytes are still emitted:\n{asm}"
    );
}

// ============================================================================
// Behaviour is unchanged
// ============================================================================

#[test]
fn a_zero_filled_local_array_reads_back_as_zero() {
    // The initialiser moves from load time to run time, so this is now a real
    // fill rather than pre-set ROM bytes.
    let mut e = run(r#"
        const R0: addr = 0x0900;
        const R1: addr = 0x0901;
        #[reset]
        fn main() {
            let buf: [u8; 8] = [0; 8];
            R0 = buf[0];
            R1 = buf[7];
            loop {}
        }
    "#);
    assert_eq!((e.mem(0x0900), e.mem(0x0901)), (0, 0));
}

#[test]
fn a_local_array_with_explicit_elements_keeps_its_values() {
    let mut e = run(r#"
        const R0: addr = 0x0900;
        const R1: addr = 0x0901;
        const R2: addr = 0x0902;
        #[reset]
        fn main() {
            let t: [u8; 3] = [11, 22, 33];
            R0 = t[0];
            R1 = t[1];
            R2 = t[2];
            loop {}
        }
    "#);
    assert_eq!((e.mem(0x0900), e.mem(0x0901), e.mem(0x0902)), (11, 22, 33));
}

#[test]
fn writes_to_a_local_array_read_back() {
    let mut e = run(r#"
        const R0: addr = 0x0900;
        const R1: addr = 0x0901;
        #[reset]
        fn main() {
            let buf: [u8; 8] = [0; 8];
            let i: u8 = 5;
            buf[i] = 0x5A;
            buf[0] = 0x11;
            R0 = buf[0];
            R1 = buf[i];
            loop {}
        }
    "#);
    assert_eq!((e.mem(0x0900), e.mem(0x0901)), (0x11, 0x5A));
}

#[test]
fn a_u16_local_array_keeps_both_bytes_per_element() {
    let mut e = run(r#"
        const LO: addr = 0x0900;
        const HI: addr = 0x0901;
        #[reset]
        fn main() {
            let w: [u16; 3] = [0; 3];
            let i: u8 = 1;
            w[i] = 0x1234;
            LO = w[i].low;
            HI = w[i].high;
            loop {}
        }
    "#);
    assert_eq!(e.mem16(0x0900), 0x1234);
}

#[test]
fn a_local_array_is_refilled_on_every_call() {
    // Initialisation is now per-call. A function that mutates its array and is
    // called twice must see the initial values again the second time.
    let mut e = run(r#"
        const R0: addr = 0x0900;
        const R1: addr = 0x0901;
        fn bump() -> u8 {
            let v: [u8; 2] = [7, 7];
            let was: u8 = v[0];
            v[0] = 99;
            return was;
        }
        #[reset]
        fn main() { R0 = bump(); R1 = bump(); loop {} }
    "#);
    assert_eq!(
        (e.mem(0x0900), e.mem(0x0901)),
        (7, 7),
        "the second call must not observe the first call's write"
    );
}

// ============================================================================
// Allocation does not overlap
// ============================================================================

#[test]
fn two_arrays_in_one_function_do_not_overlap() {
    let mut e = run(r#"
        const R0: addr = 0x0900;
        const R1: addr = 0x0901;
        #[reset]
        fn main() {
            let x: [u8; 4] = [0; 4];
            let y: [u8; 4] = [0; 4];
            x[0] = 0x11;
            y[0] = 0x22;
            R0 = x[0];
            R1 = y[0];
            loop {}
        }
    "#);
    assert_eq!((e.mem(0x0900), e.mem(0x0901)), (0x11, 0x22));
}

#[test]
fn a_callees_array_does_not_clobber_its_callers() {
    // The colouring rule that protects zero-page frames has to protect these
    // blocks too: a callee's array must not overlap a live caller's.
    let mut e = run(r#"
        const R0: addr = 0x0900;
        const R1: addr = 0x0901;
        fn inner() -> u8 {
            let scratch: [u8; 4] = [0; 4];
            scratch[0] = 0xEE;
            return scratch[0];
        }
        #[reset]
        fn main() {
            let mine: [u8; 4] = [0; 4];
            mine[0] = 0x42;
            R0 = inner();
            R1 = mine[0];
            loop {}
        }
    "#);
    assert_eq!(
        (e.mem(0x0900), e.mem(0x0901)),
        (0xEE, 0x42),
        "the callee's array must not overwrite the caller's"
    );
}

#[test]
fn an_interrupt_handlers_array_does_not_clobber_mains() {
    // A handler is not a callee of `main`, so the call-graph colouring alone
    // would let their blocks share addresses. Unlike zero page, a RAM block is
    // far too large to save and restore in the prologue, so the two regions
    // have to be disjoint by construction.
    let mut e = run(r#"
        const R0: addr = 0x0900;
        const R1: addr = 0x0901;
        static FLAG: u8 = 0;
        #[irq]
        fn on_irq() {
            let hbuf: [u8; 4] = [0; 4];
            hbuf[0] = 0xAA;
            FLAG = hbuf[0];
        }
        #[reset]
        fn main() {
            let mbuf: [u8; 4] = [0; 4];
            mbuf[0] = 0x55;
            asm { "CLI" }
            R0 = mbuf[0];
            R1 = FLAG;
            loop {}
        }
    "#);
    assert_eq!(
        e.mem(0x0900),
        0x55,
        "main's array must survive an interrupt that uses its own"
    );
}

#[test]
fn a_local_array_larger_than_a_page_is_filled_completely() {
    // Absolute,X addressing reaches 256 bytes, so anything larger needs more
    // than one fill loop. An 8-bit counter over a 300-byte block wraps and
    // leaves most of it uninitialised. Indices are u8, so the tail past 255 is
    // only reachable through a slice — `foreach_slice_over_255_elements` in
    // strings_slices.rs covers that end; here the shape is the evidence.
    let src = r#"
        const R0: addr = 0x0900;
        const R1: addr = 0x0901;
        #[reset]
        fn main() {
            let big: [u8; 300] = [7; 300];
            R0 = big[0 as u8];
            R1 = big[255 as u8];
            loop {}
        }
    "#;
    let mut e = run(src);
    assert_eq!(e.mem(0x0900), 7, "first byte");
    assert_eq!(e.mem(0x0901), 7, "last byte of the first page");

    let asm = compile_success(src);
    let chunks = asm.matches("ai_").count();
    assert!(
        chunks >= 4,
        "a 300-byte block needs two fill loops (a label and a branch each), got {chunks} \
         references:\n{asm}"
    );
}

#[test]
fn arrays_with_different_fill_values_do_not_share_one() {
    // The fill value is loaded once, outside the loop, and the loop's label
    // invalidates register tracking. Two arrays filled with different bytes
    // back to back is what catches a load being wrongly elided as redundant.
    let mut e = run(r#"
        const R0: addr = 0x0900;
        const R1: addr = 0x0901;
        #[reset]
        fn main() {
            let a: [u8; 12] = [0xAA; 12];
            let b: [u8; 12] = [0xBB; 12];
            R0 = a[11];
            R1 = b[11];
            loop {}
        }
    "#);
    assert_eq!((e.mem(0x0900), e.mem(0x0901)), (0xAA, 0xBB));
}
