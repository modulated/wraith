//! Runtime (emulator) tests for indirect calls through computed callees:
//! function pointers held in struct fields (a device vtable) and in variables.
//! This is the shape a generic bus/device protocol takes — a driver is a struct
//! of function pointers, and code calls `dev.read(reg)` without knowing which
//! driver it holds.

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
