//! `#[align]` — page-align a const array table in ROM.
//!
//! On the 6502 an indexed read (`LDA tbl,X`) costs an extra cycle when
//! `base + index` crosses a page boundary. A table that starts on a page
//! boundary and fits in a page never crosses, so every access is the fast path
//! and the timing is deterministic. `#[align]` requests that; the section
//! allocator rounds the table's address up to the next `$xx00`.

use crate::common::exec::run;
use crate::common::harness::{CompileResult, compile};

/// The message from a compile that was expected to fail.
fn error(src: &str) -> String {
    match compile(src) {
        CompileResult::SemaError(e)
        | CompileResult::ParseError(e)
        | CompileResult::CodegenError(e)
        | CompileResult::LexError(e) => e,
        CompileResult::Success(..) => panic!("expected a compile error, but it compiled:\n{src}"),
    }
}

/// A `#[align]` table forced past a page boundary still reads the right values:
/// a live table before it pushes it off `$D000`, and indexed reads resolve
/// through the moved label.
#[test]
fn an_aligned_table_reads_correctly_after_padding() {
    let mut e = run(r#"
        const OUT: addr = 0x0500;
        const OUT2: addr = 0x0501;
        const OTHER: [u8; 3] = [1, 2, 3];
        #[align]
        const TBL: [u8; 4] = [10, 20, 30, 40];
        #[reset]
        fn main() {
            let i: u8 = 2;
            let j: u8 = 1;
            OUT = TBL[i];
            OUT2 = OTHER[j];
            loop {}
        }
    "#);
    assert_eq!(e.mem(0x0500), 30, "TBL[2] through the page-aligned table");
    assert_eq!(e.mem(0x0501), 2, "OTHER[1] from the table before it");
}

/// The emitted `.ORG` for the aligned table lands on a page boundary even when
/// a preceding live table would otherwise place it mid-page.
#[test]
fn an_aligned_table_starts_on_a_page_boundary() {
    let asm = crate::common::harness::compile_success(
        "const OUT: addr = 0x0500;\n\
         const OTHER: [u8; 3] = [1, 2, 3];\n\
         #[align]\n\
         const TBL: [u8; 4] = [10, 20, 30, 40];\n\
         #[reset]\n\
         fn main() { let i: u8 = 0; OUT = TBL[i] + OTHER[i]; loop {} }\n",
    );
    // Find the .ORG immediately preceding the TBL label and check its low byte.
    let org = asm
        .lines()
        .zip(asm.lines().skip(1))
        .find_map(|(a, b)| (b.trim() == "TBL:").then(|| a.trim().to_string()))
        .expect("TBL has a preceding .ORG");
    let addr = org.strip_prefix(".ORG $").expect("an .ORG line");
    assert!(
        addr.ends_with("00"),
        "TBL should be page-aligned, got .ORG ${addr}\n{asm}"
    );
    // And it is not at $D000 (OTHER took that), so alignment actually moved it.
    assert_ne!(
        addr, "D000",
        "the preceding table should have pushed TBL to $D100"
    );
}

/// An unaligned table before an aligned one shares a page; the aligned one is on
/// the next boundary — a golden check of the padding.
#[test]
fn alignment_pushes_the_table_to_the_next_page() {
    let asm = crate::common::harness::compile_success(
        "const OUT: addr = 0x0500;\n\
         const OTHER: [u8; 3] = [1, 2, 3];\n\
         #[align]\n\
         const TBL: [u8; 4] = [10, 20, 30, 40];\n\
         #[reset]\n\
         fn main() { let i: u8 = 0; OUT = TBL[i] + OTHER[i]; loop {} }\n",
    );
    assert!(asm.contains("OTHER:"), "OTHER emitted:\n{asm}");
    assert!(
        asm.contains(".ORG $D100\nTBL:"),
        "TBL page-aligned to $D100:\n{asm}"
    );
}

#[test]
fn align_is_refused_on_a_mutable_static() {
    let e =
        error("#[align]\nstatic BUF: [u8; 4] = [0, 0, 0, 0];\n#[reset]\nfn main() { loop {} }\n");
    assert!(e.contains("#[align]") && e.contains("mutable"), "{e}");
}

#[test]
fn align_is_refused_on_a_scalar_const() {
    let e = error("#[align]\nconst X: u16 = 5;\n#[reset]\nfn main() { loop {} }\n");
    assert!(e.contains("#[align]") && e.contains("array"), "{e}");
}

#[test]
fn align_is_refused_on_an_addr() {
    let e = error("#[align]\nconst PORT: addr = 0xD000;\n#[reset]\nfn main() { loop {} }\n");
    assert!(e.contains("#[align]"), "{e}");
}

#[test]
fn align_is_refused_on_a_function() {
    let e = error("#[align]\nfn helper() {}\n#[reset]\nfn main() { helper(); loop {} }\n");
    assert!(e.contains("#[align]") && e.contains("function"), "{e}");
}

#[test]
fn align_rejects_an_argument() {
    let e = error(
        "#[align(256)]\nconst TBL: [u8; 4] = [1, 2, 3, 4];\n#[reset]\nfn main() { loop {} }\n",
    );
    assert!(e.contains("#[align]") && e.contains("no arguments"), "{e}");
}
