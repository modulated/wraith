pub mod comment_utils;
pub mod emitter;
pub mod expr;
pub mod item;
pub mod memory_layout;
pub mod peephole;
pub mod placement;
pub mod regstate;
pub mod section_allocator;
pub mod stmt;

use crate::ast::SourceFile;
use crate::sema::ProgramInfo;
use emitter::Emitter;
use item::generate_item;
use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};
use section_allocator::{AllocationSource, SectionAllocator};

/// The 6502 family member being targeted. Governs whether codegen may use the
/// 65C02's extra instructions — most relevantly the Rockwell bit ops
/// `SMB`/`RMB`/`BBR`/`BBS`, which turn a zero-page bit set/clear into a single
/// instruction. NMOS falls back to `ORA`/`AND`/`EOR` masks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TargetCpu {
    /// Original NMOS 6502 — base instruction set only.
    Nmos6502,
    /// WDC 65C02 with the Rockwell bit-manipulation instructions.
    #[default]
    Cmos65C02,
}

impl TargetCpu {
    /// True on any 65C02, where the WDC base additions are available: `STZ`,
    /// `BRA`, `PHX`/`PLX`/`PHY`/`PLY`, accumulator `INC A`/`DEC A`, `TSB`/`TRB`.
    pub fn is_cmos(self) -> bool {
        matches!(self, TargetCpu::Cmos65C02)
    }

    /// True where the Rockwell bit ops `SMB`/`RMB`/`BBR`/`BBS` are available.
    /// On this two-variant enum that is the same set as [`is_cmos`], but the
    /// distinction is real hardware — some 65C02s omit the Rockwell ops — and
    /// keeping it separate documents which extension each call site relies on.
    ///
    /// [`is_cmos`]: Self::is_cmos
    pub fn has_rockwell_bit_ops(self) -> bool {
        matches!(self, TargetCpu::Cmos65C02)
    }
}

/// Controls the verbosity level of generated assembly comments
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CommentVerbosity {
    /// Minimal comments - only function headers and critical info
    Minimal,
    /// Normal comments - function headers, operation types, basic context
    #[default]
    Normal,
    /// Verbose - full register state, detailed explanations, memory layout
    Verbose,
}

#[derive(Debug, Clone)]
pub enum CodegenError {
    Unknown,
    UnsupportedOperation(String),
    SymbolNotFound(String),
    SectionError(String),
    /// Two things were placed at overlapping addresses, or an `#[org]` was
    /// placed where no section covers it. Carries the span of the offending
    /// declaration so the diagnostic can point at it like any other error.
    AddressConflict {
        message: String,
        notes: Vec<String>,
        span: Option<crate::ast::Span>,
    },
    /// A fixed zero-page scratch pool ran out while lowering one expression —
    /// the argument-staging pool or the expression-temporary pool. This is a
    /// capacity limit of the backend, not a bug and not invalid input: the same
    /// computation split across statements compiles. Carries the blamed
    /// statement's span and a rewrite hint so it reads like any other
    /// diagnostic rather than an internal error.
    ResourceExhausted {
        message: String,
        hint: String,
        span: Option<crate::ast::Span>,
    },
    /// An internal compiler invariant was violated (a bug in the compiler, not the input).
    Internal(String),
}

impl std::fmt::Display for CodegenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CodegenError::Unknown => write!(f, "unknown error"),
            CodegenError::UnsupportedOperation(msg) => write!(f, "unsupported operation: {}", msg),
            CodegenError::SymbolNotFound(name) => write!(f, "undefined symbol '{}'", name),
            CodegenError::SectionError(msg) => write!(f, "section error: {}", msg),
            CodegenError::AddressConflict { message, notes, .. } => {
                write!(f, "{}", message)?;
                for note in notes {
                    write!(f, "\n  = note: {}", note)?;
                }
                Ok(())
            }
            CodegenError::ResourceExhausted { message, hint, .. } => {
                write!(f, "{}\n  = help: {}", message, hint)
            }
            CodegenError::Internal(msg) => write!(f, "internal compiler error: {}", msg),
        }
    }
}

impl CodegenError {
    /// A zero-page scratch pool ran out lowering one expression. `message` is
    /// the existing wording (kept verbatim so callers and the differential
    /// fuzzer recognise it); the hint is chosen from which pool it names and
    /// tells the author to split the expression across statements.
    pub fn pool_exhausted(message: &str, span: Option<crate::ast::Span>) -> CodegenError {
        let hint = if message.contains("argument-evaluation pool") {
            "this call stages more argument bytes in zero page than fit — evaluate a nested \
             call into its own `let` binding first, then pass the result"
        } else {
            "this expression holds more values in the 6502's fixed zero-page scratch space than \
             it has room for — split it into steps with intermediate `let` bindings so fewer are \
             live at once"
        };
        CodegenError::ResourceExhausted {
            message: message.to_string(),
            hint: hint.to_string(),
            span,
        }
    }

    /// Render with a source excerpt and caret, matching how parse and semantic
    /// errors are reported. Falls back to the plain message when the error
    /// carries no span.
    pub fn format_with_source_and_file(&self, source: &str, filename: Option<&str>) -> String {
        match self {
            CodegenError::AddressConflict {
                message,
                notes,
                span: Some(span),
            } => {
                let mut out = format!(
                    "error: {}\n{}",
                    message,
                    span.format_error_context(source, filename, message)
                );
                for note in notes {
                    out.push_str(&format!("\n  = note: {}", note));
                }
                out
            }
            CodegenError::ResourceExhausted {
                message,
                hint,
                span: Some(span),
            } => {
                format!(
                    "error: {}\n{}\n  = help: {}",
                    message,
                    span.format_error_context(source, filename, message),
                    hint
                )
            }
            other => format!("error: {}", other),
        }
    }
}

impl std::error::Error for CodegenError {}

/// Collects and manages string literals for emission to DATA section
/// Uses a global pool for cross-module string deduplication
pub struct StringCollector {
    strings: HashMap<String, String>, // content -> label
    /// Constant enum payloads (tag byte + field bytes), collected here so they
    /// land in DATA instead of inline in the instruction stream behind a `JMP`.
    /// Insertion order is preserved for a deterministic DATA layout; the map
    /// deduplicates identical blobs so two constructions of the same variant
    /// with the same payload share one copy.
    enum_blobs: Vec<(String, Vec<u8>)>, // (label, bytes) in insertion order
    enum_blob_labels: HashMap<Vec<u8>, String>, // bytes -> label, for dedup
    next_id: usize,
}

impl Default for StringCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl StringCollector {
    pub fn new() -> Self {
        Self {
            strings: HashMap::default(),
            enum_blobs: Vec::new(),
            enum_blob_labels: HashMap::default(),
            next_id: 0,
        }
    }

