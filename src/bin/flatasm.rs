//! `flatasm` - assemble wraith's absolute (`.ORG`-placed) compiler output into
//! a flat binary image.
//!
//! Wraith emits fully-placed absolute assembly and writes the interrupt vector
//! table directly at `$FFFA`-`$FFFF`. That is a flat-image model, not the
//! relocatable-segment model a ca65/ld65 linker config assumes, so the output
//! is assembled here rather than linked. Shares its implementation with the
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

fn parse_addr(s: &str) -> Result<u16, String> {
    let v = if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix('$')) {
        u32::from_str_radix(hex, 16)
    } else {
        s.parse::<u32>()
    }
    .map_err(|_| format!("invalid address: {s}"))?;
    u16::try_from(v).map_err(|_| format!("address out of range: {s}"))
}

fn run() -> Result<(), String> {
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
                output = Some(args.next().ok_or("-o requires a file argument")?);
            }
            "--rom" => rom = true,
            "--start" => start = Some(parse_addr(&args.next().ok_or("--start requires ADDR")?)?),
            "--end" => end = Some(parse_addr(&args.next().ok_or("--end requires ADDR")?)?),
            other if other.starts_with('-') => return Err(format!("unknown option: {other}")),
            other => {
                if input.replace(other.to_string()).is_some() {
                    return Err("only one input file may be given".to_string());
                }
            }
        }
    }

    let input = input.ok_or("no input file given")?;
    let output = output.ok_or("no output file given (-o)")?;
    if rom && (start.is_some() || end.is_some()) {
        return Err("--rom cannot be combined with --start/--end".to_string());
    }

    let (start, end) = if rom {
        (0x8000u16, 0xFFFFu16)
    } else {
        (start.unwrap_or(0x0000), end.unwrap_or(0xFFFF))
    };
    if start > end {
        return Err(format!("start ${start:04X} is above end ${end:04X}"));
    }

    let src = std::fs::read_to_string(&input).map_err(|e| format!("reading {input}: {e}"))?;
    let image = wraith::asm::assemble(&src)?;

    let slice = &image[start as usize..=end as usize];
    std::fs::write(&output, slice).map_err(|e| format!("writing {output}: {e}"))?;

    let reset = u16::from_le_bytes([image[0xFFFC], image[0xFFFD]]);
    eprintln!(
        "wrote {output}: {} bytes (${start:04X}-${end:04X}); reset vector $FFFC -> ${reset:04X}",
        slice.len()
    );
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("flatasm: {e}");
            ExitCode::FAILURE
        }
    }
}
