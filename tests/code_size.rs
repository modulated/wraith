//! Code-size benchmark.
//!
//! Records what each example program emits, against a checked-in baseline.
//! Without this there is no way to tell whether an optimization helped, or
//! whether an unrelated change made every program bigger — "smaller code" stays
//! an assertion rather than a measurement.
//!
//! Two numbers per program, because one of them is blind to half the compiler.
//! The **section** figures come from the allocator, which reserves space at
//! *placement* — before the peephole runs. A pass that deletes instructions
//! therefore moves nothing there, and every "code size unchanged" this
//! benchmark reported for a peephole change was measuring something that could
//! not move. The **instruction count** is taken from the emitted assembly, so
//! it sees the whole pipeline. Section bytes still matter — they are what has
//! to fit the memory map — so both are kept.
//!
//! Any change in either direction fails, because both are worth seeing: growth
//! is a regression to explain, and shrinkage is a win whose size belongs in the
//! diff. Re-bless with:
//!
//! ```text
//! WRAITH_BLESS_SIZES=1 cargo test --test code_size
//! ```
//!
//! The corpus is `examples/`, so it grows as the examples do and needs no
//! separate list to maintain.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Bytes emitted into each section, for one program.
#[derive(PartialEq, Eq, Clone, Copy, Default)]
struct Sizes {
    code: u32,
    data: u32,
    /// Instructions in the emitted assembly, after every pass.
    instrs: u32,
}

fn baseline_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/code_size_baseline.txt")
}

fn examples_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("examples")
}

/// Compile one program and report what it emitted, or the reason it did not.
fn measure(path: &Path) -> Result<Sizes, String> {
    let source = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let tokens = wraith::lex(&source).map_err(|e| format!("lex: {e:?}"))?;
    let ast = wraith::parser::Parser::parse(&tokens).map_err(|e| format!("parse: {e:?}"))?;
    // The path of the file itself: imports resolve relative to it, so an
    // example importing `../std/mem.wr` finds the stdlib. The memory map comes
    // from the `wraith.toml` beside the source (walking up), not the working
    // directory's — `examples/symon/` carries a map nothing like the default.
    let config = wraith::config::MemoryConfig::from_source(path);
    let mut program = wraith::sema::analyze_with_config(&ast, path.to_path_buf(), config)
        .map_err(|e| format!("sema: {e:?}"))?;
    let (asm, alloc) = wraith::codegen::generate(
        &ast,
        &mut program,
        wraith::codegen::CommentVerbosity::Minimal,
        wraith::codegen::TargetCpu::default(),
    )
    .map_err(|e| format!("codegen: {e:?}"))?;

    // A line whose first token is a three-letter uppercase mnemonic. Labels
    // end in `:`, directives start with `.`, comments with `;`.
    let instrs = asm
        .lines()
        .map(str::trim)
        .filter(|l| {
            let head = l.split_whitespace().next().unwrap_or("");
            head.len() == 3 && head.bytes().all(|b| b.is_ascii_uppercase())
        })
        .count() as u32;
    let mut sizes = Sizes {
        instrs,
        ..Sizes::default()
    };
    for stat in alloc.get_statistics() {
        match stat.name.as_str() {
            "CODE" => sizes.code = stat.used as u32,
            "DATA" => sizes.data = stat.used as u32,
            _ => {}
        }
    }
    Ok(sizes)
}

/// Collect every `.wr` under `dir`, recursively, so an example that needs its
/// own project directory (its own `wraith.toml`, a subdirectory of helpers)
/// counts too — `examples/symon/` is the first such.
fn collect_wr(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("examples dir is readable") {
        let p = entry.expect("dir entry is readable").path();
        if p.is_dir() {
            collect_wr(&p, out);
        } else if p.extension().is_some_and(|x| x == "wr") {
            out.push(p);
        }
    }
}

/// Every example, in a stable order. A top-level file is named by its bare stem
/// (`fibonacci`); one in a subdirectory keeps the directory (`symon/symon_monitor`),
/// so two examples of the same name in different projects cannot collide.
fn measure_corpus() -> BTreeMap<String, Sizes> {
    let root = examples_dir();
    let mut paths = Vec::new();
    collect_wr(&root, &mut paths);
    paths.sort();

    let mut out = BTreeMap::new();
    for p in &paths {
        let rel = p.strip_prefix(&root).unwrap_or(p);
        let name = rel.with_extension("").to_string_lossy().to_string();
        match measure(p) {
            Ok(s) => {
                out.insert(name, s);
            }
            Err(e) => panic!("example {} no longer compiles: {e}", p.display()),
        }
    }
    out
}