    /// Register a constant enum payload and return its DATA label. Identical
    /// blobs are deduplicated to a single copy.
    pub fn add_enum_blob(&mut self, bytes: Vec<u8>) -> String {
        if let Some(label) = self.enum_blob_labels.get(&bytes) {
            return label.clone();
        }
        let label = format!("ed_{}", self.next_id);
        self.next_id += 1;
        self.enum_blob_labels.insert(bytes.clone(), label.clone());
        self.enum_blobs.push((label.clone(), bytes));
        label
    }

    /// Emit all collected constant enum payloads into the DATA section.
    pub fn emit_enum_data(
        &self,
        emitter: &mut Emitter,
        section_alloc: &mut SectionAllocator,
    ) -> Result<(), CodegenError> {
        if self.enum_blobs.is_empty() {
            return Ok(());
        }

        emitter.emit_comment("============================");
        emitter.emit_comment("Constant Enum Payloads");
        emitter.emit_comment("============================");

        for (label, bytes) in &self.enum_blobs {
            let size = bytes.len() as u16;
            let addr = section_alloc
                .allocate("DATA", size)
                .map_err(CodegenError::SectionError)?;
            section_alloc.record_allocation(
                format!("enum payload {}", label),
                addr,
                size,
                AllocationSource::Section("DATA".to_string()),
                None,
            );
            emitter.emit_org(addr);
            emitter.emit_label(label);
            for chunk in bytes.chunks(16) {
                let bytes_str = chunk
                    .iter()
                    .map(|b| format!("${:02X}", b))
                    .collect::<Vec<_>>()
                    .join(", ");
                emitter.emit_raw(&format!("    .BYTE {}", bytes_str));
            }
        }

        Ok(())
    }

    /// Register a string and get its label (deduplicated automatically)
    /// Uses content-based hashing for consistent labels across modules
    pub fn add_string(&mut self, content: String) -> String {
        if let Some(label) = self.strings.get(&content) {
            // Deduplication: return existing label
            label.clone()
        } else {
            // Use content-based label for cross-module consistency
            let label = generate_string_label(&content, self.next_id);
            self.next_id += 1;
            self.strings.insert(content, label.clone());
            label
        }
    }

    /// Register a string using a global pool for cross-module deduplication
    /// Returns the label from the global pool, or creates a new one
    pub fn add_string_with_pool(
        &mut self,
        content: String,
        global_pool: &mut HashMap<String, String>,
    ) -> String {
        // First check local cache
        if let Some(label) = self.strings.get(&content) {
            return label.clone();
        }

        // Check global pool
        if let Some(label) = global_pool.get(&content) {
            // Add to local cache for future lookups
            self.strings.insert(content, label.clone());
            return label.clone();
        }

        // Create new label using content-based hashing
        let label = generate_string_label(&content, self.next_id);
        self.next_id += 1;

        // Add to both local and global pools
        self.strings.insert(content.clone(), label.clone());
        global_pool.insert(content, label.clone());

        label
    }

    /// Validate that all strings are within the 256-byte limit
    pub fn validate_strings(&self) -> Result<(), String> {
        for (content, label) in &self.strings {
            if content.len() > 255 {
                return Err(format!(
                    "String literal '{}' exceeds 256 byte limit: {} bytes",
                    label,
                    content.len()
                ));
            }
        }
        Ok(())
    }

    /// Emit all collected strings to DATA section
    pub fn emit_strings(
        &self,
        emitter: &mut Emitter,
        section_alloc: &mut SectionAllocator,
    ) -> Result<(), CodegenError> {
        if self.strings.is_empty() {
            return Ok(());
        }

        emitter.emit_comment("============================");
        emitter.emit_comment("String Literal Data");
        emitter.emit_comment("============================");

        for (content, label) in &self.strings {
            // Allocate in DATA section
            // Strings are limited to 256 bytes (u8 length prefix)
            let content_len = content.len();
            if content_len > 255 {
                return Err(CodegenError::UnsupportedOperation(format!(
                    "String literal exceeds 256 byte limit: {} bytes",
                    content_len
                )));
            }
            let data_size = 1 + content_len as u16; // u8 length prefix + bytes
            let addr = section_alloc
                .allocate("DATA", data_size)
                .map_err(CodegenError::SectionError)?;
            section_alloc.record_allocation(
                format!("string literal {}", label),
                addr,
                data_size,
                AllocationSource::Section("DATA".to_string()),
                None,
            );

            emitter.emit_org(addr);
            emitter.emit_label(label);

            // Emit length as u8 (single byte, max 255)
            let len = content_len as u8;
            emitter.emit_raw(&format!("    .BYTE ${:02X}  ; length = {}", len, len));

            // Emit string bytes
            if !content.is_empty() {
                // Escape special characters for display in comment
                let display = content
                    .chars()
                    .map(|c| match c {
                        '\n' => "\\n".to_string(),
                        '\r' => "\\r".to_string(),
                        '\t' => "\\t".to_string(),
                        '\0' => "\\0".to_string(),
                        '\\' => "\\\\".to_string(),
                        '"' => "\\\"".to_string(),
                        c if c.is_ascii_graphic() || c == ' ' => c.to_string(),
                        c => format!("\\x{:02X}", c as u8),
                    })
                    .collect::<String>();
                emitter.emit_comment(&format!("\"{}\"", display));

                // Emit bytes in groups of 16 for readability
                for (i, chunk) in content.as_bytes().chunks(16).enumerate() {
                    let bytes_str = chunk
                        .iter()
                        .map(|b| format!("${:02X}", b))
                        .collect::<Vec<_>>()
                        .join(", ");

                    if i == 0 && chunk.len() < content.len() {
                        emitter.emit_raw(&format!(
                            "    .BYTE {}  ; bytes 0-{}",
                            bytes_str,
                            chunk.len() - 1
                        ));
                    } else if chunk.len() < 16 {
                        let start = i * 16;
                        emitter.emit_raw(&format!(
                            "    .BYTE {}  ; bytes {}-{}",
                            bytes_str,
                            start,
                            start + chunk.len() - 1
                        ));
                    } else {
                        emitter.emit_raw(&format!("    .BYTE {}", bytes_str));
                    }
                }
            }
        }

        Ok(())
    }
}

/// Generate a unique label for a string based on its content
/// Uses a hash of the content to ensure consistent labels across modules
fn generate_string_label(content: &str, _counter: usize) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    // Use a simple hash of the content for the label
    let mut hasher = DefaultHasher::new();
    content.hash(&mut hasher);
    let hash = hasher.finish();

    // Use first 8 hex digits of hash for the label
    format!("str_{:08x}", hash)
}

