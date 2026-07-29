//! Enum and pattern-matching tests, verified by execution.
//!
//! These previously asserted that comments like `; Enum variant: Outer::Contains`
//! and `; Match statement` appeared in the output — which says the codegen took a
//! path, not that the right arm ran or that the payload arrived intact. Payload
//! extraction is exactly where truncation hides (a u16 field silently losing its
//! high byte compiles to entirely reasonable assembly), so the values are now
//! read back.

use crate::common::exec::run;

/// Run a program and read the u8 left at `$0900`.
fn out8(src: &str) -> u8 {
    run(src).mem(0x0900)
}

// ============================================================================
// Unit variants
// ============================================================================

#[test]
fn simple_enum_match_selects_the_right_arm() {
    let pick = |v: &str| {
        out8(&format!(
            r#"
            enum Direction {{ North, South, East, West }}
            const OUT: addr = 0x0900;
            #[reset]
            fn main() {{
                let d: Direction = Direction::{v};
                match d {{
                    Direction::North => {{ OUT = 1; }}
                    Direction::South => {{ OUT = 2; }}
                    Direction::East  => {{ OUT = 3; }}
                    Direction::West  => {{ OUT = 4; }}
                }}
                loop {{}}
            }}
        "#
        ))
    };
    assert_eq!(pick("North"), 1);
    assert_eq!(pick("South"), 2);
    assert_eq!(pick("East"), 3);
    assert_eq!(pick("West"), 4);
}

// ============================================================================
// Tuple payloads
// ============================================================================

#[test]
fn tuple_variant_single_u8_payload() {
    assert_eq!(
        out8(
            r#"
            enum Msg { Empty, Value(u8) }
            const OUT: addr = 0x0900;
            #[reset]
            fn main() {
                let m: Msg = Msg::Value(42);
                match m {
                    Msg::Empty    => { OUT = 0; }
                    Msg::Value(v) => { OUT = v; }
                }
                loop {}
            }
        "#
        ),
        42
    );
}

#[test]
fn tuple_variant_multi_field_payload() {
    // Each binding must get *its own* field, not the same byte twice.
    let src = r#"
        enum Message { Quit, Move(u8, u8) }
        const X: addr = 0x0900;
        const Y: addr = 0x0901;
        #[reset]
        fn main() {
            let m: Message = Message::Move(10, 20);
            match m {
                Message::Move(x, y) => { X = x; Y = y; }
                Message::Quit       => { X = 0; Y = 0; }
            }
            loop {}
        }
    "#;
    let mut e = run(src);
    assert_eq!((e.mem(0x0900), e.mem(0x0901)), (10, 20));
}

#[test]
fn tuple_variant_three_fields() {
    let src = r#"
        enum Message { Quit, ChangeColor(u8, u8, u8) }
        const R: addr = 0x0900;
        const G: addr = 0x0901;
        const B: addr = 0x0902;
        #[reset]
        fn main() {
            let m: Message = Message::ChangeColor(1, 2, 3);
            match m {
                Message::ChangeColor(r, g, b) => { R = r; G = g; B = b; }
                Message::Quit                 => { R = 0; G = 0; B = 0; }
            }
            loop {}
        }
    "#;
    let mut e = run(src);
    assert_eq!((e.mem(0x0900), e.mem(0x0901), e.mem(0x0902)), (1, 2, 3));
}

#[test]
fn tuple_variant_u16_payload_keeps_both_bytes() {
    // The high byte is where a truncating extraction shows up: 1000 is $03E8,
    // so a low-byte-only read gives $E8 and looks plausible.
    let src = r#"
        enum Result16 { Err(u8), Ok(u16) }
        const LO: addr = 0x0900;
        const HI: addr = 0x0901;
        #[reset]
        fn main() {
            let r: Result16 = Result16::Ok(1000);
            match r {
                Result16::Ok(v)  => { LO = v.low; HI = v.high; }
                Result16::Err(e) => { LO = e; HI = 0; }
            }
            loop {}
        }
    "#;
    assert_eq!(run(src).mem16(0x0900), 1000);
}

