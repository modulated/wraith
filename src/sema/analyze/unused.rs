//! Unused Item Detection
//!
//! Generates warnings for unused variables, imports, and functions.

use crate::sema::Warning;

use super::SemanticAnalyzer;

impl SemanticAnalyzer {
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
}
