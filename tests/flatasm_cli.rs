//! CLI behaviour of the `flatasm` tool: how it fails, and the one warning it
//! raises. The assembler itself is covered through `wraith::asm` elsewhere;
//! this pins the wrapper — that a misuse shows the flag list, and that a
//! zero reset vector (the "my machine won't boot" symptom) is called out.

use std::path::PathBuf;
use std::process::Command;

fn flatasm() -> Command {
    Command::new(env!("CARGO_BIN_EXE_flatasm"))
}

/// Write `body` to a uniquely named `.asm` in the temp dir and return its path.
fn write_asm(tag: &str, body: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("flatasm_{tag}_{}.asm", std::process::id()));
    std::fs::write(&p, body).unwrap();
    p
}

/// A minimal absolute program with a reset vector pointing at `main`.
const WITH_RESET: &str = "\
.ORG $8000
main:
    JMP main
.ORG $FFFC
.WORD main
";

/// The same program with no vector table, so $FFFC stays zero.
const NO_RESET: &str = "\
.ORG $8000
main:
    JMP main
";

#[test]
fn no_arguments_shows_the_usage_and_fails() {
    let out = flatasm().output().unwrap();
    assert!(!out.status.success(), "no args should be an error");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("Usage:"), "usage not shown:\n{err}");
    assert!(err.contains("--rom"), "flag list not shown:\n{err}");
}

#[test]
fn an_unknown_option_shows_the_usage_and_fails() {
    let out = flatasm()
        .args(["x.asm", "--frob", "-o", "y.bin"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("unknown option: --frob"),
        "cause not named:\n{err}"
    );
    assert!(err.contains("Usage:"), "usage not shown:\n{err}");
}

#[test]
fn help_prints_to_stdout_and_succeeds() {
    let out = flatasm().arg("--help").output().unwrap();
    assert!(out.status.success(), "--help should succeed");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("Usage:"), "help not on stdout:\n{stdout}");
}

#[test]
fn a_zero_reset_vector_warns() {
    let asm = write_asm("noreset", NO_RESET);
    let bin = asm.with_extension("bin");
    let out = flatasm()
        .args([asm.to_str().unwrap(), "--rom", "-o", bin.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "assembling a valid program should succeed"
    );
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("$FFFC -> $0000"),
        "vector not reported:\n{err}"
    );
    assert!(
        err.contains("warning:"),
        "a zero reset vector should warn:\n{err}"
    );
    assert!(
        err.contains("#[reset]"),
        "the warning should name the cause:\n{err}"
    );
    let _ = std::fs::remove_file(&asm);
    let _ = std::fs::remove_file(&bin);
}

#[test]
fn a_real_reset_vector_is_quiet() {
    let asm = write_asm("reset", WITH_RESET);
    let bin = asm.with_extension("bin");
    let out = flatasm()
        .args([asm.to_str().unwrap(), "--rom", "-o", bin.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("$FFFC -> $8000"),
        "vector not resolved to main:\n{err}"
    );
    assert!(
        !err.contains("warning:"),
        "a valid vector must not warn:\n{err}"
    );
    let _ = std::fs::remove_file(&asm);
    let _ = std::fs::remove_file(&bin);
}