#[test]
fn tuple_variant_mixed_width_fields() {
    // A u8 followed by a u16: the second field's offset depends on the first
    // field's width, so a wrong stride reads the boundary between them.
    let src = r#"
        enum Packet { None, Header(u8, u16) }
        const TAG: addr = 0x0900;
        const LO: addr = 0x0901;
        const HI: addr = 0x0902;
        #[reset]
        fn main() {
            let p: Packet = Packet::Header(7, 0x1234);
            match p {
                Packet::Header(t, v) => { TAG = t; LO = v.low; HI = v.high; }
                Packet::None         => { TAG = 0; LO = 0; HI = 0; }
            }
            loop {}
        }
    "#;
    let mut e = run(src);
    assert_eq!(e.mem(0x0900), 7, "u8 field");
    assert_eq!(e.mem16(0x0901), 0x1234, "u16 field after a u8");
}

// ============================================================================
// Match forms
// ============================================================================

#[test]
fn match_tuple_variant_with_wildcard() {
    let pick = |v: u8| {
        out8(&format!(
            r#"
            enum Msg {{ A(u8), B(u8), C }}
            const OUT: addr = 0x0900;
            #[reset]
            fn main() {{
                let sel: u8 = {v};
                let m: Msg = Msg::A(9);
                if sel == 1 {{ m = Msg::B(5); }}
                if sel == 2 {{ m = Msg::C; }}
                match m {{
                    Msg::A(x) => {{ OUT = x; }}
                    _         => {{ OUT = 99; }}
                }}
                loop {{}}
            }}
        "#
        ))
    };
    assert_eq!(pick(0), 9, "the A arm binds its payload");
    assert_eq!(pick(1), 99, "B falls to the wildcard");
    assert_eq!(pick(2), 99, "C falls to the wildcard");
}

#[test]
fn match_tuple_variant_ignoring_payload() {
    // Matching a payload-carrying variant without binding it must still work.
    assert_eq!(
        out8(
            r#"
            enum Msg { Value(u8), Empty }
            const OUT: addr = 0x0900;
            #[reset]
            fn main() {
                let m: Msg = Msg::Value(42);
                match m {
                    Msg::Value(_v) => { OUT = 1; }
                    Msg::Empty     => { OUT = 2; }
                }
                loop {}
            }
        "#
        ),
        1
    );
}

#[test]
fn match_selects_among_several_payload_variants() {
    let pick = |v: u8| {
        out8(&format!(
            r#"
            enum Op {{ Add(u8), Sub(u8), Nop }}
            const OUT: addr = 0x0900;
            #[reset]
            fn main() {{
                let sel: u8 = {v};
                let o: Op = Op::Add(10);
                if sel == 1 {{ o = Op::Sub(3); }}
                if sel == 2 {{ o = Op::Nop; }}
                match o {{
                    Op::Add(n) => {{ OUT = 100 + n; }}
                    Op::Sub(n) => {{ OUT = 100 - n; }}
                    Op::Nop    => {{ OUT = 0; }}
                }}
                loop {{}}
            }}
        "#
        ))
    };
    assert_eq!(pick(0), 110);
    assert_eq!(pick(1), 97);
    assert_eq!(pick(2), 0);
}

#[test]
fn two_enums_coexist() {
    // Distinct enum types must not share discriminant storage.
    let src = r#"
        enum Inner { Value(u8) }
        enum Outer { Contains(u8), Empty }
        const OUTER: addr = 0x0900;
        const INNER: addr = 0x0901;
        #[reset]
        fn main() {
            let i: Inner = Inner::Value(42);
            let o: Outer = Outer::Contains(10);
            match o {
                Outer::Contains(x) => { OUTER = x; }
                Outer::Empty       => { OUTER = 0; }
            }
            match i {
                Inner::Value(y) => { INNER = y; }
            }
            loop {}
        }
    "#;
    let mut e = run(src);
    assert_eq!((e.mem(0x0900), e.mem(0x0901)), (10, 42));
}

// ============================================================================
// Enums across functions and loops
// ============================================================================

