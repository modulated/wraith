//! Runtime (emulator) tests for indirect calls through computed callees:
//! function pointers held in struct fields (a device vtable), in variables, and
//! in arrays indexed by a runtime value (a driver-dispatch table). This is the
//! shape a generic bus/device protocol takes — a driver is a struct or table of
//! function pointers, and code calls `dev.read(reg)` or `handlers[i](x)` without
//! knowing which driver it holds.

use crate::common::exec::run;

#[test]
fn call_through_struct_field() {
    let mut e = run(r#"
        const OUT: addr = 0x0900;
        fn add_one(a: u8) -> u8 { return a + 1; }
        struct Dev { rd: fn(u8) -> u8 }
        #[reset]
        fn main() {
            let d: Dev = Dev { rd: add_one };
            OUT = d.rd(7);
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0900), 8, "d.rd(7) dispatched to add_one");
}

#[test]
fn struct_vtable_selects_driver_at_runtime() {
    // Two "drivers" behind the same struct shape; which one runs is decided at
    // runtime. This is the core of the device protocol.
    let pick = |which: u8| {
        let src = format!(
            r#"
            const OUT: addr = 0x0900;
            fn uart_read(r: u8) -> u8 {{ return r + 10; }}
            fn via_read(r: u8) -> u8 {{ return r + 20; }}
            struct Device {{ read: fn(u8) -> u8 }}
            static DEV: Device = Device {{ read: uart_read }};
            #[reset]
            fn main() {{
                let sel: u8 = {which};
                // Bind the driver at runtime (as device registration would),
                // then dispatch without knowing which driver is installed.
                if sel == 1 {{
                    DEV.read = via_read;
                }}
                OUT = DEV.read(5);
                loop {{}}
            }}
        "#
        );
        run(&src).mem(0x0900)
    };
    assert_eq!(pick(0), 15, "uart_read(5) = 15");
    assert_eq!(pick(1), 25, "via_read(5) = 25");
}

#[test]
fn vtable_with_two_methods() {
    // A device with both a read and a write entry point, as a real driver has.
    let mut e = run(r#"
        const SINK: addr = 0x0901;
        const OUT: addr = 0x0900;
        static LAST: u8 = 0;
        fn dev_write(v: u8) { LAST = v; }
        fn dev_read(r: u8) -> u8 { return r + 3; }
        struct Device {
            write: fn(u8),
            read: fn(u8) -> u8,
        }
        #[reset]
        fn main() {
            let d: Device = Device { write: dev_write, read: dev_read };
            d.write(0x42);
            SINK = LAST;
            OUT = d.read(4);
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0901), 0x42, "d.write stored through the vtable");
    assert_eq!(e.mem(0x0900), 7, "d.read(4) = 7");
}

#[test]
fn vtable_functions_are_not_reported_unused() {
    // A driver entry point installed in a vtable is reached only through the
    // function pointer. It must not be flagged as dead code, or every driver in
    // an OS would warn.
    let result = crate::common::harness::compile(
        r#"
        const OUT: addr = 0x0900;
        struct Device { read: fn(u8) -> u8 }
        fn uart_read(r: u8) -> u8 { return r; }
        fn via_read(r: u8) -> u8 { return r; }
        static DEV: Device = Device { read: uart_read };
        #[reset]
        fn main() {
            DEV.read = via_read;
            OUT = DEV.read(1);
            loop {}
        }
    "#,
    );
    let warnings = match result {
        crate::common::harness::CompileResult::Success(w, _) => w,
        other => panic!("expected success, got {:?}", other),
    };
    assert!(
        !warnings.contains("unused function"),
        "vtable-installed functions must not warn as unused, got:\n{}",
        warnings
    );
}

#[test]
fn function_pointer_variable_still_direct() {
    // The pre-existing function-pointer-variable path must keep working.
    let mut e = run(r#"
        const OUT: addr = 0x0900;
        fn triple(a: u8) -> u8 { return a + a + a; }
        #[reset]
        fn main() {
            let p: fn(u8) -> u8 = triple;
            OUT = p(9);
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0900), 27, "p(9) = 27");
}

