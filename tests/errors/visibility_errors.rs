//! Module visibility error tests
//!
//! Tests for `pub` keyword and import visibility checking

use crate::common::*;
use std::fs;

// ============================================================================
// Private Item Import Errors
// ============================================================================

#[test]
fn cannot_import_private_function() {
    // Create library file with private function
    fs::write("/tmp/test_vis_lib1.wr", r#"
        fn private_helper() -> u8 {
            return 42;
        }

        #[reset]
        fn main() {}
    "#).unwrap();

    assert_error_contains(
        r#"
        import { private_helper } from "/tmp/test_vis_lib1.wr";

        fn main() {
            let x: u8 = private_helper();
        }
        "#,
        "private",
    );
}

#[test]
fn cannot_import_private_const() {
    fs::write("/tmp/test_vis_lib2.wr", r#"
        const PRIVATE_VALUE: u8 = 100;

        #[reset]
        fn main() {}
    "#).unwrap();

    assert_error_contains(
        r#"
        import { PRIVATE_VALUE } from "/tmp/test_vis_lib2.wr";

        fn main() {
            let x: u8 = PRIVATE_VALUE;
        }
        "#,
        "private",
    );
}

#[test]
fn cannot_import_private_struct() {
    fs::write("/tmp/test_vis_lib3.wr", r#"
        struct PrivatePoint {
            x: u8,
            y: u8,
        }

        #[reset]
        fn main() {}
    "#).unwrap();

    assert_error_contains(
        r#"
        import { PrivatePoint } from "/tmp/test_vis_lib3.wr";

        fn main() {
            let p: PrivatePoint = PrivatePoint { x: 1, y: 2 };
        }
        "#,
        "private",
    );
}

#[test]
fn cannot_import_private_enum() {
    fs::write("/tmp/test_vis_lib4.wr", r#"
        enum PrivateColor {
            Red,
            Blue,
        }

        #[reset]
        fn main() {}
    "#).unwrap();

    assert_error_contains(
        r#"
        import { PrivateColor } from "/tmp/test_vis_lib4.wr";

        fn main() {
            let c: PrivateColor = PrivateColor::Red;
        }
        "#,
        "private",
    );
}

#[test]
fn cannot_import_private_addr() {
    fs::write("/tmp/test_vis_lib5.wr", r#"
        const PRIVATE_IO: addr = 0x6000;

        #[reset]
        fn main() {}
    "#).unwrap();

    assert_error_contains(
        r#"
        import { PRIVATE_IO } from "/tmp/test_vis_lib5.wr";

        fn main() {
            PRIVATE_IO = 42;
        }
        "#,
        "private",
    );
}

// ============================================================================
// Public Item Import Success
// ============================================================================

#[test]
fn can_import_public_function() {
    fs::write("/tmp/test_vis_pub1.wr", r#"
        pub fn helper() -> u8 {
            return 42;
        }

        #[reset]
        fn main() {}
    "#).unwrap();

    let _asm = compile_success(r#"
        import { helper } from "/tmp/test_vis_pub1.wr";

        fn main() {
            let x: u8 = helper();
        }
    "#);
}

#[test]
fn can_import_public_const() {
    fs::write("/tmp/test_vis_pub2.wr", r#"
        pub const PUBLIC_VALUE: u8 = 100;

        #[reset]
        fn main() {}
    "#).unwrap();

    let _asm = compile_success(r#"
        import { PUBLIC_VALUE } from "/tmp/test_vis_pub2.wr";

        fn main() {
            let x: u8 = PUBLIC_VALUE;
        }
    "#);
}

#[test]
fn can_import_public_struct() {
    fs::write("/tmp/test_vis_pub3.wr", r#"
        pub struct PublicPoint {
            x: u8,
            y: u8,
        }

        #[reset]
        fn main() {}
    "#).unwrap();

    let _asm = compile_success(r#"
        import { PublicPoint } from "/tmp/test_vis_pub3.wr";

        fn main() {
            let p: PublicPoint = PublicPoint { x: 1, y: 2 };
        }
    "#);
}

#[test]
fn can_import_public_enum() {
    fs::write("/tmp/test_vis_pub4.wr", r#"
        pub enum PublicColor {
            Red,
            Blue,
        }

        #[reset]
        fn main() {}
    "#).unwrap();

    let _asm = compile_success(r#"
        import { PublicColor } from "/tmp/test_vis_pub4.wr";

        fn main() {
            let c: PublicColor = PublicColor::Red;
        }
    "#);
}

#[test]
fn can_import_public_addr() {
    fs::write("/tmp/test_vis_pub5.wr", r#"
        pub const PUBLIC_IO: addr = 0x6000;

        #[reset]
        fn main() {}
    "#).unwrap();

    let _asm = compile_success(r#"
        import { PUBLIC_IO } from "/tmp/test_vis_pub5.wr";

        fn main() {
            PUBLIC_IO = 42;
        }
    "#);
}

// ============================================================================
// Mixed Visibility
// ============================================================================

#[test]
fn can_import_public_from_mixed_module() {
    fs::write("/tmp/test_vis_mixed.wr", r#"
        pub fn public_func() -> u8 {
            return private_func();
        }

        fn private_func() -> u8 {
            return 42;
        }

        #[reset]
        fn main() {}
    "#).unwrap();

    let _asm = compile_success(r#"
        import { public_func } from "/tmp/test_vis_mixed.wr";

        fn main() {
            let x: u8 = public_func();
        }
    "#);
}

#[test]
fn cannot_import_private_from_mixed_module() {
    fs::write("/tmp/test_vis_mixed2.wr", r#"
        pub fn public_func() -> u8 {
            return 10;
        }

        fn private_func() -> u8 {
            return 20;
        }

        #[reset]
        fn main() {}
    "#).unwrap();

    assert_error_contains(
        r#"
        import { private_func } from "/tmp/test_vis_mixed2.wr";

        fn main() {
            let x: u8 = private_func();
        }
        "#,
        "private",
    );
}

#[test]
fn import_multiple_public_items() {
    fs::write("/tmp/test_vis_multi.wr", r#"
        pub fn func1() -> u8 { return 1; }
        pub fn func2() -> u8 { return 2; }
        pub const VALUE: u8 = 42;

        #[reset]
        fn main() {}
    "#).unwrap();

    let _asm = compile_success(r#"
        import { func1, func2, VALUE } from "/tmp/test_vis_multi.wr";

        fn main() {
            let a: u8 = func1();
            let b: u8 = func2();
            let c: u8 = VALUE;
        }
    "#);
}

#[test]
fn error_when_one_import_is_private() {
    fs::write("/tmp/test_vis_partial.wr", r#"
        pub fn public_item() -> u8 { return 1; }
        fn private_item() -> u8 { return 2; }

        #[reset]
        fn main() {}
    "#).unwrap();

    assert_error_contains(
        r#"
        import { public_item, private_item } from "/tmp/test_vis_partial.wr";

        fn main() {
            let x: u8 = public_item();
        }
        "#,
        "private",
    );
}