/// Emit stdlib math functions (mul16, div16) if they were used
fn emit_stdlib_math_functions(
    emitter: &mut Emitter,
    section_alloc: &mut SectionAllocator,
) -> Result<(), CodegenError> {
    if !emitter.needs_mul16 && !emitter.needs_div16 && !emitter.needs_mod16 {
        return Ok(()); // Nothing to emit
    }

    emitter.emit_comment("============================================================");
    emitter.emit_comment("Standard Library Math Functions");
    emitter.emit_comment("Automatically included for u16 multiplication, division, modulo");
    emitter.emit_comment("============================================================");

    if emitter.needs_mul16 {
        // Exact machine-code size of the routine below; verified against the
        // emitted bytes at the end of the block.
        const MUL16_BYTES: u16 = 61;
        let org_addr = section_alloc
            .allocate("CODE", MUL16_BYTES)
            .map_err(CodegenError::SectionError)?;
        let start = emitter.output_len();
        emitter.emit_org(org_addr);
        emitter.emit_comment("Function: mul16");
        emitter.emit_comment("  Params: a: u16 in $D9-$DA, b: u16 in $DB-$DC");
        emitter.emit_comment("  Returns: u16 in A/Y (low/high)");
        emitter.emit_comment(&format!("  Location: ${:04X}", org_addr));
        emitter.emit_label("mul16");

        // Emit mul16 implementation
        // Memory layout: $D0-$D1 multiplicand, $D2-$D3 result,
        //               $D4-$D5 multiplier, $D6 loop counter
        emitter.emit_raw("    LDA #$00");
        emitter.emit_raw("    STA $D2"); // result_low at $D2
        emitter.emit_raw("    STA $D3"); // result_high at $D3
        emitter.emit_raw("    LDA $D9");
        emitter.emit_raw("    STA $D0"); // param_a_low at $D0
        emitter.emit_raw("    LDA $DA");
        emitter.emit_raw("    STA $D1"); // param_a_high at $D1
        emitter.emit_raw("    LDA $DB");
        emitter.emit_raw("    STA $D4"); // param_b_low at $D4
        emitter.emit_raw("    LDA $DC");
        emitter.emit_raw("    STA $D5"); // param_b_high at $D5
        emitter.emit_raw("    LDX #$10");
        emitter.emit_raw("    STX $D6"); // loop_counter at $D6
        emitter.emit_raw("    mul16_loop:");
        emitter.emit_raw("    LDA $D4");
        emitter.emit_raw("    LSR A");
        emitter.emit_raw("    BCC mul16_skip_add");
        emitter.emit_raw("    CLC");
        emitter.emit_raw("    LDA $D2");
        emitter.emit_raw("    ADC $D0");
        emitter.emit_raw("    STA $D2");
        emitter.emit_raw("    LDA $D3");
        emitter.emit_raw("    ADC $D1");
        emitter.emit_raw("    STA $D3");
        emitter.emit_raw("    mul16_skip_add:");
        emitter.emit_raw("    LSR $D5");
        emitter.emit_raw("    ROR $D4");
        emitter.emit_raw("    ASL $D0");
        emitter.emit_raw("    ROL $D1");
        emitter.emit_raw("    DEC $D6");
        emitter.emit_raw("    BNE mul16_loop");
        emitter.emit_raw("    LDA $D2");
        emitter.emit_raw("    LDY $D3");
        emitter.emit_raw("    RTS");
        verify_raw_routine_fits("mul16", emitter.output_since(start), MUL16_BYTES)?;
    }

    if emitter.needs_div16 {
        const DIV16_BYTES: u16 = 92;
        let org_addr = section_alloc
            .allocate("CODE", DIV16_BYTES)
            .map_err(CodegenError::SectionError)?;
        let start = emitter.output_len();
        emitter.emit_org(org_addr);
        emitter.emit_comment("Function: div16");
        emitter.emit_comment("  Params: a: u16 in $D9-$DA, b: u16 in $DB-$DC");
        emitter.emit_comment("  Returns: u16 in A/Y (low/high)");
        emitter.emit_comment(&format!("  Location: ${:04X}", org_addr));
        emitter.emit_label("div16");

        // Emit div16 implementation using proper remainder register
        // Memory layout: $D0-$D1 dividend, $D2-$D3 divisor, $D4-$D5 quotient,
        //               $D6-$D7 remainder, $D8 loop counter

        // Zero check - return 0xFFFF for division by zero
        emitter.emit_raw("    LDA $DB");
        emitter.emit_raw("    ORA $DC");
        emitter.emit_raw("    BNE div16_not_zero");
        emitter.emit_raw("    LDA #$FF");
        emitter.emit_raw("    TAY");
        emitter.emit_raw("    JMP div16_done");

        emitter.emit_raw("    div16_not_zero:");
        // Initialize quotient and remainder to 0
        emitter.emit_raw("    LDA #$00");
        emitter.emit_raw("    STA $D4"); // quotient_low
        emitter.emit_raw("    STA $D5"); // quotient_high
        emitter.emit_raw("    STA $D6"); // remainder_low
        emitter.emit_raw("    STA $D7"); // remainder_high

        // Copy dividend to working storage
        emitter.emit_raw("    LDA $D9");
        emitter.emit_raw("    STA $D0"); // dividend_low
        emitter.emit_raw("    LDA $DA");
        emitter.emit_raw("    STA $D1"); // dividend_high

        // Copy divisor to working storage
        emitter.emit_raw("    LDA $DB");
        emitter.emit_raw("    STA $D2"); // divisor_low
        emitter.emit_raw("    LDA $DC");
        emitter.emit_raw("    STA $D3"); // divisor_high

        // Loop counter = 16
        emitter.emit_raw("    LDA #$10");
        emitter.emit_raw("    STA $D8");

        emitter.emit_raw("    div16_loop:");
        // Shift dividend left, high bit goes into remainder
        emitter.emit_raw("    ASL $D0");
        emitter.emit_raw("    ROL $D1");
        emitter.emit_raw("    ROL $D6"); // Carry from dividend -> remainder
        emitter.emit_raw("    ROL $D7");

        // Shift quotient left to make room for next bit
        emitter.emit_raw("    ASL $D4");
        emitter.emit_raw("    ROL $D5");

        // Compare remainder with divisor (16-bit)
        emitter.emit_raw("    LDA $D7"); // remainder_high
        emitter.emit_raw("    CMP $D3"); // divisor_high
        emitter.emit_raw("    BCC div16_skip"); // remainder < divisor
        emitter.emit_raw("    BNE div16_sub"); // remainder > divisor
        // High bytes equal, compare low bytes
        emitter.emit_raw("    LDA $D6"); // remainder_low
        emitter.emit_raw("    CMP $D2"); // divisor_low
        emitter.emit_raw("    BCC div16_skip"); // remainder < divisor

        emitter.emit_raw("    div16_sub:");
        // remainder -= divisor
        emitter.emit_raw("    SEC");
        emitter.emit_raw("    LDA $D6");
        emitter.emit_raw("    SBC $D2");
        emitter.emit_raw("    STA $D6");
        emitter.emit_raw("    LDA $D7");
        emitter.emit_raw("    SBC $D3");
        emitter.emit_raw("    STA $D7");
        // Set quotient bit 0
        emitter.emit_raw("    INC $D4");

        emitter.emit_raw("    div16_skip:");
        emitter.emit_raw("    DEC $D8");
        emitter.emit_raw("    BNE div16_loop");

        // Return quotient in A/Y
        emitter.emit_raw("    LDA $D4");
        emitter.emit_raw("    LDY $D5");

        emitter.emit_raw("    div16_done:");
        emitter.emit_raw("    RTS");
        verify_raw_routine_fits("div16", emitter.output_since(start), DIV16_BYTES)?;
    }

    if emitter.needs_mod16 {
        const MOD16_BYTES: u16 = 92;
        let org_addr = section_alloc
            .allocate("CODE", MOD16_BYTES)
            .map_err(CodegenError::SectionError)?;
        let start = emitter.output_len();
        emitter.emit_org(org_addr);
        emitter.emit_comment("Function: mod16");
        emitter.emit_comment("  Params: a: u16 in $D9-$DA, b: u16 in $DB-$DC");
        emitter.emit_comment("  Returns: u16 remainder in A/Y (low/high)");
        emitter.emit_comment(&format!("  Location: ${:04X}", org_addr));
        emitter.emit_label("mod16");

        // Emit mod16 implementation - same as div16 but returns remainder
        // Memory layout: $D0-$D1 dividend, $D2-$D3 divisor, $D4-$D5 quotient,
        //               $D6-$D7 remainder, $D8 loop counter

        // Zero check - return 0xFFFF for modulo by zero
        emitter.emit_raw("    LDA $DB");
        emitter.emit_raw("    ORA $DC");
        emitter.emit_raw("    BNE mod16_not_zero");
        emitter.emit_raw("    LDA #$FF");
        emitter.emit_raw("    TAY");
        emitter.emit_raw("    JMP mod16_done");

        emitter.emit_raw("    mod16_not_zero:");
        // Initialize quotient and remainder to 0
        emitter.emit_raw("    LDA #$00");
        emitter.emit_raw("    STA $D4"); // quotient_low
        emitter.emit_raw("    STA $D5"); // quotient_high
        emitter.emit_raw("    STA $D6"); // remainder_low
        emitter.emit_raw("    STA $D7"); // remainder_high

        // Copy dividend to working storage
        emitter.emit_raw("    LDA $D9");
        emitter.emit_raw("    STA $D0"); // dividend_low
        emitter.emit_raw("    LDA $DA");
        emitter.emit_raw("    STA $D1"); // dividend_high

        // Copy divisor to working storage
        emitter.emit_raw("    LDA $DB");
        emitter.emit_raw("    STA $D2"); // divisor_low
        emitter.emit_raw("    LDA $DC");
        emitter.emit_raw("    STA $D3"); // divisor_high

        // Loop counter = 16
        emitter.emit_raw("    LDA #$10");
        emitter.emit_raw("    STA $D8");

        emitter.emit_raw("    mod16_loop:");
        // Shift dividend left, high bit goes into remainder
        emitter.emit_raw("    ASL $D0");
        emitter.emit_raw("    ROL $D1");
        emitter.emit_raw("    ROL $D6"); // Carry from dividend -> remainder
        emitter.emit_raw("    ROL $D7");

        // Shift quotient left to make room for next bit
        emitter.emit_raw("    ASL $D4");
        emitter.emit_raw("    ROL $D5");

        // Compare remainder with divisor (16-bit)
        emitter.emit_raw("    LDA $D7"); // remainder_high
        emitter.emit_raw("    CMP $D3"); // divisor_high
        emitter.emit_raw("    BCC mod16_skip"); // remainder < divisor
        emitter.emit_raw("    BNE mod16_sub"); // remainder > divisor
        // High bytes equal, compare low bytes
        emitter.emit_raw("    LDA $D6"); // remainder_low
        emitter.emit_raw("    CMP $D2"); // divisor_low
        emitter.emit_raw("    BCC mod16_skip"); // remainder < divisor

        emitter.emit_raw("    mod16_sub:");
        // remainder -= divisor
        emitter.emit_raw("    SEC");
        emitter.emit_raw("    LDA $D6");
        emitter.emit_raw("    SBC $D2");
        emitter.emit_raw("    STA $D6");
        emitter.emit_raw("    LDA $D7");
        emitter.emit_raw("    SBC $D3");
        emitter.emit_raw("    STA $D7");
        // Set quotient bit 0
        emitter.emit_raw("    INC $D4");

        emitter.emit_raw("    mod16_skip:");
        emitter.emit_raw("    DEC $D8");
        emitter.emit_raw("    BNE mod16_loop");

        // Return REMAINDER in A/Y (difference from div16)
        emitter.emit_raw("    LDA $D6");
        emitter.emit_raw("    LDY $D7");

        emitter.emit_raw("    mod16_done:");
        emitter.emit_raw("    RTS");
        verify_raw_routine_fits("mod16", emitter.output_since(start), MOD16_BYTES)?;
    }

    Ok(())
}

