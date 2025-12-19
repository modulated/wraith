//! Assembly Emitter
//!
//! Helper for generating formatted 6502 assembly code.

pub struct Emitter {
    output: String,
    indent: usize,
    label_counter: usize,
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

    pub fn emit_org(&mut self, address: u16) {
        self.output.push_str(&format!("    * = ${:04X}\n", address));
    }

    pub fn finish(self) -> String {
        self.output
    }
}
