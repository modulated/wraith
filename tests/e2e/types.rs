//! Type tests (arrays, structs, enums, signed integers), verified by execution.
//!
//! These previously asserted that `LDA` and `STA` appear somewhere — true of
//! essentially any program, and true whether or not the value is right. One test
//! even documented "-5 = 0xFB" in a comment while asserting only that a load
//! existed. They now read back the values the program actually produced.
//!
//! Two tests stay at the assembly level on purpose: constant-array *data layout*
//! and the shorthand-expansion shape are properties of what is emitted, not of a
//! computed result.

use crate::common::exec::run;
use crate::common::{assert_asm_contains, compile_success};

/// Run a program body and read back the u8 left in `OUT` ($0900).
fn eval(body: &str) -> u8 {
    let src = format!(
        r#"
        const OUT: addr = 0x0900;
        #[reset]
        fn main() {{
            {body}
            loop {{}}
        }}
    "#
    );
    run(&src).mem(0x0900)
}

/// Run a program body and read back the u16 left in LO/HI ($0900/$0901).
fn eval16(body: &str) -> u16 {
    let src = format!(
        r#"
        const LO: addr = 0x0900;
        const HI: addr = 0x0901;
        #[reset]
        fn main() {{
            {body}
            loop {{}}
        }}
    "#
    );
    run(&src).mem16(0x0900)
}

// ============================================================================
// Arrays: reading
// ============================================================================

#[test]
fn array_literal_elements_readable() {
    assert_eq!(eval("let a: [u8; 5] = [1, 2, 3, 4, 5]; OUT = a[0 as u8];"), 1);
    assert_eq!(eval("let a: [u8; 5] = [1, 2, 3, 4, 5]; OUT = a[4 as u8];"), 5);
}

#[test]
fn array_fill_sets_every_element() {
    assert_eq!(eval("let a: [u8; 8] = [7; 8]; OUT = a[0 as u8];"), 7);
    assert_eq!(eval("let a: [u8; 8] = [7; 8]; OUT = a[7 as u8];"), 7);
}

#[test]
fn array_index_constant() {
    assert_eq!(eval("let a: [u8; 5] = [10, 20, 30, 40, 50]; OUT = a[2 as u8];"), 30);
}

#[test]
fn array_index_variable() {
    // A wrong index scale or a stale register shows up here, not in a `LDA` check.
    assert_eq!(
        eval("let a: [u8; 5] = [10, 20, 30, 40, 50]; let i: u8 = 3; OUT = a[i];"),
        40
    );
}

#[test]
fn array_index_boundaries() {
    assert_eq!(eval("let a: [u8; 3] = [11, 22, 33]; OUT = a[0 as u8];"), 11);
    assert_eq!(eval("let a: [u8; 3] = [11, 22, 33]; OUT = a[2 as u8];"), 33);
}

#[test]
fn array_index_u16_elements_scales_by_two() {
    // Element 2 of a u16 array lives at byte offset 4; an unscaled index reads
    // the wrong half of element 1.
    assert_eq!(
        eval16(
            "let a: [u16; 4] = [0x1111, 0x2222, 0x3333, 0x4444]; let i: u8 = 2;
             let v: u16 = a[i]; LO = v.low; HI = v.high;"
        ),
        0x3333
    );
}

// ============================================================================
// Arrays: writing
// ============================================================================

#[test]
fn array_write_constant_index() {
    assert_eq!(
        eval("let a: [u8; 4] = [0; 4]; a[1 as u8] = 99; OUT = a[1 as u8];"),
        99
    );
}

#[test]
fn array_write_variable_index() {
    assert_eq!(
        eval("let a: [u8; 4] = [0; 4]; let i: u8 = 2; a[i] = 77; OUT = a[i];"),
        77
    );
}

#[test]
fn array_write_expression_index() {
    assert_eq!(
        eval("let a: [u8; 6] = [0; 6]; let i: u8 = 1; a[i + 2] = 55; OUT = a[3 as u8];"),
        55
    );
}

#[test]
fn array_write_in_loop_fills_each_slot() {
    // Sums the written values back: proves every slot got its own index.
    assert_eq!(
        eval(
            "let a: [u8; 5] = [0; 5];
             for i in 0..5 { a[i] = i; }
             let s: u8 = 0;
             for j in 0..5 { s = s + a[j]; }
             OUT = s;"
        ),
        10
    );
}

#[test]
fn array_write_does_not_disturb_neighbours() {
    assert_eq!(
        eval("let a: [u8; 4] = [1, 2, 3, 4]; a[1 as u8] = 99; OUT = a[2 as u8];"),
        3,
        "writing a[1] must leave a[2] alone"
    );
}

