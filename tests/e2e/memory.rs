//! End-to-end tests for memory and addresses

use crate::common::*;

#[test]
fn address_declaration() {
    let asm = compile_success(
        r#"
        const SCREEN: addr = 0x0400;
        fn main() {
            SCREEN = 42;
        }
    "#,
    );

    assert_asm_contains(&asm, "SCREEN = $0400");
    assert_asm_contains(&asm, "STA SCREEN");
}

#[test]
fn constant_address_expression() {
    let asm = compile_success(
        r#"
        const BASE: addr = 0x0400;
        const OFFSET: addr = BASE + 0x0010;
        fn main() {
            OFFSET = 42;
        }
    "#,
    );

    assert_asm_contains(&asm, "OFFSET = $0410");
    assert_asm_contains(&asm, "STA OFFSET");
}

#[test]
fn read_only_addr_cannot_write() {
    assert_sema_error(
        r#"
        const STATUS: read addr = 0x0400;
        fn main() {
            STATUS = 42;
        }
    "#,
    );
}

#[test]
fn read_only_addr_can_read() {
    let asm = compile_success(
        r#"
        const STATUS: read addr = 0x6000;
        fn main() {
            let x: u8 = STATUS;
        }
    "#,
    );
    assert_asm_contains(&asm, "STATUS = $6000");
    assert_asm_contains(&asm, "LDA STATUS");
}

#[test]
fn write_only_addr_cannot_read() {
    assert_sema_error(
        r#"
        const CONTROL: write addr = 0x0400;
        fn main() {
            let x: u8 = CONTROL;
        }
    "#,
    );
}

#[test]
fn write_only_addr_can_write() {
    let asm = compile_success(
        r#"
        const CONTROL: write addr = 0x6000;
        fn main() {
            CONTROL = 42;
        }
    "#,
    );
    assert_asm_contains(&asm, "CONTROL = $6000");
    assert_asm_contains(&asm, "STA CONTROL");
}

#[test]
fn read_write_addr_default() {
    // Default addr type supports both reading and writing
    let asm = compile_success(
        r#"
        const PORT: addr = 0x6000;
        fn main() {
            PORT = 42;
            let x: u8 = PORT;
        }
    "#,
    );
    assert_asm_contains(&asm, "PORT = $6000");
    assert_asm_contains(&asm, "STA PORT");
    // Note: codegen may optimize out the LDA if value is still in accumulator
}

#[test]
fn access_modifier_only_for_addr() {
    // Access modifiers should only be valid for addr type
    assert_parse_error(
        r#"
        const VALUE: read u8 = 42;
        fn main() {}
    "#,
    );
}

#[test]
fn addr_cannot_be_variable() {
    // addr type can only be used in const declarations, not variables
    assert_sema_error(
        r#"
        fn main() {
            let x: addr = 0x42;
        }
    "#,
    );
}

#[test]
fn addr_cannot_be_parameter() {
    // addr type can only be used in const declarations, not function parameters
    assert_sema_error(
        r#"
        fn test_func(port: addr) {}
        fn main() {}
    "#,
    );
}

#[test]
fn addr_cannot_be_return_type() {
    // addr type can only be used in const declarations, not return types
    assert_sema_error(
        r#"
        fn get_port() -> addr {
            return 0x42;
        }
        fn main() {}
    "#,
    );
}

#[test]
fn addr_cannot_be_struct_field() {
    // addr type can only be used in const declarations, not struct fields
    assert_sema_error(
        r#"
        struct Device {
            port: addr,
        }
        fn main() {}
    "#,
    );
}

#[test]
fn addr_cannot_be_enum_tuple_variant() {
    // addr type can only be used in const declarations, not enum tuple variants
    assert_sema_error(
        r#"
        enum IO {
            Port(addr),
        }
        fn main() {}
    "#,
    );
}

#[test]
fn addr_cannot_be_enum_struct_variant() {
    // addr type can only be used in const declarations, not enum struct variants
    assert_sema_error(
        r#"
        enum IO {
            Port { address: addr },
        }
        fn main() {}
    "#,
    );
}

#[test]
fn addr_size_is_one_byte() {
    // addr should store u8 values (1 byte), not u16
    let asm = compile_success(
        r#"
        const PORT: addr = 0x6000;
        fn main() {
            PORT = 0xFF;  // Single byte write
        }
    "#,
    );
    assert_asm_contains(&asm, "STA PORT");
    // Should NOT contain STY (which would indicate 2-byte storage)
    assert!(!asm.contains("STY $6001"));
}

