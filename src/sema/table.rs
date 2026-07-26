//! Symbol Table
//!
//! Handles scoping and symbol lookups.

use rustc_hash::FxHashMap as HashMap;

use super::types::Type;
use crate::ast::AccessMode;

#[derive(Debug, Clone, PartialEq)]
pub enum SymbolKind {
    Variable,
    Function,
    Type, // Struct or Enum name
    Constant,
    Address, // Memory-mapped address declaration (addr keyword)
}

#[derive(Debug, Clone, PartialEq)]
pub enum SymbolLocation {
    ZeroPage(u8),
    Absolute(u16),
    None, // For types or compile-time constants
    /// Offset within the enclosing function's frame, assigned during analysis.
    /// The `finalize_frames` pass rewrites every `FrameOffset` into a concrete
    /// `ZeroPage(frame_base + offset)` before code generation. Encountering a
    /// `FrameOffset` in codegen is an internal error (frame finalization was skipped).
    FrameOffset(u8),
}

#[derive(Debug, Clone, PartialEq)]
pub struct SymbolInfo {
    pub name: String,
    pub kind: SymbolKind,
    pub ty: Type,
    pub location: SymbolLocation,
    pub mutable: bool,
    /// Access mode for address declarations (read-only, write-only, or read-write)
    pub access_mode: Option<AccessMode>,
    /// Visibility: true if marked with `pub`, false if private
    pub is_pub: bool,
    /// The function this symbol is defined in (None for global symbols)
    pub containing_function: Option<String>,
    /// True if this symbol is a function parameter (distinguishes params from
    /// locals now that both live in the function frame). Replaces the old
    /// address-range test that assumed parameters occupied $80-$BF.
    pub is_param: bool,
}

#[derive(Debug, Clone)]
pub struct SymbolTable {
    scopes: Vec<HashMap<String, SymbolInfo>>,
}

impl SymbolTable {
    pub fn new() -> Self {
        Self {
            scopes: vec![HashMap::default()],
        }
    }

    pub fn enter_scope(&mut self) {
        self.scopes.push(HashMap::default());
    }

    pub fn exit_scope(&mut self) {
        if self.scopes.len() > 1 {
            self.scopes.pop();
        }
    }

    pub fn insert(&mut self, name: String, info: SymbolInfo) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name, info);
        }
    }

    pub fn lookup(&self, name: &str) -> Option<&SymbolInfo> {
        for scope in self.scopes.iter().rev() {
            if let Some(info) = scope.get(name) {
                return Some(info);
            }
        }
        None
    }

    /// Check if a symbol exists in the current (innermost) scope
    pub fn defined_in_current_scope(&self, name: &str) -> bool {
        if let Some(scope) = self.scopes.last() {
            scope.contains_key(name)
        } else {
            false
        }
    }

    /// Every symbol in the outermost (module) scope, sorted by name.
    ///
    /// Used by glob imports (`import { * } from "m.wr"`), which need to
    /// enumerate a module's items rather than look up known names. Sorted so
    /// the resulting import order — and therefore code layout and any
    /// diagnostics — does not depend on hash iteration order.
    pub fn module_symbols(&self) -> Vec<(&String, &SymbolInfo)> {
        let mut symbols: Vec<(&String, &SymbolInfo)> = match self.scopes.first() {
            Some(scope) => scope.iter().collect(),
            None => Vec::new(),
        };
        symbols.sort_by(|a, b| a.0.cmp(b.0));
        symbols
    }
}

impl Default for SymbolTable {
    fn default() -> Self {
        Self::new()
    }
}
