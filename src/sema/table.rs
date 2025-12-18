//! Symbol Table
//!
//! Handles scoping and symbol lookups.

use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
pub enum SymbolType {
    Variable,
    Function,
    Constant,
}

#[derive(Debug, Clone)]
pub struct SymbolInfo {
    pub name: String,
    pub kind: SymbolType,
    // TODO: Add type info, memory location, etc.
}

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
}
