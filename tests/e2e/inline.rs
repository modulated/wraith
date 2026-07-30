//! `#[inline]` function tests, verified by execution.
//!
//! An inline function's body is emitted into its *caller's* output, which makes
//! it easy for the two to be confused for one another. The case that broke:
//! inline assembly refers to parameters as `{a}`, and the substitution resolved
//! a name by matching the symbol's owning function against whichever function
//! codegen thought it was emitting — the caller. Every `{param}` in an inline
//! function therefore failed to resolve, local or imported, with `undefined
//! symbol 'a'`. The whole `min`/`max`/`clamp` section of `std/math.wr` was
//! uncallable.
//!
//! Nothing caught it because no test executed an inline function that took an
//! argument. These do.

use crate::common::exec::run;

/// Run a program and read the u8 left at `$0900`.
fn out8(src: &str) -> u8 {
    run(src).mem(0x0900)
}

// ============================================================================
// Local inline functions
// ============================================================================

#[test]
fn inline_asm_body_resolves_its_parameters() {
    let pick = |a: u8, b: u8| {
        out8(&format!(
            r#"
            const OUT: addr = 0x0900;
            #[inline]
            fn imin(a: u8, b: u8) -> u8 {{
                asm {{
                    "LDA {{a}}",
                    "CMP {{b}}",
                    "BCC ISMIN",
                    "LDA {{b}}",
                    "ISMIN:",
                }}
            }}
            #[reset]
            fn main() {{
                OUT = imin({a}, {b});
                loop {{}}
            }}
        "#
        ))
    };
    assert_eq!(pick(3, 7), 3);
    assert_eq!(pick(7, 3), 3);
    assert_eq!(pick(5, 5), 5, "equal operands");
    assert_eq!(pick(0, 255), 0, "boundary");
}

#[test]
fn inline_wraith_body_resolves_its_parameters() {
    // The same scoping question, without any assembly: an ordinary body
    // referring to its own parameters.
    assert_eq!(
        out8(
            r#"
            const OUT: addr = 0x0900;
            #[inline]
            fn add3(a: u8, b: u8, c: u8) -> u8 { return a + b + c; }
            #[reset]
            fn main() {
                OUT = add3(10, 20, 12);
                loop {}
            }
        "#
        ),
        42
    );
}

#[test]
fn each_parameter_gets_its_own_slot() {
    // A substitution that resolved every `{...}` to the same address would
    // still compile and would still look plausible in the output.
    assert_eq!(
        out8(
            r#"
            const OUT: addr = 0x0900;
            #[inline]
            fn pick_second(a: u8, b: u8, c: u8) -> u8 {
                asm { "LDA {b}", }
            }
            #[reset]
            fn main() {
                OUT = pick_second(1, 2, 3);
                loop {}
            }
        "#
        ),
        2
    );
}

#[test]
fn the_same_inline_function_can_be_called_twice() {
    // Both expansions emit the body's assembly labels, so they must be
    // uniquified per call site or the assembler sees a duplicate.
    assert_eq!(
        out8(
            r#"
            const OUT: addr = 0x0900;
            #[inline]
            fn imax(a: u8, b: u8) -> u8 {
                asm { "LDA {a}", "CMP {b}", "BCS ISMAX", "LDA {b}", "ISMAX:", }
            }
            #[reset]
            fn main() {
                let x: u8 = imax(3, 9);
                OUT = imax(x, 4);
                loop {}
            }
        "#
        ),
        9
    );
}

#[test]
fn an_inline_call_does_not_disturb_the_callers_locals() {
    // The expansion writes arguments into the callee's frame slots. Those are
    // colored against the caller's, but the caller's live values have to come
    // through the expansion intact either way.
    let src = r#"
        const R0: addr = 0x0900;
        const R1: addr = 0x0901;
        const R2: addr = 0x0902;
        #[inline]
        fn imin(a: u8, b: u8) -> u8 {
            asm { "LDA {a}", "CMP {b}", "BCC ISMIN", "LDA {b}", "ISMIN:", }
        }
        #[reset]
        fn main() {
            let before: u8 = 11;
            let after: u8 = 22;
            let m: u8 = imin(3, 7);
            R0 = before;
            R1 = m;
            R2 = after;
            loop {}
        }
    "#;
    let mut e = run(src);
    assert_eq!(
        (e.mem(0x0900), e.mem(0x0901), e.mem(0x0902)),
        (11, 3, 22),
        "locals live across the expansion must survive it"
    );
}

