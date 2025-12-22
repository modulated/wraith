//! Assembly Emitter
//!
//! Helper for generating formatted 6502 assembly code.

use super::memory_layout::MemoryLayout;
use super::regstate::{RegisterState, RegisterValue};

/// Loop context for break/continue statements
#[derive(Debug, Clone)]
pub struct LoopContext {
    /// Label to jump to for continue (loop start)
    pub continue_label: String,
    /// Label to jump to for break (loop end)
    pub break_label: String,
}

pub struct Emitter {
    output: String,
    #[allow(dead_code)]
    indent: usize,
    label_counter: usize,
    match_counter: u32,
    pub memory_layout: MemoryLayout,
    /// Register state tracking for optimization
    pub reg_state: RegisterState,
    /// Stack of loop contexts for break/continue
    loop_stack: Vec<LoopContext>,
    /// Inline depth tracking (>0 means we're generating inline code)
    inline_depth: u32,
}

impl Default for Emitter {
    fn default() -> Self {
        Self::new()
    }
}

impl Emitter {
    pub fn new() -> Self {
        Self {
            output: String::new(),
            indent: 0,
            label_counter: 0,
            match_counter: 0,
            memory_layout: MemoryLayout::new(),
            reg_state: RegisterState::new(),
            loop_stack: Vec::new(),
            inline_depth: 0,
        }
    }

    pub fn next_label(&mut self, prefix: &str) -> String {
        self.label_counter += 1;
        format!("{}_{}", prefix, self.label_counter)
    }

    pub fn next_match_id(&mut self) -> u32 {
        let id = self.match_counter;
        self.match_counter += 1;
        id
    }

    pub fn emit_label(&mut self, label: &str) {
        self.output.push_str(label);
        self.output.push_str(":\n");
    }

    pub fn emit_inst(&mut self, mnemonic: &str, operand: &str) {
        self.output.push_str("    ");
        self.output.push_str(mnemonic);
        if !operand.is_empty() {
            self.output.push(' ');
            self.output.push_str(operand);
        }
        self.output.push('\n');
    }

    pub fn emit_comment(&mut self, comment: &str) {
        self.output.push_str("    ; ");
        self.output.push_str(comment);
        self.output.push('\n');
    }

    pub fn emit_raw(&mut self, line: &str) {
        self.output.push_str(line);
        self.output.push('\n');
    }

    pub fn emit_org(&mut self, address: u16) {
        self.output.push_str(&format!("    * = ${:04X}\n", address));
    }

    pub fn emit_byte(&mut self, value: u8) {
        self.output.push_str(&format!("    .byte ${:02X}\n", value));
    }

    pub fn emit_bytes(&mut self, values: &[u8]) {
        if values.is_empty() {
            return;
        }

        self.output.push_str("    .byte ");
        for (i, byte) in values.iter().enumerate() {
            if i > 0 {
                self.output.push_str(", ");
            }
            self.output.push_str(&format!("${:02X}", byte));
        }
        self.output.push('\n');
    }

    pub fn emit_word(&mut self, value: u16) {
        // Emit 16-bit value in little-endian format
        let low = (value & 0xFF) as u8;
        let high = ((value >> 8) & 0xFF) as u8;
        self.output.push_str(&format!("    .byte ${:02X}, ${:02X}\n", low, high));
    }

    pub fn finish(self) -> String {
        self.output
    }

    // ========================================================================
    // OPTIMIZED LOAD METHODS (with register state tracking)
    // ========================================================================

    /// Load immediate value into A, skipping if already loaded
    pub fn emit_lda_immediate(&mut self, value: i64) {
        let reg_val = RegisterValue::Immediate(value);
        if !self.reg_state.a_contains(&reg_val) {
            self.emit_inst("LDA", &format!("#${:02X}", value as u8));
            self.reg_state.set_a(reg_val);
        }
        // If already in A, skip the load (optimization!)
    }

    /// Load from zero page into A, skipping if already loaded
    pub fn emit_lda_zp(&mut self, addr: u8) {
        let reg_val = RegisterValue::ZeroPage(addr);
        if !self.reg_state.a_contains(&reg_val) {
            self.emit_inst("LDA", &format!("${:02X}", addr));
            self.reg_state.set_a(reg_val);
        }
    }

    /// Load from absolute address into A, skipping if already loaded
    pub fn emit_lda_abs(&mut self, addr: u16) {
        let reg_val = RegisterValue::Variable(addr);
        if !self.reg_state.a_contains(&reg_val) {
            self.emit_inst("LDA", &format!("${:04X}", addr));
            self.reg_state.set_a(reg_val);
        }
    }

    /// Store A to zero page and update register tracking
    pub fn emit_sta_zp(&mut self, addr: u8) {
        self.emit_inst("STA", &format!("${:02X}", addr));
        // After STA, the memory location now contains what's in A
        // IMPORTANT: A still contains the same value!
        // So we can optimize subsequent LDA of the same address

        // Invalidate if any OTHER register was tracking this location
        self.reg_state.invalidate_zero_page(addr);

        // Now update A to also indicate it matches this memory location
        // This allows LDA from this address to be optimized away
        self.reg_state.set_a(RegisterValue::ZeroPage(addr));

        // Alternative: we could keep the original value if it was an immediate
        // For now, tracking the memory location allows the optimization to work
    }

    /// Store A to absolute address and update register tracking
    pub fn emit_sta_abs(&mut self, addr: u16) {
        self.emit_inst("STA", &format!("${:04X}", addr));

        // Same logic as emit_sta_zp
        self.reg_state.invalidate_memory(addr);
        self.reg_state.set_a(RegisterValue::Variable(addr));
    }

    /// Invalidate all register tracking (call on branches, function calls, etc.)
    pub fn invalidate_registers(&mut self) {
        self.reg_state.invalidate_all();
    }

    /// Mark that A register contains an unknown value (after arithmetic, etc.)
    pub fn mark_a_unknown(&mut self) {
        self.reg_state.modify_a();
    }

    // ========================================================================
    // LOOP CONTEXT MANAGEMENT (for break/continue)
    // ========================================================================

    /// Push a new loop context onto the stack
    pub fn push_loop(&mut self, continue_label: String, break_label: String) {
        self.loop_stack.push(LoopContext {
            continue_label,
            break_label,
        });
    }

    /// Pop the current loop context from the stack
    pub fn pop_loop(&mut self) {
        self.loop_stack.pop();
    }

    /// Get the current loop context (for break/continue)
    pub fn current_loop(&self) -> Option<&LoopContext> {
        self.loop_stack.last()
    }

    // ========================================================================
    // INLINE CONTEXT MANAGEMENT (for inline function expansion)
    // ========================================================================

    /// Push an inline context (increment depth)
    pub fn push_inline(&mut self) {
        self.inline_depth += 1;
    }

    /// Pop an inline context (decrement depth)
    pub fn pop_inline(&mut self) {
        if self.inline_depth > 0 {
            self.inline_depth -= 1;
        }
    }

    /// Check if we're currently generating inline code
    pub fn is_inlining(&self) -> bool {
        self.inline_depth > 0
    }
}