/// Fail the build if a hand-written raw stdlib routine grew past the ROM window
/// reserved for it, instead of letting the following `.ORG` silently overlap its
/// tail. `emitted` is the routine's assembly text (from `output_since`).
fn verify_raw_routine_fits(name: &str, emitted: &str, reserved: u16) -> Result<(), CodegenError> {
    let actual = Emitter::measure_asm(emitted);
    if actual > reserved {
        return Err(CodegenError::SectionError(format!(
            "stdlib `{name}` is {actual} bytes but only {reserved} were reserved; \
             update its reservation in emit_stdlib_math_functions"
        )));
    }
    Ok(())
}

/// Turn any overlapping address ranges into a compile error.
///
/// Overlaps are reported against everything that occupies space — functions,
/// string and const-array data, and the hardware's interrupt vectors — because
/// an `#[org]` landing on data or on the reset vector fails just as completely
/// as one landing on another function, and does so silently.
///
/// The diagnostic points at whichever of the two was placed by hand, since that
/// is the one the programmer can move; the allocator's own placements can only
/// collide with something that was pinned.
fn report_address_conflicts(section_alloc: &SectionAllocator) -> Result<(), CodegenError> {
    let conflicts = section_alloc.check_conflicts();
    let Some((first, second)) = conflicts.first() else {
        return Ok(());
    };

    // Prefer to blame — and point at — the explicitly placed side.
    let (blamed, other) = if second.source.is_fixed() && !first.source.is_fixed() {
        (second, first)
    } else {
        (first, second)
    };

    let mut notes = vec![
        format!(
            "'{}' occupies ${:04X}-${:04X} ({})",
            blamed.name, blamed.start, blamed.end, blamed.source
        ),
        format!(
            "'{}' occupies ${:04X}-${:04X} ({})",
            other.name, other.start, other.end, other.source
        ),
    ];
    if matches!(other.source, AllocationSource::Reserved) {
        notes.push("the 6502 reads its reset and interrupt vectors from this range".to_string());
    }
    if conflicts.len() > 1 {
        notes.push(format!(
            "{} further conflict{} not shown",
            conflicts.len() - 1,
            if conflicts.len() == 2 { "" } else { "s" }
        ));
    }

    Err(CodegenError::AddressConflict {
        message: format!("'{}' overlaps '{}'", blamed.name, other.name),
        notes,
        span: blamed.span.or(other.span),
    })
}

