//! End-to-end tests for static frame allocation, recursion saves, the
//! no-hardware-stack operand spill, math-param relocation, frame-overflow
//! diagnostics, and interrupt-handler zero-page preservation.

use crate::common::*;

/// Non-tail recursion in a `let` initializer must still save/restore the callee
/// frame via the software stack ($0200 / $FF), regardless of expression position.
#[test]
fn recursion_in_let_position_saves_frame() {
    let asm = compile_success(
        r#"
        fn f(n: u8) -> u8 {
            if n == 0 { return 0; }
            let r: u8 = f(n - 1);
            return r + 1;
        }
        fn main() { let x: u8 = f(5); }
        "#,
    );
    // Software-stack frame save (not the hardware stack).
    assert_asm_contains(&asm, "$0200,X");
    assert_asm_contains(&asm, "$FF");
}

/// Non-tail recursion as a bare statement is also covered (the old heuristic only
/// fired when the call was a binary-op left operand).
#[test]
fn recursion_in_statement_position_saves_frame() {
    let asm = compile_success(
        r#"
        fn f(n: u8) -> u8 {
            if n == 0 { return 0; }
            f(n - 1);
            return n;
        }
        fn main() { let x: u8 = f(3); }
        "#,
    );
    assert_asm_contains(&asm, "$0200,X");
}

/// `f(a) + f(b)` for u8: the left operand is spilled to the software stack across
/// the second call — never the 6502 hardware stack (no PHA/PLA around the spill).
#[test]
fn u8_call_plus_call_spills_without_hardware_stack() {
    let asm = compile_success(
        r#"
        fn f(x: u8) -> u8 { return x + 1; }
        const OUT: addr = 0x0400;
        fn main() {
            let r: u8 = f(3) + f(5);
            OUT = r;
        }
        "#,
    );
    // Left operand spilled to the software stack.
    assert_asm_contains(&asm, "$0200,X");
    // Two calls to f are emitted.
    assert_asm_count(&asm, "JSR f", 2);
}

/// 16-bit multiply passes its parameters in the relocated $D9-$DC region, never
/// the old $80-$83 (which is now inside the frame region).
#[test]
fn mul16_uses_relocated_params() {
    let asm = compile_success(
        r#"
        const OUT: addr = 0x0400;
        fn main() {
            let a: u16 = 300;
            let b: u16 = 7;
            let c: u16 = a * b;
            OUT = c as u8;
        }
        "#,
    );
    // Parameters land in the relocated $D9-$DC region.
    assert_asm_contains(&asm, "$D9");
    assert_asm_contains(&asm, "$DC");
}

/// A single frame larger than the 144-byte frame region is a clear compile error.
#[test]
fn oversized_frame_reports_overflow() {
    assert_error_contains(
        r#"
        struct Big { data: [u8; 200] }
        fn main() {
            let b: Big = { data: [0; 200] };
            b.data[0] = 1;
        }
        "#,
        "frame region overflow",
    );
}

/// Overflow is a property of the whole call chain, not just one function: many
/// modestly-sized functions whose frames must all be live at once (a deep call
/// chain) collectively exceed the 144-byte frame region and are rejected at
/// compile time, rather than silently overwriting each other's zero page. The
/// diagnostic names the function whose frame ran off the end of the region.
#[test]
fn deep_call_chain_overflows_frame_region() {
    // 12 functions, each with 16 bytes of locals (8 x u16), chained f0 -> f1 ->
    // ... -> f11. 12 * 16 = 192 bytes > 144, and because each callee's frame is
    // colored above its caller's, all 12 frames are simultaneously live.
    let mut src = String::new();
    for i in 0..12 {
        let decls = (0..8)
            .map(|j| format!("let v{j}: u16 = {j} as u16;"))
            .collect::<Vec<_>>()
            .join(" ");
        // Keep every local used, and call the next function in the chain.
        let uses = (0..8)
            .map(|j| format!("v{j} = v{j} + (1 as u16);"))
            .collect::<Vec<_>>()
            .join(" ");
        let call = if i < 11 {
            format!("f{}();", i + 1)
        } else {
            String::new()
        };
        src.push_str(&format!("fn f{i}() {{ {decls} {call} {uses} }}\n"));
    }
    src.push_str("#[reset]\nfn main() { f0(); loop {} }\n");

    assert_error_contains(&src, "frame region overflow");
}

/// The overflow diagnostic reports the offending call chain / function, so the
/// programmer can see where the budget was blown rather than just that it was.
#[test]
fn frame_overflow_diagnostic_names_offender() {
    assert_error_contains(
        r#"
        struct Huge { data: [u8; 250] }
        fn hog() {
            let h: Huge = { data: [0; 250] };
            h.data[0] = 1;
        }
        #[reset]
        fn main() { hog(); loop {} }
        "#,
        "hog",
    );
}

/// Mutual (non-tail) recursion compiles; both internal edges save frames.
#[test]
fn mutual_recursion_compiles() {
    let asm = compile_success(
        r#"
        fn is_even(n: u8) -> u8 {
            if n == 0 { return 1; }
            return is_odd(n - 1);
        }
        fn is_odd(n: u8) -> u8 {
            if n == 0 { return 0; }
            return is_even(n - 1);
        }
        fn main() { let x: u8 = is_even(4); }
        "#,
    );
    assert_asm_contains(&asm, "JSR is_odd");
    assert_asm_contains(&asm, "JSR is_even");
}

