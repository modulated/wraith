//! Tail Call Optimization Detection
//!
//! Analyzes functions to detect tail recursive calls that can be optimized.

use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};

use crate::ast::{Expr, Item, SourceFile, Span, Spanned, Stmt};
use crate::sema::TailCallInfo;

use super::SemanticAnalyzer;

impl SemanticAnalyzer {
    /// Analyze all functions for tail recursive calls
    /// This pass runs after all other analysis is complete
    pub(super) fn analyze_tail_calls(
        &mut self,
        source: &SourceFile,
    ) -> HashMap<String, TailCallInfo> {
        let mut tail_call_info = HashMap::default();

        for item in &source.items {
            if let Item::Function(func) = &item.node {
                let func_name = func.name.node.clone();
                let info = self.detect_tail_recursion(&func_name, &func.body);

                // Update function metadata if tail recursion detected
                if !info.tail_recursive_returns.is_empty()
                    && let Some(metadata) = self.function_metadata.get_mut(&func_name)
                {
                    metadata.has_tail_recursion = true;
                }

                tail_call_info.insert(func_name, info);
            }
        }

        tail_call_info
    }

    /// Detect tail recursive calls in a function body
    fn detect_tail_recursion(&self, func_name: &str, body: &Spanned<Stmt>) -> TailCallInfo {
        let mut tail_recursive_returns = HashSet::default();
        self.find_tail_recursive_returns(func_name, body, &mut tail_recursive_returns);

        TailCallInfo {
            tail_recursive_returns,
        }
    }

    /// Recursively find return statements with tail recursive calls
    fn find_tail_recursive_returns(
        &self,
        func_name: &str,
        stmt: &Spanned<Stmt>,
        tail_recursive_returns: &mut HashSet<Span>,
    ) {
        match &stmt.node {
            Stmt::Return(Some(expr)) => {
                // Check if this is a direct call to the same function
                if let Expr::Call { function, .. } = &expr.node
                    && function.node == func_name
                {
                    // This is a tail recursive call!
                    tail_recursive_returns.insert(stmt.span);
                }
            }

            // Recurse into block statements
            Stmt::Block(stmts) => {
                for s in stmts {
                    self.find_tail_recursive_returns(func_name, s, tail_recursive_returns);
                }
            }

            // Recurse into if/else (both branches must be checked)
            Stmt::If {
                then_branch,
                else_branch,
                ..
            } => {
                self.find_tail_recursive_returns(func_name, then_branch, tail_recursive_returns);
                if let Some(alt) = else_branch {
                    self.find_tail_recursive_returns(func_name, alt, tail_recursive_returns);
                }
            }

            // Recurse into while loops
            Stmt::While { body, .. } => {
                self.find_tail_recursive_returns(func_name, body, tail_recursive_returns);
            }

            // Recurse into loop
            Stmt::Loop { body } => {
                self.find_tail_recursive_returns(func_name, body, tail_recursive_returns);
            }

            // Recurse into for loops
            Stmt::For { body, .. } => {
                self.find_tail_recursive_returns(func_name, body, tail_recursive_returns);
            }

            // Recurse into match arms
            Stmt::Match { arms, .. } => {
                for arm in arms {
                    self.find_tail_recursive_returns(func_name, &arm.body, tail_recursive_returns);
                }
            }

            _ => {
                // Other statements (VarDecl, Assignment, etc.) don't contain returns
            }
        }
    }
}
