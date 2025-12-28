# Wraith Monitor - 6502 Homebrew Computer OS

A simple monitor/debugger for a 6502 homebrew computer, written in Wraith. This provides a serial terminal interface for inspecting and controlling your 6502 system.

## Hardware Requirements

### Memory Map

-   **RAM**: `0x0000 - 0x7FFF` (32KB)
-   **ROM**: `0x8000 - 0xDFFF` (24KB)
-   **I/O**: `0xE000 - 0xFFFC` (Memory-mapped peripherals)

### Peripherals

-   **TL16C550B UART** at `0xE000` (16550-compatible serial port)
-   **6522 VIA** at `0xE010` (versatile interface adapter)

### UART Configuration

-   **Baud Rate**: 9600 (configurable in `hardware.wr`)
-   **Format**: 8N1 (8 data bits, no parity, 1 stop bit)
-   **Crystal**: 1.8432 MHz (standard for UART timing)

## Features

### Current Features

-   ✅ Serial terminal communication via UART
-   ✅ Memory read/write commands
-   ✅ Memory dump with hex display
-   ✅ Jump to address (execute code)
-   ✅ Simple command-line interface

### Planned Features

-   ⏳ Program upload via serial (Intel HEX or binary)
-   ⏳ Disassembler
-   ⏳ Register display
-   ⏳ Breakpoints
-   ⏳ Single-step execution

## Commands

### Help

```
> H
```

Displays list of available commands.

### Read Memory

```
> R AAAA
```

Read and display a single byte from address `AAAA`.

Example:

```
> R 0200
0200: 41
```

### Write Memory

```
> W AAAA DD
```

Write byte `DD` to address `AAAA`.

Example:

```
> W 0200 FF
OK
```

### Dump Memory

```
> D AAAA BBBB
```

Display hex dump from address `AAAA` to `BBBB` (inclusive).

Example:

```
> D 0200 021F
0200: 41 42 43 44 45 46 47 48 49 4A 4B 4C 4D 4E 4F 50
0210: 51 52 53 54 55 56 57 58 59 5A 00 00 00 00 00 00
```

### Go/Execute

```
> G AAAA
```

Jump to address `AAAA` and begin execution.

Example:

```
> G 0300
Jump to 0300
```

**Note**: If the executed code returns (via RTS), control returns to the monitor.

## File Structure

```
monitor/
├── hardware.wr    # Hardware definitions and memory map
├── uart.wr        # UART driver (TL16C550B)
├── monitor.wr     # Main monitor/command interpreter
├── vectors.wr     # Interrupt vectors and startup
└── README.md      # This file
```

## Memory Usage

### Zero Page (`0x0000 - 0x00FF`)

-   `0x00F0 - 0x00F4`: Temporary storage for monitor commands

### Low RAM (`0x0200 - 0x02FF`)

-   `0x0200 - 0x023F`: Command line input buffer (64 bytes)

### ROM (`0x8000 - 0xFFFF`)

-   `0x8000`: Reset entry point (monitor main loop)
-   `0xE000`: NMI handler
-   `0xE003`: IRQ handler
-   `0xFFFA - 0xFFFF`: Interrupt vectors

## Hardware Setup

### Minimal Setup

For basic operation, you only need:

1. 6502 CPU
2. 32KB RAM (0x0000-0x7FFF)
3. 24KB ROM (0x8000-0xDFFF) - programmed with this monitor
4. TL16C550B UART at 0xE000
5. Serial connection to PC (USB-to-serial adapter)

### Address Decoding

Example logic for 32KB RAM / 24KB ROM:

```
/RAM_CS  = A15                    (when A15=0, RAM selected)
/ROM_CS  = /A15 & /A14 & /A13     (when 0x8000-0xDFFF)
/UART_CS = A15 & A14 & A13 & /A12 (when 0xE000-0xEFFF)
```

## Terminal Setup

Configure your serial terminal:

-   **Baud**: 9600
-   **Data bits**: 8
-   **Parity**: None
-   **Stop bits**: 1
-   **Flow control**: None
-   **Local echo**: Off (monitor echoes input)

### Recommended Terminal Programs

-   **macOS/Linux**: `screen`, `minicom`, `picocom`
-   **Windows**: PuTTY, TeraTerm
-   **Cross-platform**: CoolTerm, RealTerm

Example using `screen`:

```bash
screen /dev/ttyUSB0 9600
```

Example using `picocom`:

```bash
picocom -b 9600 /dev/ttyUSB0
```

## Example Session

```
Wraith Monitor v1.0
Type 'H' for help

> H
Commands:
R AAAA - Read
W AAAA DD - Write
G AAAA - Go
D AAAA BBBB - Dump

> R 8000
8000: 4C

> W 0200 41
OK

> D 0200 020F
0200: 41 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00

> G 8000
Jump to 8000

Wraith Monitor v1.0
Type 'H' for help

>
```

## Extending the Monitor

### Adding New Commands

1. Add command handler function in `monitor.wr`
2. Add case in `process_command()` dispatcher
3. Update `print_help()` with new command

### Adding Peripherals

1. Define addresses in `hardware.wr`
2. Create driver module (e.g., `via.wr` for 6522)
3. Import and use in monitor or user programs

### Example: Adding a "Fill Memory" Command

```wraith
// In monitor.wr

fn cmd_fill() {
    // Fill memory range with byte value
    mut addr: u16 = TEMP_ADDR;
    mut end: u16 = TEMP_ADDR2;
    mut value: u8 = TEMP_VAL;

    loop {
        if addr > end { break; }
        mem_write(addr, value);
        addr = addr + 1;
    }
    print_ok();
}

// In process_command():
} else if cmd == 0x46 {  // 'F' - Fill
    pos = parse_hex_word(pos);
    if pos != 0xFF {
        TEMP_ADDR2 = TEMP_ADDR;
        pos = skip_spaces(pos);
        pos = parse_hex_word(pos);
        if pos != 0xFF {
            TEMP_ADDR2 = TEMP_ADDR;
            pos = skip_spaces(pos);
            pos = parse_hex_byte(pos);
            if pos != 0xFF {
                cmd_fill();
                return;
            }
        }
    }
}
```

## Troubleshooting

### No Output on Terminal

1. Check UART wiring (TX/RX crossed?)
2. Verify baud rate matches (9600)
3. Check ROM is programmed correctly
4. Verify UART chip select decoding
5. Test UART with loopback (TX->RX)

### Garbled Output

-   Wrong baud rate divisor (check crystal frequency)
-   Incorrect line format (should be 8N1)
-   Bad crystal or clock

### Commands Don't Work

-   Check command buffer is in RAM, not ROM
-   Verify zero page is writable
-   Test memory write with oscilloscope/logic analyzer

## License

This example is part of the Wraith project and is provided as educational material for building 6502-based systems.