/// Is this item a non-mutable array `static`, i.e. a const table for the DATA
/// section?
fn is_const_array(item: &crate::ast::Spanned<crate::ast::Item>) -> bool {
    match &item.node {
        crate::ast::Item::Static(s) => {
            !s.mutable && matches!(s.ty.node, crate::ast::TypeExpr::Array { .. })
        }
        _ => false,
    }
}

/// Should this item be emitted?
///
/// `program.reachable_symbols` is the closure of calls and references from the
/// program's entry points (see `SemanticAnalyzer::reachable_symbols`); anything
/// outside it cannot be executed and would only pad the ROM. This applies to
/// the file being compiled as well as to imported modules — importing a module
/// pulls in its entire file, but an unreachable function is dead either way,
/// and sema has already warned about the ones written here.
///
/// Type definitions carry no code, and imports are handled by the caller.
pub(crate) fn is_live(item: &crate::ast::Spanned<crate::ast::Item>, program: &ProgramInfo) -> bool {
    match &item.node {
        crate::ast::Item::Function(f) => program.reachable_symbols.contains(&f.name.node),
        crate::ast::Item::Static(s) => program.reachable_symbols.contains(&s.name.node),
        _ => true,
    }
}

/// Find a function's AST node by name, in the root module or an imported one.
fn find_function<'a>(
    ast: &'a SourceFile,
    program: &'a ProgramInfo,
    name: &str,
) -> Option<&'a crate::ast::Function> {
    ast.items
        .iter()
        .chain(program.imported_items.iter())
        .find_map(|item| match &item.node {
            crate::ast::Item::Function(f) if f.name.node == name => Some(&**f),
            _ => None,
        })
}

/// Decide which auto-inline candidates to actually inline, and mark them
/// `is_inline` so placement and emission skip their bodies and every call site
/// expands them.
///
/// Candidacy (non-entry, non-`#[inline]`, no explicit placement, scalar/void
/// return, body+params captured) was set in sema. Here we add the constraints
/// that need the whole-program view — never inline an address-taken function
/// (its pointer needs a real address) or one in a call cycle (infinite
/// expansion) — and apply the size/reuse heuristic against real measured sizes:
///
/// - a single call site always wins (the out-of-line body disappears);
/// - a leaf body of ≤3 bytes (smaller than the `JSR` it replaces) wins anywhere;
/// - a larger leaf body wins where it stays size-neutral: `B·(N−1) ≤ 3N+1`.
///
/// Inlining a candidate is always *correct*; the heuristic only governs whether
/// it is a size win, so an imperfect call count can never miscompile.
fn select_auto_inline(
    ast: &SourceFile,
    program: &mut ProgramInfo,
    verbosity: CommentVerbosity,
    target: TargetCpu,
    layout: &memory_layout::MemoryLayout,
) -> Result<(), CodegenError> {
    use crate::ast::Expr;
    use rustc_hash::{FxHashMap, FxHashSet};

    // Count direct call sites per callee across the whole program, note callers
    // that make any call at all (non-leaf), and collect every identifier that
    // appears in an inline-asm block. A function a `JSR`/`JMP` inside inline asm
    // targets by name is not visible as an `Expr::Call`, so inlining it would
    // drop its definition and leave that reference dangling — exclude any
    // function named anywhere in inline asm (over-broad but always safe; asm
    // blocks are rare).
    let mut call_sites: FxHashMap<String, u32> = FxHashMap::default();
    let mut non_leaf: FxHashSet<String> = FxHashSet::default();
    // Functions that contain inline asm, or are named inside inline asm.
    // Inlining an asm-containing function is unreliable — inline expansion runs
    // `uniquify_asm_labels`, which mangles bare operands (an external `JSR
    // asm_only` becomes `JSR asm_only_1`). And a function a `JSR` inside asm
    // targets by name is invisible to the `Expr::Call` count, so dropping its
    // definition would dangle that reference. Exclude both (asm blocks are rare).
    let mut asm_unsafe: FxHashSet<String> = FxHashSet::default();
    for item in ast.items.iter().chain(program.imported_items.iter()) {
        if let crate::ast::Item::Function(f) = &item.node {
            let caller = f.name.node.clone();
            crate::sema::analyze::escape::walk_stmts(&f.body, &mut |s| {
                if let crate::ast::Stmt::Asm { lines } = &s.node {
                    asm_unsafe.insert(caller.clone());
                    for line in lines {
                        for tok in line
                            .instruction
                            .split(|c: char| !c.is_alphanumeric() && c != '_')
                        {
                            if !tok.is_empty() {
                                asm_unsafe.insert(tok.to_string());
                            }
                        }
                    }
                }
                crate::sema::analyze::escape::walk_exprs_in_stmt(s, &mut |e| {
                    if let Expr::Call { function, .. } = &e.node {
                        *call_sites.entry(function.node.clone()).or_insert(0) += 1;
                        non_leaf.insert(caller.clone());
                    }
                });
            });
        }
    }

    // Functions in any call cycle (direct or mutual recursion) must never be
    // inlined; owned so it doesn't borrow `program` across the mutation below.
    let recursive: FxHashSet<String> = program
        .recursive_call_edges
        .iter()
        .flat_map(|(a, b)| [a.clone(), b.clone()])
        .collect();

    let candidates: Vec<String> = program
        .function_metadata
        .iter()
        .filter(|(_, m)| m.inline_candidate)
        .map(|(n, _)| n.clone())
        .collect();

    let mut to_inline: Vec<String> = Vec::new();
    for name in &candidates {
        if program.address_taken_functions.contains(name)
            || recursive.contains(name)
            || asm_unsafe.contains(name)
        {
            continue;
        }
        let n = call_sites.get(name).copied().unwrap_or(0);
        if n == 0 {
            continue; // unreachable; dead-code elimination handles it
        }
        let Some(func) = find_function(ast, program, name) else {
            continue;
        };
        let is_leaf = !non_leaf.contains(name);
        let inline = if n == 1 {
            true
        } else if is_leaf {
            // `measure` pads every function by a safety slack; subtract it to
            // get the true body size the heuristic reasons about.
            let size = placement::measure(
                func,
                program,
                verbosity,
                target,
                layout,
                &mut StringCollector::new(),
            )?
            .saturating_sub(placement::MEASURE_SLACK) as u32;
            size <= 3 || size * (n - 1) <= 3 * n + 1
        } else {
            false
        };
        if inline {
            to_inline.push(name.clone());
        }
    }

    for name in to_inline {
        if let Some(m) = program.function_metadata.get_mut(&name) {
            m.is_inline = true;
        }
    }
    Ok(())
}

