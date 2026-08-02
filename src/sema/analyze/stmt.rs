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
                    // Propagate the function's return type as the expected type so
                    // literals and anonymous struct literals in return position are
                    // typed by it, exactly as `let x: T = ...` does for its init.
                    let saved_expected = self.expected_type.take();
                    self.expected_type = self.current_return_type.clone();
                    let ty = self.check_expr(e);
                    self.expected_type = saved_expected;
                    ty?
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
                    // The pattern must make sense for the scrutinee before it
                    // is trusted for bindings: a pattern naming a different
                    // enum compares tags against the wrong discriminants, and
                    // an out-of-range literal silently truncates in the
                    // emitted CMP (`match x { 300 => ... }` with `x: u8`
                    // matched x == 44).
                    self.check_pattern_type(&arm.pattern, &match_ty)?;

                    // Enter new scope for pattern bindings
                    self.table.enter_scope();

                    // Add pattern bindings to scope
                    self.add_pattern_bindings(&arm.pattern.node, arm.pattern.span, &match_ty)?;

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
                    // Also record hand-written JSR/JMP calls to wraith functions as
                    // call-graph edges, so finalize_frames colors the target's frame
                    // above this function's frame (a function reachable only via
                    // inline asm would otherwise overlap its caller).
                    self.record_asm_call_edges(&line.instruction);
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
            // Arrays of structs are stored inline (element data lives directly at
            // the variable's address so arr[i].field addresses statically); other
            // arrays are a 2-byte pointer to (ROM) element data.
            Type::Array(elem, n) if matches!(&**elem, Type::Named(t) if self.type_registry.get_struct(t).is_some()) => {
                self.type_size(elem) * n
            }
            Type::Array(_, _) => 2, // Array pointer (data lives in RAM, below)
            Type::String => 2,      // String pointer (16-bit address)
            Type::Function(_, _) => 2, // Function pointer (16-bit code address)
            Type::Slice(_) => 4,    // Fat pointer: 2-byte base + 2-byte length
            Type::Pointer(_) => 2,  // 16-bit address
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
        // A frame offset is one byte; a larger inline value used to be
        // clamped or truncated (`as u8`), so the variable's tail aliased
        // whatever the frame handed out next.
        if alloc_size > 255 {
            return Err(SemaError::Custom {
                message: format!(
                    "local '{}' needs {} bytes; a variable stored inline in the frame holds at most 255",
                    name.node, alloc_size
                ),
                span: name.span,
            });
        }
        let offset = self.frame_alloc(alloc_size as u8);
        let location = SymbolLocation::FrameOffset(offset);

        // A non-struct-element array's frame slot holds a *pointer*; the data
        // itself needs storage. It used to be emitted inline in the CODE
        // section, which is ROM on a real board, so writing to a local array
        // did nothing. Reserve a block in RAM instead, as an offset within this
        // function's block — `finalize_frames` lays the blocks out and rebases
        // these to absolute addresses once the call graph is known.
        if let Type::Array(elem, n) = &declared_ty
            && !matches!(&**elem, Type::Named(t) if self.type_registry.get_struct(t).is_some())
        {
            let bytes = (self.type_size(elem) * n) as u16;
            if let Some(f) = self.current_function.clone() {
                let at = self.array_cursor;
                self.array_cursor += bytes;
                self.local_arrays.insert(
                    name.span,
                    crate::sema::LocalArray {
                        addr: at,
                        size: bytes,
                        function: f.clone(),
                    },
                );
                let entry = self.array_block_sizes.entry(f).or_insert(0);
                *entry = (*entry).max(self.array_cursor);
            }
        }

        // An enum local's frame slot likewise holds a pointer, and what it
        // points at must be the variable's *own* storage: runtime-constructed
        // enums are otherwise built in shared codegen scratch, so two live
        // enums (or any later temp use) would destroy an earlier one's bytes.
        // Reserve a per-declaration block in the same colored region arrays
        // use; codegen copies the constructed bytes into it on binding.
        if let Type::Named(n) = &declared_ty
            && let Some(enum_def) = self.type_registry.get_enum(n)
        {
            let bytes = (enum_def.total_size as u16).max(1);
            if let Some(f) = self.current_function.clone() {
                let at = self.array_cursor;
                self.array_cursor += bytes;
                self.enum_blocks.insert(
                    name.span,
                    crate::sema::LocalArray {
                        addr: at,
                        size: bytes,
                        function: f.clone(),
                    },
                );
                let entry = self.array_block_sizes.entry(f).or_insert(0);
                *entry = (*entry).max(self.array_cursor);
            }
        }

        // A `str<N>` buffer owns a `[u8 len][N bytes]` block in RAM so its
        // contents can be edited in place. Reserve it in the same call-graph-
        // colored region arrays and enums use; recording it here also marks the
        // declaration as a writable string (see `analyze_assign`).
        if let crate::ast::TypeExpr::StringBuf { capacity } = &ty.node {
            let cap = *capacity;
            // The buffer must start from a string literal that fits. Runtime
            // initialization (from another `str`) is a later addition; for now
            // the source is a literal so the fit is checked at compile time.
            if let Expr::Literal(crate::ast::Literal::String(s)) = &init.node {
                if s.len() > cap {
                    return Err(SemaError::Custom {
                        message: format!(
                            "string literal is {} bytes but the buffer capacity is {}",
                            s.len(),
                            cap
                        ),
                        span: init.span,
                    });
                }
            } else {
                return Err(SemaError::Custom {
                    message: "a `str<N>` buffer must be initialized with a string literal"
                        .to_string(),
                    span: init.span,
                });
            }
            let bytes = cap as u16 + 1; // length prefix + content
            if let Some(f) = self.current_function.clone() {
                let at = self.array_cursor;
                self.array_cursor += bytes;
                self.string_buffers.insert(
                    name.span,
                    crate::sema::LocalArray {
                        addr: at,
                        size: bytes,
                        function: f.clone(),
                    },
                );
                let entry = self.array_block_sizes.entry(f).or_insert(0);
                *entry = (*entry).max(self.array_cursor);
            }
        }

        let info = SymbolInfo {
            name: name.node.clone(),
            kind: SymbolKind::Variable,
            ty: declared_ty,
            location,
            mutable,
            access_mode: None,
            is_pub: false, // Local variables are never public
            containing_function: self.current_function.clone(),
            is_param: false,
            decl_span: Some(name.span),
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

        // Writing an element into a `str` is only meaningful for a `str<N>`
        // buffer, which owns editable RAM. A plain `str` may point at a ROM
        // literal, so the store would silently do nothing on real hardware —
        // reject it here rather than emit a dead write.
        if let Expr::Index { object, .. } = &target.node
            && matches!(self.resolved_types.get(&object.span), Some(Type::String))
        {
            let is_buffer = self
                .lvalue_root(target)
                .and_then(|root| self.table.lookup(root))
                .and_then(|sym| sym.decl_span)
                .is_some_and(|ds| self.string_buffers.contains_key(&ds));
            if !is_buffer {
                return Err(SemaError::Custom {
                    message: "cannot write through a `str`; declare it as a `str<N>` \
                              buffer to edit its bytes in place"
                        .to_string(),
                    span: target.span,
                });
            }
        }
        // Give the value the target type as its expected type so a literal (or
        // negative literal) infers against the destination, e.g. `RESULT = -10`
        // storing 246 into a u8, or `x = 127` into an i8.
        let saved_expected = self.expected_type.take();
        self.expected_type = Some(target_ty.clone());
        let value_ty = self.check_expr(value)?;
        self.expected_type = saved_expected;

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

        // An element or field assignment writes to whatever the chain is rooted
        // at, so the root is what has to be mutable. Without this, `LUT[i] = v`
        // on a `const` compiled and emitted a store into ROM — a no-op on real
        // hardware, and invisible under the emulator's flat memory. Deref is
        // deliberately *not* followed: `*p = v` writes to the pointee, not to
        // `p`, exactly as `const char *p` does in C.
        if !matches!(target.node, Expr::Variable(_))
            && let Some(root) = self.lvalue_root(target)
            && let Some(info) = self.table.lookup(root)
            && !info.mutable
        {
            let what = if info.kind == SymbolKind::Constant {
                format!(
                    "cannot assign to '{}': a const lives in ROM, so the store would \
                     silently do nothing on real hardware",
                    root
                )
            } else {
                format!("cannot assign through '{}': it is immutable", root)
            };
            return Err(SemaError::Custom {
                message: what,
                span: target.span,
            });
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

    /// The variable an lvalue chain is rooted at, if the write lands in that
    /// variable's own storage.
    ///
    /// Peels indexing and field access, but stops the moment the chain passes
    /// through a *reference*, because the write then lands somewhere else
    /// entirely and the name in hand says nothing about it. Two things count as
    /// a reference: a pointer, however spelled (`*p`, `p[i]`, `p.field`), and a
    /// parameter, since this language has always passed aggregates by address.
    /// `const char *p` does not make `*p` const in C either.
    ///
    /// Returns None in those cases, leaving the write unconstrained. Nothing is
    /// lost by that: the check exists to catch a `const` root, and a `const` is
    /// never a parameter.
    pub(super) fn lvalue_root<'a>(&self, expr: &'a Spanned<Expr>) -> Option<&'a String> {
        let mut cur = expr;
        loop {
            match &cur.node {
                Expr::Variable(name) => return Some(name),
                Expr::Index { object, .. }
                | Expr::Field { object, .. }
                | Expr::Slice { object, .. } => {
                    if self.denotes_a_reference(object) {
                        return None;
                    }
                    cur = object;
                }
                // A `.len`/`.low`/`.high` that sema re-resolved as a struct
                // field access peels exactly like a plain `Expr::Field`.
                Expr::SliceLen(inner) | Expr::U16Low(inner) | Expr::U16High(inner)
                    if self.accessor_fields.contains(&cur.span) =>
                {
                    if self.denotes_a_reference(inner) {
                        return None;
                    }
                    cur = inner;
                }
                Expr::Paren(inner) => cur = inner,
                _ => return None,
            }
        }
    }

    /// Whether this expression holds an address rather than the bytes themselves.
    fn denotes_a_reference(&self, expr: &Spanned<Expr>) -> bool {
        if matches!(self.resolved_types.get(&expr.span), Some(Type::Pointer(_))) {
            return true;
        }
        matches!(&expr.node, Expr::Variable(n)
            if self.table.lookup(n).is_some_and(|s| s.is_param))
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

        // 16-bit counters are u16-only for now: the loop machinery's compare
        // is unsigned (wrong for i16) and its increment is binary (wrong for
        // BCD b16). Fail loudly rather than miscompile.
        if matches!(
            var_ty,
            Type::Primitive(PrimitiveType::I16) | Type::Primitive(PrimitiveType::B16)
        ) {
            return Err(SemaError::Custom {
                message: format!(
                    "for loop counter '{}' has type {}; 16-bit loop counters are currently u16-only",
                    var_name.node,
                    var_ty.display_name()
                ),
                span: var_name.span,
            });
        }

        // Check for duplicate loop variable (shouldn't happen in new scope, but check anyway)
        if self.table.defined_in_current_scope(&var_name.node) {
            return Err(SemaError::DuplicateSymbol {
                name: var_name.node.clone(),
                span: var_name.span,
                previous_span: None,
            });
        }

        // Allocate frame space matching the counter's size: a 16-bit loop
        // variable needs a zero-page pair (low/high), not a single byte.
        let counter_size = var_ty.size().max(1) as u8;
        let offset = self.frame_alloc(counter_size);
        let info = SymbolInfo {
            name: var_name.node.clone(),
            kind: SymbolKind::Variable,
            ty: var_ty.clone(),
            location: SymbolLocation::FrameOffset(offset),
            mutable: true,
            access_mode: None,
            is_pub: false, // Local variables are never public
            containing_function: self.current_function.clone(),
            is_param: false,
            decl_span: Some(var_name.span),
        };
        self.table.insert(var_name.node.clone(), info.clone());
        // Register the loop variable at its own declaration span so codegen can
        // resolve its address directly (instead of the old body-span hack).
        self.resolved_symbols.insert(var_name.span, info);

        // Check range bounds if not already checked
        if var_type.is_some() {
            self.check_expr(&range.start)?;
            self.check_expr(&range.end)?;
        }

        // The bounds must be representable in the counter type: a wider
        // constant truncates at the compare (`for i: u8 in 0..300` ran 44
        // iterations), and a wider runtime bound wraps the counter.
        self.check_for_bound(&range.start, &var_ty)?;
        self.check_for_bound(&range.end, &var_ty)?;

        // A non-constant range end must survive the whole loop body, which may
        // itself run nested loops, scratch-clobbering expressions, or calls.
        // Give it a hidden frame slot (colored with the call graph like any
        // local) rather than a shared zero-page scratch byte. Sibling loops
        // reuse each other's released slots (their lifetimes never overlap);
        // a nested loop allocates fresh because the outer slot is still held.
        let bound_slot = if !self.folded_constants.contains_key(&range.end.span) {
            let bound_offset = self.loop_bound_alloc(counter_size);
            self.loop_bound_slots.insert(
                range.end.span,
                SymbolInfo {
                    name: format!("<loop bound for {}>", var_name.node),
                    kind: SymbolKind::Variable,
                    ty: var_ty,
                    location: SymbolLocation::FrameOffset(bound_offset),
                    mutable: true,
                    access_mode: None,
                    is_pub: false,
                    containing_function: self.current_function.clone(),
                    is_param: false,
                    decl_span: None,
                },
            );
            Some((bound_offset, counter_size))
        } else {
            None
        };

        // Analyze body
        self.loop_depth += 1;
        self.analyze_stmt(body)?;
        self.loop_depth -= 1;

        // The bound dies with the loop; let sibling loops reuse its bytes.
        if let Some((offset, size)) = bound_slot {
            self.loop_bound_release(offset, size);
        }

        self.table.exit_scope();

        Ok(())
    }

    /// A for-loop bound must be representable in the counter's type. Constant
    /// bounds are checked by value; a runtime bound is checked by type, so a
    /// u16 expression driving a u8 counter is rejected rather than compared
    /// against its own low byte.
    fn check_for_bound(&self, bound: &Spanned<Expr>, counter_ty: &Type) -> Result<(), SemaError> {
        let fits_by_value = |v: i64| match counter_ty {
            Type::Primitive(PrimitiveType::U8) => (0..=255).contains(&v),
            Type::Primitive(PrimitiveType::I8) => (-128..=127).contains(&v),
            Type::Primitive(PrimitiveType::U16) => (0..=65535).contains(&v),
            _ => true,
        };

        if let Ok(val) = eval_const_expr_with_env(bound, &self.const_env)
            && let Some(v) = val.as_integer()
        {
            if fits_by_value(v) {
                return Ok(());
            }
            return Err(SemaError::Custom {
                message: format!(
                    "for loop bound {} does not fit in counter type {}",
                    v,
                    counter_ty.display_name()
                ),
                span: bound.span,
            });
        }

        if let Some(bound_ty) = self.resolved_types.get(&bound.span) {
            let too_wide = matches!(
                (counter_ty, bound_ty),
                (
                    Type::Primitive(PrimitiveType::U8),
                    Type::Primitive(PrimitiveType::U16)
                ) | (
                    Type::Primitive(PrimitiveType::U8),
                    Type::Primitive(PrimitiveType::I16)
                ) | (
                    Type::Primitive(PrimitiveType::I8),
                    Type::Primitive(PrimitiveType::U16)
                ) | (
                    Type::Primitive(PrimitiveType::I8),
                    Type::Primitive(PrimitiveType::I16)
                )
            );
            if too_wide {
                return Err(SemaError::Custom {
                    message: format!(
                        "for loop bound of type {} does not fit in counter type {}; \
                         widen the counter or narrow the bound with a cast",
                        bound_ty.display_name(),
                        counter_ty.display_name()
                    ),
                    span: bound.span,
                });
            }
        }
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

        // Extract element type from array, slice, or string
        let element_ty = match &iterable_ty {
            Type::Array(elem_ty, _size) => (**elem_ty).clone(),
            Type::Slice(elem_ty) => (**elem_ty).clone(),
            Type::String => Type::Primitive(PrimitiveType::Char), // a string is an array of chars
            _ => {
                return Err(SemaError::TypeMismatch {
                    expected: "array, slice, or string".to_string(),
                    found: iterable_ty.display_name(),
                    span: iterable.span,
                });
            }
        };

        // ForEach lowers to an 8-bit counter in X with an 8-bit compare, and
        // u16 elements scale the offset in a byte: beyond 255 elements (or 127
        // for u16) the bound or the offset silently wraps — a 300-element
        // array iterated 44 times, a 200-element u16 array re-reading elements
        // 0..71. Reject rather than miscompile.
        if let Type::Array(_, size) = &iterable_ty {
            let elem_wide = matches!(
                element_ty,
                Type::Primitive(PrimitiveType::U16 | PrimitiveType::I16 | PrimitiveType::B16)
            );
            let limit = if elem_wide { 127 } else { 255 };
            if *size > limit {
                return Err(SemaError::Custom {
                    message: format!(
                        "cannot iterate a {}-element {} array with `for .. in`: the loop \
                         counter and offset are 8-bit, so at most {} elements work. Iterate \
                         a slice of it in two halves, or index it with a `for i in 0..N` \
                         loop instead",
                        size,
                        if elem_wide { "u16-element" } else { "" },
                        limit
                    ),
                    span: iterable.span,
                });
            }
        }

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
            let idx_off = self.frame_alloc(1);
            let idx_info = SymbolInfo {
                name: idx_var.node.clone(),
                kind: SymbolKind::Variable,
                ty: Type::Primitive(PrimitiveType::U8),
                location: SymbolLocation::FrameOffset(idx_off),
                mutable: true,
                access_mode: None,
                is_pub: false,
                containing_function: self.current_function.clone(),
                is_param: false,
                decl_span: Some(idx_var.span),
            };
            self.table.insert(idx_var.node.clone(), idx_info.clone());
            self.resolved_symbols.insert(idx_var.span, idx_info);
        }

        // Allocate storage for loop variable
        // u16/i16/b16 types need 2 bytes
        let elem_size = if matches!(
            var_ty,
            Type::Primitive(PrimitiveType::U16)
                | Type::Primitive(PrimitiveType::I16)
                | Type::Primitive(PrimitiveType::B16)
        ) {
            2
        } else {
            1
        };
        let offset = self.frame_alloc(elem_size);
        let info = SymbolInfo {
            name: var_name.node.clone(),
            kind: SymbolKind::Variable,
            ty: var_ty,
            location: SymbolLocation::FrameOffset(offset),
            mutable: true,
            access_mode: None,
            is_pub: false, // Local variables are never public
            containing_function: self.current_function.clone(),
            is_param: false,
            decl_span: Some(var_name.span),
        };
        self.table.insert(var_name.node.clone(), info.clone());
        // Add to resolved_symbols so codegen can find it
        self.resolved_symbols.insert(var_name.span, info);

        // The loop counter must survive the whole body: the body is arbitrary
        // code that may clobber X (a u8 multiply uses it as a bit counter, a
        // nested loop as its own counter), so a counter kept only in X gets
        // corrupted. Give every foreach a hidden counter slot (colored with the
        // call graph) keyed by the iterable's span, in the same map the
        // for-range loop uses for non-constant bounds. A slice needs 16 bits
        // (it can exceed 255 elements); a string or array is bounded to 255, so
        // one byte is enough.
        let counter_size: u8 = if matches!(iterable_ty, Type::Slice(_)) {
            2
        } else {
            1
        };
        let counter_off = self.frame_alloc(counter_size);
        self.loop_bound_slots.insert(
            iterable.span,
            SymbolInfo {
                name: format!("<iter counter for {}>", var_name.node),
                kind: SymbolKind::Variable,
                ty: if counter_size == 2 {
                    Type::Primitive(PrimitiveType::U16)
                } else {
                    Type::Primitive(PrimitiveType::U8)
                },
                location: SymbolLocation::FrameOffset(counter_off),
                mutable: true,
                access_mode: None,
                is_pub: false,
                containing_function: self.current_function.clone(),
                is_param: false,
                decl_span: None,
            },
        );

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
    /// Check a match pattern against the scrutinee's type.
    ///
    /// Nothing used to validate this: a pattern naming a different enum
    /// compiled and compared tags against the wrong discriminants, and a
    /// literal that doesn't fit the scrutinee silently truncated in the
    /// emitted CMP. Struct/enum variants also get their shape checked here
    /// (unknown variant, wrong binding count), which `add_pattern_bindings`
    /// otherwise ignores by producing no bindings.
    pub(super) fn check_pattern_type(
        &mut self,
        pattern: &Spanned<crate::ast::Pattern>,
        match_ty: &Type,
    ) -> Result<(), SemaError> {
        use crate::ast::Pattern;
        match &pattern.node {
            Pattern::EnumVariant {
                enum_name,
                variant,
                bindings,
            } => {
                let Type::Named(scrut_name) = match_ty else {
                    return Err(SemaError::TypeMismatch {
                        expected: format!("'{}'", enum_name.node),
                        found: match_ty.display_name(),
                        span: pattern.span,
                    });
                };
                if scrut_name != &enum_name.node {
                    return Err(SemaError::Custom {
                        message: format!(
                            "pattern is for enum '{}' but the matched value is '{}'",
                            enum_name.node, scrut_name
                        ),
                        span: pattern.span,
                    });
                }
                let enum_def =
                    self.type_registry
                        .get_enum(scrut_name)
                        .ok_or_else(|| SemaError::Custom {
                            message: format!("enum '{}' not found", scrut_name),
                            span: pattern.span,
                        })?;
                let variant_info =
                    enum_def
                        .get_variant(&variant.node)
                        .ok_or_else(|| SemaError::Custom {
                            message: format!(
                                "variant '{}' not found in enum '{}'",
                                variant.node, scrut_name
                            ),
                            span: variant.span,
                        })?;
                use crate::sema::type_defs::VariantData as TypeDefVariantData;
                match (&variant_info.data, bindings.len()) {
                    (TypeDefVariantData::Unit, 0) => {}
                    (TypeDefVariantData::Unit, _) => {
                        return Err(SemaError::Custom {
                            message: format!(
                                "variant '{}' takes no data, but the pattern binds {} name(s)",
                                variant.node,
                                bindings.len()
                            ),
                            span: pattern.span,
                        });
                    }
                    (TypeDefVariantData::Tuple(fields), n) if n != fields.len() => {
                        return Err(SemaError::Custom {
                            message: format!(
                                "variant '{}' has {} field(s), but the pattern binds {}",
                                variant.node,
                                fields.len(),
                                n
                            ),
                            span: pattern.span,
                        });
                    }
                    _ => {}
                }
                Ok(())
            }
            Pattern::Literal(lit) => self.check_pattern_literal_fits(lit, match_ty),
            Pattern::Range { start, end, .. } => {
                self.check_pattern_literal_fits(start, match_ty)?;
                self.check_pattern_literal_fits(end, match_ty)
            }
            Pattern::Wildcard | Pattern::Variable(_) => Ok(()),
        }
    }

    /// A literal (or range bound) in a pattern must be representable in the
    /// scrutinee's type; the emitted CMP is a plain byte compare and would
    /// otherwise match the truncated value.
    fn check_pattern_literal_fits(
        &mut self,
        lit: &Spanned<Expr>,
        match_ty: &Type,
    ) -> Result<(), SemaError> {
        let Ok(val) = eval_const_expr_with_env(lit, &self.const_env) else {
            return Ok(()); // not constant: nothing to check
        };
        let Some(int_val) = val.as_integer() else {
            return Ok(());
        };
        let fits = match match_ty {
            Type::Primitive(PrimitiveType::U8) => (0..=255).contains(&int_val),
            Type::Primitive(PrimitiveType::I8) => (-128..=127).contains(&int_val),
            Type::Primitive(PrimitiveType::U16) | Type::Primitive(PrimitiveType::Addr) => {
                (0..=65535).contains(&int_val)
            }
            Type::Primitive(PrimitiveType::I16) => (-32768..=32767).contains(&int_val),
            _ => true,
        };
        if !fits {
            return Err(SemaError::Custom {
                message: format!(
                    "pattern literal {} cannot match a value of type {}",
                    int_val,
                    match_ty.display_name()
                ),
                span: lit.span,
            });
        }
        Ok(())
    }

    pub(super) fn add_pattern_bindings(
        &mut self,
        pattern: &Pattern,
        pattern_span: crate::Span,
        match_ty: &Type,
    ) -> Result<(), SemaError> {
        match pattern {
            Pattern::EnumVariant {
                enum_name,
                variant,
                bindings,
            } => {
                // Resolve each binding's type. Tuple variants bind positionally;
                // struct variants bind by matching the binding name to a field.
                // Clone the data out first so the type_registry borrow is released
                // before allocating frame slots (which needs &mut self).
                let variant_data = self
                    .type_registry
                    .get_enum(&enum_name.node)
                    .and_then(|enum_def| enum_def.variants.iter().find(|v| v.name == variant.node))
                    .map(|variant_def| variant_def.data.clone());

                let binding_types: Vec<Option<Type>> = match &variant_data {
                    Some(VariantData::Tuple(field_types)) => bindings
                        .iter()
                        .enumerate()
                        .map(|(i, _)| field_types.get(i).cloned())
                        .collect(),
                    Some(VariantData::Struct(fields)) => bindings
                        .iter()
                        .map(|b| {
                            fields
                                .iter()
                                .find(|f| f.name == b.name.node)
                                .map(|f| f.ty.clone())
                        })
                        .collect(),
                    _ => Vec::new(),
                };

                for (binding, field_ty) in bindings.iter().zip(binding_types.iter()) {
                    if let Some(field_ty) = field_ty {
                        let field_size = self.type_size(field_ty) as u8;
                        let off = self.frame_alloc(field_size.max(1));
                        let info = SymbolInfo {
                            name: binding.name.node.clone(),
                            kind: SymbolKind::Variable,
                            ty: field_ty.clone(),
                            location: SymbolLocation::FrameOffset(off),
                            mutable: false,
                            access_mode: None,
                            is_pub: false, // Pattern bindings are never public
                            containing_function: self.current_function.clone(),
                            is_param: false,
                            decl_span: Some(binding.name.span),
                        };
                        self.table.insert(binding.name.node.clone(), info.clone());
                        // Also add to resolved_symbols so codegen can find it
                        self.resolved_symbols.insert(binding.name.span, info);
                    }
                }
            }
            Pattern::Variable(name) => {
                // Bind the entire matched value
                let bind_size = self.type_size(match_ty) as u8;
                let off = self.frame_alloc(bind_size.max(1));
                let info = SymbolInfo {
                    name: name.clone(),
                    kind: SymbolKind::Variable,
                    ty: match_ty.clone(),
                    location: SymbolLocation::FrameOffset(off),
                    mutable: false,
                    access_mode: None,
                    is_pub: false, // Pattern bindings are never public
                    containing_function: self.current_function.clone(),
                    is_param: false,
                    decl_span: Some(pattern_span),
                };
                self.table.insert(name.clone(), info.clone());
                // Pattern::Variable carries no span of its own, but the arm's
                // pattern span is unique per arm, so key the binding on it. This
                // lets match codegen find the binding's storage to copy the
                // scrutinee into (the arm-body scope is popped after analysis,
                // so a name lookup would fail at codegen time).
                self.resolved_symbols.insert(pattern_span, info);
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
