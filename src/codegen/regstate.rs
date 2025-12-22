//! Register State Tracking
//!
//! Tracks the current contents of the 6502 registers (A, X, Y)
//! to avoid redundant loads when a value is already in a register.

/// Represents a value that might be in a register
#[derive(Debug, Clone, PartialEq)]
pub enum RegisterValue {
    /// Immediate value (literal)
    Immediate(i64),
    /// Variable at a memory address
    Variable(u16),
    /// Zero page variable
    ZeroPage(u8),
    /// Unknown/untracked value
    Unknown,
}

/// Tracks the current state of all registers
#[derive(Debug, Clone)]
pub struct RegisterState {
    /// Current value in A register
    pub a_reg: RegisterValue,
    /// Current value in X register
    pub x_reg: RegisterValue,
    /// Current value in Y register
    pub y_reg: RegisterValue,
}

impl Default for RegisterState {
    fn default() -> Self {
        Self::new()
    }
}

impl RegisterState {
    /// Create a new register state with all registers unknown
    pub fn new() -> Self {
        Self {
            a_reg: RegisterValue::Unknown,
            x_reg: RegisterValue::Unknown,
            y_reg: RegisterValue::Unknown,
        }
    }

    /// Mark all registers as unknown (invalidate all tracking)
    pub fn invalidate_all(&mut self) {
        self.a_reg = RegisterValue::Unknown;
        self.x_reg = RegisterValue::Unknown;
        self.y_reg = RegisterValue::Unknown;
    }

    /// Set the value in A register
    pub fn set_a(&mut self, value: RegisterValue) {
        self.a_reg = value;
    }

    /// Set the value in X register
    pub fn set_x(&mut self, value: RegisterValue) {
        self.x_reg = value;
    }

    /// Set the value in Y register
    pub fn set_y(&mut self, value: RegisterValue) {
        self.y_reg = value;
    }

    /// Check if A register contains a specific value
    pub fn a_contains(&self, value: &RegisterValue) -> bool {
        &self.a_reg == value
    }

    /// Check if we know what's in A register
    pub fn a_is_known(&self) -> bool {
        !matches!(self.a_reg, RegisterValue::Unknown)
    }

    /// Invalidate all register contents that reference a memory location
    /// Called when memory is modified (STA, STX, STY)
    pub fn invalidate_memory(&mut self, addr: u16) {
        if matches!(self.a_reg, RegisterValue::Variable(a) if a == addr) {
            self.a_reg = RegisterValue::Unknown;
        }
        if matches!(self.x_reg, RegisterValue::Variable(a) if a == addr) {
            self.x_reg = RegisterValue::Unknown;
        }
        if matches!(self.y_reg, RegisterValue::Variable(a) if a == addr) {
            self.y_reg = RegisterValue::Unknown;
        }
    }

    /// Invalidate all register contents that reference a zero page location
    pub fn invalidate_zero_page(&mut self, addr: u8) {
        if matches!(self.a_reg, RegisterValue::ZeroPage(a) if a == addr) {
            self.a_reg = RegisterValue::Unknown;
        }
        if matches!(self.x_reg, RegisterValue::ZeroPage(a) if a == addr) {
            self.x_reg = RegisterValue::Unknown;
        }
        if matches!(self.y_reg, RegisterValue::ZeroPage(a) if a == addr) {
            self.y_reg = RegisterValue::Unknown;
        }
    }

    /// Transfer A to X (TAX instruction)
    pub fn transfer_a_to_x(&mut self) {
        self.x_reg = self.a_reg.clone();
    }

    /// Transfer A to Y (TAY instruction)
    pub fn transfer_a_to_y(&mut self) {
        self.y_reg = self.a_reg.clone();
    }

    /// Transfer X to A (TXA instruction)
    pub fn transfer_x_to_a(&mut self) {
        self.a_reg = self.x_reg.clone();
    }

    /// Transfer Y to A (TYA instruction)
    pub fn transfer_y_to_a(&mut self) {
        self.a_reg = self.y_reg.clone();
    }

    /// Mark A as containing an unknown value (after operations like ADC, SBC, etc.)
    pub fn modify_a(&mut self) {
        self.a_reg = RegisterValue::Unknown;
    }

    /// Mark X as containing an unknown value
    pub fn modify_x(&mut self) {
        self.x_reg = RegisterValue::Unknown;
    }

    /// Mark Y as containing an unknown value
    pub fn modify_y(&mut self) {
        self.y_reg = RegisterValue::Unknown;
    }
}