#[test]
fn array_write_u16_elements() {
    assert_eq!(
        eval16(
            "let a: [u16; 3] = [0 as u16; 3]; let i: u8 = 1; a[i] = 0xBEEF;
             let v: u16 = a[i]; LO = v.low; HI = v.high;"
        ),
        0xBEEF
    );
}

// ============================================================================
// Structs
// ============================================================================

#[test]
fn struct_field_read() {
    let src = r#"
        struct Point { x: u8, y: u8 }
        const OUT: addr = 0x0900;
        #[reset]
        fn main() {
            let p: Point = Point { x: 3, y: 9 };
            OUT = p.y;
            loop {}
        }
    "#;
    assert_eq!(run(src).mem(0x0900), 9);
}

#[test]
fn struct_multiple_fields_are_distinct() {
    let src = r#"
        struct Rect { w: u8, h: u8, tag: u8 }
        const F0: addr = 0x0900;
        const F1: addr = 0x0901;
        const F2: addr = 0x0902;
        #[reset]
        fn main() {
            let r: Rect = Rect { w: 4, h: 5, tag: 6 };
            F0 = r.w; F1 = r.h; F2 = r.tag;
            loop {}
        }
    "#;
    let mut e = run(src);
    assert_eq!((e.mem(0x0900), e.mem(0x0901), e.mem(0x0902)), (4, 5, 6));
}

#[test]
fn struct_field_u16_keeps_both_bytes() {
    let src = r#"
        struct Wide { tag: u8, val: u16 }
        const LO: addr = 0x0900;
        const HI: addr = 0x0901;
        #[reset]
        fn main() {
            let w: Wide = Wide { tag: 1, val: 0x1234 };
            LO = w.val.low;
            HI = w.val.high;
            loop {}
        }
    "#;
    assert_eq!(run(src).mem16(0x0900), 0x1234);
}

#[test]
fn struct_field_write_then_read() {
    let src = r#"
        struct Point { x: u8, y: u8 }
        const OUT: addr = 0x0900;
        #[reset]
        fn main() {
            let p: Point = Point { x: 1, y: 2 };
            p.y = 42;
            OUT = p.y;
            loop {}
        }
    "#;
    assert_eq!(run(src).mem(0x0900), 42);
}

// ============================================================================
// Enums
// ============================================================================

#[test]
fn enum_unit_variant_discriminates() {
    let pick = |v: &str| {
        let src = format!(
            r#"
            enum State {{ Idle, Running, Done }}
            const OUT: addr = 0x0900;
            #[reset]
            fn main() {{
                let s: State = State::{v};
                match s {{
                    State::Idle    => {{ OUT = 1; }}
                    State::Running => {{ OUT = 2; }}
                    State::Done    => {{ OUT = 3; }}
                }}
                loop {{}}
            }}
        "#
        );
        run(&src).mem(0x0900)
    };
    assert_eq!(pick("Idle"), 1);
    assert_eq!(pick("Running"), 2);
    assert_eq!(pick("Done"), 3);
}

#[test]
fn enum_tuple_payload_round_trips() {
    let src = r#"
        enum Msg { Ping, Value(u8) }
        const OUT: addr = 0x0900;
        #[reset]
        fn main() {
            let m: Msg = Msg::Value(77);
            match m {
                Msg::Ping      => { OUT = 0; }
                Msg::Value(v)  => { OUT = v; }
            }
            loop {}
        }
    "#;
    assert_eq!(run(src).mem(0x0900), 77);
}

// ============================================================================
// Signed integers — two's complement is the whole point
// ============================================================================

#[test]
fn i8_negative_values_are_twos_complement() {
    // -5 is $FB and -10 is $F6; the old test only asserted an LDA existed.
    assert_eq!(eval("let x: i8 = -5; OUT = x as u8;"), 0xFB);
    assert_eq!(eval("let y: i8 = -10; OUT = y as u8;"), 0xF6);
}

#[test]
fn i8_arithmetic_across_zero() {
    assert_eq!(eval("let a: i8 = -5; let b: i8 = 3; OUT = (a + b) as u8;"), 0xFE); // -2
    assert_eq!(eval("let a: i8 = 5; let b: i8 = 10; OUT = (a - b) as u8;"), 0xFB); // -5
}

#[test]
fn i16_basic_operations() {
    assert_eq!(
        eval16("let a: i16 = 1000; let b: i16 = 234; let c: i16 = a + b; LO = c.low; HI = c.high;"),
        1234
    );
}

#[test]
fn i8_to_i16_sign_extends() {
    // -5 must become $FFFB, not $00FB.
    assert_eq!(
        eval16("let a: i8 = -5; let b: i16 = a as i16; LO = b.low; HI = b.high;"),
        0xFFFB
    );
}

