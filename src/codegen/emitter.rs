//! Assembly Emitter
//!
//! Helper for generating formatted 6502 assembly code.

pub struct Emitter {
    output: String,
    #[allow(dead_code)]
    indent: usize,
    label_counter: usize,
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
        }
    }

    pub fn next_label(&mut self, prefix: &str) -> String {
        self.label_counter += 1;
        format!("{}_{}", prefix, self.label_counter)
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
}
