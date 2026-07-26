//! Unused Item Detection
//!
//! Generates warnings for unused variables, imports, and functions.

use crate::sema::Warning;
use rustc_hash::FxHashSet as HashSet;

use super::SemanticAnalyzer;

impl SemanticAnalyzer {
    /// Record that the function currently being analyzed references `name`.
    ///
    /// Feeds the liveness walk that decides which imported items reach the
    /// output. References outside any function body — a `static` initializer,
    /// say — are keyed under `None` and treated as always live, since there is
    /// no enclosing item whose removal would take them with it.
    pub(super) fn record_symbol_ref(&mut self, name: &str) {
        self.symbol_refs
            .entry(self.current_function.clone())
            .or_default()
            .insert(name.to_string());
    }

    /// Check for unused variables and parameters, generate warnings
    pub(super) fn check_unused_variables(&mut self) {
        // Check unused local variables
        for (var_name, var_span) in &self.declared_variables {
            if !self.used_variables.contains(var_name) {
                self.warnings.push(Warning::UnusedVariable {
                    name: var_name.clone(),
                    span: *var_span,
                });
            }
        }

        // Check unused function parameters
        // Skip parameters starting with _ (convention for intentionally unused)
        for (param_name, param_span) in &self.declared_parameters {
            if !param_name.starts_with('_') && !self.used_variables.contains(param_name) {
                self.warnings.push(Warning::UnusedParameter {
                    name: param_name.clone(),
                    span: *param_span,
                });
            }
        }

        // Clear for next function/scope
        self.declared_variables.clear();
        self.declared_parameters.clear();
        self.used_variables.clear();
    }

    /// Extract variable references from inline assembly template strings
    /// Variables are referenced as {var_name} or {struct.field}
    pub(super) fn extract_asm_variables(&mut self, instruction: &str) {
        let mut chars = instruction.chars().peekable();

        while let Some(ch) = chars.next() {
            if ch == '{' {
                // Extract variable name between { and }
                let mut var_name = String::new();

                while let Some(&next_ch) = chars.peek() {
                    if next_ch == '}' {
                        chars.next(); // Consume the '}'
                        break;
                    }
                    var_name.push(next_ch);
                    chars.next();
                }

                // Handle struct field access: {struct.field}
                // Mark the base variable (before the dot) as used
                let base_var = if let Some(dot_pos) = var_name.find('.') {
                    &var_name[..dot_pos]
                } else {
                    &var_name
                };

                if !base_var.is_empty() {
                    // Mark variable as used
                    self.used_variables.insert(base_var.to_string());
                    self.all_used_symbols.insert(base_var.to_string());
                    self.record_symbol_ref(base_var);
                }
            }
        }
    }

    /// Record `JSR`/`JMP <function>` targets in an inline-assembly line as
    /// call-graph edges from the current function. Only operands that name a
    /// known wraith function are added (local asm labels and addresses are
    /// ignored). This keeps frame coloring correct for functions that are only
    /// ever invoked from hand-written assembly.
    pub(super) fn record_asm_call_edges(&mut self, instruction: &str) {
        let Some(caller) = self.current_function.clone() else {
            return;
        };
        // Drop any trailing assembler comment.
        let code = instruction.split(';').next().unwrap_or("");
        let mut tokens = code.split_whitespace();
        while let Some(tok) = tokens.next() {
            let mnemonic = tok.to_ascii_uppercase();
            if (mnemonic == "JSR" || mnemonic == "JMP")
                && let Some(operand) = tokens.next()
            {
                // Strip label punctuation / substitution braces / addressing syntax.
                let target = operand.trim_matches(|c: char| !c.is_alphanumeric() && c != '_');
                if !target.is_empty() && self.function_metadata.contains_key(target) {
                    // A function invoked from assembly is genuinely used.
                    self.called_functions.insert(target.to_string());
                    self.all_used_symbols.insert(target.to_string());
                    self.record_symbol_ref(target);
                    if target != caller {
                        self.call_edges
                            .entry(caller.clone())
                            .or_default()
                            .insert(target.to_string());
                    }
                }
            }
        }
    }

    /// Check for unused imports and generate warnings
    /// This should be called at the end of file analysis, after all symbols have been used
    pub(super) fn check_unused_imports(&mut self) {
        // all_used_symbols tracks usage across entire file
        // Check which imported symbols were never used
        for (import_name, import_span) in &self.imported_symbols {
            if !self.all_used_symbols.contains(import_name) {
                self.warnings.push(Warning::UnusedImport {
                    name: import_name.clone(),
                    span: *import_span,
                });
            }
        }
    }

    /// Check for unused functions and generate warnings
    pub(super) fn check_unused_functions(&mut self) {
        // Check which declared functions were never called
        for (func_name, func_span) in &self.declared_functions {
            if !self.called_functions.contains(func_name) {
                self.warnings.push(Warning::UnusedFunction {
                    name: func_name.clone(),
                    span: *func_span,
                });
            }
        }
    }

    /// Every symbol the program can actually reach, as a transitive closure over
    /// `symbol_refs` from the root module.
    ///
    /// Codegen drops imported items outside this set. Importing a module makes
    /// its *whole* file available — an imported function may call a sibling the
    /// importing file never named — so the set has to be computed over the merged
    /// reference graph rather than from the import list.
    ///
    /// The roots are everything the root module defines (its own items are always
    /// emitted, dead or not: a file's contents are its author's business, and an
    /// unused one already warns), every function whose address is taken (it can
    /// be reached through a pointer that no static edge records), and anything
    /// referenced outside a function body, such as a `static`'s initializer.
    ///
    /// Being wrong in the direction of keeping too much is harmless; dropping a
    /// live function is a link error. Two things make the walk safe rather than
    /// optimistic: `record_asm_call_edges` turns a `JSR` inside inline assembly
    /// into an edge, and taking a function's address counts as a root.
    pub(super) fn reachable_symbols(
        &self,
        root_module: &crate::ast::SourceFile,
    ) -> HashSet<String> {
        let mut worklist: Vec<String> = Vec::new();

        // Root module items are always live.
        for item in &root_module.items {
            if let Some(name) = item_name(&item.node) {
                worklist.push(name);
            }
        }
        // Reached through a function pointer, so no call edge names them.
        worklist.extend(self.address_taken_functions.iter().cloned());
        // Module-level references (static initializers) have no owning item.
        if let Some(refs) = self.symbol_refs.get(&None) {
            worklist.extend(refs.iter().cloned());
        }

        let mut reached: HashSet<String> = worklist.iter().cloned().collect();
        while let Some(name) = worklist.pop() {
            let Some(refs) = self.symbol_refs.get(&Some(name)) else {
                continue;
            };
            for r in refs {
                if reached.insert(r.clone()) {
                    worklist.push(r.clone());
                }
            }
        }
        reached
    }
}

/// The name a top-level item defines, if it defines one.
fn item_name(item: &crate::ast::Item) -> Option<String> {
    use crate::ast::Item;
    match item {
        Item::Function(f) => Some(f.name.node.clone()),
        Item::Static(s) => Some(s.name.node.clone()),
        Item::Struct(s) => Some(s.name.node.clone()),
        Item::Enum(e) => Some(e.name.node.clone()),
        Item::Address(a) => Some(a.name.node.clone()),
        Item::Import(_) => None,
    }
}