#[test]
fn tuple_variant_returned_from_function() {
    assert_eq!(
        out8(
            r#"
            enum Status { Ok(u8), Fail }
            const OUT: addr = 0x0900;
            fn make() -> Status { return Status::Ok(55); }
            #[reset]
            fn main() {
                let s: Status = make();
                match s {
                    Status::Ok(v) => { OUT = v; }
                    Status::Fail  => { OUT = 0; }
                }
                loop {}
            }
        "#
        ),
        55
    );
}

#[test]
fn tuple_variant_matched_in_a_loop() {
    // The payload must be re-extracted each iteration, not read once.
    assert_eq!(
        out8(
            r#"
            enum Item { Val(u8), None }
            const OUT: addr = 0x0900;
            #[reset]
            fn main() {
                let total: u8 = 0;
                for i in 0..4 {
                    let it: Item = Item::Val(i);
                    match it {
                        Item::Val(v) => { total = total + v; }
                        Item::None   => { }
                    }
                }
                OUT = total;
                loop {}
            }
        "#
        ),
        6 // 0 + 1 + 2 + 3
    );
}

#[test]
fn jump_table_dispatch_preserves_the_payload_pointer() {
    // Regression: with 3+ arms the match switches from sequential BEQ dispatch
    // to a jump table, and the jump vector used to be written over the very
    // zero-page pointer the arm bodies dereference to read their payload. Every
    // arm then extracted a byte of its own code. Four payload-carrying variants
    // keep the jump-table strategy selected.
    let pick = |v: u8| {
        out8(&format!(
            r#"
            enum Op {{ A(u8), B(u8), C(u8), D(u8) }}
            const OUT: addr = 0x0900;
            #[reset]
            fn main() {{
                let sel: u8 = {v};
                let o: Op = Op::A(11);
                if sel == 1 {{ o = Op::B(22); }}
                if sel == 2 {{ o = Op::C(33); }}
                if sel == 3 {{ o = Op::D(44); }}
                match o {{
                    Op::A(n) => {{ OUT = n; }}
                    Op::B(n) => {{ OUT = n; }}
                    Op::C(n) => {{ OUT = n; }}
                    Op::D(n) => {{ OUT = n; }}
                }}
                loop {{}}
            }}
        "#
        ))
    };
    assert_eq!(pick(0), 11);
    assert_eq!(pick(1), 22);
    assert_eq!(pick(2), 33);
    assert_eq!(pick(3), 44);
}

// ============================================================================
// Patterns checked against the scrutinee
//
// Nothing used to validate a pattern against the matched value's type: a
// pattern naming a different enum compared tags against the wrong
// discriminants, and an out-of-range literal silently truncated in the
// emitted CMP (`300` matched a u8 as 44).
// ============================================================================

fn expect_error(src: &str) -> String {
    match crate::common::harness::compile(src) {
        crate::common::harness::CompileResult::SemaError(e)
        | crate::common::harness::CompileResult::CodegenError(e) => e,
        crate::common::harness::CompileResult::Success(..) => {
            panic!("expected a compile error, but it compiled")
        }
        other => panic!("expected an error, got {other:?}"),
    }
}

#[test]
fn a_pattern_naming_a_different_enum_is_rejected() {
    let err = expect_error(
        r#"
        const OUT: addr = 0x0900;
        enum A { X, Y }
        enum B { Z }
        #[reset]
        fn main() {
            let a: A = A::X;
            match a {
                B::Z => { OUT = 1; }
                _ => { OUT = 2; }
            }
            loop {}
        }
    "#,
    );
    assert!(
        err.contains("enum 'B' but the matched value is 'A'"),
        "{err}"
    );
}

#[test]
fn a_literal_pattern_out_of_range_for_the_scrutinee_is_rejected() {
    let err = expect_error(
        r#"
        const OUT: addr = 0x0900;
        fn f(x: u8) -> u8 {
            match x {
                300 => { return 1; }
                _ => { return 2; }
            }
        }
        #[reset]
        fn main() { OUT = f(44); loop {} }
    "#,
    );
    assert!(err.contains("300 cannot match a value of type u8"), "{err}");
}

