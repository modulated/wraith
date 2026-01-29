//! Statement Analysis
//!
//! Type checking and semantic analysis for all statement variants.

use crate::ast::{Expr, Pattern, PrimitiveType, Span, Spanned, Stmt};
use crate::sema::const_eval::eval_const_expr_with_env;
use crate::sema::table::{SymbolInfo, SymbolKind, SymbolLocation};
use crate::sema::type_defs::VariantData;
use crate::sema::types::Type;
use crate::sema::{SemaError, Warning};
use rustc_hash::FxHashSet as HashSet;

use super::SemanticAnalyzer;

impl SemanticAnalyzer {
    pub(super) fn analyze_stmt(&mut self, stmt: &Spanned<Stmt>) -> Result<(), SemaError> {
        match &stmt.node {
            Stmt::Block(stmts) => {
                self.table.enter_scope();
                let mut found_terminator = false;
                for s in stmts.iter() {
                    if found_terminator {
                        // Warn about unreachable code after return/break/continue
                        self.warnings
                            .push(Warning::UnreachableCode { span: s.span });
                        // Track for dead code elimination in codegen
                        self.unreachable_stmts.insert(s.span);
                        // Continue analyzing (don't skip) to find other errors/warnings
                    }

                    self.analyze_stmt(s)?;

                    // Check if this statement terminates control flow
                    if matches!(s.node, Stmt::Return(_) | Stmt::Break | Stmt::Continue) {
                        found_terminator = true;
                    }
                }
                self.table.exit_scope();
            }
            Stmt::VarDecl {
                name,
                ty,
                init,
                mutable,
            } => {
                self.analyze_var_decl(name, ty, init, *mutable)?;
            }
            Stmt::Assign { target, value } => {
                self.analyze_assign(target, value)?;
            }
            Stmt::Return(expr) => {
                let expr_ty = if let Some(e) = expr {
                    self.check_expr(e)?
                } else {
                    Type::Void
                };

                if let Some(ret_ty) = &self.current_return_type {
                    // Check if return expression type can be implicitly converted to return type
                    if !expr_ty.is_implicitly_convertible_to(ret_ty) {
                        return Err(SemaError::ReturnTypeMismatch {
                            expected: ret_ty.display_name(),
                            found: expr_ty.display_name(),
                            span: expr.as_ref().map(|e| e.span).unwrap_or(stmt.span),
                        });
                    }
                }
            }
            Stmt::If {
                condition,
                then_branch,
                else_branch,
            } => {
                let cond_ty = self.check_expr(condition)?;
                if cond_ty != Type::Primitive(PrimitiveType::Bool) {
                    return Err(SemaError::TypeMismatch {
                        expected: "bool".to_string(),
                        found: cond_ty.display_name(),
                        span: condition.span,
                    });
                }
                self.analyze_stmt(then_branch)?;
                if let Some(else_b) = else_branch {
                    self.analyze_stmt(else_b)?;
                }
            }
            Stmt::While { condition, body } => {
                let cond_ty = self.check_expr(condition)?;
                if cond_ty != Type::Primitive(PrimitiveType::Bool) {
                    return Err(SemaError::TypeMismatch {
                        expected: "bool".to_string(),
                        found: cond_ty.display_name(),
                        span: condition.span,
                    });
                }
                self.loop_depth += 1;
                self.analyze_stmt(body)?;
                self.loop_depth -= 1;
            }
            Stmt::For {
                var_name,
                var_type,
                range,
                body,
            } => {
                self.analyze_for_loop(var_name, var_type, range, body)?;
            }
            Stmt::ForEach {
                var_name,
                var_type,
                iterable,
                body,
                index_var,
            } => {
                self.analyze_foreach_loop(var_name, var_type, iterable, body, index_var.as_ref())?;
            }
            Stmt::Loop { body } => {
                self.loop_depth += 1;
                self.analyze_stmt(body)?;
                self.loop_depth -= 1;
            }
            Stmt::Expr(expr) => {
                self.check_expr(expr)?;
            }
            Stmt::Break => {
                if self.loop_depth == 0 {
                    return Err(SemaError::BreakOutsideLoop { span: stmt.span });
                }
            }
            Stmt::Continue => {
                if self.loop_depth == 0 {
                    return Err(SemaError::BreakOutsideLoop { span: stmt.span });
                }
            }
            Stmt::Match { expr, arms } => {
                // Check the matched expression type
                let match_ty = self.check_expr(expr)?;

                // Check exhaustiveness for enum types
                if let Type::Named(enum_name) = &match_ty {
                    self.check_match_exhaustiveness(enum_name, arms, stmt.span)?;
                }

                // Analyze each arm
                for arm in arms {
                    // Enter new scope for pattern bindings
                    self.table.enter_scope();

                    // Add pattern bindings to scope
                    self.add_pattern_bindings(&arm.pattern.node, &match_ty)?;

                    // Analyze arm body
                    self.analyze_stmt(&arm.body)?;

                    self.table.exit_scope();
                }
            }
            Stmt::Asm { lines } => {
                // Parse inline assembly to extract variable references
                // Variables are referenced as {var_name} or {struct.field}
                for line in lines {
                    self.extract_asm_variables(&line.instruction);
                }
            }
        }
        Ok(())
    }

