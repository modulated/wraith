//! End-to-end tests for static frame allocation, recursion saves, the
//! no-hardware-stack operand spill, math-param relocation, frame-overflow
//! diagnostics, and interrupt-handler zero-page preservation.
//!
//! Frames are statically colored into the zero page, so every bug in this area
//! has the same shape: two things share a byte and one silently overwrites the
//! other. Assembly assertions are poor at catching that — `$0200,X` appearing
//! somewhere says a spill was emitted, not that the value came back intact.
//! So each test here that *can* run its program does, and checks the value the
//! frame machinery was supposed to protect.
//!
//! The assembly assertions that remain are the ones execution genuinely cannot
//! make: that the spill uses the software stack rather than the 6502 hardware
//! stack (both produce correct results — only one survives an interrupt), and
//! that a given number of calls was emitted.

use crate::common::exec::run;
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

/// The same recursion, run: `f(n)` counts back down to 0 and adds 1 on the way
/// out, so `f(5)` is 5 — but only if each level's `n` and `r` survive the
/// nested call. A frame save that is emitted but wrong (saving the wrong span,
/// restoring in the wrong order) still produces the `$0200,X` the test above
/// looks for, and still returns garbage here.
#[test]
fn recursion_in_let_position_returns_the_right_value() {
    let mut e = run(r#"
        const OUT: addr = 0x0400;
        fn f(n: u8) -> u8 {
            if n == 0 { return 0; }
            let r: u8 = f(n - 1);
            return r + 1;
        }
        #[reset]
        fn main() { OUT = f(5); loop {} }
    "#);
    assert_eq!(e.mem(0x0400), 5, "f(5) unwinds five levels, adding 1 each");
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

/// Run of the above: the discarded recursive call must not disturb `n`, which
/// the very next statement returns. Without the frame save `f(3)` returns
/// whatever the deepest call left behind (0) instead of 3.
#[test]
fn recursion_in_statement_position_preserves_the_caller_frame() {
    let mut e = run(r#"
        const OUT: addr = 0x0400;
        fn f(n: u8) -> u8 {
            if n == 0 { return 0; }
            f(n - 1);
            return n;
        }
        #[reset]
        fn main() { OUT = f(3); loop {} }
    "#);
    assert_eq!(e.mem(0x0400), 3, "`n` must survive the discarded call");
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
    // Never the 6502 hardware stack: this is the part execution cannot show,
    // because a PHA/PLA spill computes the same answer. It only diverges under
    // an interrupt, and it eats the 256-byte hardware stack that call return
    // addresses live in.
    assert_asm_not_contains(&asm, "PHA");
}

/// Run of the above: `f(3) + f(5)` is 4 + 6 = 10. The whole point of the spill
/// is that the left operand survives the second call; if it does not, the
/// result is `f(5) + f(5)` = 12, or the second result doubled — all of which
/// emit exactly the assembly the test above accepts.
#[test]
fn u8_call_plus_call_computes_both_operands() {
    let mut e = run(r#"
        fn f(x: u8) -> u8 { return x + 1; }
        const OUT: addr = 0x0400;
        #[reset]
        fn main() {
            let r: u8 = f(3) + f(5);
            OUT = r;
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0400), 10, "f(3) + f(5) = 4 + 6");
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

/// What the relocation is *for*: $80-$83 now lies inside the frame region, so a
/// multiply still using it would scribble on whichever function's locals were
/// colored there. Keeping locals live across the multiply turns that collision
/// into a wrong answer instead of an address nobody checks.
#[test]
fn mul16_does_not_clobber_live_locals() {
    let mut e = run(r#"
        const PROD_LO: addr = 0x0400;
        const PROD_HI: addr = 0x0401;
        const KEEP_LO: addr = 0x0402;
        const KEEP_HI: addr = 0x0403;
        #[reset]
        fn main() {
            let a: u16 = 300;
            let b: u16 = 7;
            let keep: u16 = 0xBEEF;
            let c: u16 = a * b;
            PROD_LO = c.low;  PROD_HI = c.high;
            KEEP_LO = keep.low; KEEP_HI = keep.high;
            loop {}
        }
    "#);
    assert_eq!(e.mem16(0x0400), 2100, "300 * 7");
    assert_eq!(
        e.mem16(0x0402),
        0xBEEF,
        "a local held across the multiply must be untouched by its scratch"
    );
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

/// Mutual recursion also has to be *right*. The two functions sit in a call
/// cycle, so neither can be colored strictly above the other; a mistake here
/// has them sharing `n` and the parity answer inverts or sticks.
#[test]
fn mutual_recursion_computes_parity() {
    let mut e = run(r#"
        const EVEN4: addr = 0x0400;
        const EVEN5: addr = 0x0401;
        fn is_even(n: u8) -> u8 {
            if n == 0 { return 1; }
            return is_odd(n - 1);
        }
        fn is_odd(n: u8) -> u8 {
            if n == 0 { return 0; }
            return is_even(n - 1);
        }
        #[reset]
        fn main() {
            EVEN4 = is_even(4);
            EVEN5 = is_even(5);
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0400), 1, "4 is even");
    assert_eq!(e.mem(0x0401), 0, "5 is not even");
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
    // `RTI` is a real instruction; the save/restore used to be checked by
    // matching the emitted *comments* ("Save zero-page state"), which a
    // comment-wording change breaks and a dropped `STA`/`LDA` pair does not.
    // The behavioral test below is what actually holds the invariant.
    assert_asm_contains(&asm, "RTI");
}

/// What the save/restore is actually for. A handler is not part of main's call
/// graph, so frame coloring gives it the *same* zero-page bytes as main —
/// verified below rather than assumed. Overlap is fine only because the handler
/// saves those bytes on entry and puts them back before `RTI`.
///
/// The test reads main's frame bytes straight out of the emulator's zero page
/// after servicing an interrupt: they must hold exactly what main left there.
/// A dropped save, a save of the wrong span, or a restore in the wrong order
/// all show up here, and none of them changes the emitted comments the previous
/// version of this test was matching on.
#[test]
fn interrupt_handler_restores_the_frame_bytes_it_shares_with_main() {
    const SRC: &str = r#"
        const COUNT: addr = 0x0400;
        #[irq]
        fn on_irq() {
            let t: u8 = COUNT;
            let u: u8 = t + 1;
            COUNT = u;
        }
        #[reset]
        fn main() {
            let a: u8 = 0x5A;
            let b: u8 = 0x3C;
            COUNT = 0;
            asm { "CLI" }
            loop {}
        }
    "#;

    // The overlap this test depends on, stated rather than assumed: if coloring
    // ever separates the two frames, the assertions below stop proving anything
    // and this tells us to rewrite the test rather than silently passing.
    let program = analyze_only(SRC).expect("program should analyze");
    let main_frame = *program.function_frames.get("main").expect("main frame");
    let irq_frame = *program.function_frames.get("on_irq").expect("irq frame");
    let overlaps = irq_frame.base < main_frame.base + main_frame.size
        && main_frame.base < irq_frame.base + irq_frame.size;
    assert!(
        overlaps,
        "expected the handler frame (${:02X}) to share bytes with main's (${:02X}); \
         the save/restore this test exercises would no longer be load-bearing",
        irq_frame.base, main_frame.base
    );

    let mut e = run(SRC);
    let base = main_frame.base as u16;
    assert_eq!(
        (e.mem(base), e.mem(base + 1)),
        (0x5A, 0x3C),
        "main's locals before the interrupt"
    );

    e.pulse_irq();

    assert_eq!(e.mem(0x0400), 1, "the handler ran");
    assert_eq!(
        (e.mem(base), e.mem(base + 1)),
        (0x5A, 0x3C),
        "the handler must restore the frame bytes it borrowed from main"
    );
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