#[test]
fn signed_comparison_orders_negatives_correctly() {
    let src = r#"
        const OUT: addr = 0x0900;
        #[reset]
        fn main() {
            let a: i8 = -5;
            let b: i8 = 3;
            if a < b { OUT = 1; } else { OUT = 2; }
            loop {}
        }
    "#;
    assert_eq!(run(src).mem(0x0900), 1, "-5 < 3 under signed comparison");
}

#[test]
fn unsigned_cast_reinterprets_the_bits() {
    assert_eq!(eval("let a: i8 = -1; OUT = a as u8;"), 0xFF);
    assert_eq!(eval("let a: u8 = 200; OUT = (a as i8) as u8;"), 200);
}

// ============================================================================
// Shorthand array syntax
// ============================================================================

#[test]
fn shorthand_array_fills_all_elements() {
    assert_eq!(eval("let a: [u8; 6] = [3; 6]; OUT = a[5 as u8];"), 3);
}

#[test]
fn shorthand_array_zero_fill() {
    assert_eq!(eval("let a: [u8; 4] = [0; 4]; OUT = a[2 as u8];"), 0);
}

#[test]
fn shorthand_array_u16_elements() {
    assert_eq!(
        eval16(
            "let a: [u16; 3] = [0x0102; 3]; let i: u8 = 2;
             let v: u16 = a[i]; LO = v.low; HI = v.high;"
        ),
        0x0102
    );
}

#[test]
fn shorthand_and_explicit_arrays_agree() {
    assert_eq!(eval("let a: [u8; 3] = [9; 3]; OUT = a[1 as u8];"), 9);
    assert_eq!(eval("let b: [u8; 3] = [9, 9, 9]; OUT = b[1 as u8];"), 9);
}

#[test]
fn shorthand_array_with_bool() {
    assert_eq!(
        eval("let a: [bool; 4] = [true; 4]; if a[2 as u8] { OUT = 1; } else { OUT = 2; }"),
        1
    );
}

// ============================================================================
// Data layout — legitimately assembly-level
// ============================================================================

#[test]
fn global_u16_array_data_is_little_endian() {
    // Const u16 arrays must emit two little-endian bytes per element, not a
    // single truncated byte. This is about the emitted data, not a value.
    let asm = compile_success(
        r#"
        const TABLE: [u16; 3] = [0x1111, 0x2222, 0x3333];
        const FILL: [u16; 3] = [0x00AB; 3];
        // Read both so they survive dead-code elimination; this test is about
        // the bytes they emit, not about whether unused tables are kept. The
        // index is a variable because a constant one is folded away.
        fn main() { let i: u8 = 0; let a: u16 = TABLE[i]; let b: u16 = FILL[i]; }
    "#,
    );
    assert_asm_contains(&asm, ".BYTE $11, $11, $22, $22, $33, $33");
    assert_asm_contains(&asm, ".BYTE $AB, $00, $AB, $00, $AB, $00");
}

#[test]
fn shorthand_array_size_one_does_not_expand() {
    // A codegen-shape check: `[42]` for a 1-element array is already exact and
    // must not go through the fill expansion.
    let asm = compile_success("fn main() { let single: [u8; 1] = [42]; }");
    assert!(!asm.contains("Expanding [value]"));
    assert_asm_contains(&asm, ".BYTE $2A");
}

// ============================================================================
// Array element literals adopt the declared element type
// ============================================================================

#[test]
fn array_literal_elements_adopt_declared_type() {
    // The element type is stated in the declaration, so the literals take it.
    // Previously the array's type was inferred bottom-up from the first element,
    // so `[u16; 3] = [0, 0, 0]` inferred `[u8; 3]` and failed to match, and
    // `[1, 2, 300]` failed because 300 did not match the u8 inferred from `1`.
    assert_eq!(
        eval16("let a: [u16; 3] = [1, 2, 300]; let v: u16 = a[2 as u8]; LO = v.low; HI = v.high;"),
        300
    );
    assert_eq!(
        eval16("let a: [u16; 3] = [0, 0, 0]; let v: u16 = a[1 as u8]; LO = v.low; HI = v.high;"),
        0
    );
}

#[test]
fn array_fill_adopts_declared_element_type() {
    assert_eq!(
        eval16("let a: [u16; 4] = [1000; 4]; let v: u16 = a[3 as u8]; LO = v.low; HI = v.high;"),
        1000
    );
}

#[test]
fn signed_array_literals_adopt_declared_type() {
    assert_eq!(
        eval16("let a: [i16; 2] = [-100, -200]; let v: i16 = a[0 as u8]; LO = v.low; HI = v.high;"),
        0xFF9C // -100
    );
}

#[test]
fn array_element_out_of_range_still_errors() {
    // Adoption types the literal; it does not silently truncate one that cannot fit.
    let r = crate::common::compile("fn main() { let a: [u8; 2] = [1, 300]; }");
    assert!(
        !matches!(r, crate::common::harness::CompileResult::Success(..)),
        "300 does not fit a u8 element"
    );
}