    fn analyze_var_decl(
        &mut self,
        name: &Spanned<String>,
        ty: &Spanned<crate::ast::TypeExpr>,
        init: &Spanned<Expr>,
        mutable: bool,
    ) -> Result<(), SemaError> {
        let declared_ty = self.resolve_type(&ty.node)?;

        // Check for invalid addr usage in variable declarations
        if matches!(declared_ty, Type::Primitive(PrimitiveType::Addr)) {
            return Err(SemaError::InvalidAddrUsage {
                context: "variable declarations".to_string(),
                span: ty.span,
            });
        }

        // Set expected type context for anonymous struct literals
        self.expected_type = Some(declared_ty.clone());
        let init_ty = self.check_expr(init)?;
        self.expected_type = None;

        // Allow Bool to U8 assignment (booleans are 0/1 bytes in 6502)
        // Check if initializer type can be implicitly converted to declared type
        if !init_ty.is_implicitly_convertible_to(&declared_ty) {
            return Err(SemaError::TypeMismatch {
                expected: declared_ty.display_name(),
                found: init_ty.display_name(),
                span: init.span,
            });
        }

        // Check for duplicate variable in current scope
        if self.table.defined_in_current_scope(&name.node) {
            return Err(SemaError::DuplicateSymbol {
                name: name.node.clone(),
                span: name.span,
                previous_span: None,
            });
        }

        // Allocate in zero page using allocator
        // Arrays (pointers) and u16/i16/b16 types need 2 bytes
        // Named types: structs need their full size, enums need 2 bytes (pointer)
        let alloc_size = match &declared_ty {
            Type::Array(_, _) => 2, // Array pointer
            Type::Primitive(PrimitiveType::U16)
            | Type::Primitive(PrimitiveType::I16)
            | Type::Primitive(PrimitiveType::B16) => 2,
            Type::Named(type_name) => {
                // Check if it's a struct (allocate full size) or enum (allocate pointer)
                if let Some(struct_def) = self.type_registry.get_struct(type_name) {
                    struct_def.total_size
                } else if self.type_registry.get_enum(type_name).is_some() {
                    2 // Enums are stored as pointers to their data
                } else {
                    1 // Unknown type, default to 1
                }
            }
            _ => 1,
        };
        let addr = if alloc_size > 1 {
            self.zp_allocator.allocate_range(alloc_size as u8)?
        } else {
            self.zp_allocator.allocate()?
        };
        let location = SymbolLocation::ZeroPage(addr);

        let info = SymbolInfo {
            name: name.node.clone(),
            kind: SymbolKind::Variable,
            ty: declared_ty,
            location,
            mutable,
            access_mode: None,
            is_pub: false, // Local variables are never public
            containing_function: self.current_function.clone(),
        };
        self.table.insert(name.node.clone(), info.clone());
        // Also add to resolved_symbols so codegen can find it
        self.resolved_symbols.insert(name.span, info);

        // Track declared variable for unused variable warnings
        self.declared_variables.push((name.node.clone(), name.span));

        Ok(())
    }

