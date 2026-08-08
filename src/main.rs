use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use wraith::{Parser, codegen, lex};

const VERSION: &str = env!("CARGO_PKG_VERSION");

// ANSI color codes
const GREEN: &str = "\x1b[92m";
const RED: &str = "\x1b[91m";
const YELLOW: &str = "\x1b[93m";
const RESET: &str = "\x1b[0m";

// Shell completion scripts, shipped in completions/ so packagers can install
// them directly and `--completions <shell>` can print them without the two
// copies drifting apart.
const COMPLETION_BASH: &str = include_str!("../completions/wraith.bash");
const COMPLETION_ZSH: &str = include_str!("../completions/_wraith");
const COMPLETION_FISH: &str = include_str!("../completions/wraith.fish");

fn main() {
    let args = std::env::args().collect::<Vec<String>>();

    // Parse arguments
    let mut verbosity = codegen::CommentVerbosity::Normal;
    let mut target = codegen::TargetCpu::default();
    let mut input_file: Option<String> = None;
    let mut out_dir: Option<PathBuf> = None;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--version" | "-v" => {
                println!("Wraith Compiler {}", VERSION);
                return;
            }
            "--help" | "-h" => {
                print_usage(&args[0], true);
                return;
            }
            "--completions" => {
                let Some(shell) = args.get(i + 1) else {
                    eprintln!("{}Error:{} --completions requires a shell name", RED, RESET);
                    eprintln!("       valid options: bash, zsh, fish");
                    std::process::exit(1);
                };
                match shell.as_str() {
                    "bash" => print!("{}", COMPLETION_BASH),
                    "zsh" => print!("{}", COMPLETION_ZSH),
                    "fish" => print!("{}", COMPLETION_FISH),
                    other => {
                        eprintln!("{}Error:{} unsupported shell: {}", RED, RESET, other);
                        eprintln!("       valid options: bash, zsh, fish");
                        std::process::exit(1);
                    }
                }
                return;
            }
            "--out" | "-o" => {
                if i + 1 < args.len() {
                    out_dir = Some(PathBuf::from(&args[i + 1]));
                    i += 2;
                } else {
                    eprintln!("{}Error:{} --out requires a directory", RED, RESET);
                    std::process::exit(1);
                }
            }
            "--comments" | "-c" => {
                if i + 1 < args.len() {
                    verbosity = match args[i + 1].as_str() {
                        "minimal" | "min" => codegen::CommentVerbosity::Minimal,
                        "normal" => codegen::CommentVerbosity::Normal,
                        "verbose" | "v" => codegen::CommentVerbosity::Verbose,
                        other => {
                            eprintln!("{}Error:{} unknown verbosity level: {}", RED, RESET, other);
                            eprintln!("       valid options: minimal, normal, verbose");
                            std::process::exit(1);
                        }
                    };
                    i += 2;
                } else {
                    eprintln!("{}Error:{} --comments requires an argument", RED, RESET);
                    std::process::exit(1);
                }
            }
            "--cpu" => {
                if i + 1 < args.len() {
                    target = match args[i + 1].as_str() {
                        "65c02" | "65C02" | "cmos" => codegen::TargetCpu::Cmos65C02,
                        "6502" | "nmos" => codegen::TargetCpu::Nmos6502,
                        other => {
                            eprintln!("{}Error:{} unknown CPU target: {}", RED, RESET, other);
                            eprintln!("       valid options: 65c02 (default), 6502");
                            std::process::exit(1);
                        }
                    };
                    i += 2;
                } else {
                    eprintln!("{}Error:{} --cpu requires an argument", RED, RESET);
                    std::process::exit(1);
                }
            }
            arg if !arg.starts_with('-') => {
                if input_file.is_some() {
                    eprintln!("{}Error:{} multiple input files not supported", RED, RESET);
                    std::process::exit(1);
                }
                input_file = Some(arg.to_string());
                i += 1;
            }
            unknown => {
                eprintln!("{}Error:{} unknown option: {}", RED, RESET, unknown);
                print_usage(&args[0], false);
                std::process::exit(1);
            }
        }
    }

    let file = match input_file {
        Some(f) => f,
        None => {
            print_usage(&args[0], false);
            std::process::exit(1);
        }
    };
    let start_time = Instant::now();

    // `-o/--out` takes a directory, but a file path is a natural mistake:
    // `-o build/main.asm` would otherwise create a directory literally called
    // `main.asm` and write `main.asm` inside it. Take the intent (the parent
    // directory) and say so.
    let out_dir = out_dir.map(|d| {
        let resolved = out_dir_or_file_parent(&d);
        if resolved != d {
            eprintln!(
                "{}Warning:{} -o/--out expects a directory, not a file name: '{}'",
                YELLOW,
                RESET,
                d.display()
            );
            eprintln!("       using '{}' instead", resolved.display());
        }
        resolved.to_path_buf()
    });

    // Validate the project's memory map before doing any work. A malformed
    // wraith.toml is otherwise silently swallowed and replaced with the default
    // map, so the compile would succeed against a layout the user never asked
    // for.
    if let Err(e) = wraith::config::Config::check_project_file() {
        eprintln!("{}Error:{} malformed wraith.toml: {}", RED, RESET, e);
        std::process::exit(1);
    }

    // Read source file
    println!("{}{:>12}{} {}", YELLOW, "Compiling", RESET, file);
    let source = match fs::read_to_string(&file) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{}Error:{} {}: {}", RED, RESET, file, e);
            std::process::exit(1);
        }
    };

    // Lex
    let tokens = match lex(&source) {
        Ok(tokens) => tokens,
        Err(e) => {
            eprintln!("{}Error:{} Lexical analysis failed", RED, RESET);
            // Render with source carets like every other front-end error,
            // not as the error struct's Debug output.
            let span = wraith::Span::new(e.span.start, e.span.end);
            eprintln!(
                "{}",
                span.format_error_context(&source, Some(&file), &e.message)
            );
            std::process::exit(1);
        }
    };

    // Parse
    let ast = match Parser::parse(&tokens) {
        Ok(ast) => ast,
        Err(e) => {
            eprintln!("{}", e.format_with_source_and_file(&source, Some(&file)));
            std::process::exit(1);
        }
    };

    // Print imports
    for item in &ast.items {
        if let wraith::ast::Item::Import(import) = &item.node {
            println!(
                "{}{:>12}{} {}",
                YELLOW, "Importing", RESET, import.path.node
            );
        }
    }

    // Semantic analysis
    let file_path = PathBuf::from(&file);
    let program_info = match wraith::sema::analyze_with_path(&ast, file_path) {
        Ok(info) => info,
        Err(e) => {
            eprintln!("{}", e.format_with_source_and_file(&source, Some(&file)));
            std::process::exit(1);
        }
    };

    // Display warnings
    for warning in &program_info.warnings {
        eprintln!(
            "{}",
            warning.format_with_source_and_file(&source, Some(&file))
        );
        eprintln!(); // Add blank line between warnings
    }

    // Code generation
    let (code, section_alloc) = match codegen::generate(&ast, &program_info, verbosity, target) {
        Ok(result) => result,
        Err(e) => {
            eprintln!("{}", e.format_with_source_and_file(&source, Some(&file)));
            std::process::exit(1);
        }
    };

    // Write output
    let out_file = output_path(&file, out_dir.as_deref());
    if let Some(dir) = out_file.parent().filter(|d| !d.as_os_str().is_empty())
        && let Err(e) = fs::create_dir_all(dir)
    {
        eprintln!(
            "{}Error:{} could not create {}: {}",
            RED,
            RESET,
            dir.display(),
            e
        );
        std::process::exit(1);
    }
    if let Err(e) = fs::write(&out_file, &code) {
        eprintln!(
            "{}Error:{} could not write to {}: {}",
            RED,
            RESET,
            out_file.display(),
            e
        );
        std::process::exit(1);
    }

    let elapsed = start_time.elapsed();
    let elapsed_ms = elapsed.as_secs_f64() * 1000.0;

    println!(
        "{}{:>12}{} {} in {:.2}ms",
        GREEN,
        "Finished",
        RESET,
        out_file.display(),
        elapsed_ms
    );

    // Print section statistics
    let stats = section_alloc.get_statistics();
    for stat in stats {
        if stat.used > 0 {
            println!(
                "{}{:>12}{} {}",
                YELLOW,
                stat.name,
                RESET,
                stat.format_compact()
            );
        }
    }
}

