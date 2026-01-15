//! Module visibility error tests
//!
//! Tests for `pub` keyword and import visibility checking

use crate::common::*;
use std::env;
use std::fs;

// Helper to write a temp file and return its formatted string path (for imports)
fn write_temp_file(filename: &str, content: &str) -> String {
    let temp_dir = env::temp_dir();
    let file_path = temp_dir.join(filename);
    fs::write(&file_path, content).unwrap();
    // Return path with forward slashes for Wraith import compatibility
    file_path.to_string_lossy().replace("\\", "/")
}

// ============================================================================
// Private Item Import Errors
// ============================================================================

#[test]
fn cannot_import_private_function() {
    // Create library file with private function
    let lib_path = write_temp_file(
        "test_vis_lib1.wr",
        r#"
        fn private_helper() -> u8 {
            return 42;
        }

        #[reset]
        fn main() {}
    "#,
    );

    assert_error_contains(
        &format!(
            r#"
        import {{ private_helper }} from "{}";

        fn main() {{
            let x: u8 = private_helper();
        }}
        "#,
            lib_path
        ),
        "private",
    );
}

#[test]
fn cannot_import_private_const() {
    let lib_path = write_temp_file(
        "test_vis_lib2.wr",
        r#"
        const PRIVATE_VALUE: u8 = 100;

        #[reset]
        fn main() {}
    "#,
    );

    assert_error_contains(
        &format!(
            r#"
        import {{ PRIVATE_VALUE }} from "{}";

        fn main() {{
            let x: u8 = PRIVATE_VALUE;
        }}
        "#,
            lib_path
        ),
        "private",
    );
}

#[test]
fn cannot_import_private_struct() {
    let lib_path = write_temp_file(
        "test_vis_lib3.wr",
        r#"
        struct PrivatePoint {
            x: u8,
            y: u8,
        }

        #[reset]
        fn main() {}
    "#,
    );

    assert_error_contains(
        &format!(
            r#"
        import {{ PrivatePoint }} from "{}";

        fn main() {{
            let p: PrivatePoint = PrivatePoint {{ x: 1, y: 2 }};
        }}
        "#,
            lib_path
        ),
        "private",
    );
}

#[test]
fn cannot_import_private_enum() {
    let lib_path = write_temp_file(
        "test_vis_lib4.wr",
        r#"
        enum PrivateColor {
            Red,
            Blue,
        }

        #[reset]
        fn main() {}
    "#,
    );

    assert_error_contains(
        &format!(
            r#"
        import {{ PrivateColor }} from "{}";

        fn main() {{
            let c: PrivateColor = PrivateColor::Red;
        }}
        "#,
            lib_path
        ),
        "private",
    );
}

#[test]
fn cannot_import_private_addr() {
    let lib_path = write_temp_file(
        "test_vis_lib5.wr",
        r#"
        const PRIVATE_IO: addr = 0x6000;

        #[reset]
        fn main() {}
    "#,
    );

    assert_error_contains(
        &format!(
            r#"
        import {{ PRIVATE_IO }} from "{}";

        fn main() {{
            PRIVATE_IO = 42;
        }}
        "#,
            lib_path
        ),
        "private",
    );
}

// ============================================================================
// Public Item Import Success
// ============================================================================

#[test]
fn can_import_public_function() {
    let lib_path = write_temp_file(
        "test_vis_pub1.wr",
        r#"
        pub fn helper() -> u8 {
            return 42;
        }

        #[reset]
        fn main() {}
    "#,
    );

    let _asm = compile_success(&format!(
        r#"
        import {{ helper }} from "{}";

        fn main() {{
            let x: u8 = helper();
        }}
    "#,
        lib_path
    ));
}

#[test]
fn can_import_public_const() {
    let lib_path = write_temp_file(
        "test_vis_pub2.wr",
        r#"
        pub const PUBLIC_VALUE: u8 = 100;

        #[reset]
        fn main() {}
    "#,
    );

    let _asm = compile_success(&format!(
        r#"
        import {{ PUBLIC_VALUE }} from "{}";

        fn main() {{
            let x: u8 = PUBLIC_VALUE;
        }}
    "#,
        lib_path
    ));
}