    fn analyze_assign(
        &mut self,
        target: &Spanned<Expr>,
        value: &Spanned<Expr>,
    ) -> Result<(), SemaError> {
        // Set flag to indicate we're checking assignment target (not reading value)
        self.checking_assignment_target = true;
        let target_ty = self.check_expr(target)?;
        self.checking_assignment_target = false;
        let value_ty = self.check_expr(value)?;

        // Special handling for slice assignment: arr[start..end] = [values]
        if let Expr::Slice {
            object: _,
            start,
            end,
            inclusive,
        } = &target.node
        {
            // For slice assignment, check that RHS is an array with matching length
            if let Type::Array(_, rhs_size) = &value_ty {
                // Try to evaluate slice bounds as constants
                if let (Ok(start_val), Ok(end_val)) = (
                    eval_const_expr_with_env(start, &self.const_env),
                    eval_const_expr_with_env(end, &self.const_env),
                ) && let (Some(s), Some(e)) = (start_val.as_integer(), end_val.as_integer())
                {
                    let actual_end = if *inclusive { e + 1 } else { e };
                    let slice_len = (actual_end - s) as usize;

                    if *rhs_size != slice_len {
                        return Err(SemaError::Custom {
                            message: format!(
                                "slice length ({}) does not match array length ({})",
                                slice_len, rhs_size
                            ),
                            span: value.span,
                        });
                    }
                }
                // If bounds aren't constant, we'll check at runtime (or in codegen)
            } else {
                return Err(SemaError::TypeMismatch {
                    expected: "array".to_string(),
                    found: value_ty.display_name(),
                    span: value.span,
                });
            }
        } else {
            // Allow Bool to U8 assignment (booleans are 0/1 bytes in 6502)
            // Check if value type can be implicitly converted to target type
            if !value_ty.is_implicitly_convertible_to(&target_ty) {
                return Err(SemaError::TypeMismatch {
                    expected: target_ty.display_name(),
                    found: value_ty.display_name(),
                    span: value.span,
                });
            }
        }

        // Check mutability and access mode
        if let Expr::Variable(name) = &target.node
            && let Some(info) = self.table.lookup(name)
        {
            // Check for writing to read-only address
            if info.kind == SymbolKind::Address
                && let Some(crate::ast::AccessMode::Read) = info.access_mode
            {
                return Err(SemaError::ReadOnlyWrite {
                    name: name.clone(),
                    span: target.span,
                });
            }
            // Check general mutability
            if !info.mutable {
                return Err(SemaError::ImmutableAssignment {
                    symbol: name.clone(),
                    span: target.span,
                });
            }
        }

        Ok(())
    }

    fn analyze_for_loop(
        &mut self,
        var_name: &Spanned<String>,
        var_type: &Option<Spanned<crate::ast::TypeExpr>>,
        range: &crate::ast::Range,
        body: &Spanned<Stmt>,
    ) -> Result<(), SemaError> {
        // Create a new scope for the loop variable
        self.table.enter_scope();

        // Determine loop variable type (explicit or inferred)
        let var_ty = if let Some(ty) = var_type {
            self.resolve_type(&ty.node)?
        } else {
            // Infer type from range bounds
            let start_ty = self.check_expr(&range.start)?;
            let end_ty = self.check_expr(&range.end)?;

            // Use the larger of the two types
            match (start_ty, end_ty) {
                (Type::Primitive(PrimitiveType::U16), _)
                | (_, Type::Primitive(PrimitiveType::U16)) => Type::Primitive(PrimitiveType::U16),
                (Type::Primitive(PrimitiveType::I16), _)
                | (_, Type::Primitive(PrimitiveType::I16)) => Type::Primitive(PrimitiveType::I16),
                _ => Type::Primitive(PrimitiveType::U8), // Default to u8
            }
        };

        // Check for duplicate loop variable (shouldn't happen in new scope, but check anyway)
        if self.table.defined_in_current_scope(&var_name.node) {
            return Err(SemaError::DuplicateSymbol {
                name: var_name.node.clone(),
                span: var_name.span,
                previous_span: None,
            });
        }

        let addr = self.zp_allocator.allocate()?;
        let info = SymbolInfo {
            name: var_name.node.clone(),
            kind: SymbolKind::Variable,
            ty: var_ty,
            location: SymbolLocation::ZeroPage(addr),
            mutable: true,
            access_mode: None,
            is_pub: false, // Local variables are never public
            containing_function: self.current_function.clone(),
        };
        self.table.insert(var_name.node.clone(), info);

        // Check range bounds if not already checked
        if var_type.is_some() {
            self.check_expr(&range.start)?;
            self.check_expr(&range.end)?;
        }

        // Analyze body
        self.loop_depth += 1;
        self.analyze_stmt(body)?;
        self.loop_depth -= 1;

        self.table.exit_scope();

        Ok(())
    }

