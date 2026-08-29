//! `flatasm` - assemble wraith's absolute (`.ORG`-placed) compiler output into
//! a flat binary image.
//!
//! Wraith emits fully-placed absolute assembly and writes the interrupt vector
//! table directly at `$FFFA`-`$FFFF`. That is a flat-image model, not the
//! relocatable-segment model like some other assemblers/linkers. Shares its implementation with the
//! test harness via `wraith::asm`.
//!
//! Usage:
//!     flatasm program.asm -o program.bin           # full 64 KB image
//!     flatasm program.asm -o program.rom --rom      # $8000-$FFFF ROM (32 KB)
//!     flatasm program.asm -o out.bin --start 0x8000 --end 0xFFFF

use std::process::ExitCode;

const USAGE: &str = "\
Usage: flatasm <input.asm> -o <output> [--rom | --start ADDR --end ADDR]

  -o, --output FILE   output binary (required)
      --rom           emit the $8000-$FFFF ROM image (32 KB)
      --start ADDR    first address of the slice to emit (e.g. 0x8000)
      --end ADDR      last address of the slice to emit, inclusive (e.g. 0xFFFF)
  -h, --help          show this help

With no slice options the full 64 KB image is written (byte i = address i).";

/// How the tool failed, so `main` can decide whether to show usage. A mistake
/// in *how it was invoked* is one the usage text helps fix, so it is printed
/// alongside; a failure while doing the work (a missing file, a bad `.asm`)
/// stands on its own, the way the `wraith` compiler reports its own errors.
enum AppError {
    Usage(String),
    Runtime(String),
}

fn parse_addr(s: &str) -> Result<u16, AppError> {
    let v = if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix('$')) {
        u32::from_str_radix(hex, 16)
    } else {
        s.parse::<u32>()
    }
    .map_err(|_| AppError::Usage(format!("invalid address: {s}")))?;
    u16::try_from(v).map_err(|_| AppError::Usage(format!("address out of range: {s}")))
}

fn run() -> Result<(), AppError> {
    use AppError::{Runtime, Usage};
    let mut input: Option<String> = None;
    let mut output: Option<String> = None;
    let mut rom = false;
    let mut start: Option<u16> = None;
    let mut end: Option<u16> = None;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(());
            }
            "-o" | "--output" => {
                output = Some(
                    args.next()
                        .ok_or(Usage("-o requires a file argument".into()))?,
                );
            }
            "--rom" => rom = true,
            "--start" => {
                start = Some(parse_addr(
                    &args.next().ok_or(Usage("--start requires ADDR".into()))?,
                )?)
            }
            "--end" => {
                end = Some(parse_addr(
                    &args.next().ok_or(Usage("--end requires ADDR".into()))?,
                )?)
            }
            other if other.starts_with('-') => {
                return Err(Usage(format!("unknown option: {other}")));
            }
            other => {
                if input.replace(other.to_string()).is_some() {
                    return Err(Usage("only one input file may be given".into()));
                }
            }
        }
    }

    let input = input.ok_or(Usage("no input file given".into()))?;
    let output = output.ok_or(Usage("no output file given (-o)".into()))?;
    if rom && (start.is_some() || end.is_some()) {
        return Err(Usage("--rom cannot be combined with --start/--end".into()));
    }

    let (start, end) = if rom {
        (0x8000u16, 0xFFFFu16)
    } else {
        (start.unwrap_or(0x0000), end.unwrap_or(0xFFFF))
    };
    if start > end {
        return Err(Usage(format!("start ${start:04X} is above end ${end:04X}")));
    }

    let src =
        std::fs::read_to_string(&input).map_err(|e| Runtime(format!("reading {input}: {e}")))?;
    let image = wraith::asm::assemble(&src).map_err(Runtime)?;

    let slice = &image[start as usize..=end as usize];
    std::fs::write(&output, slice).map_err(|e| Runtime(format!("writing {output}: {e}")))?;

    let reset = u16::from_le_bytes([image[0xFFFC], image[0xFFFD]]);
    eprintln!(
        "wrote {output}: {} bytes (${start:04X}-${end:04X}); reset vector $FFFC -> ${reset:04X}",
        slice.len()
    );

    // A reset vector of $0000 almost never means "boot at $0000" — it means the
    // vector was never set, and on a real machine the CPU jumps into the zero
    // page and runs garbage. The vector is only populated when a function is
    // marked `#[reset]`, so the usual cause is an `.asm` assembled before that
    // attribute was added. Warn only when the emitted slice actually contains
    // the vector, so a deliberate partial slice stays quiet.
    if reset == 0x0000 && start <= 0xFFFC && end >= 0xFFFD {
        eprintln!(
            "flatasm: {YELLOW}warning:{RESET} the reset vector at $FFFC is $0000 — on reset the \
             CPU will jump to $0000"
        );
        eprintln!(
            "  the vector is set only for a function marked #[reset]; if your program has one, \
             this .asm is likely stale — recompile it"
        );
    }
    Ok(())
}

const YELLOW: &str = "\x1b[33m";
const RESET: &str = "\x1b[0m";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        // A misuse of the tool: the flag list is part of the fix, so show it.
        Err(AppError::Usage(msg)) => {
            eprintln!("flatasm: {msg}\n");
            eprintln!("{USAGE}");
            ExitCode::FAILURE
        }
        // A failure while working: the message stands alone.
        Err(AppError::Runtime(msg)) => {
            eprintln!("flatasm: {msg}");
            ExitCode::FAILURE
        }
    }
}
