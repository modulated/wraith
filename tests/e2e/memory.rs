//! End-to-end tests for memory and addresses

use crate::common::*;

#[test]
fn address_declaration() {
    let asm = compile_success(r#"
        const SCREEN: addr = 0x0400;
        fn main() {
            SCREEN = 42;
        }
    "#);

    assert_asm_contains(&asm, "SCREEN = $0400");
    assert_asm_contains(&asm, "STA SCREEN");
}

#[test]
fn constant_address_expression() {
    let asm = compile_success(r#"
        const BASE: addr = 0x0400;
        const OFFSET: addr = BASE + 0x0010;
        fn main() {
            OFFSET = 42;
        }
    "#);

    assert_asm_contains(&asm, "OFFSET = $0410");
    assert_asm_contains(&asm, "STA OFFSET");
}

#[test]
fn read_only_addr_cannot_write() {
    assert_sema_error(r#"
        const STATUS: read addr = 0x0400;
        fn main() {
            STATUS = 42;
        }
    "#);
}

#[test]
fn read_only_addr_can_read() {
    let asm = compile_success(r#"
        const STATUS: read addr = 0x6000;
        fn main() {
            let x: u8 = STATUS;
        }
    "#);
    assert_asm_contains(&asm, "STATUS = $6000");
    assert_asm_contains(&asm, "LDA STATUS");
}

#[test]
fn write_only_addr_cannot_read() {
    assert_sema_error(r#"
        const CONTROL: write addr = 0x0400;
        fn main() {
            let x: u8 = CONTROL;
        }
    "#);
}

#[test]
fn write_only_addr_can_write() {
    let asm = compile_success(r#"
        const CONTROL: write addr = 0x6000;
        fn main() {
            CONTROL = 42;
        }
    "#);
    assert_asm_contains(&asm, "CONTROL = $6000");
    assert_asm_contains(&asm, "STA CONTROL");
}

#[test]
fn read_write_addr_default() {
    // Default addr type supports both reading and writing
    let asm = compile_success(r#"
        const PORT: addr = 0x6000;
        fn main() {
            PORT = 42;
            let x: u8 = PORT;
        }
    "#);
    assert_asm_contains(&asm, "PORT = $6000");
    assert_asm_contains(&asm, "STA PORT");
    // Note: codegen may optimize out the LDA if value is still in accumulator
}

#[test]
fn access_modifier_only_for_addr() {
    // Access modifiers should only be valid for addr type
    assert_parse_error(r#"
        const VALUE: read u8 = 42;
        fn main() {}
    "#);
}

#[test]
fn addr_cannot_be_variable() {
    // addr type can only be used in const declarations, not variables
    assert_sema_error(r#"
        fn main() {
            let x: addr = 0x42;
        }
    "#);
}

#[test]
fn addr_cannot_be_parameter() {
    // addr type can only be used in const declarations, not function parameters
    assert_sema_error(r#"
        fn test_func(port: addr) {}
        fn main() {}
    "#);
}

#[test]
fn addr_cannot_be_return_type() {
    // addr type can only be used in const declarations, not return types
    assert_sema_error(r#"
        fn get_port() -> addr {
            return 0x42;
        }
        fn main() {}
    "#);
}

#[test]
fn addr_cannot_be_struct_field() {
    // addr type can only be used in const declarations, not struct fields
    assert_sema_error(r#"
        struct Device {
            addr port,
        }
        fn main() {}
    "#);
}

#[test]
fn addr_cannot_be_enum_tuple_variant() {
    // addr type can only be used in const declarations, not enum tuple variants
    assert_sema_error(r#"
        enum IO {
            Port(addr),
        }
        fn main() {}
    "#);
}

#[test]
fn addr_cannot_be_enum_struct_variant() {
    // addr type can only be used in const declarations, not enum struct variants
    assert_sema_error(r#"
        enum IO {
            Port { addr address },
        }
        fn main() {}
    "#);
}

#[test]
fn addr_size_is_one_byte() {
    // addr should store u8 values (1 byte), not u16
    let asm = compile_success(r#"
        const PORT: addr = 0x6000;
        fn main() {
            PORT = 0xFF;  // Single byte write
        }
    "#);
    assert_asm_contains(&asm, "STA PORT");
    // Should NOT contain STY (which would indicate 2-byte storage)
    assert!(!asm.contains("STY $6001"));
}