    fn analyze_foreach_loop(
        &mut self,
        var_name: &Spanned<String>,
        var_type: &Option<Spanned<crate::ast::TypeExpr>>,
        iterable: &Spanned<Expr>,
        body: &Spanned<Stmt>,
        index_var: Option<&Spanned<String>>,
    ) -> Result<(), SemaError> {
        // Create a new scope for the loop variables
        self.table.enter_scope();

        // Check the iterable expression (should be an array or string)
        let iterable_ty = self.check_expr(iterable)?;

        // Extract element type from array type or string
        let element_ty = match &iterable_ty {
            Type::Array(elem_ty, _size) => (**elem_ty).clone(),
            Type::String => Type::Primitive(PrimitiveType::U8), // String elements are u8
            _ => {
                return Err(SemaError::TypeMismatch {
                    expected: "array or string".to_string(),
                    found: iterable_ty.display_name(),
                    span: iterable.span,
                });
            }
        };

        // Determine loop variable type (explicit or inferred from array element type)
        let var_ty = if let Some(ty) = var_type {
            let declared_ty = self.resolve_type(&ty.node)?;
            // Check that declared type matches element type
            if declared_ty != element_ty {
                return Err(SemaError::TypeMismatch {
                    expected: element_ty.display_name(),
                    found: declared_ty.display_name(),
                    span: ty.span,
                });
            }
            declared_ty
        } else {
            element_ty
        };

        // Allocate storage for index variable if present
        if let Some(idx_var) = index_var {
            let idx_addr = self.zp_allocator.allocate()?;
            let idx_info = SymbolInfo {
                name: idx_var.node.clone(),
                kind: SymbolKind::Variable,
                ty: Type::Primitive(PrimitiveType::U8),
                location: SymbolLocation::ZeroPage(idx_addr),
                mutable: true,
                access_mode: None,
                is_pub: false,
                containing_function: self.current_function.clone(),
            };
            self.table.insert(idx_var.node.clone(), idx_info.clone());
            self.resolved_symbols.insert(idx_var.span, idx_info);
        }

        // Allocate storage for loop variable
        // u16/i16 types need 2 bytes
        let addr = if matches!(
            var_ty,
            Type::Primitive(PrimitiveType::U16) | Type::Primitive(PrimitiveType::I16)
        ) {
            self.zp_allocator.allocate_range(2)?
        } else {
            self.zp_allocator.allocate()?
        };
        let info = SymbolInfo {
            name: var_name.node.clone(),
            kind: SymbolKind::Variable,
            ty: var_ty,
            location: SymbolLocation::ZeroPage(addr),
            mutable: true,
            access_mode: None,
            is_pub: false, // Local variables are never public
            containing_function: self.current_function.clone(),
        };
        self.table.insert(var_name.node.clone(), info.clone());
        // Add to resolved_symbols so codegen can find it
        self.resolved_symbols.insert(var_name.span, info);

        // Analyze body
        self.loop_depth += 1;
        self.analyze_stmt(body)?;
        self.loop_depth -= 1;

        self.table.exit_scope();

        Ok(())
    }