/// An interrupt handler saves zero-page scratch it may clobber, wrapped inside
/// the register save/restore, and returns with RTI.
#[test]
fn interrupt_handler_preserves_zero_page() {
    let asm = compile_success(
        r#"
        const COUNT: addr = 0x0400;
        #[irq]
        fn on_irq() {
            let t: u8 = COUNT;
            COUNT = t + 1;
        }
        #[reset]
        fn main() { loop {} }
        "#,
    );
    assert_asm_contains(&asm, "Save zero-page state");
    assert_asm_contains(&asm, "Restore zero-page state");
    assert_asm_contains(&asm, "RTI");
}

/// A function invoked only via a hand-written `JSR` in an inline-asm block still
/// gets a call-graph edge, so its frame is colored above the asm caller's frame
/// (without the edge it would be a root at $40 and overlap the caller).
#[test]
fn asm_jsr_creates_frame_edge() {
    let program = analyze_only(
        r#"
        fn helper() -> u8 { let h: u8 = 9; return h; }
        fn caller() {
            let c: u8 = 1;
            asm { "JSR helper" }
        }
        fn main() { caller(); }
        "#,
    )
    .expect("program should analyze");

    let caller = *program.function_frames.get("caller").expect("caller frame");
    let helper = *program.function_frames.get("helper").expect("helper frame");

    assert!(
        helper.base >= caller.base + caller.size,
        "asm JSR edge should place helper (base ${:02X}) above caller (base ${:02X}, size {})",
        helper.base,
        caller.base,
        caller.size
    );
}

/// Recursion inside an interrupt handler's call graph is rejected (the frame
/// save/restore stack is not reentrant under preemption).
#[test]
fn recursion_in_interrupt_is_rejected() {
    assert_error_contains(
        r#"
        fn rec(n: u8) -> u8 {
            if n == 0 { return 0; }
            let r: u8 = rec(n - 1);
            return r + 1;
        }
        #[irq]
        fn on_irq() { let x: u8 = rec(3); }
        #[reset]
        fn main() { loop {} }
        "#,
        "interrupt handler",
    );
}

/// Return the formatted warnings from a successful compile (panics otherwise).
fn warnings_of(source: &str) -> String {
    match compile(source) {
        CompileResult::Success(warnings, _) => warnings,
        other => panic!("expected successful compilation, got {:?}", other),
    }
}

/// A non-tail recursive function with a large frame is flagged: each recursive
/// call saves the whole frame to the 256-byte software stack, so the safe
/// recursion depth is shallow and deep recursion would silently overflow it.
#[test]
fn large_frame_recursion_warns() {
    let warnings = warnings_of(
        r#"
        fn walk(n: u8) -> u16 {
            let a: u16 = 1 as u16; let b: u16 = 2 as u16; let c: u16 = 3 as u16;
            let d: u16 = 4 as u16; let e: u16 = 5 as u16; let f: u16 = 6 as u16;
            let g: u16 = 7 as u16; let h: u16 = 8 as u16; let i: u16 = 9 as u16;
            if n == 0 { return a + b + c + d + e + f + g + h + i; }
            return (n as u16) + walk(n - 1);
        }
        #[reset]
        fn main() { let x: u16 = walk(50); loop {} }
        "#,
    );
    assert!(
        warnings.contains("recursive function `walk`") && warnings.contains("software stack"),
        "expected a deep-recursion warning for the large-frame recursive function, got:\n{}",
        warnings
    );
}

/// A tail-recursive function is optimized into a loop and never saves a frame,
/// so it must NOT be flagged even with a large frame.
#[test]
fn tail_recursion_not_flagged_even_with_large_frame() {
    let warnings = warnings_of(
        r#"
        fn walk(n: u8, acc: u16) -> u16 {
            let a: u16 = 1 as u16; let b: u16 = 2 as u16; let c: u16 = 3 as u16;
            let d: u16 = 4 as u16; let e: u16 = 5 as u16; let f: u16 = 6 as u16;
            let g: u16 = 7 as u16; let h: u16 = 8 as u16; let i: u16 = 9 as u16;
            if n == 0 { return acc + a + b + c + d + e + f + g + h + i; }
            return walk(n - 1, acc + (n as u16));
        }
        #[reset]
        fn main() { let x: u16 = walk(50, 0); loop {} }
        "#,
    );
    assert!(
        !warnings.contains("software stack"),
        "tail-recursive function should not get a deep-recursion warning, got:\n{}",
        warnings
    );
}

/// A small-frame recursive function is bounded by the ~128-level hardware-stack
/// limit shared by all non-tail recursion, not by frame size, so it is not
/// flagged by the (frame-size-specific) deep-recursion warning.
#[test]
fn small_frame_recursion_not_flagged() {
    let warnings = warnings_of(
        r#"
        fn deep_sum(n: u8) -> u16 {
            if n == 0 { return 0; }
            return (n as u16) + deep_sum(n - 1);
        }
        #[reset]
        fn main() { let x: u16 = deep_sum(200); loop {} }
        "#,
    );
    assert!(
        !warnings.contains("software stack"),
        "small-frame recursion should not get a deep-recursion warning, got:\n{}",
        warnings
    );
}