// ============================================================================
// Executed addr behaviour
//
// The tests above check the emitted form (`SCREEN = $0400`, `STA SCREEN`) and
// the access-modifier rules, which are compile-time properties. None of them
// shows a write actually reaching the address it names, or a read returning
// what is there. These do.
// ============================================================================

use crate::common::exec::run;

#[test]
fn addr_write_lands_at_the_named_address() {
    let mut e = run(r#"
        const PORT: addr = 0x0905;
        #[reset]
        fn main() {
            PORT = 0x5A;
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0905), 0x5A);
}

#[test]
fn addr_read_returns_memory_contents() {
    // Write through one addr and read it back through another naming the same
    // location: proves the read is a real load from that address.
    let mut e = run(r#"
        const SRC: addr = 0x0905;
        const OUT: addr = 0x0900;
        #[reset]
        fn main() {
            SRC = 0x33;
            let v: u8 = SRC;
            OUT = v + 1;
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0900), 0x34);
}

#[test]
fn addr_write_is_one_byte_only() {
    // An addr names a single byte; writing must not disturb the next location,
    // which would corrupt the neighbouring register of a real device.
    let mut e = run(r#"
        const PORT: addr = 0x0905;
        const NEXT: addr = 0x0906;
        #[reset]
        fn main() {
            NEXT = 0x11;
            PORT = 0xFF;
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0905), 0xFF);
    assert_eq!(e.mem(0x0906), 0x11, "the following byte must be untouched");
}

#[test]
fn constant_address_expression_resolves() {
    // A computed base + offset must name the address the arithmetic gives.
    let mut e = run(r#"
        const BASE: u16 = 0x0900;
        const SLOT: addr = BASE + 6;
        #[reset]
        fn main() {
            SLOT = 0x77;
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0906), 0x77, "BASE + 6 resolves to $0906");
}

#[test]
fn addr_read_modify_write() {
    // The counter idiom an interrupt handler uses.
    let mut e = run(r#"
        const CTR: addr = 0x0905;
        #[reset]
        fn main() {
            CTR = 10;
            CTR = CTR + 5;
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0905), 15);
}

#[test]
#[should_panic(expected = "store into ROM")]
fn a_wild_store_into_rom_fails_loudly_on_the_emulator() {
    // TestBus used to be 64 KB of writable RAM: this store succeeded silently,
    // exactly as it would NOT on real hardware. Sema can't see through a
    // pointer, so the emulator itself refuses the write.
    run(r#"
        #[reset]
        fn main() {
            let p: &u8 = 0xD000 as &u8;
            *p = 42;
            loop {}
        }
    "#);
}

// ---------------------------------------------------------------------------
// Runtime index range: the 8-bit index register caps runtime-indexable arrays
// ---------------------------------------------------------------------------

fn index_error(src: &str) -> String {
    match compile(src) {
        CompileResult::SemaError(e) | CompileResult::CodegenError(e) => e,
        other => panic!("expected a compile error, got {other:?}"),
    }
}

#[test]
fn a_runtime_index_into_a_large_two_byte_array_is_rejected() {
    // [u16; 200] scaled by a runtime index overflows the 8-bit index register
    // (offset 2*i wraps past 255); this used to read the wrong element silently.
    let e = index_error(
        r#"
        const LO: addr = 0x0900;
        static T: [u16; 200] = [0 as u16; 200];
        #[reset]
        fn main() { let i: u8 = 150; let r: u16 = T[i]; LO = r.low; loop {} }
        "#,
    );
    assert!(e.contains("too large to index"), "{e}");
}

#[test]
fn a_runtime_indexed_store_into_a_large_two_byte_array_is_rejected() {
    let e = index_error(
        r#"
        static T: [u16; 200] = [0 as u16; 200];
        #[reset]
        fn main() { let i: u8 = 100; T[i] = 5 as u16; loop {} }
        "#,
    );
    assert!(e.contains("too large to index"), "{e}");
}

#[test]
fn a_runtime_index_into_a_128_element_two_byte_array_is_allowed() {
    // 128 elements -> max offset 2*127 = 254, still within the 8-bit index.
    let _asm = compile_success(
        r#"
        const LO: addr = 0x0900;
        static T: [u16; 128] = [0 as u16; 128];
        #[reset]
        fn main() { let i: u8 = 100; let r: u16 = T[i]; LO = r.low; loop {} }
        "#,
    );
}
