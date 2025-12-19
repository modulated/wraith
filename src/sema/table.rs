//! Symbol Table
//!
//! Handles scoping and symbol lookups.

use std::collections::HashMap;

use super::types::Type;

#[derive(Debug, Clone, PartialEq)]
pub enum SymbolKind {
    Variable,
    Function,
    Type, // Struct or Enum name
    Constant,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SymbolLocation {
    ZeroPage(u8),
    Stack(i8),
    Absolute(u16),
    None, // For types or compile-time constants
}

#[derive(Debug, Clone)]
pub struct SymbolInfo {
    pub name: String,
    pub kind: SymbolKind,
    pub ty: Type,
    pub location: SymbolLocation,
    pub mutable: bool,
}

#[derive(Debug, Clone)]
pub struct SymbolTable {
    scopes: Vec<HashMap<String, SymbolInfo>>,
}

impl SymbolTable {
    pub fn new() -> Self {
        Self {
            scopes: vec![HashMap::new()],
        }
    }

    pub fn enter_scope(&mut self) {
        self.scopes.push(HashMap::new());
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
}

impl Default for SymbolTable {
    fn default() -> Self {
        Self::new()
    }
}