/// Work out where the generated assembly goes.
///
/// Without `--out` the `.asm` sits next to its source. With `--out` only the
/// file name is kept and the directory is replaced, so `--out build` puts
/// `src/game/main.wr` at `build/main.asm`.
///
/// The extension is swapped via `Path`, not by substring replacement: a path
/// like `.wraith/main.wr` contains `.wr` twice, and rewriting every occurrence
/// would produce `.asmaith/main.asm` — a directory that does not exist.
fn output_path(input: &str, out_dir: Option<&Path>) -> PathBuf {
    let input = Path::new(input);
    let with_ext = input.with_extension("asm");
    match out_dir {
        Some(dir) => {
            let name = with_ext.file_name().unwrap_or(with_ext.as_os_str());
            dir.join(name)
        }
        None => with_ext,
    }
}

/// `-o/--out` takes a directory. A value that names the `.asm` file itself
/// (`build/main.asm`) is a file path, not a directory — return the directory
/// part so the caller can warn and proceed with what was meant. A bare file
/// name (`main.asm`, no parent) maps to the current directory. Only `.asm`
/// triggers this: dotted directory names (`build/v1.2`) are legitimate.
fn out_dir_or_file_parent(dir: &Path) -> &Path {
    if dir
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("asm"))
    {
        dir.parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."))
    } else {
        dir
    }
}

