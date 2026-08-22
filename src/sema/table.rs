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
    /// Span of the name at the declaration site. Codegen keys per-declaration
    /// state (e.g. an enum local's data block) on it, and a use site finds the
    /// declaration through the resolved symbol.
    pub decl_span: Option<crate::ast::Span>,
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

    /// The visible name closest to `target`, for a "did you mean" hint.
    ///
    /// Returns a name within a small edit distance (1 for short targets, 2
    /// otherwise) — close enough to be a likely typo, not a coincidence. `None`
    /// when nothing is near, so the caller adds no misleading suggestion.
    pub fn closest_name(&self, target: &str) -> Option<String> {
        let limit = if target.len() <= 3 { 1 } else { 2 };
        let mut best: Option<(usize, &str)> = None;
        for scope in &self.scopes {
            for name in scope.keys() {
                let d = levenshtein(target, name);
                if d <= limit && d > 0 && best.is_none_or(|(bd, _)| d < bd) {
                    best = Some((d, name));
                }
            }
        }
        best.map(|(_, n)| n.to_string())
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

/// Levenshtein edit distance between two identifiers, for typo suggestions.
/// Small strings, so the straightforward two-row dynamic program is plenty.
pub(crate) fn levenshtein(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut curr = vec![0usize; b.len() + 1];
    for (i, ca) in a.chars().enumerate() {
        curr[0] = i + 1;
        for (j, &cb) in b.iter().enumerate() {
            let cost = if ca == cb { 0 } else { 1 };
            curr[j + 1] = (prev[j] + cost).min(prev[j + 1] + 1).min(curr[j] + 1);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[b.len()]
}

#[cfg(test)]
mod suggestion_tests {
    use super::*;

    #[test]
    fn closest_name_finds_a_near_typo() {
        let mut t = SymbolTable::new();
        t.insert("counter".to_string(), dummy());
        t.insert("index".to_string(), dummy());
        assert_eq!(t.closest_name("countr").as_deref(), Some("counter"));
        assert_eq!(t.closest_name("indx").as_deref(), Some("index"));
    }

    #[test]
    fn closest_name_declines_when_nothing_is_close() {
        let mut t = SymbolTable::new();
        t.insert("counter".to_string(), dummy());
        assert_eq!(t.closest_name("xyzzy"), None);
    }

    fn dummy() -> SymbolInfo {
        SymbolInfo {
            name: String::new(),
            kind: SymbolKind::Variable,
            ty: crate::sema::types::Type::Primitive(crate::ast::PrimitiveType::U8),
            location: SymbolLocation::None,
            mutable: true,
            access_mode: None,
            is_pub: false,
            containing_function: None,
            is_param: false,
            decl_span: None,
        }
    }
}