#[test]
fn a_negative_literal_pattern_on_an_unsigned_scrutinee_is_rejected() {
    let err = expect_error(
        r#"
        const OUT: addr = 0x0900;
        #[reset]
        fn main() {
            let x: u8 = 251;
            match x {
                -5 => { OUT = 1; }
                _ => { OUT = 2; }
            }
            loop {}
        }
    "#,
    );
    assert!(err.contains("-5 cannot match a value of type u8"), "{err}");
}

#[test]
fn a_tuple_variant_pattern_with_the_wrong_binding_count_is_rejected() {
    let err = expect_error(
        r#"
        const OUT: addr = 0x0900;
        enum E { Pair(u8, u8) }
        #[reset]
        fn main() {
            let e: E = E::Pair(1, 2);
            match e {
                E::Pair(a, b, c) => { OUT = a + b + c; }
            }
            loop {}
        }
    "#,
    );
    assert!(err.contains("2 field(s), but the pattern binds 3"), "{err}");
}

#[test]
fn a_match_expression_checks_patterns_too() {
    let err = expect_error(
        r#"
        const OUT: addr = 0x0900;
        enum A { X, Y }
        enum B { Z }
        #[reset]
        fn main() {
            let a: A = A::X;
            let r: u8 = match a {
                B::Z => 1,
                _ => 2,
            };
            OUT = r;
            loop {}
        }
    "#,
    );
    assert!(
        err.contains("enum 'B' but the matched value is 'A'"),
        "{err}"
    );
}

// ============================================================================
// Runtime enum values own their storage
//
// A runtime-constructed enum used to be a pointer into shared codegen
// scratch: the second construction overwrote the first's bytes, and any
// later use of the $20 temp destroyed a live enum's payload. Binding now
// copies the bytes into the variable's own per-declaration block.
// ============================================================================

#[test]
fn two_live_enums_do_not_alias() {
    // mk constructs Value in the same scratch both times; before the copy,
    // `e1` and `e2` were the same address and matching e1 extracted 2.
    let mut e = run(r#"
        const R0: addr = 0x0900;
        const R1: addr = 0x0901;
        enum Msg { Ping, Value(u8) }
        fn mk(n: u8) -> Msg { return Msg::Value(n); }
        #[reset]
        fn main() {
            let e1: Msg = mk(1);
            let e2: Msg = mk(2);
            match e1 {
                Msg::Ping => { R0 = 0; }
                Msg::Value(v) => { R0 = v; }
            }
            match e2 {
                Msg::Ping => { R1 = 0; }
                Msg::Value(v) => { R1 = v; }
            }
            loop {}
        }
    "#);
    assert_eq!((e.mem(0x0900), e.mem(0x0901)), (1, 2));
}

#[test]
fn an_enum_payload_survives_later_temp_use() {
    // Every binary op uses the same scratch the enum was constructed in.
    let mut e = run(r#"
        const OUT: addr = 0x0900;
        enum Msg { Ping, Value(u8) }
        #[reset]
        fn main() {
            let m: Msg = Msg::Value(42);
            let x: u8 = 3 + 4 + 5 + 6;
            match m {
                Msg::Ping => { OUT = 0; }
                Msg::Value(v) => { OUT = v + x; }
            }
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0900), 42 + 18);
}

#[test]
fn reassigning_an_enum_replaces_only_that_variable() {
    let mut e = run(r#"
        const R0: addr = 0x0900;
        const R1: addr = 0x0901;
        enum Msg { Ping, Value(u8) }
        fn mk(n: u8) -> Msg { return Msg::Value(n); }
        #[reset]
        fn main() {
            let a: Msg = mk(1);
            let b: Msg = mk(2);
            a = mk(3);
            match a {
                Msg::Ping => { R0 = 0; }
                Msg::Value(v) => { R0 = v; }
            }
            match b {
                Msg::Ping => { R1 = 0; }
                Msg::Value(v) => { R1 = v; }
            }
            loop {}
        }
    "#);
    assert_eq!((e.mem(0x0900), e.mem(0x0901)), (3, 2));
}