#[test]
fn a_u16_left_operand_survives_an_indirect_call() {
    // contains_call missed CallIndirect, so `(x + x)` was parked in the $F0
    // pool across `ops.run(...)` — whose callee's own u16 arithmetic writes
    // the same pool — and the "restored" left operand was the callee's
    // product. (100 + 100) + run(3) = 200 + 12 = 212.
    let mut e = run(r#"
        const LO: addr = 0x0900;
        const HI: addr = 0x0901;
        struct Ops { run: fn(u16) -> u16 }
        fn big(v: u16) -> u16 { return v * 3 + v; }
        static ops: Ops = Ops { run: big };
        #[reset]
        fn main() {
            let x: u16 = 100;
            let p: u16 = (x + x) + ops.run(3 as u16);
            LO = p.low;
            HI = p.high;
            loop {}
        }
    "#);
    assert_eq!(e.mem16(0x0900), 212);
}

// ---------------------------------------------------------------------------
// Function-pointer *tables*: an array of drivers dispatched by a runtime index
// (a syscall / device-by-number table). Distinct from the struct-field vtable
// above — the callee is `table[i]`, so the indexed load must scale by the
// element size and load both bytes of the pointer.
// ---------------------------------------------------------------------------

#[test]
fn static_function_pointer_table_dispatches_by_index() {
    let pick = |i: u8| {
        let src = format!(
            r#"
            const OUT: addr = 0x0900;
            fn d0(a: u8) -> u8 {{ return a + 1; }}
            fn d1(a: u8) -> u8 {{ return a + 2; }}
            fn d2(a: u8) -> u8 {{ return a + 3; }}
            static TABLE: [fn(u8) -> u8; 3] = [d0, d1, d2];
            #[reset]
            fn main() {{ let i: u8 = {i}; OUT = TABLE[i](10); loop {{}} }}
        "#
        );
        run(&src).mem(0x0900)
    };
    // Before the fix each of these returned 0 (single-byte load, index left in
    // Y as the pretend high byte).
    assert_eq!((pick(0), pick(1), pick(2)), (11, 12, 13));
}

#[test]
fn const_function_pointer_table_dispatches_by_index() {
    // A const table lives in ROM; the same indexed dispatch must work.
    let pick = |i: u8| {
        let src = format!(
            r#"
            const OUT: addr = 0x0900;
            fn d0(a: u8) -> u8 {{ return a + 100; }}
            fn d1(a: u8) -> u8 {{ return a + 200; }}
            const TABLE: [fn(u8) -> u8; 2] = [d0, d1];
            #[reset]
            fn main() {{ let i: u8 = {i}; OUT = TABLE[i](1); loop {{}} }}
        "#
        );
        run(&src).mem(0x0900)
    };
    assert_eq!((pick(0), pick(1)), (101, 201));
}

#[test]
fn function_pointer_table_installed_at_runtime() {
    // Device registration: a table entry is rebound at runtime, then dispatched
    // through without knowing which driver is installed.
    let mut e = run(r#"
        const OUT: addr = 0x0900;
        fn uart_read(r: u8) -> u8 { return r + 10; }
        fn via_read(r: u8) -> u8 { return r + 20; }
        static TABLE: [fn(u8) -> u8; 2] = [uart_read, uart_read];
        #[reset]
        fn main() {
            TABLE[1] = via_read;      // register a driver into slot 1
            let i: u8 = 1;
            OUT = TABLE[i](5);
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0900), 25, "slot 1 now dispatches to via_read");
}

#[test]
fn local_function_pointer_table_dispatches_by_index() {
    // A dispatch table built in a local (RAM frame), not a static.
    let pick = |i: u8| {
        let src = format!(
            r#"
            const OUT: addr = 0x0900;
            fn d0(a: u8) -> u8 {{ return a + 1; }}
            fn d1(a: u8) -> u8 {{ return a + 2; }}
            fn d2(a: u8) -> u8 {{ return a + 3; }}
            #[reset]
            fn main() {{
                let table: [fn(u8) -> u8; 3] = [d0, d1, d2];
                let i: u8 = {i};
                OUT = table[i](10);
                loop {{}}
            }}
        "#
        );
        run(&src).mem(0x0900)
    };
    assert_eq!((pick(0), pick(1), pick(2)), (11, 12, 13));
}

// ---------------------------------------------------------------------------
// Frame colouring across an indirect call.
//
// Frames are coloured from the call graph, and an indirect call contributes no
// edge to it — there is no callee name to record. Colouring read that silence
// as permission to overlay the caller's frame with the driver's, so a local
// held across `dev.write(c)` came back as the driver's parameter. Found by
// `examples/device_drivers.wr`, which prints a byte it sampled earlier and
// printed the last character of the preceding string instead.
// ---------------------------------------------------------------------------

#[test]
fn a_local_survives_an_indirect_call() {
    let mut e = run(r#"
        const OUT: addr = 0x0900;
        const S: str = "abc";
        static SINK: u8 = 0;
        struct D { write: fn(u8) }
        fn emit(c: u8) { SINK = c; }
        static DEV: D = D { write: emit };
        fn print(s: str) { for c in s { DEV.write(c as u8); } }
        #[reset]
        fn main() {
            let sample: u8 = 0x5A;
            print(S);
            OUT = sample;
            loop {}
        }
    "#);
    // Before the fix this was 0x63 — 'c', the last byte `emit` was handed.
    assert_eq!(
        e.mem(0x0900),
        0x5A,
        "the driver must not land on main's frame"
    );
}

#[test]
fn an_indirect_call_in_an_argument_list_leaves_the_other_arguments_alone() {
    // The staging hazard of `tests/e2e/nested_calls.rs`, with the nested callee
    // reached through a pointer: `keep`'s parameters are half written when the
    // driver runs, and which driver it is is unknown at compile time. This one
    // already worked — the inline shelter parks the written parameters on the
    // software stack across any nested call, indirect included — so it is a pin
    // on a shape the colouring fix above deliberately does *not* cover.
    let mut e = run(r#"
        const OUT: addr = 0x0900;
        static SINK: u8 = 0;
        struct D { read: fn() -> u8 }
        fn sample() -> u8 { let t: u8 = SINK; let u: u8 = t + 1; return u + t; }
        static DEV: D = D { read: sample };
        fn keep(a: u8, b: u8) -> u8 { return a + b - b; }
        #[reset]
        fn main() {
            OUT = keep(200, DEV.read());
            loop {}
        }
    "#);
    assert_eq!(
        e.mem(0x0900),
        200,
        "the first argument must survive the driver"
    );
}

#[test]
fn a_function_pointer_passes_as_an_argument() {
    // A function pointer is two bytes with the high byte in Y, and it is the
    // one wide kind that is neither a number nor reached by address — so a
    // call site that lists the wide types by hand tends to leave it off.
    // Two sites had:
    //
    //   * A direct call and an inlined call both reserved *one* staging byte
    //     for a `fn(u8) -> u8` parameter and stored only the low half. Every
    //     parameter after it then landed a byte early, so `apply(bump, 7)`
    //     called through `(low(bump), 7)` and passed 7 as nothing.
    //   * Relaying a function-pointer *variable* emitted `LDA #<g` — the name
    //     assembled as a label rather than read from the variable's slot. A
    //     parameter has no label at all, so the assembler rejected it; a
    //     local that shadowed a real function name would have silently
    //     called the wrong one.
    //
    // Both halves are exercised here: `bump` by name and `g` by variable,
    // through a callee big enough not to be inlined and one that is.
    let mut e = run(r#"
        const OUT0: addr = 0x0900;
        const OUT1: addr = 0x0901;
        const OUT2: addr = 0x0902;
        fn bump(a: u8) -> u8 { return a + 1; }
        fn apply(f: fn(u8) -> u8, v: u8) -> u8 {
            let acc: u8 = 0;
            let i: u8 = 0;
            while i < 3 { acc = acc + f(v); i = i + 1; }
            return acc;
        }
        #[inline]
        fn once(f: fn(u8) -> u8, v: u8) -> u8 { return f(v); }
        #[reset]
        fn main() {
            let g: fn(u8) -> u8 = bump;
            OUT0 = apply(bump, 7);
            OUT1 = apply(g, 7);
            OUT2 = once(g, 40);
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0900), 24, "by name, through a direct call");
    assert_eq!(e.mem(0x0901), 24, "by variable, through a direct call");
    assert_eq!(e.mem(0x0902), 41, "by variable, through an inlined call");
}
