//! Execution test harness: compile a Wraith program, assemble its output to a
//! flat 64 KB image, run it on a real 6502 emulator, and read back memory.
//!
//! This is what lets tests assert on *runtime behavior* (does `0 <= 0` actually
//! evaluate to 1?) instead of just on generated assembly strings. It is fully
//! self-contained: a small flat assembler for the compiler's exact output
//! dialect (`.ORG`/`.BYTE`/`.WORD`/`.RES`, standard addressing modes) plus the
//! `mos6502` emulator crate - no external assembler/linker required.
//!
//! Shared test infrastructure: this module is compiled into several test
//! binaries but only exercised by some, so unused-item warnings are expected.
#![allow(dead_code)]

use mos6502::cpu::CPU;
use mos6502::instruction::{Mos65C02, W65C02S};
use mos6502::memory::Bus;

use super::compile_success;

// ============================================================================
// Flat 6502 assembler
// ============================================================================
//
// The assembler lives in the library (`wraith::asm`) so the compiler binaries
// (e.g. `flatasm`) and this test harness share one implementation. The harness
// only ever feeds it well-formed compiler output, so a rejected image is a bug
// in codegen or the assembler, not bad input -> unwrap.

/// Assemble the compiler's asm text into a flat 64 KB image.
pub fn assemble(asm: &str) -> [u8; 65536] {
    wraith::asm::assemble(asm).expect("assembler rejected compiler output")
}

// ============================================================================
// Execution harness
// ============================================================================

/// A flat 64 KB RAM bus with software-controllable IRQ/NMI lines. The crate's
/// default `Memory` bus never asserts an interrupt, so tests that exercise
/// `#[irq]`/`#[nmi]` handlers use this to pulse the lines under their control.
pub struct TestBus {
    ram: Vec<u8>,
    irq: bool,
    nmi: bool,
    /// Address ranges treated as ROM: a store into one panics, because on
    /// real hardware it silently does nothing. Matches the default config's
    /// CODE and DATA sections (where the compiler places code and const
    /// data); a program has no business writing either.
    rom_ranges: Vec<(u16, u16)>,
    /// Memory-mapped devices. Addresses they claim bypass RAM entirely, so a
    /// read can have side effects (consuming a FIFO byte, clearing a flag).
    pub devices: super::devices::Devices,
}

impl TestBus {
    fn new() -> Self {
        Self {
            ram: vec![0u8; 65536],
            irq: false,
            nmi: false,
            rom_ranges: vec![(0x8000, 0xBFFF), (0xD000, 0xEFFF)],
            devices: super::devices::Devices::default(),
        }
    }
}

impl Bus for TestBus {
    fn get_byte(&mut self, address: u16) -> u8 {
        match self.devices.read(address) {
            Some(v) => v,
            None => self.ram[address as usize],
        }
    }
    fn set_byte(&mut self, address: u16, value: u8) {
        if self.devices.write(address, value) {
            return;
        }
        assert!(
            !self
                .rom_ranges
                .iter()
                .any(|&(lo, hi)| (lo..=hi).contains(&address)),
            "store into ROM at ${address:04X} — a no-op on real hardware that \
             the emulator's flat RAM would silently accept"
        );
        self.ram[address as usize] = value;
    }
    fn irq_pending(&mut self) -> bool {
        // Either the harness pulsed the line, or a device is asserting it.
        self.irq || self.devices.irq_asserted()
    }
    fn nmi_pending(&mut self) -> bool {
        self.nmi
    }
}

/// Result of running a compiled Wraith program on the emulator.
pub struct Exec {
    cpu: CPU<TestBus, W65C02S>,
    /// Address of the terminating `loop {}` (JMP-to-self) the program settled
    /// into. Interrupt pulses run the handler and return control here.
    idle_pc: u16,
    pub steps: usize,
    pub halted: bool,
}

impl Exec {
    /// The attached devices, for feeding input and inspecting captured output.
    pub fn devices(&mut self) -> &mut super::devices::Devices {
        &mut self.cpu.memory.devices
    }

    /// Bytes the program transmitted through the UART, as a string.
    pub fn uart_output(&mut self) -> String {
        let tx = &self
            .cpu
            .memory
            .devices
            .uart
            .as_ref()
            .expect("no UART attached")
            .tx;
        String::from_utf8_lossy(tx).to_string()
    }

    /// Queue bytes for the program to receive on the UART.
    pub fn uart_feed(&mut self, bytes: &[u8]) {
        self.cpu
            .memory
            .devices
            .uart
            .as_mut()
            .expect("no UART attached")
            .feed(bytes);
    }

    /// Run up to `max_steps` more instructions, stopping early if the program
    /// settles back into a JMP-to-self idle loop. Used after feeding a device so
    /// the driver can observe the new state.
    pub fn resume(&mut self, max_steps: usize) {
        for _ in 0..max_steps {
            let pc_before = self.cpu.registers.program_counter;
            if !self.cpu.single_step() {
                break;
            }
            if self.cpu.registers.program_counter == pc_before {
                self.idle_pc = pc_before;
                break;
            }
        }
    }

    /// Read a byte of memory after execution.
    pub fn mem(&mut self, addr: u16) -> u8 {
        self.cpu.memory.get_byte(addr)
    }

    /// Read a little-endian 16-bit word after execution.
    pub fn mem16(&mut self, addr: u16) -> u16 {
        u16::from_le_bytes([self.mem(addr), self.mem(addr.wrapping_add(1))])
    }