/// Render the usage text. `to_stdout` is true when help was explicitly asked
/// for (`--help`/`-h`) — help on request is normal output and belongs on
/// stdout; usage printed alongside an error stays on stderr.
fn print_usage(program: &str, to_stdout: bool) {
    let usage = format!(
        "Usage: {program} [OPTIONS] <input.wr>\n\
         \n\
         Options:\n\
         \x20 -h, --help              Print this help message\n\
         \x20 -v, --version           Print version information\n\
         \x20 -c, --comments LEVEL    Set comment verbosity in generated assembly\n\
         \x20                         LEVEL: minimal, normal (default), verbose\n\
         \x20     --cpu TARGET        Target CPU (default: 65c02)\n\
         \x20                         TARGET: 65c02, 6502\n\
         \x20 -o, --out DIR           Write the .asm output to DIR instead of\n\
         \x20                         alongside the source. DIR is created if needed.\n\
         \x20     --completions SHELL Print a shell completion script and exit\n\
         \x20                         SHELL: bash, zsh, fish"
    );
    if to_stdout {
        println!("{usage}");
    } else {
        eprintln!("{usage}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_sits_beside_the_source_without_out() {
        assert_eq!(output_path("main.wr", None), PathBuf::from("main.asm"));
        assert_eq!(
            output_path("src/game/main.wr", None),
            PathBuf::from("src/game/main.asm")
        );
    }

    #[test]
    fn out_dir_replaces_the_directory_and_keeps_the_file_name() {
        assert_eq!(
            output_path("src/game/main.wr", Some(Path::new("build"))),
            PathBuf::from("build/main.asm")
        );
        assert_eq!(
            output_path("main.wr", Some(Path::new("/tmp/out"))),
            PathBuf::from("/tmp/out/main.asm")
        );
    }

    #[test]
    fn only_the_extension_is_rewritten() {
        // Substring replacement would turn this into `.asmaith/main.asm`.
        assert_eq!(
            output_path(".wraith/main.wr", None),
            PathBuf::from(".wraith/main.asm")
        );
        // A stem containing the extension text must survive intact.
        assert_eq!(
            output_path("rewrite.wr", None),
            PathBuf::from("rewrite.asm")
        );
    }

    #[test]
    fn a_source_without_the_wr_extension_still_gets_one() {
        assert_eq!(output_path("main", None), PathBuf::from("main.asm"));
        assert_eq!(
            output_path("main", Some(Path::new("build"))),
            PathBuf::from("build/main.asm")
        );
    }

    #[test]
    fn an_asm_file_as_out_dir_is_taken_as_its_parent() {
        // `-o build/main.asm` means `build/`, not a directory called main.asm.
        assert_eq!(
            out_dir_or_file_parent(Path::new("build/main.asm")),
            Path::new("build")
        );
        assert_eq!(
            out_dir_or_file_parent(Path::new("main.asm")),
            Path::new(".")
        );
        assert_eq!(
            out_dir_or_file_parent(Path::new("build/MAIN.ASM")),
            Path::new("build")
        );
    }

    #[test]
    fn dotted_directory_names_are_not_mistaken_for_files() {
        assert_eq!(
            out_dir_or_file_parent(Path::new("build/v1.2")),
            Path::new("build/v1.2")
        );
        assert_eq!(
            out_dir_or_file_parent(Path::new("build")),
            Path::new("build")
        );
    }

    #[test]
    fn completion_scripts_are_present_for_every_advertised_shell() {
        assert!(COMPLETION_BASH.contains("complete -o filenames -F _wraith wraith"));
        assert!(COMPLETION_ZSH.contains("#compdef wraith"));
        assert!(COMPLETION_FISH.contains("complete -c wraith"));

        // Every flag the usage text advertises should be completable. fish
        // declares long options unprefixed (`-l out`), the others spell them out.
        for flag in ["help", "version", "comments", "out", "completions"] {
            let long = format!("--{flag}");
            assert!(COMPLETION_BASH.contains(&long), "bash is missing {long}");
            assert!(COMPLETION_ZSH.contains(&long), "zsh is missing {long}");
            assert!(
                COMPLETION_FISH.contains(&format!("-l {flag}")),
                "fish is missing {long}"
            );
        }

        // Argument values must be offered, not just the flags themselves.
        for script in [COMPLETION_BASH, COMPLETION_ZSH, COMPLETION_FISH] {
            assert!(script.contains("minimal normal verbose"));
            assert!(script.contains("bash zsh fish"));
        }
    }
}