#[test]
fn an_inline_function_can_be_called_from_a_non_entry_function() {
    // `current_function` is restored after the expansion, so the enclosing
    // function's own context has to be intact for the code that follows.
    assert_eq!(
        out8(
            r#"
            const OUT: addr = 0x0900;
            #[inline]
            fn double(a: u8) -> u8 { asm { "LDA {a}", "ASL A", } }
            fn helper(v: u8) -> u8 {
                let d: u8 = double(v);
                return d + 1;
            }
            #[reset]
            fn main() {
                OUT = helper(20);
                loop {}
            }
        "#
        ),
        41
    );
}

#[test]
fn an_inline_function_may_call_another_inline_function() {
    assert_eq!(
        out8(
            r#"
            const OUT: addr = 0x0900;
            #[inline]
            fn inc(a: u8) -> u8 { asm { "LDA {a}", "CLC", "ADC #$01", } }
            #[inline]
            fn inc2(a: u8) -> u8 { return inc(a) + 1; }
            #[reset]
            fn main() {
                OUT = inc2(5);
                loop {}
            }
        "#
        ),
        7
    );
}

// ============================================================================
// Imported inline functions
// ============================================================================

#[test]
fn imported_inline_min_and_max_work() {
    // These are the real std/math.wr definitions, and they were the original
    // symptom: `import { min } from "math.wr"` failed to compile at all.
    let src = r#"
        import { min, max } from "math.wr";
        const LO: addr = 0x0900;
        const HI: addr = 0x0901;
        #[reset]
        fn main() {
            LO = min(37, 42);
            HI = max(37, 42);
            loop {}
        }
    "#;
    let mut e = run(src);
    assert_eq!((e.mem(0x0900), e.mem(0x0901)), (37, 42));
}

#[test]
fn imported_inline_clamp_covers_all_three_branches() {
    let pick = |v: u8| {
        out8(&format!(
            r#"
            import {{ clamp }} from "math.wr";
            const OUT: addr = 0x0900;
            #[reset]
            fn main() {{
                OUT = clamp({v}, 10, 20);
                loop {{}}
            }}
        "#
        ))
    };
    assert_eq!(pick(5), 10, "below the low bound");
    assert_eq!(pick(15), 15, "in range");
    assert_eq!(pick(99), 20, "above the high bound");
    assert_eq!(pick(10), 10, "on the low bound");
    assert_eq!(pick(20), 20, "on the high bound");
}

#[test]
fn imported_inline_functions_reach_through_a_glob() {
    assert_eq!(
        out8(
            r#"
            import { * } from "math.wr";
            const OUT: addr = 0x0900;
            #[reset]
            fn main() {
                OUT = min(max(1, 5), 3);
                loop {}
            }
        "#
        ),
        3
    );
}

#[test]
fn an_imported_inline_function_is_expanded_not_called() {
    // Inlining is the point: there should be no JSR to it, and no standalone
    // body emitted for it either.
    let asm = crate::common::harness::compile_success(
        r#"
        import { min } from "math.wr";
        const OUT: addr = 0x0900;
        #[reset]
        fn main() {
            OUT = min(3, 7);
            loop {}
        }
    "#,
    );
    assert!(!asm.contains("JSR min"), "inline call must not emit a JSR");
    assert!(
        !asm.lines().any(|l| l.trim_end() == "min:"),
        "no standalone body should be emitted for an inline function"
    );
    assert!(asm.contains("CMP"), "the body should be expanded in place");
}

// ============================================================================
// Argument width
// ============================================================================
//
// Arguments were stored with a bare `STA` whatever the parameter's type, so
// anything wider than a byte arrived with only its low byte set. It compiled,
// and the callee then read whatever happened to be in the next slot.