    pub fn a(&self) -> u8 {
        self.cpu.registers.accumulator
    }
    pub fn x(&self) -> u8 {
        self.cpu.registers.index_x
    }
    pub fn y(&self) -> u8 {
        self.cpu.registers.index_y
    }

    /// Assert the IRQ line for exactly one servicing: with the program paused in
    /// its idle loop, take the interrupt, run the handler, and resume at idle.
    /// The program must have enabled IRQs (e.g. `asm { "CLI" }`) for this to fire.
    pub fn pulse_irq(&mut self) {
        self.pulse(true);
    }

    /// Assert the NMI line for one edge-triggered servicing.
    pub fn pulse_nmi(&mut self) {
        self.pulse(false);
    }

    /// Hold the IRQ line asserted for a while and report whether it stayed
    /// masked (the CPU never left the idle loop). Used to verify the I-flag
    /// blocks IRQs. Deasserts the line before returning.
    pub fn irq_stays_masked(&mut self) -> bool {
        let idle = self.idle_pc;
        self.cpu.memory.irq = true;
        let mut masked = true;
        for _ in 0..2000 {
            self.cpu.single_step();
            if self.cpu.registers.program_counter != idle {
                masked = false;
                break;
            }
        }
        self.cpu.memory.irq = false;
        masked
    }

    fn pulse(&mut self, irq: bool) {
        let idle = self.idle_pc;
        let mut budget = 200_000usize;

        // Assert the line and step until the interrupt is taken (PC leaves the
        // idle loop). NMI is non-maskable; IRQ needs the I-flag clear.
        if irq {
            self.cpu.memory.irq = true;
        } else {
            self.cpu.memory.nmi = true;
        }
        while self.cpu.registers.program_counter == idle && budget > 0 {
            self.cpu.single_step();
            budget -= 1;
        }
        assert!(
            self.cpu.registers.program_counter != idle,
            "interrupt was never serviced (IRQ needs `CLI`, or no handler is installed)"
        );

        // Deassert so a level-triggered IRQ fires exactly once, and an NMI edge
        // can re-arm for the next pulse.
        if irq {
            self.cpu.memory.irq = false;
        } else {
            self.cpu.memory.nmi = false;
        }

        // Run the handler to completion (RTI returns to the idle loop).
        while self.cpu.registers.program_counter != idle && budget > 0 {
            self.cpu.single_step();
            budget -= 1;
        }
        assert!(
            budget > 0,
            "interrupt handler did not return to the idle loop"
        );
    }
}

/// Compile, assemble, and run a Wraith program to completion.
///
/// Execution starts at the reset vector ($FFFC) and runs until the program
/// reaches its terminating `loop {}` (a `JMP` to its own address, detected as
/// the program counter no longer advancing) or a step budget is exhausted.
/// Panics if the program does not halt within the budget.
pub fn run(source: &str) -> Exec {
    run_with_devices(source, super::devices::Devices::default())
}

/// Compile and run a program against a machine with the given memory-mapped
/// devices attached. Device reads/writes have real side effects (FIFOs drain,
/// flags clear, timers count), so drivers can be exercised end to end.
///
/// Like [`run`], this panics if the program does not halt within the step
/// budget. A driver that legitimately spins waiting on a device the test
/// never feeds should use [`run_with_devices_expect_spin`] — silence on a
/// hang is indistinguishable from success otherwise.
pub fn run_with_devices(source: &str, devices: super::devices::Devices) -> Exec {
    run_with_devices_impl(source, devices, true)
}

/// [`run_with_devices`] without the halt assertion, for programs that are
/// *meant* to be left spinning (a driver blocked on a FIFO the test refills
/// via [`Exec::resume`]). Check [`Exec::halted`] yourself.
pub fn run_with_devices_expect_spin(source: &str, devices: super::devices::Devices) -> Exec {
    run_with_devices_impl(source, devices, false)
}

fn run_with_devices_impl(
    source: &str,
    devices: super::devices::Devices,
    require_halt: bool,
) -> Exec {
    let asm = compile_success(source);
    let image = assemble(&asm);

    let mut bus = TestBus::new();
    bus.devices = devices;
    // Load the full 64 KB image directly: the default Bus::set_bytes truncates
    // its length to u16 (65536 -> 0), so it would copy nothing.
    bus.ram.copy_from_slice(&image);
    let mut cpu = CPU::new(bus, Mos65C02::<true, true>);
    cpu.reset();

    const BUDGET: usize = 20_000_000;
    let mut steps = 0usize;
    let mut halted = false;
    let mut idle_pc = 0u16;
    while steps < BUDGET {
        let pc_before = cpu.registers.program_counter;
        let executed = cpu.single_step();
        steps += 1;
        if !executed {
            // Couldn't decode an instruction (bad opcode) or a wait state.
            halted = true;
            idle_pc = cpu.registers.program_counter;
            break;
        }
        if cpu.registers.program_counter == pc_before {
            // JMP-to-self: the program's terminating `loop {}`.
            halted = true;
            idle_pc = pc_before;
            break;
        }
    }
    assert!(
        halted || !require_halt,
        "program did not halt within {} steps (possible infinite loop or missing `loop {{}}`; \
         if it is meant to spin on a device, use run_with_devices_expect_spin)",
        BUDGET
    );

    Exec {
        cpu,
        idle_pc,
        steps,
        halted,
    }
}