/// Narrow each interrupt handler's scratch save to the zero-page bytes its
/// reachable code actually writes.
///
/// A handler saves shared scratch so a preempted main thread's in-flight values
/// survive it, but it only needs to save an address it might *write*: one it
/// never writes, it cannot corrupt. Codegen knows the exact addresses where
/// sema's AST scan does not, so this emits each reachable function's body into a
/// throwaway emitter, unions the zero-page writes, and records them as the save
/// set (minus the frame span, which is saved separately).
///
/// It leaves the save at the full region (`scratch_addrs = None`) whenever the
/// graph is opaque — an indirect call, inline `asm`, a 16-bit math routine, or a
/// reachable function whose body cannot be scanned — because then the writes it
/// can see are not the whole story. Runs after inlining is decided, so an inline
/// callee's writes surface through the caller that expands it; inline functions
/// are skipped here so their frame-less bodies are never emitted in isolation.
fn narrow_interrupt_scratch(
    ast: &SourceFile,
    program: &mut ProgramInfo,
    verbosity: CommentVerbosity,
    target: TargetCpu,
    layout: &crate::codegen::memory_layout::MemoryLayout,
) {
    use crate::sema::table::SymbolLocation;
    use rustc_hash::FxHashSet as HashSet;

    // Named locations (a `const addr` register is stored by name, and is
    // `Absolute` even when its address is in zero page), so a store to one can
    // be classed as zero-page or not.
    let symbol_addrs: std::collections::HashMap<String, u16> = program
        .resolved_symbols
        .values()
        .filter_map(|s| match s.location {
            SymbolLocation::Absolute(a) => Some((s.name.clone(), a)),
            SymbolLocation::ZeroPage(a) => Some((s.name.clone(), a as u16)),
            _ => None,
        })
        .collect();

    let handlers: Vec<String> = program
        .interrupt_save_info
        .iter()
        .filter(|(_, si)| si.save_scratch && si.scratch_addrs.is_none())
        .map(|(h, _)| h.clone())
        .collect();

    for handler in handlers {
        let si = &program.interrupt_save_info[&handler];
        let reachable = si.reachable.clone();
        let frame_addrs: HashSet<u8> = si
            .shared_frames
            .iter()
            .flat_map(|(base, len)| (0..*len).map(move |i| base.wrapping_add(i)))
            .collect();

        let mut written = [false; 256];
        let mut opaque = false;
        for fname in &reachable {
            // An inline callee is expanded into a scanned caller, so its writes
            // are already counted; emitting its frame-less body alone would not.
            if program
                .function_metadata
                .get(fname)
                .is_some_and(|m| m.is_inline)
            {
                continue;
            }
            let Some(func) = find_function(ast, program, fname) else {
                opaque = true; // a name we cannot scan (e.g. a raw stdlib routine)
                break;
            };
            let mut e = Emitter::new(verbosity);
            e.target = target;
            e.memory_layout = layout.clone();
            e.symbol_addrs = symbol_addrs.clone();
            e.set_current_function(fname.clone());
            if stmt::generate_stmt(&func.body, &mut e, program, &mut StringCollector::new())
                .is_err()
            {
                opaque = true; // the real pass will surface the same error
                break;
            }
            // A call into a math routine or an indirect target writes scratch
            // this scan never saw; inline `asm` and unresolved stores set the
            // flag directly. Any of them, and the narrowing is unsafe.
            if e.zp_write_opaque
                || e.needs_mul16
                || e.needs_div16
                || e.needs_mod16
                || e.needs_indirect_call
            {
                opaque = true;
                break;
            }
            for (a, w) in e.zp_written.iter().enumerate() {
                written[a] |= *w;
            }
        }

        if opaque {
            continue; // keep the conservative full-region save
        }
        let scratch: Vec<u8> = (0..=255u8)
            .filter(|a| written[*a as usize] && !frame_addrs.contains(a))
            .collect();
        if let Some(si) = program.interrupt_save_info.get_mut(&handler) {
            si.scratch_addrs = Some(scratch);
        }
    }
}

/// Turn every `InitByte::StrLow`/`StrHigh` in a `static`'s startup image into
/// the `FnLow`/`FnHigh` label pair the emitters already know how to write.
///
/// Sema knows the literal but not its label; the string collector assigns
/// labels and deduplicates. Rather than teach both emitters about a third kind
/// of address, the content is resolved once, here, and everything downstream
/// sees the same two-bytes-the-assembler-fills-in it saw for a function.
fn resolve_string_inits(program: &mut ProgramInfo, strings: &mut StringCollector) {
    use crate::sema::InitByte;
    for init in program.static_inits.iter_mut() {
        for b in init.bytes.iter_mut() {
            match b {
                InitByte::StrLow(text) => {
                    let label = strings.add_string(text.clone());
                    *b = InitByte::FnLow(label);
                }
                InitByte::StrHigh(text) => {
                    let label = strings.add_string(text.clone());
                    *b = InitByte::FnHigh(label);
                }
                InitByte::Byte(_) | InitByte::FnLow(_) | InitByte::FnHigh(_) => {}
            }
        }
    }
}

