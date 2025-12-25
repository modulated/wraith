# Wraith

A programming language inspired by Rust compiled to 6502 assembly.

## Configuration

Wraith uses a `wraith.toml` configuration file to define memory sections for the 6502 target. The compiler looks for `wraith.toml` in the current directory when compiling. If not found, it uses default settings.

### Memory Sections

The configuration file defines memory sections where code and data can be placed:

```toml
[[sections]]
name = "STDLIB"
start = 0x8000
end = 0x8FFF
description = "Standard library functions (4KB)"

[[sections]]
name = "CODE"
start = 0x9000
end = 0xBFFF
description = "User code (12KB)"

[[sections]]
name = "DATA"
start = 0xC000
end = 0xCFFF
description = "Constants and data (4KB)"

default_section = "CODE"
```

### Default Configuration

If no `wraith.toml` is present, the compiler uses these defaults:
- **STDLIB**: `0x8000-0x8FFF` (4KB) - Standard library
- **CODE**: `0x9000-0xBFFF` (12KB) - User code (default)
- **DATA**: `0xC000-0xCFFF` (4KB) - Constants and data

Functions without an explicit `#[org]` or `#[section]` attribute are placed in the default section.