    /// Check if a match statement exhaustively covers all enum variants
    pub(super) fn check_match_exhaustiveness(
        &mut self,
        enum_name: &str,
        arms: &[crate::ast::MatchArm],
        match_span: Span,
    ) -> Result<(), SemaError> {
        // Get the enum definition
        let enum_def = if let Some(def) = self.type_registry.get_enum(enum_name) {
            def.clone()
        } else {
            // Not an enum or not found - skip exhaustiveness check
            return Ok(());
        };

        // Check if there's a wildcard pattern
        let has_wildcard = arms
            .iter()
            .any(|arm| matches!(arm.pattern.node, Pattern::Wildcard));

        if has_wildcard {
            // Wildcard covers everything - match is exhaustive
            return Ok(());
        }

        // Collect covered variants
        let mut covered_variants = HashSet::default();
        for arm in arms {
            if let Pattern::EnumVariant { variant, .. } = &arm.pattern.node {
                covered_variants.insert(variant.node.clone());
            }
        }

        // Find missing variants
        let all_variants: Vec<String> = enum_def.variants.iter().map(|v| v.name.clone()).collect();
        let missing_variants: Vec<String> = all_variants
            .iter()
            .filter(|v| !covered_variants.contains(*v))
            .cloned()
            .collect();

        if !missing_variants.is_empty() {
            // Generate warning for non-exhaustive match
            self.warnings.push(Warning::NonExhaustiveMatch {
                missing_patterns: missing_variants,
                span: match_span,
            });
        }

        Ok(())
    }

    /// Add pattern bindings to the current scope
    pub(super) fn add_pattern_bindings(
        &mut self,
        pattern: &Pattern,
        match_ty: &Type,
    ) -> Result<(), SemaError> {
        match pattern {
            Pattern::EnumVariant {
                enum_name,
                variant,
                bindings,
            } => {
                // Get enum definition to find variant field types
                if let Some(enum_def) = self.type_registry.get_enum(&enum_name.node)
                    && let Some(variant_def) =
                        enum_def.variants.iter().find(|v| v.name == variant.node)
                {
                    // Add bindings for tuple variant fields
                    match &variant_def.data {
                        VariantData::Tuple(field_types) => {
                            for (i, binding) in bindings.iter().enumerate() {
                                if let Some(field_ty) = field_types.get(i) {
                                    let addr = self.zp_allocator.allocate()?;
                                    let info = SymbolInfo {
                                        name: binding.name.node.clone(),
                                        kind: SymbolKind::Variable,
                                        ty: field_ty.clone(),
                                        location: SymbolLocation::ZeroPage(addr),
                                        mutable: false,
                                        access_mode: None,
                                        is_pub: false, // Pattern bindings are never public
                                        containing_function: self.current_function.clone(),
                                    };
                                    self.table.insert(binding.name.node.clone(), info.clone());
                                    // Also add to resolved_symbols so codegen can find it
                                    self.resolved_symbols.insert(binding.name.span, info);
                                }
                            }
                        }
                        _ => {
                            // Unit and Struct variants don't have tuple-style bindings
                        }
                    }
                }
            }
            Pattern::Variable(name) => {
                // Bind the entire matched value
                let addr = self.zp_allocator.allocate()?;
                let info = SymbolInfo {
                    name: name.clone(),
                    kind: SymbolKind::Variable,
                    ty: match_ty.clone(),
                    location: SymbolLocation::ZeroPage(addr),
                    mutable: false,
                    access_mode: None,
                    is_pub: false, // Pattern bindings are never public
                    containing_function: self.current_function.clone(),
                };
                self.table.insert(name.clone(), info.clone());
                // Also add to resolved_symbols so codegen can find it
                // Note: Pattern::Variable doesn't have a span, so we can't add to resolved_symbols here
                // This is a limitation of the current AST structure
            }
            Pattern::Wildcard => {
                // No bindings for wildcard
            }
            _ => {
                // Literal and Range patterns don't create bindings
            }
        }

        Ok(())
    }
}