pub fn generate(
    ast: &SourceFile,
    program: &mut ProgramInfo,
    verbosity: CommentVerbosity,
    target: TargetCpu,
) -> Result<(String, SectionAllocator), CodegenError> {
    use crate::sema::table::{SymbolKind, SymbolLocation};
    use rustc_hash::FxHashMap as HashMap;

    let mut emitter = Emitter::new(verbosity);
    emitter.target = target;
    // Place the software stack where the board's config says RAM is, rather than
    // baking a page into the compiler. Only its size and zero-page pointer are fixed.
    if let Some(stack) = program.memory_config.get_section("STACK") {
        emitter.memory_layout.software_stack_base = stack.start;
    }
    // Place sections from the same map sema resolved (`analyze_with_config` may
    // have set a source-relative one), rather than re-reading the working
    // directory's `wraith.toml` a second time and risking a different answer.
    let mut section_alloc = SectionAllocator::new(program.memory_config.clone());
    let mut string_collector = StringCollector::new();

    // A `str` in a `static`'s startup image carries the literal's *content*
    // out of sema, because the label belongs to the string collector, which
    // deduplicates identical literals. Register them here — before anything is
    // emitted, so the data lands in DATA with every other literal — and rewrite
    // each pair to the label reference the assembler fills in.
    resolve_string_inits(program, &mut string_collector);

    // Automatic inlining: promote auto-inline candidates whose expansion is a
    // size win to `is_inline`, before placement so the layout already excludes
    // their bodies and every call site expands them. Doing it here (not in sema)
    // is what lets the heuristic use real measured function sizes.
    select_auto_inline(
        ast,
        program,
        verbosity,
        target,
        &emitter.memory_layout.clone(),
    )?;

    // Narrow each interrupt handler's scratch save to the zero-page bytes its
    // reachable code actually writes. Runs after inlining is decided (so an
    // inline callee's writes are seen through its caller) and before placement,
    // so the measuring and real passes emit the same, smaller, prologue.
    narrow_interrupt_scratch(
        ast,
        program,
        verbosity,
        target,
        &emitter.memory_layout.clone(),
    );

    // Decide where every function goes before emitting anything. Doing this up
    // front is what lets `#[org]` reserve its range: the allocator can only
    // route other functions around a pinned one if it knows about it before it
    // starts handing out addresses. Const arrays and string literals allocate
    // from DATA further down, so this has to come before them too.
    let placement = placement::plan(
        &program.imported_items,
        ast,
        program,
        verbosity,
        target,
        &emitter.memory_layout.clone(),
        &mut section_alloc,
        &mut string_collector,
    )?;

    // Build a map of symbol names to their import source file
    let mut import_sources: HashMap<String, String> = HashMap::default();
    for item in &ast.items {
        if let crate::ast::Item::Import(import) = &item.node {
            for symbol in &import.symbols {
                import_sources.insert(symbol.node.clone(), import.path.node.clone());
            }
        }
    }

    // Emit address labels for all addresses (including imported ones)
    // Use resolved_symbols which contains all symbols that are actually used
    let mut emitted_addresses = HashSet::default();

    // Emit addresses from resolved_symbols (includes both local and imported addresses)
    for symbol in program.resolved_symbols.values() {
        if symbol.kind == SymbolKind::Address
            && let SymbolLocation::Absolute(addr) = symbol.location
            && emitted_addresses.insert(symbol.name.clone())
        {
            // Emit comment if this address was imported
            if let Some(source) = import_sources.get(&symbol.name) {
                emitter.emit_comment(&format!("Imported from {}", source));
            }
            emitter.emit_raw(&format!("{} = ${:04X}", symbol.name, addr));
        }
    }

    // Track which items have been emitted to avoid duplicates
    let mut emitted_items: HashSet<String> = HashSet::default();

    // Emit const arrays to DATA section FIRST
    // This separates read-only data from code
    let has_const_arrays = ast
        .items
        .iter()
        .any(|item| is_const_array(item) && is_live(item, program))
        || program
            .imported_items
            .iter()
            .any(|item| is_const_array(item) && is_live(item, program));

    if has_const_arrays {
        emitter.emit_comment("============================================================");
        emitter.emit_comment("Data Section (Const Arrays)");
        emitter.emit_comment("============================================================");

        // Each array now emits its own .ORG at an address taken from the DATA
        // section in wraith.toml, so there is no blanket origin here.
        emitter.emit_raw("");

        // Emit const arrays from imported modules first
        for item in &program.imported_items {
            if let crate::ast::Item::Static(s) = &item.node
                && !s.mutable
                && matches!(s.ty.node, crate::ast::TypeExpr::Array { .. })
                && is_live(item, program)
            {
                let name = s.name.node.clone();
                if emitted_items.insert(name) {
                    generate_item(
                        item,
                        &mut emitter,
                        program,
                        &placement,
                        &mut section_alloc,
                        &mut string_collector,
                    )?;
                }
            }
        }

        // Emit const arrays from main module
        for item in &ast.items {
            if let crate::ast::Item::Static(s) = &item.node
                && !s.mutable
                && matches!(s.ty.node, crate::ast::TypeExpr::Array { .. })
                && is_live(item, program)
            {
                let name = s.name.node.clone();
                if emitted_items.insert(name) {
                    generate_item(
                        item,
                        &mut emitter,
                        program,
                        &placement,
                        &mut section_alloc,
                        &mut string_collector,
                    )?;
                }
            }
        }

        emitter.emit_raw("");
    }

    // Generate code for imported items FIRST
    // This ensures that imported functions are defined before they're called
    // Only emit section header if there are actually imported items to generate
    let has_imported_code = program.imported_items.iter().any(|item| {
        !matches!(
            item.node,
            crate::ast::Item::Import(_)
                | crate::ast::Item::Address(_)
                | crate::ast::Item::Static(_)
        ) && is_live(item, program)
    });

    if has_imported_code {
        emitter.emit_comment("============================================================");
        emitter.emit_comment("Code from imported modules");
        emitter.emit_comment("============================================================");
    }

    for item in &program.imported_items {
        // Importing a module makes its whole file available, but only the part
        // the program actually reaches is worth emitting.
        if !is_live(item, program) {
            continue;
        }

        // Get the item name to check for duplicates
        let item_name = match &item.node {
            crate::ast::Item::Function(f) => Some(f.name.node.clone()),
            crate::ast::Item::Static(s) => Some(s.name.node.clone()),
            crate::ast::Item::Struct(s) => Some(s.name.node.clone()),
            crate::ast::Item::Enum(e) => Some(e.name.node.clone()),
            crate::ast::Item::Address(a) => Some(a.name.node.clone()),
            crate::ast::Item::Import(_) => None, // Skip imports
        };

        // Skip if we've already emitted this item or if it's an import
        if let Some(name) = item_name {
            if !emitted_items.insert(name.clone()) {
                continue; // Already emitted
            }
        } else {
            continue; // It's an import, skip it
        }

        // Skip address declarations - they were already emitted above
        if matches!(item.node, crate::ast::Item::Address(_)) {
            continue;
        }

        generate_item(
            item,
            &mut emitter,
            program,
            &placement,
            &mut section_alloc,
            &mut string_collector,
        )?;
    }

    // Generate code for main module items
    // Only emit section header if there are actually main module items to generate
    let has_main_code = ast.items.iter().any(|item| {
        !matches!(
            item.node,
            crate::ast::Item::Import(_)
                | crate::ast::Item::Address(_)
                | crate::ast::Item::Static(_)
        ) && is_live(item, program)
    });

    if has_main_code {
        emitter.emit_comment("============================================================");
        emitter.emit_comment("Code from main module");
        emitter.emit_comment("============================================================");
    }

    for item in &ast.items {
        // Unreachable items are dropped here as well as for imports; sema has
        // already warned about the ones written in this file.
        if !is_live(item, program) {
            continue;
        }

        // Get the item name to check for duplicates
        let item_name = match &item.node {
            crate::ast::Item::Function(f) => Some(f.name.node.clone()),
            crate::ast::Item::Static(s) => Some(s.name.node.clone()),
            crate::ast::Item::Struct(s) => Some(s.name.node.clone()),
            crate::ast::Item::Enum(e) => Some(e.name.node.clone()),
            crate::ast::Item::Address(a) => Some(a.name.node.clone()),
            crate::ast::Item::Import(_) => None,
        };

        // Skip if we've already emitted this item
        if let Some(name) = &item_name
            && !emitted_items.insert(name.clone())
        {
            continue; // Already emitted
        }

        // Skip address declarations - they were already emitted above
        if matches!(item.node, crate::ast::Item::Address(_)) {
            continue;
        }
        generate_item(
            item,
            &mut emitter,
            program,
            &placement,
            &mut section_alloc,
            &mut string_collector,
        )?;
    }

    // Emit collected string literals to DATA section
    // Content-based labels ensure cross-module deduplication
    string_collector.emit_strings(&mut emitter, &mut section_alloc)?;
    string_collector.emit_enum_data(&mut emitter, &mut section_alloc)?;

    // Emit stdlib math functions if needed
    emit_stdlib_math_functions(&mut emitter, &mut section_alloc)?;

    // Emit the indirect-call trampoline if any function pointer is called.
    if emitter.needs_indirect_call {
        let org_addr = section_alloc
            .allocate("CODE", 3)
            .map_err(CodegenError::SectionError)?;
        emitter.emit_org(org_addr);
        emitter.emit_comment("Indirect-call trampoline: JSR here after loading the");
        emitter.emit_comment("target address into the indirect vector at $EE/$EF.");
        emitter.emit_raw("__indirect_call:");
        emitter.emit_raw("    JMP ($EE)");
    }

    // Generate interrupt vector table
    if generate_interrupt_vectors(ast, &mut emitter)? {
        // The 6502 fetches its reset and interrupt vectors from $FFFA-$FFFF, so
        // that range belongs to the hardware. Record it: an #[org] overlapping
        // it silently corrupts either the handler or the reset vector, and a
        // machine that will not boot is a poor way to find out.
        section_alloc.record_allocation(
            "interrupt vector table".to_string(),
            0xFFFA,
            6,
            AllocationSource::Reserved,
            None,
        );
    }

    // Every address-space occupant has now been recorded, so overlaps can be
    // checked once, over all of them. Doing this earlier only ever compared
    // functions against each other.
    report_address_conflicts(&section_alloc)?;

    // Collect memory-mapped I/O symbols so the peephole optimizer never folds
    // their accesses. Reads and writes are tracked by declared access mode
    // (R / W / RW) so the guard matches which direction carries side effects.
    let mut volatile = peephole::VolatileSymbols::default();
    for symbol in program.resolved_symbols.values() {
        if symbol.kind == SymbolKind::Address {
            match symbol.access_mode {
                Some(crate::ast::AccessMode::Read) => {
                    volatile.reads.insert(symbol.name.clone());
                }
                Some(crate::ast::AccessMode::Write) => {
                    volatile.writes.insert(symbol.name.clone());
                }
                // ReadWrite (or unspecified, which defaults to read-write) is
                // volatile in both directions.
                _ => {
                    volatile.reads.insert(symbol.name.clone());
                    volatile.writes.insert(symbol.name.clone());
                }
            }
        }
    }

    // Apply peephole optimizations
    let target = emitter.target;
    let asm = emitter.finish();
    let lines = peephole::parse_assembly(&asm);
    let optimized = peephole::optimize(&lines, &volatile, target);
    let final_asm = peephole::lines_to_string(&optimized);

    Ok((final_asm, section_alloc))
}

