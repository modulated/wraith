//! Assembly Emitter
//!
//! Helper for generating formatted 6502 assembly code.

use super::CommentVerbosity;
use super::memory_layout::{MemoryLayout, TempAllocator};
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
    pub label_counter: usize,
    pub match_counter: u32,
    pub memory_layout: MemoryLayout,
    /// Register state tracking for optimization
    pub reg_state: RegisterState,
    /// Temporary storage allocator for zero-page temps
    pub temp_alloc: TempAllocator,
    /// Stack of loop contexts for break/continue
    loop_stack: Vec<LoopContext>,
    /// Inline depth tracking (>0 means we're generating inline code)
    inline_depth: u32,
    /// Suffix for uniquifying labels in current inline expansion
    inline_label_suffix: Option<usize>,
    /// Current byte count (tracks code size during generation)
    byte_count: u16,
    /// Track if the last instruction was a terminal instruction (RTS, RTI, or unconditional JMP)
    last_was_terminal: bool,
    /// Comment verbosity level
    pub verbosity: CommentVerbosity,
    /// Current function being generated (for tail call detection)
    current_function: Option<String>,
    /// Track if mul16 stdlib function is needed
    pub needs_mul16: bool,
    /// Track if div16 stdlib function is needed
    pub needs_div16: bool,
    /// Track if mod16 stdlib function is needed
    pub needs_mod16: bool,
    /// Track if the indirect-call trampoline is needed (function pointers)
    pub needs_indirect_call: bool,
}

impl Default for Emitter {
    fn default() -> Self {
        Self::new(CommentVerbosity::Normal)
    }
}

impl Emitter {
    pub fn new(verbosity: CommentVerbosity) -> Self {
        Self {
            output: String::with_capacity(4096),
            indent: 0,
            label_counter: 0,
            match_counter: 0,
            memory_layout: MemoryLayout::new(),
            reg_state: RegisterState::new(),
            temp_alloc: TempAllocator::new(),
            loop_stack: Vec::new(),
            inline_depth: 0,
            inline_label_suffix: None,
            byte_count: 0,
            last_was_terminal: false,
            verbosity,
            current_function: None,
            needs_mul16: false,
            needs_div16: false,
            needs_mod16: false,
            needs_indirect_call: false,
        }
    }

    /// Check if verbosity is set to minimal
    pub fn is_minimal(&self) -> bool {
        self.verbosity == CommentVerbosity::Minimal
    }

    /// Check if verbosity is set to verbose
    pub fn is_verbose(&self) -> bool {
        self.verbosity == CommentVerbosity::Verbose
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
        // A label means control flow can continue from elsewhere
        self.last_was_terminal = false;
        // ...and control arriving from elsewhere carries unknown registers, so
        // no cached belief survives a label. Keeping one here silently elides
        // loads: after a loop, a read of a variable the body last stored would
        // reuse whatever the exit branch happened to leave in A.
        self.reg_state.invalidate_all();
    }

    pub fn emit_inst(&mut self, mnemonic: &str, operand: &str) {
        self.output.push_str("    ");
        self.output.push_str(mnemonic);
        if !operand.is_empty() {
            self.output.push(' ');
            self.output.push_str(operand);
        }
        self.output.push('\n');

        // Track byte count
        self.byte_count += Self::instruction_size(mnemonic, operand);

        // Track if this is a terminal instruction (RTS, RTI, or unconditional JMP)
        self.last_was_terminal = matches!(mnemonic, "RTS" | "RTI" | "JMP");
    }

    pub fn emit_comment(&mut self, comment: &str) {
        self.output.push_str("; ");
        self.output.push_str(comment);
        self.output.push('\n');
    }

    pub fn emit_raw(&mut self, line: &str) {
        self.output.push_str(line);
        self.output.push('\n');
    }

    pub fn emit_org(&mut self, address: u16) {
        self.output.push_str(&format!(".ORG ${:04X}\n", address));
    }

    pub fn emit_word(&mut self, value: u16) {
        self.output.push_str(&format!(".WORD ${:04X}\n", value));
        self.byte_count += 2;
    }

    pub fn emit_word_label(&mut self, label: &str) {
        self.output.push_str(&format!(".WORD {}\n", label));
        self.byte_count += 2;
    }

    pub fn emit_byte(&mut self, value: u8) {
        self.output.push_str(&format!(".BYTE ${:02X}\n", value));
        self.byte_count += 1;
    }

    pub fn emit_bytes(&mut self, values: &[u8]) {
        if values.is_empty() {
            return;
        }

        self.output.push_str("    .BYTE ");
        for (i, byte) in values.iter().enumerate() {
            if i > 0 {
                self.output.push_str(", ");
            }
            self.output.push_str(&format!("${:02X}", byte));
        }
        self.output.push('\n');
        self.byte_count += values.len() as u16;
    }

