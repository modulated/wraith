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

    /// Every symbol the program can actually reach, as a transitive closure
    /// over `symbol_refs` from the program's entry points.
    ///
    /// Codegen emits only what is in this set. The walk has to span modules:
    /// importing a module makes its *whole* file available, and an imported
    /// function may call a sibling the importing file never named, so liveness
    /// cannot be read off the import list.
    ///
    /// The roots are the places execution can begin or arrive from outside the
    /// call graph:
    ///
    /// - the `#[reset]`, `#[irq]` and `#[nmi]` handlers, which the hardware
    ///   enters through the vector table, and `main`, the conventional entry
    ///   point when no `#[reset]` is given;
    /// - functions pinned by `#[org]` or `#[section]`, since the reason to fix
    ///   an address is usually to let something outside the program call it;
    /// - functions whose address is taken, reachable through a pointer that no
    ///   static call edge records;
    /// - names referenced outside any function body, such as in a `static`'s
    ///   initializer.
    ///
    /// Being wrong in the direction of keeping too much wastes ROM; dropping a
    /// live function is a jump into whatever follows it. Two things keep the
    /// walk safe rather than optimistic: `record_asm_call_edges` turns a `JSR`
    /// inside inline assembly into an edge, and address-taken functions are
    /// roots.
    pub(super) fn reachable_symbols(
        &self,
        root_module: &crate::ast::SourceFile,
    ) -> HashSet<String> {
        use crate::ast::{FnAttribute, Item};

        let mut worklist: Vec<String> = Vec::new();

        // Types carry no code and are not emitted, so only functions can be
        // roots here.
        for item in &root_module.items {
            if let Item::Function(f) = &item.node {
                let name = f.name.node.clone();
                let is_entry_point = f.attributes.iter().any(|a| {
                    matches!(
                        a,
                        FnAttribute::Reset
                            | FnAttribute::Irq
                            | FnAttribute::Nmi
                            | FnAttribute::Interrupt
                            | FnAttribute::Org(_)
                            | FnAttribute::Section(_)
                    )
                });
                if is_entry_point || name == "main" {
                    worklist.push(name);
                }
            }
        }

        // A file with no entry point at all is not a program in which
        // everything is dead — it is one where we cannot know what runs. A
        // library, or something still being written. Emitting an empty ROM
        // because `#[reset]` has not been added yet would be a poor answer, so
        // keep the whole module and let the vector table's absence speak.
        if worklist.is_empty() {
            return root_module
                .items
                .iter()
                .filter_map(|item| match &item.node {
                    Item::Function(f) => Some(f.name.node.clone()),
                    Item::Static(s) => Some(s.name.node.clone()),
                    _ => None,
                })
                .chain(self.symbol_refs.values().flatten().cloned())
                .collect();
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

    /// Warn about every root-module function and static the program cannot
    /// reach — exactly the set codegen is about to drop, so the warning and the
    /// output agree.
    ///
    /// This replaces a check that only asked whether a function was ever
    /// *called*, which missed a dead function called by another dead function
    /// and reported nothing at all about unused constants.
    pub(super) fn warn_unreachable_items(
        &mut self,
        root_module: &crate::ast::SourceFile,
        reachable: &HashSet<String>,
    ) {
        use crate::ast::Item;

        // With no entry point nothing is reachable by definition, and
        // `reachable_symbols` keeps the whole module rather than emitting an
        // empty ROM. Warning about every item in that case would be noise.
        let has_entry_point = root_module.items.iter().any(|item| match &item.node {
            crate::ast::Item::Function(f) => {
                f.name.node == "main"
                    || f.attributes.iter().any(|a| {
                        matches!(
                            a,
                            crate::ast::FnAttribute::Reset
                                | crate::ast::FnAttribute::Irq
                                | crate::ast::FnAttribute::Nmi
                                | crate::ast::FnAttribute::Interrupt
                                | crate::ast::FnAttribute::Org(_)
                                | crate::ast::FnAttribute::Section(_)
                        )
                    })
            }
            _ => false,
        });
        if !has_entry_point {
            return;
        }

        let mut warnings = Vec::new();
        for item in &root_module.items {
            match &item.node {
                Item::Function(f) => {
                    let name = &f.name.node;
                    // Inline functions are expanded at their call sites and
                    // never emitted on their own, so "unused" does not apply.
                    let is_inline = self
                        .function_metadata
                        .get(name)
                        .is_some_and(|m| m.is_inline);
                    if !is_inline && !reachable.contains(name) {
                        warnings.push(Warning::UnusedFunction {
                            name: name.clone(),
                            span: f.name.span,
                        });
                    }
                }
                Item::Static(st) => {
                    let name = &st.name.node;
                    if !reachable.contains(name) {
                        warnings.push(Warning::UnusedStatic {
                            name: name.clone(),
                            span: st.name.span,
                        });
                    }
                }
                _ => {}
            }
        }
        self.warnings.extend(warnings);
    }
}
