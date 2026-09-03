# A Symon ROM

[Symon](https://github.com/sethm/symon) is a simulator for a small 6502
single-board computer. `symon_monitor.wr` is the program that would live in its
ROM: a serial monitor whose console output also appears on the CRT.

The point of the example is not the monitor. It is that Symon is a machine
nobody designed Wraith around — its RAM, its ROM and all three of its
peripherals sit at addresses that share nothing with the repository's default
map — and adapting to it is a `wraith.toml` and a page of `const … : addr`
declarations, with no compiler flag and no change to the language.

## The machine

| Range | Device |
|---|---|
| `$0000-$7FFF` | 32 KB RAM |
| `$7000-$77CF` | the 6545's frame buffer, inside that RAM: 80 x 25 characters |
| `$8000-$800F` | 6522 VIA |
| `$8800-$8803` | 6551 ACIA, wired to the simulator's terminal window |
| `$9000-$9001` | 6545 CRTC |
| `$C000-$FFFF` | 16 KB ROM |

`wraith.toml` in this directory turns that into sections: `CODE` and `DATA` in
ROM (stopping at `$FFF9`, since `$FFFA-$FFFF` is the vector table), `BSS` and
`STACK` in low RAM, and nothing at all over the three device windows or the
frame buffer — the compiler places things only where a section says it may, so
leaving a region out of the file is how you reserve it.

The repository's own `wraith.toml` puts `CODE` at `$8000`, which on this machine
is the VIA. That is why this example has its own, and why it is built from this
directory.

## Building

```sh
cd examples/symon
cargo run --release --manifest-path ../../Cargo.toml -- symon_monitor.wr
cargo run --release --manifest-path ../../Cargo.toml --bin flatasm -- \
    symon_monitor.asm -o symon.rom --start 0xC000 --end 0xFFFF
```

Or, with the two binaries already built and on `$PATH`:

```sh
cd examples/symon
wraith symon_monitor.wr
flatasm symon_monitor.asm -o symon.rom --start 0xC000 --end 0xFFFF
```

`flatasm` prints the reset vector it resolved, which is the quickest check that
the image is the right slice: it should be an address in `$C000-$FFFF`.

```
wrote symon.rom: 16384 bytes ($C000-$FFFF); reset vector $FFFC -> $CB98
```

The `--start`/`--end` slice matters. `--rom` emits `$8000-$FFFF`, which is the
right shape for a board with 32 KB of ROM and the wrong shape for this one.

## Running

Load `symon.rom` into Symon with **File → Load ROM…**, then reset the machine.
The banner appears in the terminal window and, identically, in the CRT window.

```
Wraith on Symon
6551 console, 6522 tick, 6545 display
HELP for commands

>
```

| Command | |
|---|---|
| `HELP` | list the commands |
| `CLS` | clear the screen |
| `ECHO <text>` | print text |
| `PEEK <addr>` | read one byte, hex address |
| `POKE <addr> <bb>` | write one byte |
| `DUMP <addr>` | 64 bytes, hex and characters |
| `GO <addr>` | jump to an address |
| `TICKS` | the timer-1 tick count, or a note that the timer is not running |
| `BOUNCE` | a character bouncing off the edges; any key stops it |
| `MARQUEE <text>` | text scrolling across the middle row; any key stops it |

Commands are case-insensitive and addresses are hex without a prefix
(`dump c000`). `POKE`, `DUMP` and `GO` reach anywhere, so `poke 7000 41` puts an
`A` in the top-left corner of the display and `dump 7000` reads the screen back.

## What it exercises

Three things a real machine asks for that a language can get wrong:

**Two interrupt sources, one handler.** The ACIA raises IRQ when a byte
arrives; a 6522's timer 1 raises it about forty times a second. `#[irq] fn
on_irq` tells them apart from their status registers and shares everything it
learns through `static`s — the only channel available, because locals live in
call-graph-colored frames that a handler cannot see.

Symon's VIA, though, is a skeleton: [`Via6522.java`][via] answers every read
with `0` and drops every write, both marked `// TODO: Implement`. A half-present
peripheral is worse than an absent one, because a register that always reads
zero is indistinguishable from a timer that has not yet timed out — so a delay
loop waiting on the tick counter waits forever, and the machine appears to hang
on the first frame of an animation. `timer_init` therefore *probes*: it starts
the timer, waits about five periods for the first tick, and records what it
found. `delay_ticks` sleeps on the counter when there is one and counts when
there is not, and `TICKS` says which. An eighth of a second at boot turns a
lockup into a slower clock, which is the trade a ROM should make.

[via]: https://github.com/sethm/symon/blob/master/src/main/java/com/loomcom/symon/devices/Via6522.java

**A receive ring the handler and the foreground both touch.** `RX_HEAD` and
`RX_TAIL` have one writer each, so the ordinary path needs no lock. The
exception is the 6551's own design: reading its status register clears its
interrupt flag, so the polled transmit loop can cancel the notification for a
byte that has already arrived. `ser_putc` collects such a byte itself, inside
`disable_interrupts()`/`enable_interrupts()`, which is exactly the critical
section the language specification says a shared `static` needs — here it is,
in the one place the hardware actually forces it.

**A frame buffer addressed without a multiply.**

```rust
const ROW_ADDR: [u16; 25] = [|i| => VIDEO + (i as u16) * (COLS as u16)];
```

Fifty bytes of ROM, computed at compile time, so `vpoke` is a table lookup and
an add rather than a 16-bit multiply on every character. The count comes from
the type, so 25 is written once.

It also happens to be a fair size test: the whole monitor — two drivers, a line
editor, a hex parser, ten commands and two animations — is about 6.2 KB of the
12 KB `CODE` section and 0.6 KB of `DATA`.

## Where the guesses are

The program was written against the Symon documentation and checked on an
emulator built from it, not against the simulator itself, so three things are
worth a look if the display misbehaves:

- **Geometry.** `crtc_init` writes the whole 6545 register file from
  `CRTC_SETUP`, including 80 columns (R1), 25 rows (R6) and a display start of
  `$7000` (R12/R13). Those match `COLS`, `ROWS` and `VIDEO` at the top of the
  source; if your Symon build presents a different default panel, change the
  three constants and the corresponding table entries together.
- **The cursor.** `crt_cursor` writes the full 16-bit address into R14/R15. A
  6545 masks R14 to six bits, so a board with a narrower display bus would want
  the offset from `VIDEO` there instead of the absolute address.
- **The tick rate.** `T1_PERIOD` is 25000 cycles, which is 25 ms at 1 MHz — but
  on Symon the fallback does the work instead, and `SPIN_PER_TICK` is counted
  off the generated assembly at 73 cycles an iteration rather than measured on
  the simulator. If `BOUNCE` and `MARQUEE` run too fast or too slow for your
  Symon's clock setting, that constant is the dial.