    pub fn finish(mut self) -> String {
        // Ensure the file ends with a newline (Unix text file convention)
        if !self.output.ends_with('\n') {
            self.output.push('\n');
        }
        self.output
    }

    /// Get the current byte count (code size)
    pub fn byte_count(&self) -> u16 {
        self.byte_count
    }

    /// Reset the byte counter (used for measuring individual functions)
    pub fn reset_byte_count(&mut self) {
        self.byte_count = 0;
    }

    /// Current length of the emitted output buffer. Pair with
    /// [`Self::output_since`] to inspect what a nested generation step emitted
    /// (e.g. whether a loop body clobbers a register).
    pub fn output_len(&self) -> usize {
        self.output.len()
    }

    /// The output emitted since the given buffer position.
    pub fn output_since(&self, pos: usize) -> &str {
        &self.output[pos..]
    }

    /// Calculate the size of a 6502 instruction in bytes
    fn instruction_size(mnemonic: &str, operand: &str) -> u16 {
        if operand.is_empty() {
            // Implied or accumulator mode (1 byte)
            match mnemonic {
                "RTS" | "RTI" | "PHA" | "PLA" | "PHP" | "PLP" | "TAX" | "TAY" | "TXA" | "TYA"
                | "TXS" | "TSX" | "INX" | "INY" | "DEX" | "DEY" | "CLC" | "SEC" | "CLI" | "SEI"
                | "CLD" | "SED" | "CLV" | "NOP" | "BRK" | "ASL" | "LSR" | "ROL" | "ROR" => 1,
                _ => 1, // Default for unknown implied
            }
        } else if operand.starts_with('#') {
            // Immediate mode (2 bytes)
            2
        } else if operand.starts_with('(') {
            // Indirect modes
            if operand.contains("),Y") || operand.contains("),y") {
                // Indirect indexed: (zp),Y (2 bytes)
                2
            } else if operand.contains(",X)") || operand.contains(",x)") {
                // Indexed indirect: (zp,X) (2 bytes)
                2
            } else {
                // Indirect: (addr) (3 bytes for JMP)
                3
            }
        } else if operand.contains(",X")
            || operand.contains(",x")
            || operand.contains(",Y")
            || operand.contains(",y")
        {
            // Indexed addressing
            if operand.starts_with('$') && operand.len() <= 4 {
                // $XX format
                // Zero page indexed (2 bytes)
                2
            } else {
                // Absolute indexed (3 bytes)
                3
            }
        } else if mnemonic.starts_with('B') && mnemonic != "BIT" {
            // Branch instructions (2 bytes)
            2
        } else if operand.starts_with('$') {
            // Direct addressing
            let hex_part = operand.trim_start_matches('$');
            if hex_part.len() <= 2 {
                // Zero page (2 bytes)
                2
            } else {
                // Absolute (3 bytes)
                3
            }
        } else {
            // Label reference or symbol - assume 3 bytes (absolute)
            3
        }
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

    /// Load from symbolic address into A (for addr declarations)
    pub fn emit_lda_symbol(&mut self, symbol: &str) {
        self.emit_inst("LDA", symbol);
        // Can't track symbolic addresses precisely, so mark A as unknown
        self.reg_state.modify_a();
    }

    /// Store A to symbolic address (for addr declarations)
    pub fn emit_sta_symbol(&mut self, symbol: &str) {
        self.emit_inst("STA", symbol);
        // Can't track symbolic addresses precisely, so invalidate
        self.reg_state.modify_a();
    }

    /// Invalidate all register tracking (call on branches, function calls, etc.)
    pub fn invalidate_registers(&mut self) {
        self.reg_state.invalidate_all();
    }

    /// Drop any cached belief that a register mirrors this zero-page location.
    /// Use after a raw store (STX/STY) whose destination the emitter can't see.
    pub fn invalidate_zp(&mut self, addr: u8) {
        self.reg_state.invalidate_zero_page(addr);
    }

    /// Drop any cached belief that a register mirrors this absolute location.
    pub fn invalidate_abs(&mut self, addr: u16) {
        self.reg_state.invalidate_memory(addr);
    }

    /// Mark that A register contains an unknown value (after arithmetic, etc.)
    pub fn mark_a_unknown(&mut self) {
        self.reg_state.modify_a();
    }

    // ========================================================================
    // X REGISTER DATA OPERATIONS (use X as temporary data storage)
    // ========================================================================

    /// Save A to X register (TAX) - useful for preserving A during operations
    /// Returns true if TAX was emitted, false if A was already in X
    pub fn emit_tax(&mut self) -> bool {
        if self.reg_state.a_equals_x() {
            // A and X already have the same value, skip TAX
            return false;
        }
        self.emit_inst("TAX", "");
        self.reg_state.transfer_a_to_x();
        true
    }

    /// Restore A from X register (TXA)
    /// Returns true if TXA was emitted, false if X was already in A
    pub fn emit_txa(&mut self) -> bool {
        if self.reg_state.a_equals_x() {
            // A and X already have the same value, skip TXA
            return false;
        }
        self.emit_inst("TXA", "");
        self.reg_state.transfer_x_to_a();
        true
    }

    /// Load immediate value into X
    pub fn emit_ldx_immediate(&mut self, value: u8) {
        let reg_val = RegisterValue::Immediate(value as i64);
        if !self.reg_state.x_contains(&reg_val) {
            self.emit_inst("LDX", &format!("#${:02X}", value));
            self.reg_state.set_x(reg_val);
        }
    }

    /// Load from zero page into X
    pub fn emit_ldx_zp(&mut self, addr: u8) {
        let reg_val = RegisterValue::ZeroPage(addr);
        if !self.reg_state.x_contains(&reg_val) {
            self.emit_inst("LDX", &format!("${:02X}", addr));
            self.reg_state.set_x(reg_val);
        }
    }

    /// Store X to zero page
    pub fn emit_stx_zp(&mut self, addr: u8) {
        self.emit_inst("STX", &format!("${:02X}", addr));
        self.reg_state.invalidate_zero_page(addr);
        self.reg_state.set_x(RegisterValue::ZeroPage(addr));
    }

    /// Mark that X register contains an unknown value
    pub fn mark_x_unknown(&mut self) {
        self.reg_state.modify_x();
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
    // CONTROL FLOW TRACKING
    // ========================================================================

    /// Check if the last instruction was a terminal instruction (RTS, RTI, or JMP)
    /// This is useful to avoid emitting duplicate RTS instructions
    pub fn last_was_terminal(&self) -> bool {
        self.last_was_terminal
    }

    // ========================================================================
    // INLINE CONTEXT MANAGEMENT (for inline function expansion)
    // ========================================================================

    /// Push an inline context (increment depth)
    /// Sets a unique label suffix for this inline expansion
    pub fn push_inline(&mut self) {
        self.inline_depth += 1;
        // Assign a unique suffix for labels in this inline expansion
        self.label_counter += 1;
        self.inline_label_suffix = Some(self.label_counter);
    }

    /// Pop an inline context (decrement depth)
    pub fn pop_inline(&mut self) {
        if self.inline_depth > 0 {
            self.inline_depth -= 1;
        }
        // Clear the label suffix when exiting inline context
        if self.inline_depth == 0 {
            self.inline_label_suffix = None;
        }
    }

    /// Check if we're currently generating inline code
    pub fn is_inlining(&self) -> bool {
        self.inline_depth > 0
    }

    /// Get the current inline label suffix (if inlining)
    pub fn inline_label_suffix(&self) -> Option<usize> {
        self.inline_label_suffix
    }

    // ========================================================================
    // DATA EMISSION METHODS (for const arrays)
    // ========================================================================

    /// Emit a label for data (no formatting, just the label)
    pub fn emit_data_label(&mut self, name: &str) {
        self.output.push_str(name);
        self.output.push_str(":\n");
    }

    /// Emit a data directive (.BYTE, .RES, etc.)
    pub fn emit_data_directive(&mut self, directive: &str) {
        self.output.push_str("    ");
        self.output.push_str(directive);
        self.output.push('\n');
    }

    /// Emit .ORG directive for data placement
    pub fn emit_data_org(&mut self, address: u16) {
        self.output.push_str(&format!(".ORG ${:04X}\n", address));
    }

    // ========================================================================
    // TAIL CALL OPTIMIZATION SUPPORT
    // ========================================================================

    /// Set the current function being generated (for tail call detection)
    pub fn set_current_function(&mut self, name: String) {
        self.current_function = Some(name);
    }

    /// Clear the current function context
    pub fn clear_current_function(&mut self) {
        self.current_function = None;
    }

    /// Get the current function name (if any)
    pub fn current_function(&self) -> Option<&str> {
        self.current_function.as_deref()
    }

    /// Take the current function name, leaving none set.
    ///
    /// Paired with [`Self::restore_current_function`] to swap in a different
    /// function for the duration of a nested body — an inline expansion emits
    /// the callee's body into the caller's output, and lookups scoped by
    /// "current function" have to follow the body, not the surrounding code.
    pub fn take_current_function(&mut self) -> Option<String> {
        self.current_function.take()
    }

    /// Put back a name taken by [`Self::take_current_function`].
    pub fn restore_current_function(&mut self, name: Option<String>) {
        self.current_function = name;
    }

    /// Get the loop restart label for tail recursive functions
    pub fn tail_call_loop_label(&self) -> Option<String> {
        self.current_function
            .as_ref()
            .map(|name| format!("{}_loop_start", name))
    }

    // ========================================================================
    // SOFTWARE STACK FOR PARAMETER PRESERVATION IN RECURSION
    // ========================================================================

    /// Operand text for the software stack base, e.g. `$0200`.
    fn software_stack(&self) -> String {
        format!("${:04X}", self.memory_layout.software_stack_base)
    }

    /// Operand text for the software stack base plus a byte offset.
    fn software_stack_offset(&self, off: u16) -> String {
        format!("${:04X}", self.memory_layout.software_stack_base + off)
    }

    /// Spill a live scalar onto the software stack so it survives evaluation of a
    /// sub-expression that contains a call. `size` is 1 (value in A) or 2 (low in
    /// A, high in Y). This uses the software stack ($0200/$FF), NOT the 6502
    /// hardware stack, and nests correctly (LIFO) with `push_frame`. Clobbers X;
    /// A/Y are preserved.
    pub fn spill_scalar(&mut self, size: u8) {
        self.emit_inst("LDX", "$FF");
        let base = self.software_stack();
        self.emit_inst("STA", &format!("{},X", base)); // low byte / u8 value
        if size >= 2 {
            self.emit_inst("INX", "");
            self.emit_inst("TYA", ""); // A = high byte
            self.emit_inst("STA", &format!("{},X", base));
        }
        self.emit_inst("INX", "");
        self.emit_inst("STX", "$FF");
        // A/Y are not relied upon after a spill (the caller reloads later).
        self.reg_state.invalidate_all();
    }

    /// Reload a scalar previously saved by [`spill_scalar`] into A (and Y if
    /// `size` == 2). Clobbers X.
    pub fn reload_scalar(&mut self, size: u8) {
        self.emit_inst("LDX", "$FF");
        for _ in 0..size {
            self.emit_inst("DEX", "");
        }
        self.emit_inst("STX", "$FF");
        if size >= 2 {
            // High byte was stored second (at higher offset), low byte first.
            let base = self.software_stack();
            let base_hi = self.software_stack_offset(1);
            self.emit_inst("LDA", &format!("{},X", base_hi));
            self.emit_inst("TAY", "");
            self.emit_inst("LDA", &format!("{},X", base));
        } else {
            let base = self.software_stack();
            self.emit_inst("LDA", &format!("{},X", base));
        }
        self.reg_state.invalidate_all();
    }

    /// Push `size` bytes of a function frame (starting at `base`) to the software
    /// stack. The stack grows upward from $0200 with its pointer in $FF. Used to
    /// preserve a callee's frame across a recursive call so re-entry cannot
    /// destroy the values the caller still needs. Clobbers A and X.
    pub fn push_frame(&mut self, base: u8, size: u8) {
        if size == 0 {
            return;
        }
        // Load stack pointer into X, then push `size` bytes.
        self.emit_inst("LDX", "$FF");
        for i in 0..size {
            self.emit_inst("LDA", &format!("${:02X}", base + i));
            let stack = self.software_stack();
            self.emit_inst("STA", &format!("{},X", stack));
            if i + 1 < size {
                self.emit_inst("INX", "");
            }
        }
        // Advance the stack pointer past the pushed block.
        self.emit_inst("INX", "");
        self.emit_inst("STX", "$FF");

        self.reg_state.invalidate_all();
    }

    /// Pop `size` bytes previously pushed by [`push_frame`] back to `base`.
    /// Clobbers A and X (callers must preserve any live return value first).
    pub fn pop_frame(&mut self, base: u8, size: u8) {
        if size == 0 {
            return;
        }
        // Rewind the stack pointer by `size`.
        self.emit_inst("LDX", "$FF");
        for _ in 0..size {
            self.emit_inst("DEX", "");
        }
        self.emit_inst("STX", "$FF");

        // Restore the frame bytes.
        for i in 0..size {
            let stack = self.software_stack();
            self.emit_inst("LDA", &format!("{},X", stack));
            self.emit_inst("STA", &format!("${:02X}", base + i));
            if i + 1 < size {
                self.emit_inst("INX", "");
            }
        }

        self.reg_state.invalidate_all();
    }
}
