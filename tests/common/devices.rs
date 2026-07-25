//! Memory-mapped device models for the execution harness.
//!
//! Plain RAM cannot express hardware: reading a UART's receive register must
//! consume a byte and clear its "data ready" flag, a transmit write must be
//! captured, and a timer must count down and assert IRQ. These models give the
//! emulator that behaviour so drivers can be exercised end to end rather than
//! only inspected as assembly.
//!
//! Devices are decoded by address range. Anything not claimed by a device is
//! ordinary RAM.

use std::collections::VecDeque;

// ---------------------------------------------------------------------------
// TL16C550 UART
// ---------------------------------------------------------------------------

/// Register offsets from the UART base (DLAB=0 unless noted).
pub mod uart_reg {
    /// Receiver buffer (read) / transmitter holding (write).
    pub const RBR_THR: u16 = 0;
    /// Interrupt enable (DLAB=0) / divisor latch high (DLAB=1).
    pub const IER_DLM: u16 = 1;
    /// Interrupt identification (read) / FIFO control (write).
    pub const IIR_FCR: u16 = 2;
    /// Line control (bit 7 is DLAB).
    pub const LCR: u16 = 3;
    /// Modem control.
    pub const MCR: u16 = 4;
    /// Line status.
    pub const LSR: u16 = 5;
    /// Modem status.
    pub const MSR: u16 = 6;
    /// Scratch.
    pub const SCR: u16 = 7;
}

/// Line-status bits.
pub const LSR_DATA_READY: u8 = 0x01;
/// Transmitter holding register empty.
pub const LSR_THR_EMPTY: u8 = 0x20;
/// Transmitter empty (shift register also idle).
pub const LSR_TX_EMPTY: u8 = 0x40;
/// Interrupt-enable bit: received-data-available.
pub const IER_RX_AVAIL: u8 = 0x01;
/// Line-control bit: divisor latch access.
pub const LCR_DLAB: u8 = 0x80;

/// A TL16C550-style UART with receive and transmit FIFOs.
///
/// The test feeds bytes in with [`Uart::feed`]; the program reads them from the
/// receive register, which consumes them and clears `LSR` data-ready when the
/// FIFO empties. Bytes the program writes are captured in [`Uart::tx`].
pub struct Uart {
    pub base: u16,
    /// Bytes waiting to be read by the program.
    rx: VecDeque<u8>,
    /// Bytes the program has transmitted.
    pub tx: Vec<u8>,
    ier: u8,
    lcr: u8,
    mcr: u8,
    scr: u8,
    /// Divisor latch (written when DLAB is set); recorded so a driver's baud
    /// configuration can be asserted.
    pub divisor: u16,
}

impl Uart {
    pub fn new(base: u16) -> Self {
        Self {
            base,
            rx: VecDeque::new(),
            tx: Vec::new(),
            ier: 0,
            lcr: 0,
            mcr: 0,
            scr: 0,
            divisor: 0,
        }
    }

    /// Queue bytes for the program to receive.
    pub fn feed(&mut self, bytes: &[u8]) {
        self.rx.extend(bytes.iter().copied());
    }

    /// True when the device is asserting its interrupt line: receive data is
    /// available and the corresponding interrupt is enabled.
    pub fn irq_asserted(&self) -> bool {
        !self.rx.is_empty() && (self.ier & IER_RX_AVAIL) != 0
    }

    fn dlab(&self) -> bool {
        (self.lcr & LCR_DLAB) != 0
    }

    fn read(&mut self, off: u16) -> u8 {
        match off {
            uart_reg::RBR_THR => {
                if self.dlab() {
                    (self.divisor & 0xFF) as u8
                } else {
                    // Reading consumes a byte; an empty FIFO reads as 0.
                    self.rx.pop_front().unwrap_or(0)
                }
            }
            uart_reg::IER_DLM => {
                if self.dlab() {
                    (self.divisor >> 8) as u8
                } else {
                    self.ier
                }
            }
            // Interrupt identification: bit 0 clear means "interrupt pending".
            uart_reg::IIR_FCR => {
                if self.irq_asserted() {
                    0x04 // received data available
                } else {
                    0x01 // no interrupt pending
                }
            }
            uart_reg::LCR => self.lcr,
            uart_reg::MCR => self.mcr,
            // Transmit is instantaneous here, so the transmitter always reports
            // ready; data-ready reflects the receive FIFO.
            uart_reg::LSR => {
                let mut v = LSR_THR_EMPTY | LSR_TX_EMPTY;
                if !self.rx.is_empty() {
                    v |= LSR_DATA_READY;
                }
                v
            }
            uart_reg::MSR => 0xB0, // CTS/DSR/DCD asserted, a sane idle value
            uart_reg::SCR => self.scr,
            _ => 0,
        }
    }

