//! Dead-code elimination in the file being compiled.
//!
//! Unreachable items used to be emitted regardless — a warning was the whole
//! response. Since the liveness walk already spans modules, applying it to the
//! root module too costs nothing and keeps a ROM from carrying code that can
//! never run. The warning and the drop are computed from the same set, so what
//! is reported and what is removed cannot drift apart.
//!
//! The interesting part is not what gets dropped but what must *not*: a
//! function whose only caller is inline assembly, one reached only through a
//! pointer, one pinned to an address for something outside the program.

use crate::common::exec::run;
use crate::common::harness::{CompileResult, compile, compile_success};

fn asm_for(src: &str) -> String {
    compile_success(src)
}

fn warnings_for(src: &str) -> String {
    match compile(src) {
        CompileResult::Success(warnings, _) => warnings,
        other => panic!("expected this to compile, got {other:?}"),
    }
}

/// Labels defined at column 0, i.e. the functions and data actually emitted.
fn defined(asm: &str) -> Vec<String> {
    asm.lines()
        .filter_map(|l| l.strip_suffix(':'))
        .filter(|l| !l.starts_with(char::is_whitespace) && !l.is_empty())
        .map(str::to_string)
        .collect()
}

// ============================================================================
// Dropping
// ============================================================================

#[test]
fn an_unreachable_function_is_dropped_and_warned_about() {
    let src = r#"
        const OUT: addr = 0x0900;
        fn never_called(x: u8) -> u8 { return x + 1; }
        #[reset]
        fn main() { OUT = 7; loop {} }
    "#;
    assert!(!defined(&asm_for(src)).iter().any(|f| f == "never_called"));
    assert!(warnings_for(src).contains("unused function: `never_called`"));
}

#[test]
fn a_chain_of_dead_functions_goes_entirely() {
    // `dead_caller` calls `never_called`, so a check that only asked "is this
    // ever called?" would keep `never_called` and report nothing about it.
    let src = r#"
        const OUT: addr = 0x0900;
        fn never_called(x: u8) -> u8 { return x + 1; }
        fn dead_caller(x: u8) -> u8 { return never_called(x); }
        #[reset]
        fn main() { OUT = 7; loop {} }
    "#;
    let defined = defined(&asm_for(src));
    assert!(!defined.iter().any(|f| f == "never_called"));
    assert!(!defined.iter().any(|f| f == "dead_caller"));

    let warnings = warnings_for(src);
    assert!(warnings.contains("never_called"), "{warnings}");
    assert!(warnings.contains("dead_caller"), "{warnings}");
}

#[test]
fn an_unused_const_array_is_dropped_and_warned_about() {
    let src = r#"
        const OUT: addr = 0x0900;
        const UNUSED_TABLE: [u8; 8] = [1, 2, 3, 4, 5, 6, 7, 8];
        #[reset]
        fn main() { OUT = 7; loop {} }
    "#;
    let asm = asm_for(src);
    assert!(!asm.contains("UNUSED_TABLE"), "the table's bytes are dead");
    assert!(warnings_for(src).contains("unused constant or static: `UNUSED_TABLE`"));
}

#[test]
fn an_unused_static_is_dropped_and_warned_about() {
    let src = r#"
        const OUT: addr = 0x0900;
        static DEAD_COUNTER: u8 = 0;
        #[reset]
        fn main() { OUT = 7; loop {} }
    "#;
    let asm = asm_for(src);
    assert!(!asm.contains("DEAD_COUNTER"));
    assert!(warnings_for(src).contains("unused constant or static: `DEAD_COUNTER`"));
    // Its startup initialization must go with it, or the reset handler writes
    // to an address no label names.
    assert!(!asm.contains("Initialize mutable statics"));
}

// ============================================================================
// What must survive
// ============================================================================

#[test]
fn the_live_program_still_runs() {
    // The point of all of this is that nothing needed was removed.
    let src = r#"
        const OUT: addr = 0x0900;
        const TABLE: [u8; 4] = [10, 20, 30, 40];
        static COUNTER: u8 = 0;
        fn dead(x: u8) -> u8 { return x + 100; }
        fn helper(i: u8) -> u8 { return TABLE[i]; }
        #[reset]
        fn main() {
            COUNTER = helper(2);
            OUT = COUNTER;
            loop {}
        }
    "#;
    assert_eq!(run(src).mem(0x0900), 30);
    assert!(!defined(&asm_for(src)).iter().any(|f| f == "dead"));
}

#[test]
fn interrupt_handlers_are_roots() {
    // Nothing in the program calls an IRQ handler; the hardware enters it
    // through the vector table.
    let src = r#"
        const OUT: addr = 0x0900;
        #[irq]
        fn on_irq() { OUT = 1; }
        #[reset]
        fn main() { OUT = 0; loop {} }
    "#;
    let asm = asm_for(src);
    assert!(defined(&asm).iter().any(|f| f == "on_irq"));
    assert!(!warnings_for(src).contains("on_irq"));
}

#[test]
fn an_org_placed_function_is_a_root() {
    // Fixing an address is how you let something outside the program call in,
    // so an #[org] function is live even with no caller here.
    let src = r#"
        const OUT: addr = 0x0900;
        #[org(0xA000)]
        fn entry_point() -> u8 { return 7; }
        #[reset]
        fn main() { OUT = 0; loop {} }
    "#;
    assert!(defined(&asm_for(src)).iter().any(|f| f == "entry_point"));
    assert!(!warnings_for(src).contains("entry_point"));
}

#[test]
fn a_function_called_only_from_inline_asm_survives() {
    let src = r#"
        const OUT: addr = 0x0900;
        fn helper() -> u8 { return 42; }
        #[reset]
        fn main() {
            asm { "JSR helper", }
            OUT = 1;
            loop {}
        }
    "#;
    assert!(defined(&asm_for(src)).iter().any(|f| f == "helper"));
    assert!(!warnings_for(src).contains("unused function: `helper`"));
}

#[test]
fn a_function_reached_only_through_a_pointer_survives() {
    let src = r#"
        const OUT: addr = 0x0900;
        fn add_one(x: u8) -> u8 { return x + 1; }
        #[reset]
        fn main() {
            let f: fn(u8) -> u8 = add_one;
            OUT = f(41);
            loop {}
        }
    "#;
    assert!(defined(&asm_for(src)).iter().any(|f| f == "add_one"));
    assert_eq!(run(src).mem(0x0900), 42);
}

#[test]
fn a_static_referenced_only_by_a_live_function_survives() {
    let src = r#"
        const OUT: addr = 0x0900;
        static COUNTER: u8 = 5;
        fn bump() -> u8 { COUNTER = COUNTER + 1; return COUNTER; }
        #[reset]
        fn main() { OUT = bump(); loop {} }
    "#;
    assert!(asm_for(src).contains("COUNTER"));
    assert_eq!(run(src).mem(0x0900), 6);
}

// ============================================================================
// A file with no entry point
// ============================================================================

#[test]
fn a_module_with_no_entry_point_keeps_everything() {
    // Nothing is reachable, but that means "we cannot tell what runs", not
    // "everything is dead". Emitting an empty ROM because `#[reset]` has not
    // been written yet would be a poor answer to a half-finished file.
    let src = r#"
        const TABLE: [u8; 2] = [1, 2];
        fn helper(i: u8) -> u8 { return TABLE[i]; }
    "#;
    let asm = asm_for(src);
    assert!(defined(&asm).iter().any(|f| f == "helper"));
    assert!(asm.contains("TABLE"));
    assert_eq!(warnings_for(src), "", "and nothing is reported as unused");
}