fn render(sizes: &BTreeMap<String, Sizes>) -> String {
    let mut s = String::from(
        "# What each example emits. Regenerate with:\n\
         #   WRAITH_BLESS_SIZES=1 cargo test --test code_size\n\
         # `code`/`data` are section bytes reserved at placement; `instrs` is\n\
         # instructions in the final assembly, which is what a peephole moves.\n\
         # name code data instrs\n",
    );
    for (name, z) in sizes {
        s.push_str(&format!("{} {} {} {}\n", name, z.code, z.data, z.instrs));
    }
    s
}

fn parse_baseline(text: &str) -> BTreeMap<String, Sizes> {
    let mut out = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut it = line.split_whitespace();
        let (Some(name), Some(code), Some(data)) = (it.next(), it.next(), it.next()) else {
            continue;
        };
        out.insert(
            name.to_string(),
            Sizes {
                code: code.parse().unwrap_or(0),
                data: data.parse().unwrap_or(0),
                instrs: it.next().and_then(|v| v.parse().ok()).unwrap_or(0),
            },
        );
    }
    out
}

#[test]
fn code_size_matches_the_baseline() {
    let measured = measure_corpus();

    if std::env::var("WRAITH_BLESS_SIZES").is_ok() {
        std::fs::write(baseline_path(), render(&measured)).expect("baseline is writable");
        eprintln!("blessed {} entries", measured.len());
        return;
    }

    let baseline = match std::fs::read_to_string(baseline_path()) {
        Ok(t) => parse_baseline(&t),
        Err(_) => {
            std::fs::write(baseline_path(), render(&measured)).expect("baseline is writable");
            eprintln!(
                "no baseline yet — wrote one with {} entries; commit it",
                measured.len()
            );
            return;
        }
    };

    let mut lines = Vec::new();
    let mut total_before = 0i64;
    let mut total_after = 0i64;
    let mut instrs_before = 0i64;
    let mut instrs_after = 0i64;

    for (name, now) in &measured {
        match baseline.get(name) {
            None => lines.push(format!(
                "  {name:<24} NEW            code {:>5}  data {:>5}  instrs {:>5}",
                now.code, now.data, now.instrs
            )),
            Some(was) => {
                total_before += (was.code + was.data) as i64;
                total_after += (now.code + now.data) as i64;
                instrs_before += was.instrs as i64;
                instrs_after += now.instrs as i64;
                if was != now {
                    lines.push(format!(
                        "  {name:<24} code {:>5} -> {:<5} ({:+})   data {:>5} -> {:<5} ({:+})   \
                         instrs {:>5} -> {:<5} ({:+})",
                        was.code,
                        now.code,
                        now.code as i64 - was.code as i64,
                        was.data,
                        now.data,
                        now.data as i64 - was.data as i64,
                        was.instrs,
                        now.instrs,
                        now.instrs as i64 - was.instrs as i64,
                    ));
                }
            }
        }
    }
    for name in baseline.keys() {
        if !measured.contains_key(name) {
            lines.push(format!("  {name:<24} REMOVED"));
        }
    }

    assert!(
        lines.is_empty(),
        "emitted code changed:\n{}\n  {:<24} section {} -> {} ({:+} bytes)   \
         instrs {} -> {} ({:+})\n\n\
         If this is intended — an optimization landing, or an example edited — \
         re-bless the baseline so the win (or the cost) shows up in the diff:\n\
         \x20  WRAITH_BLESS_SIZES=1 cargo test --test code_size",
        lines.join("\n"),
        "TOTAL",
        total_before,
        total_after,
        total_after - total_before,
        instrs_before,
        instrs_after,
        instrs_after - instrs_before,
    );
}

/// The measurement has to be reproducible, or the baseline is noise.
#[test]
fn code_size_is_deterministic() {
    let a = measure_corpus();
    let b = measure_corpus();
    let differing: Vec<&String> = a.keys().filter(|k| a.get(*k) != b.get(*k)).collect();
    assert!(
        differing.is_empty(),
        "compiling the same source twice gave different sizes for: {differing:?}"
    );
}