#[test]
fn can_import_public_struct() {
    let lib_path = write_temp_file(
        "test_vis_pub3.wr",
        r#"
        pub struct PublicPoint {
            x: u8,
            y: u8,
        }

        #[reset]
        fn main() {}
    "#,
    );

    let _asm = compile_success(&format!(
        r#"
        import {{ PublicPoint }} from "{}";

        fn main() {{
            let p: PublicPoint = PublicPoint {{ x: 1, y: 2 }};
        }}
    "#,
        lib_path
    ));
}

#[test]
fn can_import_public_enum() {
    let lib_path = write_temp_file(
        "test_vis_pub4.wr",
        r#"
        pub enum PublicColor {
            Red,
            Blue,
        }

        #[reset]
        fn main() {}
    "#,
    );

    let _asm = compile_success(&format!(
        r#"
        import {{ PublicColor }} from "{}";

        fn main() {{
            let c: PublicColor = PublicColor::Red;
        }}
    "#,
        lib_path
    ));
}

#[test]
fn can_import_public_addr() {
    let lib_path = write_temp_file(
        "test_vis_pub5.wr",
        r#"
        pub const PUBLIC_IO: addr = 0x6000;

        #[reset]
        fn main() {}
    "#,
    );

    let _asm = compile_success(&format!(
        r#"
        import {{ PUBLIC_IO }} from "{}";

        fn main() {{
            PUBLIC_IO = 42;
        }}
    "#,
        lib_path
    ));
}

// ============================================================================
// Mixed Visibility
// ============================================================================

#[test]
fn can_import_public_from_mixed_module() {
    let lib_path = write_temp_file(
        "test_vis_mixed.wr",
        r#"
        pub fn public_func() -> u8 {
            return private_func();
        }

        fn private_func() -> u8 {
            return 42;
        }

        #[reset]
        fn main() {}
    "#,
    );

    let _asm = compile_success(&format!(
        r#"
        import {{ public_func }} from "{}";

        fn main() {{
            let x: u8 = public_func();
        }}
    "#,
        lib_path
    ));
}

#[test]
fn cannot_import_private_from_mixed_module() {
    let lib_path = write_temp_file(
        "test_vis_mixed2.wr",
        r#"
        pub fn public_func() -> u8 {
            return 10;
        }

        fn private_func() -> u8 {
            return 20;
        }

        #[reset]
        fn main() {}
    "#,
    );

    assert_error_contains(
        &format!(
            r#"
        import {{ private_func }} from "{}";

        fn main() {{
            let x: u8 = private_func();
        }}
        "#,
            lib_path
        ),
        "private",
    );
}

#[test]
fn import_multiple_public_items() {
    let lib_path = write_temp_file(
        "test_vis_multi.wr",
        r#"
        pub fn func1() -> u8 { return 1; }
        pub fn func2() -> u8 { return 2; }
        pub const VALUE: u8 = 42;

        #[reset]
        fn main() {}
    "#,
    );

    let _asm = compile_success(&format!(
        r#"
        import {{ func1, func2, VALUE }} from "{}";

        fn main() {{
            let a: u8 = func1();
            let b: u8 = func2();
            let c: u8 = VALUE;
        }}
    "#,
        lib_path
    ));
}

#[test]
fn error_when_one_import_is_private() {
    let lib_path = write_temp_file(
        "test_vis_partial.wr",
        r#"
        pub fn public_item() -> u8 { return 1; }
        fn private_item() -> u8 { return 2; }

        #[reset]
        fn main() {}
    "#,
    );

    assert_error_contains(
        &format!(
            r#"
        import {{ public_item, private_item }} from "{}";

        fn main() {{
            let x: u8 = public_item();
        }}
        "#,
            lib_path
        ),
        "private",
    );
}