#[test]
fn a_u16_argument_arrives_with_both_bytes() {
    let src = r#"
        const LO: addr = 0x0900;
        const HI: addr = 0x0901;
        #[inline]
        fn split(v: u16) { LO = v.low; HI = v.high; }
        #[reset]
        fn main() { split(0x1234); loop {} }
    "#;
    assert_eq!(run(src).mem16(0x0900), 0x1234);
}

#[test]
fn an_enum_argument_arrives_as_a_whole_pointer() {
    // An enum is passed as a two-byte pointer. With only the low byte written,
    // the callee dereferenced a half-formed address.
    assert_eq!(
        out8(
            r#"
            enum Direction { OUTPUT = 0xFF, INPUT = 0x00 }
            const OUT: addr = 0x0900;
            #[inline]
            fn set(d: Direction) { OUT = d as u8; }
            #[reset]
            fn main() { set(Direction::OUTPUT); loop {} }
        "#
        ),
        0xFF
    );
}

#[test]
fn an_eight_bit_argument_widens_to_a_u16_parameter() {
    // There is no high byte to copy, so it has to be zeroed rather than left
    // as whatever the slot held.
    let src = r#"
        const LO: addr = 0x0900;
        const HI: addr = 0x0901;
        #[inline]
        fn widen(v: u16) { LO = v.low; HI = v.high; }
        #[reset]
        fn main() { let small: u8 = 0x42; widen(small); loop {} }
    "#;
    assert_eq!(run(src).mem16(0x0900), 0x0042);
}

#[test]
fn mixed_width_arguments_land_in_their_own_slots() {
    let src = r#"
        const R0: addr = 0x0900;
        const R1: addr = 0x0901;
        const R2: addr = 0x0902;
        #[inline]
        fn three(x: u8, y: u16, z: u8) { R0 = x; R1 = y.high; R2 = z; }
        #[reset]
        fn main() { three(1, 0x0203, 4); loop {} }
    "#;
    let mut e = run(src);
    assert_eq!(
        (e.mem(0x0900), e.mem(0x0901), e.mem(0x0902)),
        (1, 2, 4),
        "a wide argument must not push the ones after it out of place"
    );
}

#[test]
fn a_string_argument_arrives_as_a_whole_pointer() {
    // Strings are two-byte pointers like enums; a truncated one indexes
    // whatever page the low byte happens to name.
    let src = r#"
        const OUT: addr = 0x0900;
        const MSG: str = "hi";
        #[inline]
        fn first_len(s: str) -> u16 { return s.len; }
        #[reset]
        fn main() { OUT = first_len(MSG).low; loop {} }
    "#;
    assert_eq!(run(src).mem(0x0900), 2);
}

#[test]
fn an_early_return_inside_an_inline_function_wins() {
    // An inline body has no frame to RTS from, so `return` just set A and
    // execution fell through into the rest of the body — the trailing
    // `return 0` overwrote the `return 1`.
    let src = r#"
        const OUT: addr = 0x0900;
        #[inline]
        fn test_bit(value: u8, bit: u8) -> u8 {
            if (value & (1 << bit)) != 0 {
                return 1;
            }
            return 0;
        }
        #[reset]
        fn main() {
            OUT = test_bit(0x10, 4);
            loop {}
        }
    "#;
    assert_eq!(run(src).mem(0x0900), 1);
}

#[test]
fn an_early_return_inside_a_nested_inline_call_wins() {
    // Inline expansions nest; the end labels must too.
    let src = r#"
        const OUT: addr = 0x0900;
        #[inline]
        fn is_set(v: u8) -> u8 {
            if v != 0 { return 1; }
            return 0;
        }
        #[inline]
        fn both(a: u8, b: u8) -> u8 {
            if is_set(a) == 1 {
                if is_set(b) == 1 { return 7; }
            }
            return 3;
        }
        #[reset]
        fn main() {
            OUT = both(5, 0);
            loop {}
        }
    "#;
    assert_eq!(run(src).mem(0x0900), 3);
}