/// Generate the 6502 interrupt vector table at $FFFA-$FFFF.
///
/// Returns whether the table was emitted, so the caller can reserve the range
/// it occupies for conflict checking.
fn generate_interrupt_vectors(
    ast: &SourceFile,
    emitter: &mut Emitter,
) -> Result<bool, CodegenError> {
    use crate::ast::{FnAttribute, Item};

    // Find interrupt handlers
    let mut nmi_handler: Option<String> = None;
    let mut reset_handler: Option<String> = None;
    let mut irq_handler: Option<String> = None;

    for item in &ast.items {
        if let Item::Function(func) = &item.node {
            let name = func.name.node.clone();

            for attr in &func.attributes {
                match attr {
                    FnAttribute::Nmi => {
                        if nmi_handler.is_some() {
                            return Err(CodegenError::UnsupportedOperation(
                                "Multiple NMI handlers defined".to_string(),
                            ));
                        }
                        nmi_handler = Some(name.clone());
                    }
                    FnAttribute::Reset => {
                        if reset_handler.is_some() {
                            return Err(CodegenError::UnsupportedOperation(
                                "Multiple RESET handlers defined".to_string(),
                            ));
                        }
                        reset_handler = Some(name.clone());
                    }
                    FnAttribute::Irq => {
                        if irq_handler.is_some() {
                            return Err(CodegenError::UnsupportedOperation(
                                "Multiple IRQ handlers defined".to_string(),
                            ));
                        }
                        irq_handler = Some(name.clone());
                    }
                    _ => {}
                }
            }
        }
    }

    // Only generate vector table if at least one handler is defined
    if nmi_handler.is_some() || reset_handler.is_some() || irq_handler.is_some() {
        emitter.emit_comment("============================");
        emitter.emit_comment("Interrupt Vector Table");
        emitter.emit_org(0xFFFA);

        // NMI vector at $FFFA
        if let Some(handler) = nmi_handler {
            emitter.emit_comment(&format!("NMI vector -> {}", handler));
            emitter.emit_word_label(&handler);
        } else {
            emitter.emit_comment("NMI vector (not used)");
            emitter.emit_word(0);
        }

        // RESET vector at $FFFC
        if let Some(handler) = reset_handler {
            emitter.emit_comment(&format!("RESET vector -> {}", handler));
            emitter.emit_word_label(&handler);
        } else {
            emitter.emit_comment("RESET vector (not used)");
            emitter.emit_word(0);
        }

        // IRQ/BRK vector at $FFFE
        if let Some(handler) = irq_handler {
            emitter.emit_comment(&format!("IRQ/BRK vector -> {}", handler));
            emitter.emit_word_label(&handler);
        } else {
            emitter.emit_comment("IRQ/BRK vector (not used)");
            emitter.emit_word(0);
        }
        return Ok(true);
    }

    Ok(false)
}