    fn write(&mut self, off: u16, value: u8) {
        match off {
            uart_reg::RBR_THR => {
                if self.dlab() {
                    self.divisor = (self.divisor & 0xFF00) | value as u16;
                } else {
                    self.tx.push(value);
                }
            }
            uart_reg::IER_DLM => {
                if self.dlab() {
                    self.divisor = (self.divisor & 0x00FF) | ((value as u16) << 8);
                } else {
                    self.ier = value;
                }
            }
            // FIFO control: bits 1/2 reset the receive/transmit FIFOs.
            uart_reg::IIR_FCR => {
                if value & 0x02 != 0 {
                    self.rx.clear();
                }
                if value & 0x04 != 0 {
                    self.tx.clear();
                }
            }
            uart_reg::LCR => self.lcr = value,
            uart_reg::MCR => self.mcr = value,
            uart_reg::SCR => self.scr = value,
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// 6522 VIA
// ---------------------------------------------------------------------------

/// Register offsets from the VIA base.
pub mod via_reg {
    pub const PORTB: u16 = 0x0;
    pub const PORTA: u16 = 0x1;
    pub const DDRB: u16 = 0x2;
    pub const DDRA: u16 = 0x3;
    pub const T1CL: u16 = 0x4;
    pub const T1CH: u16 = 0x5;
    pub const T1LL: u16 = 0x6;
    pub const T1LH: u16 = 0x7;
    pub const T2CL: u16 = 0x8;
    pub const T2CH: u16 = 0x9;
    pub const SR: u16 = 0xA;
    pub const ACR: u16 = 0xB;
    pub const PCR: u16 = 0xC;
    pub const IFR: u16 = 0xD;
    pub const IER: u16 = 0xE;
    pub const PORTA_NH: u16 = 0xF;
}

/// Interrupt flag/enable bit for Timer 1.
pub const VIA_IRQ_T1: u8 = 0x40;
/// Interrupt flag/enable bit for CA1 (used here as a keyboard strobe).
pub const VIA_IRQ_CA1: u8 = 0x02;
/// IFR bit 7 reads as "some enabled interrupt is asserted".
pub const VIA_IRQ_ANY: u8 = 0x80;

/// A 6522 VIA: two ports, Timer 1, and the interrupt flag/enable registers.
///
/// Timer 1 counts down with CPU cycles (advanced by [`Via::tick`]) and sets its
/// interrupt flag at underflow, reloading from the latch in free-run mode.
pub struct Via {
    pub base: u16,
    /// Value external hardware presents on port A (e.g. a keyboard scancode).
    pub port_a_in: u8,
    /// Value external hardware presents on port B.
    pub port_b_in: u8,
    /// Last value the program drove onto port A / port B.
    pub port_a_out: u8,
    pub port_b_out: u8,
    ddra: u8,
    ddrb: u8,
    t1_counter: u16,
    t1_latch: u16,
    acr: u8,
    pcr: u8,
    pub ifr: u8,
    ier: u8,
}

impl Via {
    pub fn new(base: u16) -> Self {
        Self {
            base,
            port_a_in: 0,
            port_b_in: 0,
            port_a_out: 0,
            port_b_out: 0,
            ddra: 0,
            ddrb: 0,
            t1_counter: 0,
            t1_latch: 0,
            acr: 0,
            pcr: 0,
            ifr: 0,
            ier: 0,
        }
    }

    /// Raise the CA1 interrupt flag, as an external strobe would (a key press).
    pub fn strobe_ca1(&mut self) {
        self.ifr |= VIA_IRQ_CA1;
    }

    /// Advance Timer 1 by `cycles`, setting its interrupt flag on underflow.
    pub fn tick(&mut self, cycles: u16) {
        let (next, underflowed) = self.t1_counter.overflowing_sub(cycles);
        if underflowed {
            self.ifr |= VIA_IRQ_T1;
            // Free-run mode (ACR bit 6) reloads from the latch; one-shot stops.
            self.t1_counter = if self.acr & 0x40 != 0 { self.t1_latch } else { 0 };
        } else {
            self.t1_counter = next;
        }
    }

    /// True when an enabled interrupt source is flagged.
    pub fn irq_asserted(&self) -> bool {
        (self.ifr & self.ier & 0x7F) != 0
    }

    fn read(&mut self, off: u16) -> u8 {
        match off {
            // Reading a port returns driven bits for outputs and the external
            // value for inputs, per the data-direction register.
            via_reg::PORTB => (self.port_b_out & self.ddrb) | (self.port_b_in & !self.ddrb),
            via_reg::PORTA | via_reg::PORTA_NH => {
                // Reading port A clears the CA1 flag, as the real device does.
                if off == via_reg::PORTA {
                    self.ifr &= !VIA_IRQ_CA1;
                }
                (self.port_a_out & self.ddra) | (self.port_a_in & !self.ddra)
            }
            via_reg::DDRB => self.ddrb,
            via_reg::DDRA => self.ddra,
            via_reg::T1CL => {
                // Reading the low counter clears the Timer 1 flag.
                self.ifr &= !VIA_IRQ_T1;
                (self.t1_counter & 0xFF) as u8
            }
            via_reg::T1CH => (self.t1_counter >> 8) as u8,
            via_reg::T1LL => (self.t1_latch & 0xFF) as u8,
            via_reg::T1LH => (self.t1_latch >> 8) as u8,
            via_reg::ACR => self.acr,
            via_reg::PCR => self.pcr,
            via_reg::IFR => {
                let mut v = self.ifr;
                if self.irq_asserted() {
                    v |= VIA_IRQ_ANY;
                }
                v
            }
            via_reg::IER => self.ier | 0x80,
            _ => 0,
        }
    }

    fn write(&mut self, off: u16, value: u8) {
        match off {
            via_reg::PORTB => self.port_b_out = value,
            via_reg::PORTA | via_reg::PORTA_NH => {
                if off == via_reg::PORTA {
                    self.ifr &= !VIA_IRQ_CA1;
                }
                self.port_a_out = value;
            }
            via_reg::DDRB => self.ddrb = value,
            via_reg::DDRA => self.ddra = value,
            via_reg::T1CL | via_reg::T1LL => {
                self.t1_latch = (self.t1_latch & 0xFF00) | value as u16;
            }
            // Writing the high counter latches, loads and starts the timer.
            via_reg::T1CH => {
                self.t1_latch = (self.t1_latch & 0x00FF) | ((value as u16) << 8);
                self.t1_counter = self.t1_latch;
                self.ifr &= !VIA_IRQ_T1;
            }
            via_reg::T1LH => self.t1_latch = (self.t1_latch & 0x00FF) | ((value as u16) << 8),
            via_reg::ACR => self.acr = value,
            via_reg::PCR => self.pcr = value,
            // Writing IFR clears the flags whose bits are set.
            via_reg::IFR => self.ifr &= !(value & 0x7F),
            // Bit 7 set means "enable the bits written"; clear means disable.
            via_reg::IER => {
                if value & 0x80 != 0 {
                    self.ier |= value & 0x7F;
                } else {
                    self.ier &= !(value & 0x7F);
                }
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Device set
// ---------------------------------------------------------------------------

/// The devices attached to a test machine. Addresses outside every device's
/// range fall through to RAM.
#[derive(Default)]
pub struct Devices {
    pub uart: Option<Uart>,
    pub via: Option<Via>,
}

impl Devices {
    /// Attach a TL16C550 UART at `base` (occupies 8 registers).
    pub fn with_uart(mut self, base: u16) -> Self {
        self.uart = Some(Uart::new(base));
        self
    }

    /// Attach a 6522 VIA at `base` (occupies 16 registers).
    pub fn with_via(mut self, base: u16) -> Self {
        self.via = Some(Via::new(base));
        self
    }

    /// Route a read to a device, or None if the address is plain RAM.
    pub fn read(&mut self, addr: u16) -> Option<u8> {
        if let Some(u) = &mut self.uart
            && (u.base..u.base + 8).contains(&addr)
        {
            return Some(u.read(addr - u.base));
        }
        if let Some(v) = &mut self.via
            && (v.base..v.base + 16).contains(&addr)
        {
            return Some(v.read(addr - v.base));
        }
        None
    }

    /// Route a write to a device; returns false if the address is plain RAM.
    pub fn write(&mut self, addr: u16, value: u8) -> bool {
        if let Some(u) = &mut self.uart
            && (u.base..u.base + 8).contains(&addr)
        {
            u.write(addr - u.base, value);
            return true;
        }
        if let Some(v) = &mut self.via
            && (v.base..v.base + 16).contains(&addr)
        {
            v.write(addr - v.base, value);
            return true;
        }
        false
    }

    /// True when any attached device is asserting the shared IRQ line.
    pub fn irq_asserted(&self) -> bool {
        self.uart.as_ref().is_some_and(|u| u.irq_asserted())
            || self.via.as_ref().is_some_and(|v| v.irq_asserted())
    }
}
