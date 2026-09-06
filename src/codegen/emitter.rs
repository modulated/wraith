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
    /// Nesting depth of `atomic` access regions. The outermost `enter_atomic`
    /// masks interrupts (`PHP; SEI`) and the matching `exit_atomic` restores
    /// them (`PLP`); inner enters/exits are no-ops, so a read of an atomic
    /// static inside an atomic assignment does not stack redundant guards.
    atomic_depth: u32,
    /// Suffix for uniquifying labels in current inline expansion
    inline_label_suffix: Option<usize>,
    /// End labels of in-progress inline expansions (a stack: inlines nest).
    inline_end_labels: Vec<String>,
    /// Current byte count (tracks code size during generation)
    byte_count: u16,
    /// Track if the last instruction was a terminal instruction (RTS, RTI, or unconditional JMP)
    last_was_terminal: bool,
    /// Comment verbosity level
    pub verbosity: CommentVerbosity,
    /// The CPU being targeted (governs 65C02-only instruction use).
    pub target: crate::codegen::TargetCpu,
    /// Current function being generated (for tail call detection)
    current_function: Option<String>,
    /// Track if mul16 stdlib function is needed
    pub needs_mul16: bool,
    /// Track if div16 stdlib function is needed
    pub needs_div16: bool,
    /// Track if mod16 stdlib function is needed
    pub needs_mod16: bool,
    /// Track if the q8.8 fixed-point multiply routine is needed
    pub needs_mulq88: bool,
    /// Track if the q8.8 fixed-point divide routine is needed
    pub needs_divq88: bool,
    /// Track if the indirect-call trampoline is needed (function pointers)
    pub needs_indirect_call: bool,
    /// Span of the statement currently being generated, so a zero-page pool
    /// that runs out mid-expression can point the diagnostic at the line that
    /// asked for too much at once rather than reporting a spanless internal
    /// error. Set once per statement in `generate_stmt`.
    pub blame_span: Option<crate::ast::Span>,
    /// Zero-page addresses this emitter has written at a statically known
    /// location (a store or read-modify-write to a bare `$NN`). Used to narrow
    /// an interrupt handler's scratch save to what its reachable code touches.
    pub zp_written: [bool; 256],
    /// Set when a write might reach zero page at an address this pass cannot
    /// pin — an indexed or indirect store, an inline `asm` instruction, or a
    /// raw routine body. An interrupt handler whose graph sets this keeps the
    /// full conservative scratch save rather than a narrowed one.
    pub zp_write_opaque: bool,
    /// Symbol name → absolute address, so a store to a named location (a
    /// `const addr` I/O register, which is emitted by name and is `Absolute`
    /// even when its value is in zero page) can be classed as zero-page or not.
    /// Populated only for the interrupt scratch-narrowing pass; empty otherwise,
    /// where a symbol store is treated as opaque and no narrowing is read.
    pub symbol_addrs: std::collections::HashMap<String, u16>,
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
            label_counter: 0,
            match_counter: 0,
            memory_layout: MemoryLayout::new(),
            reg_state: RegisterState::new(),
            temp_alloc: TempAllocator::new(),
            loop_stack: Vec::new(),
            inline_depth: 0,
            atomic_depth: 0,
            inline_label_suffix: None,
            inline_end_labels: Vec::new(),
            byte_count: 0,
            last_was_terminal: false,
            verbosity,
            target: crate::codegen::TargetCpu::default(),
            current_function: None,
            needs_mul16: false,
            needs_div16: false,
            needs_mod16: false,
            needs_mulq88: false,
            needs_divq88: false,
            needs_indirect_call: false,
            blame_span: None,
            zp_written: [false; 256],
            zp_write_opaque: false,
            symbol_addrs: std::collections::HashMap::new(),
        }
    }

    /// Every 6502/65C02 mnemonic that writes memory, so anything else can be
    /// treated as touching no zero page. Stores, read-modify-writes, and the
    /// 65C02 single-bit `RMBn`/`SMBn`. Stack pushes write page one, never zero
    /// page, so they are deliberately absent.
    fn writes_memory(mnemonic: &str) -> bool {
        matches!(
            mnemonic,
            "STA"
                | "STX"
                | "STY"
                | "STZ"
                | "INC"
                | "DEC"
                | "ASL"
                | "LSR"
                | "ROL"
                | "ROR"
                | "TRB"
                | "TSB"
        ) || (mnemonic.len() == 4
            && (mnemonic.starts_with("RMB") || mnemonic.starts_with("SMB"))
            && mnemonic.as_bytes()[3].is_ascii_digit())
    }

    /// Record the zero-page effect of a memory-writing instruction, for
    /// interrupt scratch narrowing. A bare `$NN` operand is a precise zero-page
    /// write; an absolute `$NNNN` or the accumulator touches no zero page; any
    /// other form that writes memory (a zero-page index, an indirect, a symbol)
    /// could land in zero page at an address this pass cannot pin, so it is
    /// recorded as opaque and forces the full save.
    fn note_zp_write(&mut self, operand: &str) {
        // Drop any trailing `; comment` a raw line may carry.
        let op = operand.split(';').next().unwrap_or("").trim();
        if op.is_empty() || op.eq_ignore_ascii_case("A") {
            return; // e.g. `ASL A`: no memory touched.
        }
        if let Some(hex) = op.strip_prefix('$') {
            if hex.len() == 2 {
                if let Ok(addr) = u8::from_str_radix(hex, 16) {
                    self.zp_written[addr as usize] = true;
                    return;
                }
            } else if hex.len() == 4 && u16::from_str_radix(hex, 16).is_ok() {
                return; // absolute: not zero page.
            }
        } else if op.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_') {
            // A bare symbol — a `const addr` I/O register, stored by name. Its
            // recorded address decides whether it lands in zero page.
            match self.symbol_addrs.get(op) {
                Some(&a) if a <= 0xFF => self.zp_written[a as usize] = true,
                Some(_) => {}                        // absolute
                None => self.zp_write_opaque = true, // a symbol we cannot resolve
            }
            return;
        }
        // A zero-page index (`$20,X`), an indirect (`($20),Y`), or anything else
        // that could reach zero page at an address this pass cannot pin.
        self.zp_write_opaque = true;
    }

    /// Build a "zero-page scratch pool exhausted" diagnostic blamed on the
    /// statement currently being generated. `detail` names the pool and where
    /// it ran out (e.g. "in an element address"); the shared hint tells the
    /// author how to rewrite. See [`CodegenError::pool_exhausted`].
    pub fn pool_error(&self, detail: &str) -> crate::codegen::CodegenError {
        crate::codegen::CodegenError::pool_exhausted(detail, self.blame_span)
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

    /// A conditional branch whose target may be arbitrarily far away — past a
    /// body of user code whose size nothing here bounds. A 6502 conditional
    /// branch reaches ±127 bytes, so emit the *inverse* branch over a `JMP`
    /// instead: the hop is always 3 bytes and the reach is the whole address
    /// space.
    ///
    /// Use this wherever the distance depends on how much code the program
    /// contains, and a plain `emit_inst("BEQ", …)` wherever it is fixed (a hop
    /// over two or three instructions this function itself emitted). The
    /// difference is not stylistic: a plain branch over a large body fails at
    /// assembly time, which is a build the programmer cannot fix from the
    /// source.
    ///
    /// `skip` is the label for the fall-through, supplied by the caller because
    /// every current site already names one.
    pub fn emit_branch_far(&mut self, cond: &str, target: &str, skip: &str) {
        let inverse = match cond {
            "BEQ" => "BNE",
            "BNE" => "BEQ",
            "BCC" => "BCS",
            "BCS" => "BCC",
            "BMI" => "BPL",
            "BPL" => "BMI",
            "BVC" => "BVS",
            "BVS" => "BVC",
            other => unreachable!("not an invertible branch: {other}"),
        };
        self.emit_inst(inverse, skip);
        self.emit_inst("JMP", target);
        self.emit_label(skip);
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

        // No belief survives a call: the callee's body is not visible here, and
        // every register and memory location the tracker mirrors may change. The
        // instruction itself touching nothing is not the point — the function it
        // transfers to is (this is the structural form of the bug behind
        // 0e2cd37, and of `x * y + x` eliding the reload of x after JSR mul16).
        if mnemonic == "JSR" {
            self.reg_state.invalidate_all();
        }

        // Record which zero-page bytes a store or read-modify-write touches, so
        // an interrupt handler's scratch save can be narrowed to them. Inline
        // `asm` reaches here too (it is emitted through `emit_inst`), so an
        // exotic addressing mode in a handler is caught as opaque.
        if Self::writes_memory(mnemonic) {
            self.note_zp_write(operand);
        }

        self.track_effect(mnemonic, operand);
    }

    /// Mirror an instruction's effect in the register tracker.
    ///
    /// The tracked load/store methods set a precise belief right after
    /// emit_inst returns, so anything done here is overwritten wherever a
    /// caller knows better — this is the floor, not the ceiling. It exists so
    /// a *raw* `emit_inst("LDX", ...)` site cannot leave a stale belief
    /// behind: a stale belief is a miscompile (the JSR case above was one),
    /// while a conservative Unknown merely costs a redundant load.
    fn track_effect(&mut self, mnemonic: &str, operand: &str) {
        // What a plain (unindexed, unindirected) operand puts in a register.
        // Indexed forms, labels and symbols say nothing precise.
        let load_value = || {
            let hex = operand
                .strip_prefix('#')
                .unwrap_or(operand)
                .strip_prefix('$')?;
            match hex.len() {
                2 => u8::from_str_radix(hex, 16).ok().map(|v| {
                    if operand.starts_with('#') {
                        RegisterValue::Immediate(v as i64)
                    } else {
                        RegisterValue::ZeroPage(v)
                    }
                }),
                4 if !operand.starts_with('#') => u16::from_str_radix(hex, 16)
                    .ok()
                    .map(RegisterValue::Variable),
                _ => None,
            }
        };
        // The location a plain store operand rewrites, as (is_zero_page, addr).
        let store_addr = || {
            let hex = operand.strip_prefix('$')?;
            match hex.len() {
                2 => u8::from_str_radix(hex, 16).ok().map(|v| (true, v as u16)),
                4 => u16::from_str_radix(hex, 16).ok().map(|v| (false, v)),
                _ => None,
            }
        };
        let invalidate_store = |reg_state: &mut RegisterState, is_zp: bool, addr: u16| {
            if is_zp {
                reg_state.invalidate_zero_page(addr as u8);
            } else {
                reg_state.invalidate_memory(addr);
            }
        };

        match mnemonic {
            "LDA" => self
                .reg_state
                .set_a(load_value().unwrap_or(RegisterValue::Unknown)),
            "LDX" => self
                .reg_state
                .set_x(load_value().unwrap_or(RegisterValue::Unknown)),
            "LDY" => self
                .reg_state
                .set_y(load_value().unwrap_or(RegisterValue::Unknown)),
            "TAX" => self.reg_state.transfer_a_to_x(),
            "TAY" => self.reg_state.transfer_a_to_y(),
            "TXA" => self.reg_state.transfer_x_to_a(),
            "TYA" => self.reg_state.transfer_y_to_a(),
            "TSX" => self.reg_state.modify_x(),
            "INX" | "DEX" => self.reg_state.modify_x(),
            "INY" | "DEY" => self.reg_state.modify_y(),
            "ADC" | "SBC" | "AND" | "ORA" | "EOR" | "PLA" => self.reg_state.modify_a(),
            "ASL" | "LSR" | "ROL" | "ROR" => {
                // The accumulator form rewrites A; the memory form rewrites
                // the location and leaves A alone.
                if operand.is_empty() || operand == "A" {
                    self.reg_state.modify_a();
                } else if let Some((is_zp, addr)) = store_addr() {
                    invalidate_store(&mut self.reg_state, is_zp, addr);
                }
            }
            "INC" | "DEC" => {
                if let Some((is_zp, addr)) = store_addr() {
                    invalidate_store(&mut self.reg_state, is_zp, addr);
                }
            }
            "STA" | "STX" | "STY" => {
                if let Some((is_zp, addr)) = store_addr() {
                    invalidate_store(&mut self.reg_state, is_zp, addr);
                    // Afterwards the stored register mirrors the location.
                    let mirrored = if is_zp {
                        RegisterValue::ZeroPage(addr as u8)
                    } else {
                        RegisterValue::Variable(addr)
                    };
                    match mnemonic {
                        "STA" => self.reg_state.set_a(mirrored),
                        "STX" => self.reg_state.set_x(mirrored),
                        _ => self.reg_state.set_y(mirrored),
                    }
                }
            }
            "PLP" | "RTI" => self.reg_state.invalidate_all(),
            _ => {}
        }
    }

    pub fn emit_comment(&mut self, comment: &str) {
        self.output.push_str("; ");
        self.output.push_str(comment);
        self.output.push('\n');
    }

    pub fn emit_raw(&mut self, line: &str) {
        self.output.push_str(line);
        self.output.push('\n');

        // A raw line carrying a store still writes zero page, so track it the
        // same as `emit_inst` does — the boolean-materialisation and 16-bit
        // routines emit their instructions this way. Labels (`foo:`),
        // directives (`.BYTE`), and comments (`;`) write nothing. The write set
        // is exhaustive for zero page, so any other mnemonic is safely ignored.
        let t = line.trim();
        let mut parts = t.splitn(2, char::is_whitespace);
        let mnemonic = parts.next().unwrap_or("");
        if Self::writes_memory(mnemonic) {
            self.note_zp_write(parts.next().unwrap_or(""));
        }
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
        if operand.is_empty() || operand.eq_ignore_ascii_case("A") {
            // Implied or accumulator mode (1 byte). Codegen normally emits
            // accumulator shifts with an empty operand, but the explicit `A`
            // form (`LSR A`) appears in hand-written raw stdlib routines and
            // must size to 1, not fall through to the 3-byte absolute arm.
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
                // `(zp,X)` is 2 bytes, but the 65C02 `JMP (abs,X)` — whose base
                // is a label or a 16-bit address, not `$XX` — is 3. Sizing it as
                // 2 would under-measure the function and land the next `.ORG`
                // inside it (the jump-table overlap class of bug).
                let base = operand.trim_start_matches('(');
                let base = base.split(',').next().unwrap_or("");
                if base.starts_with('$') && base.trim_start_matches('$').len() <= 2 {
                    2 // (zp,X)
                } else {
                    3 // JMP (abs,X)
                }
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
        } else if mnemonic.starts_with("BBR") || mnemonic.starts_with("BBS") {
            // 65C02 bit-test-branch `BBRn $zp,label` / `BBSn $zp,label`: opcode,
            // zero-page byte, relative offset (3 bytes). Its `$zp,label` operand
            // has a comma but no index register, so it would otherwise fall to
            // the generic branch arm below and be under-measured as 2 bytes.
            3
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

    /// Total machine-code size, in bytes, of a block of assembly text.
    ///
    /// Instruction lines contribute their encoded length; labels, comments,
    /// `.ORG` and blank lines contribute nothing. Used to verify that a
    /// hand-written raw routine still fits the ROM window reserved for it, so
    /// editing the routine can never silently overlap the following `.ORG`.
    pub fn measure_asm(asm: &str) -> u16 {
        let mut total = 0u16;
        for line in asm.lines() {
            let t = line.trim();
            // Skip blanks, comments, directives, and (indented) labels.
            if t.is_empty() || t.starts_with(';') || t.starts_with('.') || t.ends_with(':') {
                continue;
            }
            let mut parts = t.splitn(2, char::is_whitespace);
            let mnemonic = parts.next().unwrap_or("");
            let operand = parts
                .next()
                .unwrap_or("")
                .split(';')
                .next()
                .unwrap_or("")
                .trim();
            total += Self::instruction_size(mnemonic, operand);
        }
        total
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

    /// Begin an `atomic` access region: mask interrupts so a multi-byte read or
    /// write cannot be seen (or interrupted) half-done. Only the outermost call
    /// emits `PHP; SEI` — a nested region (a read of an atomic static inside an
    /// atomic assignment) rides the same mask. `PHP` saves the caller's
    /// interrupt-disable flag so [`exit_atomic`](Self::exit_atomic) restores it
    /// rather than blindly re-enabling: correct even when interrupts were
    /// already off.
    pub fn enter_atomic(&mut self) {
        if self.atomic_depth == 0 {
            self.emit_inst("PHP", "");
            self.emit_inst("SEI", "");
        }
        self.atomic_depth += 1;
    }

    /// End an `atomic` region, restoring the saved interrupt flag when the
    /// outermost region closes.
    pub fn exit_atomic(&mut self) {
        self.atomic_depth -= 1;
        if self.atomic_depth == 0 {
            self.emit_inst("PLP", "");
        }
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

    /// Push the end label of the inline expansion being generated. A `return`
    /// inside an inline body jumps here instead of emitting RTS — otherwise
    /// an early `return` just sets A and falls through into the *rest of the
    /// body* (which usually ends in another return, overwriting the value).
    pub fn push_inline_end(&mut self, label: String) {
        self.inline_end_labels.push(label);
    }

    /// Pop the current inline expansion's end label.
    pub fn pop_inline_end(&mut self) {
        self.inline_end_labels.pop();
    }

    /// The innermost inline expansion's end label, if any.
    pub fn inline_end_label(&self) -> Option<&str> {
        self.inline_end_labels.last().map(|s| s.as_str())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measure_asm_sizes_instructions_and_skips_structure() {
        // Accumulator shifts (`LSR A`) are 1 byte, not the 3 the absolute arm
        // would give; labels, comments, .ORG and blanks contribute nothing.
        let asm = "\
routine:
    ; a comment
    LDA #$00     ; 2
    STA $D2      ; 2 (zero page)
    LSR A        ; 1 (accumulator)
    ADC $1234    ; 3 (absolute)
    BNE routine  ; 2 (branch)
    RTS          ; 1
.ORG $9000
";
        // 2 + 2 + 1 + 3 + 2 + 1 = 11
        assert_eq!(Emitter::measure_asm(asm), 11);
    }

    #[test]
    fn a_raw_load_replaces_the_registers_belief() {
        // Tracking used to be a per-callsite convention: a raw
        // emit_inst("LDX", ...) left the tracker believing X still held the
        // old value. The belief is now replaced at the mnemonic level.
        let mut e = Emitter::default();
        e.emit_ldx_immediate(7);
        e.emit_inst("LDX", "$20"); // raw, no tracked wrapper
        assert_eq!(e.reg_state.x_reg, RegisterValue::ZeroPage(0x20));
        e.emit_inst("LDX", "label,X"); // unparseable: conservative Unknown
        assert_eq!(e.reg_state.x_reg, RegisterValue::Unknown);
    }

    #[test]
    fn a_raw_store_invalidates_mirrored_beliefs() {
        // A believed A == ZeroPage($20) must not survive a raw STA to $20
        // from a different value — and after the store, A does mirror $20.
        let mut e = Emitter::default();
        e.emit_lda_immediate(5);
        e.emit_inst("STA", "$20");
        assert_eq!(e.reg_state.a_reg, RegisterValue::ZeroPage(0x20));

        // A raw STX to the same address drops A's mirror belief.
        e.emit_inst("LDX", "#$09");
        e.emit_inst("STX", "$20");
        assert_eq!(e.reg_state.a_reg, RegisterValue::Unknown);
        assert_eq!(e.reg_state.x_reg, RegisterValue::ZeroPage(0x20));
    }

    #[test]
    fn a_raw_arithmetic_op_marks_a_unknown() {
        let mut e = Emitter::default();
        e.emit_lda_immediate(5);
        e.emit_inst("ADC", "#$01");
        assert_eq!(e.reg_state.a_reg, RegisterValue::Unknown);
    }

    #[test]
    fn a_memory_form_shift_keeps_a_but_drops_the_location() {
        let mut e = Emitter::default();
        e.emit_lda_immediate(5);
        e.emit_inst("ASL", "$20");
        assert_eq!(e.reg_state.a_reg, RegisterValue::Immediate(5));
        e.emit_inst("ASL", "A");
        assert_eq!(e.reg_state.a_reg, RegisterValue::Unknown);
    }
}
