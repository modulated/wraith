//! Expression Type Checking
//!
//! Type checking for all expression variants in the AST.

use crate::ast::{BinaryOp, Expr, PrimitiveType, Span, Spanned, Stmt};
use crate::sema::SemaError;
use crate::sema::const_eval::eval_const_expr_with_env;
use crate::sema::table::SymbolKind;
use crate::sema::types::Type;
use rustc_hash::FxHashMap as HashMap;

use super::SemanticAnalyzer;

/// The width in bits of an integer type, and whether it is signed. `None` for
/// anything whose arithmetic does not wrap at a fixed width.
fn int_width_of(ty: &Type) -> Option<(u32, bool)> {
    match ty {
        Type::Primitive(PrimitiveType::U8) => Some((8, false)),
        Type::Primitive(PrimitiveType::I8) => Some((8, true)),
        Type::Primitive(PrimitiveType::U16) => Some((16, false)),
        Type::Primitive(PrimitiveType::I16) => Some((16, true)),
        _ => None,
    }
}

impl SemanticAnalyzer {
    /// Check if an expression contains any references to addr symbols (runtime values)
    pub(super) fn contains_addr_reference(&self, expr: &Spanned<Expr>) -> bool {
        match &expr.node {
            Expr::Variable(name) => {
                // Check if this variable is an addr
                if let Some(sym) = self.table.lookup(name) {
                    sym.kind == SymbolKind::Address
                } else {
                    false
                }
            }
            Expr::Binary { left, right, .. } => {
                self.contains_addr_reference(left) || self.contains_addr_reference(right)
            }
            Expr::Unary { operand, .. } => self.contains_addr_reference(operand),
            Expr::Paren(inner) => self.contains_addr_reference(inner),
            // Every form below wraps subexpressions that may hide an addr
            // reference — `VIC as u16 + 1` used to fold to the register's
            // *address* and never read the register at all.
            Expr::Cast { expr: inner, .. } => self.contains_addr_reference(inner),
            Expr::Index { object, index } => {
                self.contains_addr_reference(object) || self.contains_addr_reference(index)
            }
            Expr::Field { object, .. } => self.contains_addr_reference(object),
            Expr::Slice {
                object, start, end, ..
            } => {
                self.contains_addr_reference(object)
                    || self.contains_addr_reference(start)
                    || self.contains_addr_reference(end)
            }
            Expr::SliceLen(inner) | Expr::U16Low(inner) | Expr::U16High(inner) => {
                self.contains_addr_reference(inner)
            }
            _ => false,
        }
    }

    pub(super) fn check_expr(&mut self, expr: &Spanned<Expr>) -> Result<Type, SemaError> {
        // Try to fold the expression if it's constant
        // Use const_env so we can fold references to const variables
        // BUT: don't fold if the expression contains references to addr (runtime values)
        let contains_addr_ref = self.contains_addr_reference(expr);
        if !contains_addr_ref && let Ok(const_val) = eval_const_expr_with_env(expr, &self.const_env)
        {
            self.folded_constants.insert(expr.span, const_val);
        }

        let result_ty = match &expr.node {
            Expr::Literal(lit) => self.check_literal(lit, expr.span)?,

            Expr::Variable(name) => self.check_variable(name, expr)?,

            Expr::Binary { left, op, right } => self.check_binary(left, op, right, expr.span)?,

            Expr::Call { function, args } => self.check_call(function, args, expr.span)?,

            Expr::CallIndirect { callee, args } => {
                // The callee must evaluate to a function pointer; check the
                // arguments against its signature and yield its return type.
                self.note_indirect_call();
                let callee_ty = self.check_expr(callee)?;
                let Type::Function(param_types, ret_ty) = callee_ty else {
                    return Err(SemaError::TypeMismatch {
                        expected: "function pointer".to_string(),
                        found: callee_ty.display_name(),
                        span: callee.span,
                    });
                };
                if args.len() != param_types.len() {
                    return Err(SemaError::ArityMismatch {
                        expected: param_types.len(),
                        found: args.len(),
                        span: expr.span,
                    });
                }
                for (arg, param_ty) in args.iter().zip(param_types.iter()) {
                    let saved = self.expected_type.take();
                    self.expected_type = Some(param_ty.clone());
                    let arg_ty = self.check_expr(arg);
                    self.expected_type = saved;
                    let arg_ty = arg_ty?;
                    if !arg_ty.is_implicitly_convertible_to(param_ty) {
                        return Err(SemaError::TypeMismatch {
                            expected: param_ty.display_name(),
                            found: arg_ty.display_name(),
                            span: arg.span,
                        });
                    }
                }
                (*ret_ty).clone()
            }

            Expr::Unary { op, operand } => self.check_unary(op, operand, expr.span)?,

            Expr::Paren(inner) => self.check_expr(inner)?,

            Expr::Cast {
                expr: inner,
                target_type,
            } => {
                // Check that the inner expression is valid
                let source_ty = self.check_expr(inner)?;

                // Validate BCD casts for constant expressions
                let target_ty = self.resolve_type(&target_type.node)?;

                Self::check_pointer_cast(&source_ty, &target_ty, expr.span)?;
                if let Type::Primitive(prim) = &target_ty
                    && matches!(
                        prim,
                        crate::ast::PrimitiveType::B8 | crate::ast::PrimitiveType::B16
                    )
                {
                    // Try to evaluate as constant to validate BCD range
                    if let Ok(value) = crate::sema::const_eval::eval_const_expr(inner) {
                        // Use the same validation as const_eval's apply_type_cast
                        use crate::sema::const_eval::validate_bcd_cast;
                        validate_bcd_cast(value, prim, expr.span)?;
                    }
                    // Note: Non-constant expressions cannot be validated at compile time
                    // This is a known limitation - runtime casts to BCD may produce invalid values
                }

                target_ty
            }

            Expr::StructInit { name, fields } => {
                // Look up the struct definition
                if !self.type_registry.structs.contains_key(&name.node) {
                    return Err(SemaError::UndefinedSymbol {
                        suggestion: self.table.closest_name(&name.node),
                        name: name.node.clone(),
                        span: name.span,
                    });
                }

                self.check_struct_init_fields(&name.node, fields)?;
                self.reserve_struct_temp(&name.node, fields, expr.span);

                Type::Named(name.node.clone())
            }

            Expr::AnonStructInit { fields } => {
                let ty = self.check_anon_struct_init(fields, expr.span)?;
                if let Type::Named(n) = &ty {
                    let n = n.clone();
                    self.reserve_struct_temp(&n, fields, expr.span);
                }
                ty
            }

            Expr::EnumVariant {
                enum_name,
                variant,
                data,
            } => self.check_enum_variant(enum_name, variant, data, expr.span)?,

            Expr::Field { object, field } => self.check_field_access(object, field)?,

            Expr::Index { object, index } => self.check_index(object, index, expr.span)?,

            Expr::Slice {
                object,
                start,
                end,
                inclusive,
            } => self.check_slice(object, start, end, *inclusive, expr.span)?,

            Expr::SliceLen(slice_expr) => {
                // Verify the expression is actually a slice, array, or string
                let slice_ty = self.check_expr(slice_expr)?;

                // A struct field named `len` wins over the built-in accessor:
                // the parser emitted SliceLen before types were known.
                if let Some(field_ty) = self.accessor_field_type(&slice_ty, "len", expr.span) {
                    field_ty
                } else {
                    // Check if it's a type that has a length
                    match &slice_ty {
                        Type::Slice(..) | Type::Array(_, _) | Type::String => {
                            // Slice/array/string length is always u16 on 6502 (our usize equivalent)
                            Type::Primitive(PrimitiveType::U16)
                        }
                        _ => {
                            return Err(SemaError::TypeMismatch {
                                expected: "slice, array, or string".to_string(),
                                found: slice_ty.display_name(),
                                span: slice_expr.span,
                            });
                        }
                    }
                }
            }

            Expr::U16Low(operand) => {
                let operand_ty = self.check_expr(operand)?;
                if let Some(field_ty) = self.accessor_field_type(&operand_ty, "low", expr.span) {
                    field_ty
                } else {
                    match &operand_ty {
                        Type::Primitive(PrimitiveType::U16)
                        | Type::Primitive(PrimitiveType::I16) => Type::Primitive(PrimitiveType::U8),
                        _ => {
                            return Err(SemaError::TypeMismatch {
                                expected: "u16 or i16".to_string(),
                                found: operand_ty.display_name(),
                                span: operand.span,
                            });
                        }
                    }
                }
            }

            Expr::U16High(operand) => {
                let operand_ty = self.check_expr(operand)?;
                if let Some(field_ty) = self.accessor_field_type(&operand_ty, "high", expr.span) {
                    field_ty
                } else {
                    match &operand_ty {
                        Type::Primitive(PrimitiveType::U16)
                        | Type::Primitive(PrimitiveType::I16) => Type::Primitive(PrimitiveType::U8),
                        _ => {
                            return Err(SemaError::TypeMismatch {
                                expected: "u16 or i16".to_string(),
                                found: operand_ty.display_name(),
                                span: operand.span,
                            });
                        }
                    }
                }
            }

            // CPU status flags - all return bool
            Expr::CpuFlagCarry
            | Expr::CpuFlagZero
            | Expr::CpuFlagOverflow
            | Expr::CpuFlagNegative => Type::Primitive(PrimitiveType::Bool),

            // Match expression
            Expr::Match {
                expr: match_expr,
                arms,
            } => {
                // Check the matched expression
                let match_ty = self.check_expr(match_expr)?;

                // Check each arm's body expression and track their types. Each
                // arm gets its own scope with the pattern's bindings in it, so
                // variable and enum-payload bindings resolve to real storage
                // (mirrors the match-statement path).
                // Sibling arms are mutually exclusive, so they share frame
                // storage: each starts from the same base, and the widest sets
                // the peak (mirrors the match-statement path).
                let arms_base = self.frame_cursor;
                let saved_free = self.loop_bound_free.clone();
                let mut arms_peak = arms_base;
                let mut arm_types = Vec::new();
                for arm in arms {
                    self.check_pattern_type(&arm.pattern, &match_ty)?;
                    self.reset_frame_to_match_base(arms_base, &saved_free);
                    self.table.enter_scope();
                    self.add_pattern_bindings(&arm.pattern.node, arm.pattern.span, &match_ty)?;
                    let arm_ty = self.check_expr(&arm.body)?;
                    self.table.exit_scope();
                    arms_peak = arms_peak.max(self.frame_cursor);
                    arm_types.push(arm_ty);
                }
                self.frame_cursor = arms_peak;
                self.loop_bound_free = saved_free;

                // All arms must have the same type (or be compatible)
                if arm_types.is_empty() {
                    return Err(SemaError::TypeMismatch {
                        expected: "at least one match arm".to_string(),
                        found: "no arms".to_string(),
                        span: expr.span,
                    });
                }

                // Unify the arm types into a single common type: each arm must be
                // implicitly convertible to (or accept) the others, e.g. mixing a
                // u8 and a u16 arm yields u16.
                let mut unified = arm_types[0].clone();
                for ty in arm_types.iter().skip(1) {
                    unified =
                        Self::unify_types(&unified, ty).ok_or_else(|| SemaError::TypeMismatch {
                            expected: unified.display_name(),
                            found: ty.display_name(),
                            span: expr.span,
                        })?;
                }
                unified
            }

            Expr::BitOp { object, kind, bit } => self.check_bitop(object, *kind, bit, expr.span)?,
        };

        // Store the resolved type for this expression so codegen can access it
        self.resolved_types.insert(expr.span, result_ty.clone());

        // Re-fold at the expression's own width now that it is known.
        //
        // The fold above runs before typing and so works in `i64`, truncating
        // once at the end. The generated code wraps after *every* operation, so
        // the two disagree whenever an intermediate leaves the type's range:
        // `(94 << 6) >> 3` on a `u8` folds to 240 in full precision but
        // computes 16 at run time, and the same expression written with a
        // variable took the run-time path — so a constant and its identical
        // runtime form gave different answers.
        if self.folded_constants.contains_key(&expr.span)
            && let Some((bits, signed)) = int_width_of(&result_ty)
            && let Ok(v) = crate::sema::const_eval::eval_const_expr_wrapping(
                expr,
                &self.const_env,
                bits,
                signed,
            )
        {
            self.folded_constants.insert(expr.span, v);
        }

        // A cast changes width on purpose, so the wrapping evaluator treats it
        // as a leaf and re-derives its operand in full precision — losing the
        // operand's own wrapping. `((15397 << 13) % 27617) as i8` is 31 through
        // a `u16` variable and something else entirely at full precision. The
        // operand's corrected fold is already in hand; narrow that instead.
        if let Some(v) = self.refold_through(expr, &result_ty) {
            self.folded_constants
                .insert(expr.span, crate::sema::const_eval::ConstValue::Integer(v));
        }

        // A `bool`-valued node has no width of its own, so the rule above
        // cannot reach it — and the full-precision fold underneath is wrong for
        // exactly the same reason: `(94 << 6) >= 229` is false at `u8` width
        // (the shift wraps to 128) and true in `i64` (6016). Recompute it from
        // the operands, which the rule above has already corrected, since every
        // subexpression is checked before its parent.
        if matches!(result_ty, Type::Primitive(PrimitiveType::Bool))
            && self.folded_constants.contains_key(&expr.span)
            && let Some(v) = self.refold_bool(expr)
        {
            self.folded_constants
                .insert(expr.span, crate::sema::const_eval::ConstValue::Bool(v));
        }

        Ok(result_ty)
    }

    /// An integer node that only passes its operand through — a cast or a pair
    /// of parentheses — recomputed from that operand's corrected fold, narrowed
    /// to this node's own width.
    fn refold_through(&self, expr: &Spanned<Expr>, ty: &Type) -> Option<i64> {
        let (bits, signed) = int_width_of(ty)?;
        let inner = match &expr.node {
            Expr::Paren(inner) => inner,
            Expr::Cast { expr: inner, .. } => inner,
            _ => return None,
        };
        let v = self.folded_constants.get(&inner.span)?.as_integer()?;
        Some(crate::sema::const_eval::narrow(v, bits, signed))
    }

    /// The already-folded value of a subexpression, as a truth value.
    fn folded_truth(&self, expr: &Spanned<Expr>) -> Option<bool> {
        use crate::sema::const_eval::ConstValue;
        match self.folded_constants.get(&expr.span)? {
            ConstValue::Bool(b) => Some(*b),
            ConstValue::Integer(v) => Some(*v != 0),
            ConstValue::String(_) => None,
        }
    }

    /// Recompute a constant `bool` from its operands' corrected folds.
    fn refold_bool(&self, expr: &Spanned<Expr>) -> Option<bool> {
        use crate::ast::UnaryOp;
        match &expr.node {
            Expr::Paren(inner) => self.folded_truth(inner),
            Expr::Unary {
                op: UnaryOp::Not,
                operand,
            } => Some(!self.folded_truth(operand)?),
            Expr::Binary { left, op, right } => match op {
                BinaryOp::And => Some(self.folded_truth(left)? && self.folded_truth(right)?),
                BinaryOp::Or => Some(self.folded_truth(left)? || self.folded_truth(right)?),
                BinaryOp::Eq
                | BinaryOp::Ne
                | BinaryOp::Lt
                | BinaryOp::Le
                | BinaryOp::Gt
                | BinaryOp::Ge => {
                    let a = self.folded_constants.get(&left.span)?.as_integer()?;
                    let b = self.folded_constants.get(&right.span)?.as_integer()?;
                    Some(match op {
                        BinaryOp::Eq => a == b,
                        BinaryOp::Ne => a != b,
                        BinaryOp::Lt => a < b,
                        BinaryOp::Le => a <= b,
                        BinaryOp::Gt => a > b,
                        _ => a >= b,
                    })
                }
                _ => None,
            },
            _ => None,
        }
    }

    /// Find a common type for two arm/branch types: identical types unify to
    /// themselves; otherwise the narrower widens to the other if implicitly
    /// convertible (e.g. u8 + u16 -> u16). Returns None if incompatible.
    fn unify_types(a: &Type, b: &Type) -> Option<Type> {
        if a == b || b.is_implicitly_convertible_to(a) {
            Some(a.clone())
        } else if a.is_implicitly_convertible_to(b) {
            Some(b.clone())
        } else {
            None
        }
    }

    fn check_literal(
        &mut self,
        lit: &crate::ast::Literal,
        span: crate::ast::Span,
    ) -> Result<Type, SemaError> {
        match lit {
            crate::ast::Literal::Integer(val) => {
                // If there is an expected integer type from context (e.g. a
                // `let x: i8 = 127;` annotation) and the value fits its range,
                // adopt it. Without this a positive literal always infers as
                // u8/u16 and cannot be assigned to a signed target.
                if let Some(Type::Primitive(expected)) = &self.expected_type {
                    let fits = match expected {
                        PrimitiveType::U8 => (0..=255).contains(val),
                        PrimitiveType::I8 => (-128..=127).contains(val),
                        PrimitiveType::U16 | PrimitiveType::Addr => (0..=65535).contains(val),
                        PrimitiveType::I16 => (-32768..=32767).contains(val),
                        _ => false,
                    };
                    if fits {
                        return Ok(Type::Primitive(*expected));
                    }
                }
                // Infer type based on value range
                if *val < 0 {
                    // Negative values
                    if *val >= -128 {
                        Ok(Type::Primitive(PrimitiveType::I8))
                    } else {
                        Ok(Type::Primitive(PrimitiveType::I16))
                    }
                } else {
                    // Positive values
                    if *val <= 255 {
                        Ok(Type::Primitive(PrimitiveType::U8))
                    } else if *val <= 65535 {
                        Ok(Type::Primitive(PrimitiveType::U16))
                    } else {
                        // Value too large for any type
                        Err(SemaError::Custom {
                            message: format!(
                                "integer literal {} is too large (max 65535 for u16)",
                                val
                            ),
                            span,
                        })
                    }
                }
            }
            crate::ast::Literal::Bool(_) => Ok(Type::Primitive(PrimitiveType::Bool)),
            crate::ast::Literal::Char(_) => Ok(Type::Primitive(PrimitiveType::Char)),
            crate::ast::Literal::String(s) => {
                // Validate string length (255 byte limit for 6502)
                if s.len() > 255 {
                    return Err(SemaError::Custom {
                        message: format!(
                            "string literal exceeds 255 byte limit: {} bytes",
                            s.len()
                        ),
                        span,
                    });
                }
                Ok(Type::String)
            }
            crate::ast::Literal::Array(elements) => {
                // A declared element type is the expected type for every element,
                // so literals adopt it: `let a: [u16; 3] = [0, 0, 0];` gives u16
                // elements rather than inferring u8 from the first one and then
                // failing to match. Same rule as literals in a binary operation —
                // it types the literal, it does not convert a value.
                let declared_elem = match &self.expected_type {
                    Some(Type::Array(elem, _)) => Some((**elem).clone()),
                    _ => None,
                };

                if elements.is_empty() {
                    // An empty array takes its element type from context when
                    // there is one, else defaults to u8.
                    let elem = declared_elem.unwrap_or(Type::Primitive(PrimitiveType::U8));
                    return Ok(Type::Array(Box::new(elem), 0));
                }

                let saved = self.expected_type.take();
                self.expected_type = declared_elem.clone();
                let first = self.check_expr(&elements[0]);
                self.expected_type = saved.clone();
                let element_ty = first?;

                // Every element must agree; each is checked against the declared
                // element type so a wider literal is not misread from the first.
                for elem in &elements[1..] {
                    let saved_inner = self.expected_type.take();
                    self.expected_type = declared_elem.clone().or(Some(element_ty.clone()));
                    let checked = self.check_expr(elem);
                    self.expected_type = saved_inner;
                    let elem_ty = checked?;
                    if elem_ty != element_ty {
                        return Err(SemaError::TypeMismatch {
                            expected: element_ty.display_name(),
                            found: elem_ty.display_name(),
                            span: elem.span,
                        });
                    }
                }

                Ok(Type::Array(Box::new(element_ty), elements.len()))
            }
            crate::ast::Literal::ArrayGen { param, body } => {
                self.check_array_gen(param, body, span)
            }
            crate::ast::Literal::ArrayFill { value, count } => {
                // `[0; 8]` for a `[u16; 8]` fills u16 elements, same rule.
                let declared_elem = match &self.expected_type {
                    Some(Type::Array(elem, _)) => Some((**elem).clone()),
                    _ => None,
                };
                let saved = self.expected_type.take();
                self.expected_type = declared_elem;
                let checked = self.check_expr(value);
                self.expected_type = saved;
                let element_ty = checked?;
                Ok(Type::Array(Box::new(element_ty), *count))
            }
        }
    }

    /// Check and evaluate a generated table, `[|i| => <expr>]`.
    ///
    /// The length is not written: it comes from the array type this expression
    /// is declared at. That is the whole reason the form is worth having — a
    /// table's length is already stated by its type, and repeating it is a
    /// chance to disagree — but it does mean there has to *be* a declared type,
    /// which is what the first error below says.
    ///
    /// Every entry is folded here, once, with the wrapping evaluator: `i` is a
    /// `u8` and the body follows the language's ordinary arithmetic, so the
    /// table holds exactly what the equivalent run-time loop would have
    /// computed. Anything else would make a table and a loop over the same
    /// expression disagree, which is the shape of a bug nobody finds.
    fn check_array_gen(
        &mut self,
        param: &Spanned<String>,
        body: &Spanned<Expr>,
        span: Span,
    ) -> Result<Type, SemaError> {
        let Some(Type::Array(elem, len)) = self.expected_type.clone() else {
            return Err(SemaError::Custom {
                message: "a generated table takes its length from the array type it is \
                          declared at, so it needs one: `const T: [u8; 16] = [|i| => …];`"
                    .to_string(),
                span,
            });
        };

        // `i` is a `u8`, so it cannot reach past the 256th entry — and on this
        // machine an index register cannot either.
        if len > 256 {
            return Err(SemaError::Custom {
                message: format!(
                    "a generated table holds at most 256 entries, because its index is a \
                     `u8`; this one declares {len}"
                ),
                span,
            });
        }

        // The body is checked once, with `i` in scope, against the element
        // type — so a mismatch is reported against the expression the reader
        // wrote rather than against one of 256 copies of it.
        let u8_ty = Type::Primitive(PrimitiveType::U8);
        self.table.enter_scope();
        self.table.insert(
            param.node.clone(),
            crate::sema::table::SymbolInfo {
                name: param.node.clone(),
                kind: crate::sema::table::SymbolKind::Constant,
                ty: u8_ty.clone(),
                location: crate::sema::table::SymbolLocation::None,
                mutable: false,
                access_mode: None,
                is_pub: false,
                containing_function: None,
                is_param: false,
                decl_span: Some(param.span),
            },
        );
        let saved = self.expected_type.take();
        self.expected_type = Some((*elem).clone());
        let checked = self.check_expr(body);
        self.expected_type = saved;
        self.table.exit_scope();

        let body_ty = checked?;
        if !body_ty.is_implicitly_convertible_to(&elem) {
            return Err(SemaError::TypeMismatch {
                expected: elem.display_name(),
                found: body_ty.display_name(),
                span: body.span,
            });
        }

        self.fold_array_gen(param, body, &elem, len, span)?;
        Ok(Type::Array(elem, len))
    }

    /// Fold every entry of a generated table and record it against `span`.
    ///
    /// Shared by the two ways one is declared. A `const` or `static` array is
    /// flattened during *registration* and its initialiser never reaches
    /// `check_expr`, so folding only in the type checker left exactly the
    /// declaration this feature exists for — a ROM table — unfolded.
    ///
    /// `i` enters the constant environment the way a `const` does, so the body
    /// may also name other constants; the binding is removed afterwards so it
    /// cannot leak into a later declaration.
    pub(super) fn fold_array_gen(
        &mut self,
        param: &Spanned<String>,
        body: &Spanned<Expr>,
        elem: &Type,
        len: usize,
        span: Span,
    ) -> Result<(), SemaError> {
        let (bits, signed) = int_width_of(elem).ok_or_else(|| SemaError::Custom {
            message: format!(
                "a generated table's elements must be an integer type, not {}",
                elem.display_name()
            ),
            span,
        })?;

        let mut values = Vec::with_capacity(len);
        let saved_binding = self.const_env.remove(&param.node);
        for i in 0..len {
            self.const_env.insert(
                param.node.clone(),
                crate::sema::const_eval::ConstValue::Integer(i as i64),
            );
            let folded = crate::sema::const_eval::eval_const_expr_wrapping(
                body,
                &self.const_env,
                bits,
                signed,
            );
            let v = match folded {
                Ok(v) => v,
                Err(e) => {
                    match saved_binding {
                        Some(prev) => self.const_env.insert(param.node.clone(), prev),
                        None => self.const_env.remove(&param.node),
                    };
                    // "constant 'x' not found" would name the one thing that
                    // *is* bound, so say what the body has to be instead.
                    return Err(match e {
                        SemaError::Custom { .. } => SemaError::Custom {
                            message: "a generated table's body must be a constant expression \
                                      of its index: it becomes data before the program runs, \
                                      so there is nothing to compute it with"
                                .to_string(),
                            span: body.span,
                        },
                        other => other,
                    });
                }
            };
            let Some(n) = v.as_integer() else {
                match saved_binding {
                    Some(prev) => self.const_env.insert(param.node.clone(), prev),
                    None => self.const_env.remove(&param.node),
                };
                return Err(SemaError::Custom {
                    message: "a generated table's body must produce a number".to_string(),
                    span: body.span,
                });
            };
            values.push(n);
        }
        match saved_binding {
            Some(prev) => self.const_env.insert(param.node.clone(), prev),
            None => self.const_env.remove(&param.node),
        };

        self.generated_tables.insert(span, values);
        Ok(())
    }

    fn check_variable(&mut self, name: &str, expr: &Spanned<Expr>) -> Result<Type, SemaError> {
        // Every mention, counted here because every mention comes through here.
        *self.name_mentions.entry(name.to_string()).or_insert(0) += 1;

        let info = if let Some(info) = self.table.lookup(name) {
            info.clone()
        } else {
            return Err(SemaError::UndefinedSymbol {
                suggestion: self.table.closest_name(name),
                name: name.to_string(),
                span: expr.span,
            });
        };

        // Check for reading from write-only address (skip if this is an assignment target)
        if !self.checking_assignment_target
            && info.kind == SymbolKind::Address
            && let Some(crate::ast::AccessMode::Write) = info.access_mode
        {
            return Err(SemaError::WriteOnlyRead {
                name: name.to_string(),
                span: expr.span,
            });
        }

        self.resolved_symbols.insert(expr.span, info.clone());

        // Using a function's bare name as a value takes its address: record it so
        // codegen routes its arguments through the fixed indirect-arg staging block.
        // Taking the address also counts as using the function (it will be reached
        // through a function pointer), so it is not reported as dead code.
        if info.kind == SymbolKind::Function {
            self.address_taken_functions.insert(name.to_string());
            self.called_functions.insert(name.to_string());
        }

        // Mark variable as used (for unused variable/parameter warnings)
        self.used_variables.insert(name.to_string());
        // Also track in all_used_symbols (for unused import warnings)
        self.all_used_symbols.insert(name.to_string());
        // And as a liveness edge, so an imported item referenced only from
        // here is not dropped from the output.
        self.record_symbol_ref(name);

        Ok(info.ty)
    }

    /// Is this operand an integer expression whose type nothing pins down, so
    /// it can adopt the other operand's? An integer literal, a unary-negated
    /// one (`-5`), any of those in parentheses, and any binary combination of
    /// them: `(37 >> 1)` is as free to be `i8` as `18` is. A cast is
    /// deliberately excluded — it names the type it produces.
    fn is_adaptable_int_literal(expr: &Expr) -> bool {
        use crate::ast::{Literal, UnaryOp};
        match expr {
            Expr::Literal(Literal::Integer(_)) => true,
            Expr::Unary {
                op: UnaryOp::Neg,
                operand,
            } => matches!(&operand.node, Expr::Literal(Literal::Integer(_))),
            Expr::Paren(inner) => Self::is_adaptable_int_literal(&inner.node),
            Expr::Binary { left, right, .. } => {
                Self::is_adaptable_int_literal(&left.node)
                    && Self::is_adaptable_int_literal(&right.node)
            }
            _ => false,
        }
    }

    /// Every literal value written inside an adaptable expression. The type is
    /// chosen to hold the literals as the programmer wrote them, not the value
    /// the expression computes — that value depends on the type, which is what
    /// is being decided.
    fn collect_adaptable_ints(expr: &Expr, out: &mut Vec<i64>) {
        use crate::ast::{Literal, UnaryOp};
        match expr {
            Expr::Literal(Literal::Integer(v)) => out.push(*v),
            Expr::Unary {
                op: UnaryOp::Neg,
                operand,
            } => {
                if let Expr::Literal(Literal::Integer(v)) = &operand.node {
                    out.push(-*v);
                }
            }
            Expr::Paren(inner) => Self::collect_adaptable_ints(&inner.node, out),
            Expr::Binary { left, right, .. } => {
                Self::collect_adaptable_ints(&left.node, out);
                Self::collect_adaptable_ints(&right.node, out);
            }
            _ => {}
        }
    }

    fn literal_fits(v: i64, p: PrimitiveType) -> bool {
        match p {
            PrimitiveType::U8 => (0..=255).contains(&v),
            PrimitiveType::I8 => (-128..=127).contains(&v),
            PrimitiveType::U16 | PrimitiveType::Addr => (0..=65535).contains(&v),
            PrimitiveType::I16 => (-32768..=32767).contains(&v),
            _ => false,
        }
    }

    /// The type two bare literal operands should share. The ambient expectation
    /// wins when it holds both values (so `let x: i16 = 3 - 5;` still computes
    /// at 16 bits); otherwise the narrowest type that holds both.
    fn common_literal_type(left: &Expr, right: &Expr, expected: Option<&Type>) -> Option<Type> {
        let fallback = expected.cloned();
        let mut values = Vec::new();
        Self::collect_adaptable_ints(left, &mut values);
        Self::collect_adaptable_ints(right, &mut values);
        if values.is_empty() {
            return fallback;
        }
        if let Some(Type::Primitive(p)) = expected
            && values.iter().all(|v| Self::literal_fits(*v, *p))
        {
            return fallback;
        }
        let lo = *values.iter().min().unwrap();
        let hi = *values.iter().max().unwrap();
        let prim = if lo >= 0 {
            if hi <= 255 {
                PrimitiveType::U8
            } else if hi <= 65535 {
                PrimitiveType::U16
            } else {
                return fallback;
            }
        } else if lo >= -128 && hi <= 127 {
            PrimitiveType::I8
        } else if lo >= -32768 && hi <= 32767 {
            PrimitiveType::I16
        } else {
            return fallback;
        };
        Some(Type::Primitive(prim))
    }

    /// The bit width a shift count is measured against, for the types a shift
    /// applies to. `None` for anything else, which the operator check below
    /// rejects on its own terms.
    fn shift_width_bits(ty: &Type) -> Option<u32> {
        use crate::ast::PrimitiveType as P;
        match ty {
            Type::Primitive(P::U8 | P::I8 | P::B8) => Some(8),
            Type::Primitive(P::U16 | P::I16 | P::B16) => Some(16),
            _ => None,
        }
    }

    fn check_binary(
        &mut self,
        left: &Spanned<Expr>,
        op: &BinaryOp,
        right: &Spanned<Expr>,
        span: crate::ast::Span,
    ) -> Result<Type, SemaError> {
        // Literals-only width adaptation: a bare integer literal operand adopts
        // the other operand's (integer) type, so it participates at the correct
        // width in any operand position and for any operator — including
        // comparisons in a condition, where there is no ambient expected type
        // (e.g. `if a < 5` with `a: u16`). Only literals adapt; two *variables*
        // of different widths stay an error, because Wraith performs no implicit
        // type conversions. The literal is type-checked with the sibling's type
        // as the expected type; if it does not fit, adaptation simply does not
        // happen and the usual mismatch error is produced. A negated literal
        // (`-5`) and parenthesized literals count too (check_unary already
        // honors the expected type for a negated literal).
        let left_is_int_lit = Self::is_adaptable_int_literal(&left.node);
        let right_is_int_lit = Self::is_adaptable_int_literal(&right.node);

        let (left_ty, right_ty) = if right_is_int_lit && !left_is_int_lit {
            let lt = self.check_expr(left)?;
            let saved = self.expected_type.take();
            self.expected_type = Some(lt.clone());
            let rt = self.check_expr(right);
            self.expected_type = saved;
            (lt, rt?)
        } else if left_is_int_lit && !right_is_int_lit {
            let rt = self.check_expr(right)?;
            let saved = self.expected_type.take();
            self.expected_type = Some(rt.clone());
            let lt = self.check_expr(left);
            self.expected_type = saved;
            (lt?, rt)
        } else if left_is_int_lit && right_is_int_lit {
            // Two bare literals inform nothing about each other, so each falls
            // back to its own default — `-5` to `i8`, `3` to `u8` — and the
            // operator then rejects the pair even though one type holds both
            // values. Give them a shared expected type: the ambient one when it
            // holds both, else the narrowest that does. `if (-5 - 3) < n` is the
            // shape that hits this, where no ambient type exists to adopt.
            let common =
                Self::common_literal_type(&left.node, &right.node, self.expected_type.as_ref());
            let saved = std::mem::replace(&mut self.expected_type, common);
            let lt = self.check_expr(left);
            let rt = self.check_expr(right);
            self.expected_type = saved;
            (self.poison_on_err(lt), self.poison_on_err(rt))
        } else {
            // Neither side informs the other's type, so check both: two bad
            // names in `p + q` should report together rather than one per
            // recompile. (The literal-adaptation branches above deliberately
            // keep `?` — there the first operand's type is what the second is
            // checked against, so continuing past a failure would be guessing.)
            let lt = self.check_expr(left);
            let rt = self.check_expr(right);
            (self.poison_on_err(lt), self.poison_on_err(rt))
        };

        // A poisoned operand has already reported. Anything below would only add
        // a second diagnostic about an operator applied to `<unknown>`.
        if matches!(left_ty, Type::Error) || matches!(right_ty, Type::Error) {
            return Ok(Type::Error);
        }

        // A shift count the compiler can see is at or past the width of the
        // value shifts every bit out. The result is then a constant — 0, or -1
        // for an arithmetic right shift of a negative value — and the operand
        // plays no part in it.
        //
        // A warning rather than an error, unlike the zero divisor: the
        // behaviour is defined and useful (`x >> 15` is a sign test, and
        // clearing a value by shifting it out is a real if unusual idiom), so
        // this points at a probable mistake rather than forbidding one.
        if matches!(op, BinaryOp::Shl | BinaryOp::Shr)
            && let Some(width) = Self::shift_width_bits(&left_ty)
            && let Ok(v) = eval_const_expr_with_env(right, &self.const_env)
            && let Some(count) = v.as_integer()
            && count >= i64::from(width)
        {
            // An arithmetic right shift keeps the sign bit, so a negative
            // value saturates to -1 rather than 0.
            let result = if matches!(op, BinaryOp::Shr) && left_ty.is_signed() {
                "0 or -1"
            } else {
                "0"
            };
            self.warnings
                .push(crate::sema::Warning::ShiftCountAtOrPastWidth {
                    op: if matches!(op, BinaryOp::Shl) {
                        "<<"
                    } else {
                        ">>"
                    },
                    count,
                    ty: left_ty.display_name(),
                    width,
                    result,
                    span: right.span,
                });
        }

        // A divisor that is *known to be zero* is refused. `x / 0` has a
        // defined answer — the all-ones sentinel, see the specification — but
        // no program means it: the value carries no information about `x`, and
        // writing it is always a mistake rather than a choice. Catching it
        // costs nothing, because it is exactly the case the compiler can see.
        //
        // Only a constant divisor, which is the limit of what can be decided
        // here. A variable that happens to hold zero at run time still gets the
        // sentinel, which is why the sentinel is defined at all.
        if matches!(op, BinaryOp::Div | BinaryOp::Mod)
            && let Ok(v) = eval_const_expr_with_env(right, &self.const_env)
            && v.as_integer() == Some(0)
        {
            return Err(SemaError::Custom {
                message: format!(
                    "{} by zero: the divisor is always zero here. The result would be the \
                     all-ones value this language defines for it, which says nothing about \
                     the dividend — so this is a mistake rather than a choice",
                    if matches!(op, BinaryOp::Div) {
                        "division"
                    } else {
                        "modulo"
                    }
                ),
                span: right.span,
            });
        }

        // No binary operator applies to a pointer. This has to be said
        // explicitly: the compatibility gate further down is `left_ty ==
        // right_ty`, so `p + q` on two `&u8`s passes it and emits a 16-bit add
        // using the A:Y convention on values that arrived in A:X — a wrong
        // answer with no diagnostic anywhere.
        //
        // Arithmetic is `p[i]`, scaled by the element width. Comparison goes
        // through `as u16`, which puts the address in the register pair the
        // 16-bit compare paths expect.
        if matches!(left_ty, Type::Pointer(_)) || matches!(right_ty, Type::Pointer(_)) {
            // Equality on two pointers of the *same* type compares the
            // addresses — the natural null check for the linked lists that
            // `struct Node { next: &Node }` now makes expressible is
            // `p == 0 as &Node`. Ordering and arithmetic stay rejected: a
            // relative order between two heap-less addresses is rarely meaningful
            // and `<`/`+` on the A:X pointer pair would miscompile.
            if matches!(op, BinaryOp::Eq | BinaryOp::Ne) && left_ty == right_ty {
                return Ok(Type::Primitive(PrimitiveType::Bool));
            }
            let hint = match op {
                BinaryOp::Add | BinaryOp::Sub => {
                    "index instead, as `p[i]`, which scales by the element width"
                }
                BinaryOp::Eq
                | BinaryOp::Ne
                | BinaryOp::Lt
                | BinaryOp::Le
                | BinaryOp::Gt
                | BinaryOp::Ge => "compare the addresses, as `p as u16 == q as u16`",
                _ => "cast to `u16` first if you mean to operate on the address",
            };
            return Err(SemaError::InvalidBinaryOp {
                op: format!("{:?} ({})", op, hint),
                left_ty: left_ty.display_name(),
                right_ty: right_ty.display_name(),
                span,
            });
        }

        // String operators: `+` concatenates (str), `==`/`!=` compare (bool).
        if matches!((&left_ty, &right_ty), (Type::String, Type::String)) {
            match op {
                BinaryOp::Add => {
                    // Result is also a string type
                    return Ok(Type::String);
                }
                BinaryOp::Eq | BinaryOp::Ne => {
                    return Ok(Type::Primitive(PrimitiveType::Bool));
                }
                _ => {
                    return Err(SemaError::InvalidBinaryOp {
                        op: format!("{:?} (strings support '+', '==', and '!=')", op),
                        left_ty: left_ty.display_name(),
                        right_ty: right_ty.display_name(),
                        span,
                    });
                }
            }
        }

        // BCD type validation
        if let (Type::Primitive(left_prim), Type::Primitive(right_prim)) = (&left_ty, &right_ty)
            && (left_prim.is_bcd() || right_prim.is_bcd())
        {
            // Rule: Both operands must be same BCD type
            if left_prim != right_prim {
                return Err(SemaError::InvalidBinaryOp {
                    op: format!("{:?}", op),
                    left_ty: left_ty.display_name(),
                    right_ty: right_ty.display_name(),
                    span,
                });
            }

            // Only allow Add, Sub, comparisons on BCD
            match op {
                BinaryOp::Add | BinaryOp::Sub => {} // Hardware supported
                BinaryOp::Eq
                | BinaryOp::Ne
                | BinaryOp::Lt
                | BinaryOp::Le
                | BinaryOp::Gt
                | BinaryOp::Ge => {} // Comparisons work

                BinaryOp::Mul | BinaryOp::Div | BinaryOp::Mod => {
                    return Err(SemaError::InvalidBinaryOp {
                        op: format!("{:?} (not supported on BCD, convert to binary first)", op),
                        left_ty: left_ty.display_name(),
                        right_ty: right_ty.display_name(),
                        span,
                    });
                }

                BinaryOp::BitAnd
                | BinaryOp::BitOr
                | BinaryOp::BitXor
                | BinaryOp::Shl
                | BinaryOp::Shr => {
                    return Err(SemaError::InvalidBinaryOp {
                        op: format!("{:?} (bitwise ops not allowed on BCD)", op),
                        left_ty: left_ty.display_name(),
                        right_ty: right_ty.display_name(),
                        span,
                    });
                }

                _ => {
                    return Err(SemaError::InvalidBinaryOp {
                        op: format!("{:?}", op),
                        left_ty: left_ty.display_name(),
                        right_ty: right_ty.display_name(),
                        span,
                    });
                }
            }
        }

        // Special handling for shift operations: allow u16 to be shifted by u8
        // (shift amounts realistically never exceed 255)
        let types_compatible = if matches!(op, BinaryOp::Shl | BinaryOp::Shr) {
            // Allow same-type shifts (u8 >> u8, u16 >> u16, etc.), or any
            // integer (incl. signed i8/i16) shifted by a u8 count — shift
            // amounts realistically never exceed 255.
            left_ty == right_ty
                || (matches!(right_ty, Type::Primitive(PrimitiveType::U8))
                    && matches!(
                        left_ty,
                        Type::Primitive(
                            PrimitiveType::U8
                                | PrimitiveType::I8
                                | PrimitiveType::U16
                                | PrimitiveType::I16
                        )
                    ))
        } else {
            // For all other operations, types must match
            left_ty == right_ty
        };

        if !types_compatible {
            return Err(SemaError::InvalidBinaryOp {
                op: format!("{:?}", op),
                left_ty: left_ty.display_name(),
                right_ty: right_ty.display_name(),
                span,
            });
        }

        // Comparison and logical operators return Bool
        match op {
            BinaryOp::Eq
            | BinaryOp::Ne
            | BinaryOp::Lt
            | BinaryOp::Le
            | BinaryOp::Gt
            | BinaryOp::Ge
            | BinaryOp::And
            | BinaryOp::Or => Ok(Type::Primitive(PrimitiveType::Bool)),
            // Arithmetic and bitwise operators return the operand type
            _ => Ok(left_ty),
        }
    }

    fn check_call(
        &mut self,
        function: &Spanned<String>,
        args: &[Spanned<Expr>],
        span: crate::ast::Span,
    ) -> Result<Type, SemaError> {
        // Mark function as used (for unused variable/import warnings)
        self.used_variables.insert(function.node.clone());
        self.all_used_symbols.insert(function.node.clone());
        self.record_symbol_ref(&function.node);

        // Track function call for unused function detection
        self.called_functions.insert(function.node.clone());

        // Is the callee a function-pointer *variable* (indirect call) rather
        // than a named function? Record its symbol under the call's span so
        // codegen can find its storage (the local scope is popped by then).
        let is_fnptr_var = self
            .table
            .lookup(&function.node)
            .is_some_and(|s| s.kind == SymbolKind::Variable && matches!(s.ty, Type::Function(..)));
        if is_fnptr_var {
            if let Some(s) = self.table.lookup(&function.node) {
                self.resolved_symbols.insert(function.span, s.clone());
            }
            self.note_indirect_call();
        } else if let Some(caller) = &self.current_function {
            // Record a call-graph edge (caller -> callee) for frame coloring and
            // recursion detection. Only for real named functions.
            self.call_edges
                .entry(caller.clone())
                .or_default()
                .insert(function.node.clone());

            // A call nested inside this call's *arguments* runs while this
            // callee's parameter block is half-written, so the two are live at
            // once and their frames must not share addresses. The call graph
            // alone does not say so — they are siblings under a common caller,
            // which colouring is otherwise free to overlay — so record the edge
            // that makes it true.
            //
            // Without it `outer(v, inner(200, 201), v)` passed 200 as `outer`'s
            // first argument: `inner`'s parameters had been coloured over
            // `outer`'s. If the nested callee transitively calls this one, the
            // edge closes a cycle, and the resulting save/restore is not
            // over-caution — the parameters really are clobbered.
            let mut nested = Vec::new();
            for arg in args {
                Self::collect_called_names(&arg.node, &mut nested);
            }
            for callee in nested {
                if callee != function.node {
                    self.call_edges
                        .entry(function.node.clone())
                        .or_default()
                        .insert(callee);
                }
            }
        }

        // Verify function signature: check that it's a function and get param/return types
        let (param_types, ret_type) = if let Some(info) = self.table.lookup(&function.node) {
            if let Type::Function(param_types, ret_type) = &info.ty {
                (param_types.clone(), ret_type.clone())
            } else {
                return Err(SemaError::TypeMismatch {
                    expected: "function".to_string(),
                    found: info.ty.display_name(),
                    span: function.span,
                });
            }
        } else {
            return Err(SemaError::UndefinedSymbol {
                suggestion: self.table.closest_name(&function.node),
                name: function.node.clone(),
                span: function.span,
            });
        };

        if args.len() != param_types.len() {
            return Err(SemaError::ArityMismatch {
                expected: param_types.len(),
                found: args.len(),
                span,
            });
        }
        // Each argument is checked against its parameter type, not the ambient
        // expected type (e.g. the `let x: u16 = f(4)` target). Save/restore the
        // outer context so a literal argument infers against the parameter.
        let saved_expected = self.expected_type.take();
        for (arg, param_ty) in args.iter().zip(param_types.iter()) {
            self.expected_type = Some(param_ty.clone());
            // Arguments are independent of each other, so a bad one is recorded
            // and the rest are still checked — `f(bad1, bad2)` reports both.
            // `Type::Error` then satisfies the conversion check below, so the
            // argument that already reported does not also report a mismatch.
            let arg_ty = self.check_expr(arg);
            let arg_ty = self.poison_on_err(arg_ty);
            // Check if argument type can be implicitly converted to parameter type
            if !arg_ty.is_implicitly_convertible_to(param_ty) {
                self.record(SemaError::TypeMismatch {
                    expected: param_ty.display_name(),
                    found: arg_ty.display_name(),
                    span: arg.span,
                });
            }
        }
        self.expected_type = saved_expected;
        Ok(*ret_type)
    }

    /// The legality matrix for casts that involve a pointer on either side.
    ///
    /// Casts are otherwise unchecked in this language — a cast is how you say
    /// "I know what I am doing". A pointer is the one place where being wrong
    /// is silent rather than merely surprising, so both directions are pinned
    /// down:
    ///
    /// - `p as u16` and `p as &U` keep all 16 bits. Anything narrower would
    ///   throw away the page and leave a plausible-looking zero-page address.
    /// - `n as &T` accepts an integer of any width (an 8-bit one names a
    ///   zero-page byte), but nothing else. `true as &u8` or `s as &u8` have no
    ///   meaning, and letting them through would produce a pointer to whatever
    ///   the representation happened to be.
    fn check_pointer_cast(
        source: &Type,
        target: &Type,
        span: crate::ast::Span,
    ) -> Result<(), SemaError> {
        let integer = |t: &Type| {
            matches!(
                t,
                Type::Primitive(
                    PrimitiveType::U8
                        | PrimitiveType::I8
                        | PrimitiveType::U16
                        | PrimitiveType::I16
                        | PrimitiveType::Addr
                )
            )
        };

        if matches!(source, Type::Pointer(_))
            && !matches!(
                target,
                Type::Pointer(_) | Type::Primitive(PrimitiveType::U16 | PrimitiveType::I16)
            )
        {
            return Err(SemaError::Custom {
                message: format!(
                    "cannot cast a pointer to {}; an address is 16 bits, so only \
                     `as u16`, `as i16` or another pointer type keeps it whole",
                    target.display_name()
                ),
                span,
            });
        }

        if matches!(target, Type::Pointer(_))
            && !matches!(source, Type::Pointer(_))
            && !integer(source)
        {
            return Err(SemaError::Custom {
                message: format!(
                    "cannot cast {} to a pointer; only an integer address can be",
                    source.display_name()
                ),
                span,
            });
        }

        Ok(())
    }

    /// Type-check `&operand`.
    ///
    /// Only a few expression forms have an address to take. A literal, a call
    /// result or an arithmetic result lives in a register or a scratch byte
    /// that is about to be reused, so `&` on one would hand back an address
    /// with nothing behind it.
    ///
    /// Several *named* things are rejected too, each for its own reason:
    ///
    /// - an immutable `const` is recorded at the sentinel address `Absolute(0)`
    ///   and referenced by ROM label, so `&CONST` would silently mean `$0000`;
    /// - an `addr` declaration carries a read/write access mode that is checked
    ///   at the symbol, and a pointer would launder that check away;
    /// - a function name already *is* its address;
    /// - a string, slice or enum variable is already a pointer.
    fn check_addr_of(
        &mut self,
        operand: &Spanned<Expr>,
        span: crate::ast::Span,
    ) -> Result<Type, SemaError> {
        let reject = |what: &str, hint: &str| {
            Err(SemaError::Custom {
                message: if hint.is_empty() {
                    format!("cannot take the address of {}", what)
                } else {
                    format!("cannot take the address of {}; {}", what, hint)
                },
                span,
            })
        };

        match &operand.node {
            Expr::Paren(inner) => self.check_addr_of(inner, span),

            Expr::Variable(name) => {
                let sym = match self.table.lookup(name) {
                    Some(s) => s.clone(),
                    None => {
                        return Err(SemaError::UndefinedSymbol {
                            suggestion: self.table.closest_name(name),
                            name: name.clone(),
                            span: operand.span,
                        });
                    }
                };
                match sym.kind {
                    SymbolKind::Constant => {
                        return reject(
                            &format!("the constant '{}'", name),
                            "constants live in ROM and are referenced by label, not by address",
                        );
                    }
                    SymbolKind::Address => {
                        return reject(
                            &format!("the address declaration '{}'", name),
                            "its read/write access mode is checked at the name; \
                             write `0x1234 as &u8` if that is really what you want",
                        );
                    }
                    SymbolKind::Function => {
                        return reject(
                            &format!("the function '{}'", name),
                            "a function name is already its address",
                        );
                    }
                    _ => {}
                }
                // Run the ordinary variable check so the symbol is resolved,
                // marked used, and recorded as a liveness edge.
                let ty = self.check_expr(operand)?;
                match ty {
                    // An array decays to a pointer to its first element: the
                    // variable's slot already holds exactly that pointer.
                    Type::Array(elem, _) => Ok(Type::Pointer(elem)),
                    Type::String | Type::Slice(_) => reject(
                        &format!("'{}'", name),
                        "it is already a reference; pass it directly",
                    ),
                    Type::Named(ref n) if self.type_registry.get_enum(n).is_some() => reject(
                        &format!("'{}'", name),
                        "an enum value is already a pointer; pass it directly",
                    ),
                    other => Ok(Type::Pointer(Box::new(other))),
                }
            }

            Expr::Index { .. } => {
                let elem_ty = self.check_expr(operand)?;
                Ok(Type::Pointer(Box::new(elem_ty)))
            }

            Expr::Field { .. } => {
                let field_ty = self.check_expr(operand)?;
                Ok(Type::Pointer(Box::new(field_ty)))
            }

            // `&*p` is just `p`.
            Expr::Unary {
                op: crate::ast::UnaryOp::Deref,
                operand: inner,
            } => self.check_expr(inner),

            _ => reject(
                "a temporary",
                "only a variable, an element or a field has an address",
            ),
        }
    }

    fn check_unary(
        &mut self,
        op: &crate::ast::UnaryOp,
        operand: &Spanned<Expr>,
        span: crate::ast::Span,
    ) -> Result<Type, SemaError> {
        // `&x` inspects the operand's *form* before checking it, because only a
        // few forms have an address at all.
        if matches!(op, crate::ast::UnaryOp::AddrOf) {
            return self.check_addr_of(operand, span);
        }

        let operand_ty = self.check_expr(operand)?;

        // Check type compatibility with the operator
        match op {
            crate::ast::UnaryOp::AddrOf => unreachable!("handled above"),
            crate::ast::UnaryOp::Deref => match operand_ty {
                Type::Pointer(inner) => match *inner {
                    // A struct or enum pointee has no by-value form to produce:
                    // this language has no aggregate temporaries, so a field is
                    // the unit of access.
                    Type::Named(ref n) => Err(SemaError::Custom {
                        message: format!(
                            "cannot dereference a pointer to '{}' as a value; read a field \
                             instead, as `p.field`",
                            n
                        ),
                        span,
                    }),
                    Type::Array(..) | Type::Slice(_) | Type::String | Type::Void => {
                        Err(SemaError::Custom {
                            message: "cannot dereference a pointer to this type as a value"
                                .to_string(),
                            span,
                        })
                    }
                    scalar => Ok(scalar),
                },
                other => Err(SemaError::InvalidUnaryOp {
                    op: "*".to_string(),
                    operand_ty: other.display_name(),
                    span,
                }),
            },
            crate::ast::UnaryOp::Neg => {
                // Negation works on numeric types and always yields a signed
                // result: `-5` is i8, not u8. `5` on its own infers as u8, so
                // without this the operand type would leak through and code like
                // `let x: i8 = -5;` would fail to type-check.
                // Negation is arithmetic, so `bool` and `char` are rejected —
                // `-true` used to pass because both are primitives. The integer
                // and BCD types (and `addr`, a byte MMIO value) still negate.
                if !matches!(
                    operand_ty,
                    Type::Primitive(
                        PrimitiveType::U8
                            | PrimitiveType::I8
                            | PrimitiveType::U16
                            | PrimitiveType::I16
                            | PrimitiveType::B8
                            | PrimitiveType::B16
                            | PrimitiveType::Addr
                    )
                ) {
                    return Err(SemaError::InvalidUnaryOp {
                        op: "-".to_string(),
                        operand_ty: operand_ty.display_name(),
                        span,
                    });
                }
                // For a literal operand, choose the signed width from the
                // negated magnitude so `-5` is i8 and `-200` widens to i16.
                // For any other operand keep the operand's own type: negating an
                // unsigned value is a two's-complement wrap that stays the same
                // width/signedness (so `-X` on a u8 addr remains u8-assignable).
                if let Expr::Literal(crate::ast::Literal::Integer(v)) = &operand.node {
                    let neg = -(*v);
                    // Honor an explicit integer target: `-10` stored into a u8
                    // becomes u8 (two's-complement wrap to 246), `-5` into an i8
                    // stays i8. BCD is excluded so its cast range-checks the value.
                    if let Some(Type::Primitive(exp)) = &self.expected_type
                        && !exp.is_bcd()
                        && matches!(
                            exp,
                            PrimitiveType::U8
                                | PrimitiveType::I8
                                | PrimitiveType::U16
                                | PrimitiveType::I16
                                | PrimitiveType::Addr
                        )
                    {
                        let fits = match exp.size_bytes() {
                            1 => (-128..=255).contains(&neg),
                            _ => (-32768..=65535).contains(&neg),
                        };
                        if fits {
                            return Ok(Type::Primitive(*exp));
                        }
                    }
                    // No usable context: pick the signed width from the magnitude.
                    if (-128..=127).contains(&neg) {
                        Ok(Type::Primitive(PrimitiveType::I8))
                    } else if (-32768..=32767).contains(&neg) {
                        Ok(Type::Primitive(PrimitiveType::I16))
                    } else {
                        // Past i16 there is no type that holds the value:
                        // `let x: i16 = -40000;` used to claim i16 here and
                        // wrap to 25536 at codegen, while the `const` form of
                        // the same declaration correctly errored.
                        Err(SemaError::Custom {
                            message: format!(
                                "integer literal {} is out of range (min -32768 for i16)",
                                neg
                            ),
                            span,
                        })
                    }
                } else {
                    Ok(operand_ty)
                }
            }
            crate::ast::UnaryOp::BitNot => {
                // Bitwise NOT works on integer types
                if operand_ty.is_primitive() {
                    Ok(operand_ty)
                } else {
                    Err(SemaError::InvalidUnaryOp {
                        op: "~".to_string(),
                        operand_ty: operand_ty.display_name(),
                        span,
                    })
                }
            }
            crate::ast::UnaryOp::Not => {
                // Logical NOT returns bool
                Ok(Type::Primitive(PrimitiveType::Bool))
            }
        }
    }

    fn check_anon_struct_init(
        &mut self,
        fields: &[crate::ast::FieldInit],
        span: crate::ast::Span,
    ) -> Result<Type, SemaError> {
        // Get expected type from context (set during VarDecl analysis)
        let struct_name = match &self.expected_type {
            Some(Type::Named(name)) => name.clone(),
            Some(other_ty) => {
                return Err(SemaError::TypeMismatch {
                    expected: "struct type".to_string(),
                    found: other_ty.display_name(),
                    span,
                });
            }
            None => {
                return Err(SemaError::Custom {
                    message: "Cannot infer struct type for anonymous struct literal. Use explicit type: StructName { ... }".to_string(),
                    span,
                });
            }
        };

        // Verify struct exists
        if !self.type_registry.structs.contains_key(&struct_name) {
            return Err(SemaError::UndefinedSymbol {
                suggestion: self.table.closest_name(&struct_name),
                name: struct_name.clone(),
                span,
            });
        }

        self.check_struct_init_fields(&struct_name, fields)?;

        // Store the resolved struct name for codegen
        self.resolved_struct_names.insert(span, struct_name.clone());

        Ok(Type::Named(struct_name))
    }

    /// Check a struct literal's fields against the definition: every named
    /// field must exist, and each value is checked with the field's declared
    /// type as its expected type, so a literal adopts the field's width and a
    /// value of the wrong type errors. An omitted field is fine — the
    /// flattener zero-fills it.
    /// Reserve scratch RAM for a struct literal that cannot be emitted as
    /// constant bytes.
    ///
    /// A literal whose fields are all constants is emitted into the CODE
    /// section and evaluates to a pointer at those bytes — nothing to build at
    /// run time. One with a computed field has to be *assembled* somewhere
    /// writable, and ROM is not it. The block is allocated per literal site
    /// from the same call-graph-colored region local array data uses, so
    /// functions that can never be active at once share the space.
    ///
    /// Reserved for every computed literal, including those that turn out to
    /// be initializing a variable directly (where codegen writes the fields
    /// straight to the destination and never touches this block). Predicting
    /// that here would mean duplicating codegen's destination rules and
    /// silently emitting a "constant expressions only" error whenever the two
    /// drifted apart; a colored block per site is the cheaper mistake.
    fn reserve_struct_temp(
        &mut self,
        struct_name: &str,
        fields: &[crate::ast::FieldInit],
        span: crate::ast::Span,
    ) {
        let all_constant = fields
            .iter()
            .all(|f| matches!(f.value.node, crate::ast::Expr::Literal(_)));
        if all_constant {
            return;
        }

        let bytes = self.type_size(&Type::Named(struct_name.to_string())) as u16;
        if bytes == 0 {
            return;
        }

        let Some(f) = self.current_function.clone() else {
            return;
        };
        let at = self.array_cursor;
        self.array_cursor += bytes;
        self.struct_temps.insert(
            span,
            crate::sema::LocalArray {
                addr: at,
                size: bytes,
                function: f.clone(),
            },
        );
        let entry = self.array_block_sizes.entry(f).or_insert(0);
        *entry = (*entry).max(self.array_cursor);
    }

    /// Record, per callee, the bytes of binary-operand spill that are live
    /// across a call to it from `func_name`.
    ///
    /// `warn_deep_recursion` needs the true per-level cost of a recursive call,
    /// and the frame save is only part of it. When a binary operation's *right*
    /// operand contains a call, codegen cannot keep the left operand in a
    /// register or the zero-page pool across the `JSR`, so it spills it to the
    /// same 256-byte software stack the frame save uses (see
    /// `codegen::expr::binary`, `needs_spill`). Nested binaries stack their
    /// spills, so the cost along a path is their sum.
    ///
    /// This is what made the old warning miss the plainest possible case:
    /// `return (n as u16) + s(n - 1)` saves a one-byte frame but *also* spills
    /// the two-byte left operand across the call, so each level costs three
    /// bytes and the real limit is 85, not 256.
    ///
    /// Keyed by callee because only calls that close a recursion cycle matter;
    /// the caller side is resolved once SCCs are known.
    pub(super) fn record_call_spills(&mut self, func_name: &str, body: &Spanned<Stmt>) {
        let mut spills: HashMap<String, u16> = HashMap::default();
        self.walk_stmt_spills(body, 0, &mut spills);
        if !spills.is_empty() {
            self.call_spill_bytes.insert(func_name.to_string(), spills);
        }
    }

    fn walk_stmt_spills(&self, stmt: &Spanned<Stmt>, carried: u16, out: &mut HashMap<String, u16>) {
        match &stmt.node {
            Stmt::Block(stmts) => {
                for s in stmts {
                    self.walk_stmt_spills(s, carried, out);
                }
            }
            Stmt::VarDecl { init, .. } => self.walk_expr_spills(init, carried, out),
            Stmt::Assign { target, value } => {
                self.walk_expr_spills(target, carried, out);
                self.walk_expr_spills(value, carried, out);
            }
            Stmt::Expr(e) => self.walk_expr_spills(e, carried, out),
            Stmt::Return(Some(e)) => self.walk_expr_spills(e, carried, out),
            Stmt::Return(None) | Stmt::Break | Stmt::Continue | Stmt::Asm { .. } => {}
            Stmt::If {
                condition,
                then_branch,
                else_branch,
            } => {
                self.walk_expr_spills(condition, carried, out);
                self.walk_stmt_spills(then_branch, carried, out);
                if let Some(e) = else_branch {
                    self.walk_stmt_spills(e, carried, out);
                }
            }
            Stmt::While { condition, body } => {
                self.walk_expr_spills(condition, carried, out);
                self.walk_stmt_spills(body, carried, out);
            }
            Stmt::Loop { body } => self.walk_stmt_spills(body, carried, out),
            Stmt::For { body, .. } => self.walk_stmt_spills(body, carried, out),
            Stmt::ForEach { iterable, body, .. } => {
                self.walk_expr_spills(iterable, carried, out);
                self.walk_stmt_spills(body, carried, out);
            }
            Stmt::Match { expr, arms } => {
                self.walk_expr_spills(expr, carried, out);
                for arm in arms {
                    self.walk_stmt_spills(&arm.body, carried, out);
                }
            }
        }
    }

    fn walk_expr_spills(&self, expr: &Spanned<Expr>, carried: u16, out: &mut HashMap<String, u16>) {
        match &expr.node {
            Expr::Binary { left, right, .. } => {
                // Codegen spills the left operand only when evaluating the right
                // one runs a call that would clobber it.
                let spill = if Self::expr_contains_call(right) {
                    self.spill_width(left)
                } else {
                    0
                };
                self.walk_expr_spills(left, carried, out);
                self.walk_expr_spills(right, carried + spill, out);
            }
            Expr::Call { function, args } => {
                let e = out.entry(function.node.clone()).or_insert(0);
                *e = (*e).max(carried);
                for a in args {
                    self.walk_expr_spills(a, carried, out);
                }
            }
            Expr::CallIndirect { callee, args } => {
                self.walk_expr_spills(callee, carried, out);
                for a in args {
                    self.walk_expr_spills(a, carried, out);
                }
            }
            Expr::Unary { operand, .. } => self.walk_expr_spills(operand, carried, out),
            Expr::Cast { expr: inner, .. }
            | Expr::Paren(inner)
            | Expr::SliceLen(inner)
            | Expr::U16Low(inner)
            | Expr::U16High(inner) => self.walk_expr_spills(inner, carried, out),
            Expr::Field { object, .. } => self.walk_expr_spills(object, carried, out),
            Expr::Index { object, index } => {
                self.walk_expr_spills(object, carried, out);
                self.walk_expr_spills(index, carried, out);
            }
            Expr::Slice {
                object, start, end, ..
            } => {
                self.walk_expr_spills(object, carried, out);
                self.walk_expr_spills(start, carried, out);
                self.walk_expr_spills(end, carried, out);
            }
            Expr::BitOp { object, bit, .. } => {
                self.walk_expr_spills(object, carried, out);
                self.walk_expr_spills(bit, carried, out);
            }
            Expr::StructInit { fields, .. } | Expr::AnonStructInit { fields } => {
                for f in fields {
                    self.walk_expr_spills(&f.value, carried, out);
                }
            }
            Expr::EnumVariant { data, .. } => match data {
                crate::ast::VariantData::Unit => {}
                crate::ast::VariantData::Tuple(elems) => {
                    for d in elems {
                        self.walk_expr_spills(d, carried, out);
                    }
                }
                crate::ast::VariantData::Struct(fields) => {
                    for f in fields {
                        self.walk_expr_spills(&f.value, carried, out);
                    }
                }
            },
            Expr::Match { expr: inner, arms } => {
                self.walk_expr_spills(inner, carried, out);
                for arm in arms {
                    self.walk_expr_spills(&arm.body, carried, out);
                }
            }
            Expr::Literal(crate::ast::Literal::Array(elems)) => {
                for e in elems {
                    self.walk_expr_spills(e, carried, out);
                }
            }
            _ => {}
        }
    }

    /// Bytes codegen spills for a left operand: two for a 16-bit value, one
    /// otherwise (`codegen::expr::binary` picks the same way).
    fn spill_width(&self, left: &Spanned<Expr>) -> u16 {
        match self.resolved_types.get(&left.span) {
            Some(t) if self.type_size(t) >= 2 => 2,
            _ => 1,
        }
    }

    /// Whether evaluating `expr` can run a `JSR`.
    ///
    /// Feeds the recursion-depth cost model: an operand live across a call is
    /// spilled to the same 256-byte software stack the frame saves use, so
    /// missing one under-counts what a recursion level costs and the warning
    /// fires too late. Exhaustive for that reason — a new `Expr` variant has to
    /// be classified rather than defaulting to "no call".
    fn expr_contains_call(expr: &Spanned<Expr>) -> bool {
        match &expr.node {
            Expr::Call { .. } | Expr::CallIndirect { .. } => true,
            Expr::Binary { left, right, .. } => {
                Self::expr_contains_call(left) || Self::expr_contains_call(right)
            }
            Expr::Unary { operand, .. } => Self::expr_contains_call(operand),
            Expr::Cast { expr: i, .. }
            | Expr::Paren(i)
            | Expr::SliceLen(i)
            | Expr::U16Low(i)
            | Expr::U16High(i) => Self::expr_contains_call(i),
            Expr::Field { object, .. } => Self::expr_contains_call(object),
            Expr::Index { object, index } => {
                Self::expr_contains_call(object) || Self::expr_contains_call(index)
            }
            Expr::BitOp { object, bit, .. } => {
                Self::expr_contains_call(object) || Self::expr_contains_call(bit)
            }
            Expr::StructInit { fields, .. } | Expr::AnonStructInit { fields } => {
                fields.iter().any(|f| Self::expr_contains_call(&f.value))
            }
            Expr::EnumVariant { data, .. } => match data {
                crate::ast::VariantData::Unit => false,
                crate::ast::VariantData::Tuple(elems) => elems.iter().any(Self::expr_contains_call),
                crate::ast::VariantData::Struct(fields) => {
                    fields.iter().any(|f| Self::expr_contains_call(&f.value))
                }
            },
            Expr::Match { expr: i, arms } => {
                Self::expr_contains_call(i)
                    || arms.iter().any(|a| Self::expr_contains_call(&a.body))
            }
            Expr::Literal(crate::ast::Literal::Array(elems)) => {
                elems.iter().any(Self::expr_contains_call)
            }
            Expr::Literal(crate::ast::Literal::ArrayFill { value, .. }) => {
                Self::expr_contains_call(value)
            }
            Expr::Slice {
                object, start, end, ..
            } => {
                Self::expr_contains_call(object)
                    || Self::expr_contains_call(start)
                    || Self::expr_contains_call(end)
            }
            // Genuinely call-free.
            Expr::Literal(_)
            | Expr::Variable(_)
            | Expr::CpuFlagCarry
            | Expr::CpuFlagZero
            | Expr::CpuFlagOverflow
            | Expr::CpuFlagNegative => false,
        }
    }

    /// Note that the function being analysed dispatches through a function
    /// pointer, so its frame has to stay clear of every possible target.
    fn note_indirect_call(&mut self) {
        if let Some(caller) = &self.current_function {
            self.indirect_callers.insert(caller.clone());
        }
    }

    /// Every named function called anywhere inside `expr`, including through
    /// nested argument lists. Used to record the extra frame-interference edges
    /// a nested call creates; an indirect call has no name to record and its
    /// arguments are sheltered across the nested call instead, so it
    /// contributes nothing here.
    fn collect_called_names(expr: &Expr, out: &mut Vec<String>) {
        let walk =
            |e: &Spanned<Expr>, out: &mut Vec<String>| Self::collect_called_names(&e.node, out);
        match expr {
            Expr::Call { function, args } => {
                out.push(function.node.clone());
                for a in args {
                    walk(a, out);
                }
            }
            Expr::CallIndirect { callee, args } => {
                walk(callee, out);
                for a in args {
                    walk(a, out);
                }
            }
            Expr::Binary { left, right, .. } => {
                walk(left, out);
                walk(right, out);
            }
            Expr::Unary { operand, .. } => walk(operand, out),
            Expr::Cast { expr: i, .. }
            | Expr::Paren(i)
            | Expr::SliceLen(i)
            | Expr::U16Low(i)
            | Expr::U16High(i) => walk(i, out),
            Expr::Field { object, .. } => walk(object, out),
            Expr::Index { object, index } => {
                walk(object, out);
                walk(index, out);
            }
            Expr::Slice {
                object, start, end, ..
            } => {
                walk(object, out);
                walk(start, out);
                walk(end, out);
            }
            Expr::BitOp { object, bit, .. } => {
                walk(object, out);
                walk(bit, out);
            }
            Expr::StructInit { fields, .. } | Expr::AnonStructInit { fields } => {
                for f in fields {
                    walk(&f.value, out);
                }
            }
            Expr::EnumVariant { data, .. } => match data {
                crate::ast::VariantData::Unit => {}
                crate::ast::VariantData::Tuple(elems) => {
                    for e in elems {
                        walk(e, out);
                    }
                }
                crate::ast::VariantData::Struct(fields) => {
                    for f in fields {
                        walk(&f.value, out);
                    }
                }
            },
            Expr::Match { expr: i, arms } => {
                walk(i, out);
                for a in arms {
                    walk(&a.body, out);
                }
            }
            Expr::Literal(crate::ast::Literal::Array(elems)) => {
                for e in elems {
                    walk(e, out);
                }
            }
            Expr::Literal(_)
            | Expr::Variable(_)
            | Expr::CpuFlagCarry
            | Expr::CpuFlagZero
            | Expr::CpuFlagOverflow
            | Expr::CpuFlagNegative => {}
        }
    }

    fn check_struct_init_fields(
        &mut self,
        struct_name: &str,
        fields: &[crate::ast::FieldInit],
    ) -> Result<(), SemaError> {
        let def = self
            .type_registry
            .structs
            .get(struct_name)
            .expect("checked by the caller")
            .clone();

        for field in fields {
            let field_info =
                def.get_field(&field.name.node)
                    .ok_or_else(|| SemaError::FieldNotFound {
                        struct_name: struct_name.to_string(),
                        field_name: field.name.node.clone(),
                        span: field.name.span,
                    })?;

            let saved = self.expected_type.take();
            self.expected_type = Some(field_info.ty.clone());
            let value_ty = self.check_expr(&field.value);
            self.expected_type = saved;
            let value_ty = value_ty?;

            if !value_ty.is_implicitly_convertible_to(&field_info.ty) {
                return Err(SemaError::TypeMismatch {
                    expected: field_info.ty.display_name(),
                    found: value_ty.display_name(),
                    span: field.value.span,
                });
            }
        }
        Ok(())
    }

    fn check_enum_variant(
        &mut self,
        enum_name: &Spanned<String>,
        variant: &Spanned<String>,
        data: &crate::ast::VariantData,
        span: crate::ast::Span,
    ) -> Result<Type, SemaError> {
        // Look up the enum definition
        let enum_def = self
            .type_registry
            .get_enum(&enum_name.node)
            .ok_or_else(|| SemaError::UndefinedSymbol {
                suggestion: self.table.closest_name(&enum_name.node),
                name: enum_name.node.clone(),
                span: enum_name.span,
            })?;

        // Verify the variant exists
        let variant_info =
            enum_def
                .get_variant(&variant.node)
                .ok_or_else(|| SemaError::Custom {
                    message: format!(
                        "variant '{}' not found in enum '{}'",
                        variant.node, enum_name.node
                    ),
                    span: variant.span,
                })?;

        // Type check the variant data
        use crate::ast::VariantData;
        use crate::sema::type_defs::VariantData as TypeDefVariantData;

        match (&variant_info.data, data) {
            (TypeDefVariantData::Unit, VariantData::Unit) => {
                // Unit variant - ok
            }
            (TypeDefVariantData::Tuple(field_types), VariantData::Tuple(values)) => {
                // Type check each tuple field
                if values.len() != field_types.len() {
                    return Err(SemaError::Custom {
                        message: format!(
                            "variant '{}' expects {} fields, got {}",
                            variant.node,
                            field_types.len(),
                            values.len()
                        ),
                        span,
                    });
                }

                // Clone field types to avoid borrowing issues
                let expected_types = field_types.clone();
                for (value_expr, expected_ty) in values.iter().zip(expected_types.iter()) {
                    // The payload type is the value's expected type, so a
                    // literal adopts it: `E::V(5)` for `V(u16)` works the way
                    // `f(5)` for `f(x: u16)` does.
                    let saved = self.expected_type.take();
                    self.expected_type = Some(expected_ty.clone());
                    let value_ty = self.check_expr(value_expr);
                    self.expected_type = saved;
                    let value_ty = value_ty?;

                    if !value_ty.is_implicitly_convertible_to(expected_ty) {
                        return Err(SemaError::TypeMismatch {
                            expected: expected_ty.display_name(),
                            found: value_ty.display_name(),
                            span: value_expr.span,
                        });
                    }
                }
            }
            (TypeDefVariantData::Struct(field_infos), VariantData::Struct(field_inits)) => {
                // Clone field infos to avoid borrowing issues
                let field_info_vec = field_infos.clone();

                // Type check struct variant fields
                for field_init in field_inits {
                    // Find the expected type for this field
                    let field_info = field_info_vec
                        .iter()
                        .find(|f| f.name == field_init.name.node)
                        .ok_or_else(|| SemaError::FieldNotFound {
                            struct_name: enum_name.node.clone(),
                            field_name: field_init.name.node.clone(),
                            span: field_init.name.span,
                        })?;

                    let saved = self.expected_type.take();
                    self.expected_type = Some(field_info.ty.clone());
                    let value_ty = self.check_expr(&field_init.value);
                    self.expected_type = saved;
                    let value_ty = value_ty?;

                    if !value_ty.is_implicitly_convertible_to(&field_info.ty) {
                        return Err(SemaError::TypeMismatch {
                            expected: field_info.ty.display_name(),
                            found: value_ty.display_name(),
                            span: field_init.value.span,
                        });
                    }
                }
            }
            _ => {
                return Err(SemaError::Custom {
                    message: format!("variant data mismatch for '{}'", variant.node),
                    span,
                });
            }
        }

        // Return the enum type
        Ok(Type::Named(enum_name.node.clone()))
    }

    /// Re-decide a `.len`/`.low`/`.high` access by the object's type.
    ///
    /// The parser emits the built-in accessor node (`SliceLen`, `U16Low`,
    /// `U16High`) for those three names before types are known, which would
    /// make a struct field with one of those names unreachable. If the object
    /// is a struct (or, like `check_field_access`, a pointer to one) that
    /// actually has a field by that name, this is a field access: record the
    /// expression's span in `accessor_fields` so codegen emits one, and
    /// return the field's type. Otherwise return None and the caller falls
    /// through to the built-in meaning.
    fn accessor_field_type(
        &mut self,
        object_ty: &Type,
        name: &str,
        span: crate::ast::Span,
    ) -> Option<Type> {
        let named = match object_ty {
            Type::Named(n) => n,
            Type::Pointer(inner) => match &**inner {
                Type::Named(n) => n,
                _ => return None,
            },
            _ => return None,
        };
        let field = self.type_registry.get_struct(named)?.get_field(name)?;
        self.accessor_fields.insert(span);
        Some(field.ty.clone())
    }

    /// The column layout of `expr`, if it names an array declared `#[soa]`.
    ///
    /// Cloned rather than borrowed so the caller can go on to check
    /// subexpressions; the layout is a name and a length.
    pub(super) fn soa_layout_of(&self, expr: &Spanned<Expr>) -> Option<crate::sema::SoaLayout> {
        match &expr.node {
            Expr::Variable(n) => self.soa_arrays.get(n).cloned(),
            Expr::Paren(inner) => self.soa_layout_of(inner),
            _ => None,
        }
    }

    /// Refuse a use of an SoA element that would need it to be contiguous.
    ///
    /// This is the cost of the layout, stated where it is paid. Every legal
    /// use — `arr[i].field`, read or written — is consumed by
    /// [`Self::check_field_access`] before the index node is ever checked on
    /// its own, so reaching the index arm at all *is* the error: a binding, a
    /// `&`, an argument, a return, a whole-element assignment. One rule with
    /// one implementation, rather than a list of forbidden shapes that a new
    /// syntax could quietly slip past.
    fn refuse_soa_element(
        &self,
        array: &Spanned<Expr>,
        span: crate::ast::Span,
    ) -> Option<SemaError> {
        let layout = self.soa_layout_of(array)?;
        let name = match &array.node {
            Expr::Variable(n) => n.clone(),
            _ => "this array".to_string(),
        };
        Some(SemaError::Custom {
            message: format!(
                "an element of `{name}` has no address of its own: `#[soa]` stores the array \
                 as one column per field, so no `{}` is in one piece. Reach the \
                 data a field at a time — `{name}[i].{}` — or remove `#[soa]` to store whole \
                 records",
                layout.elem,
                self.type_registry
                    .get_struct(&layout.elem)
                    .and_then(|d| d.fields.first())
                    .map(|f| f.name.clone())
                    .unwrap_or_else(|| "field".to_string()),
            ),
            span,
        })
    }

    fn check_field_access(
        &mut self,
        object: &Spanned<Expr>,
        field: &Spanned<String>,
    ) -> Result<Type, SemaError> {
        // `arr[i].f` — the shape that makes columns worth suggesting. Counted
        // for every array, not only the marked ones: an unmarked array whose
        // every mention is this shape is what the suggestion looks for.
        if let Expr::Index { object: array, .. } = &object.node
            && let Expr::Variable(n) = &array.node
        {
            *self.indexed_field_reads.entry(n.clone()).or_insert(0) += 1;
        }

        // `arr[i].f` on an SoA array is a column entry. It is checked here, as
        // one step, because the element it would otherwise be composed through
        // does not exist — and checking the index node on its own is exactly
        // what [`Self::refuse_soa_element`] rejects.
        if let Expr::Index {
            object: array,
            index,
        } = &object.node
            && let Some(layout) = self.soa_layout_of(array)
        {
            self.check_expr(array)?;
            let saved = self.expected_type.take();
            let index_ty = self.check_expr(index);
            self.expected_type = saved;
            let index_ty = index_ty?;
            if !matches!(
                index_ty,
                Type::Primitive(PrimitiveType::U8 | PrimitiveType::I8)
            ) {
                return Err(SemaError::TypeMismatch {
                    expected: "u8".to_string(),
                    found: index_ty.display_name(),
                    span: index.span,
                });
            }
            let sdef =
                self.type_registry
                    .get_struct(&layout.elem)
                    .ok_or_else(|| SemaError::Custom {
                        message: format!("struct '{}' not found", layout.elem),
                        span: object.span,
                    })?;
            return sdef
                .get_field(&field.node)
                .map(|f| f.ty.clone())
                .ok_or_else(|| SemaError::FieldNotFound {
                    struct_name: layout.elem.clone(),
                    field_name: field.node.clone(),
                    span: field.span,
                });
        }

        // Get the type of the object
        let object_ty = self.check_expr(object)?;

        // One level of pointer is looked through, so `p.field` means
        // `(*p).field`. A struct *parameter* is already a pointer under the
        // hood and `s.field` works on it, so making `&Struct` behave
        // differently would be gratuitous. Only one level: a pointer to a
        // pointer to a struct has to be dereferenced explicitly.
        let object_ty = match &object_ty {
            Type::Pointer(inner) if matches!(**inner, Type::Named(_)) => (**inner).clone(),
            _ => object_ty,
        };

        // Extract struct name from the type
        let struct_name = match &object_ty {
            Type::Named(name) => name,
            _ => {
                return Err(SemaError::TypeMismatch {
                    expected: "struct".to_string(),
                    found: object_ty.display_name(),
                    span: object.span,
                });
            }
        };

        // Look up the struct definition
        let struct_def =
            self.type_registry
                .get_struct(struct_name)
                .ok_or_else(|| SemaError::Custom {
                    message: format!("struct '{}' not found", struct_name),
                    span: object.span,
                })?;

        // Find the field and return its type
        let field_info =
            struct_def
                .get_field(&field.node)
                .ok_or_else(|| SemaError::FieldNotFound {
                    struct_name: struct_name.clone(),
                    field_name: field.node.clone(),
                    span: field.span,
                })?;

        Ok(field_info.ty.clone())
    }

    fn check_index(
        &mut self,
        object: &Spanned<Expr>,
        index: &Spanned<Expr>,
        _span: crate::ast::Span,
    ) -> Result<Type, SemaError> {
        // The index is a byte offset (u8/i8) and the object is an aggregate —
        // neither should inherit the *element* type the surrounding context
        // expects. Without clearing it, `let r: u16 = arr[3]` pushes u16 onto
        // the literal `3`, which then fails the u8/i8 index gate below. (A
        // variable index keeps its own declared type, so only constant indices
        // hit this.)
        let saved_expected = self.expected_type.take();

        if let Some(e) = self.refuse_soa_element(object, _span) {
            return Err(e);
        }

        // Type check the index expression (should be integer)
        let index_ty = self.check_expr(index)?;
        if !matches!(
            index_ty,
            Type::Primitive(PrimitiveType::U8 | PrimitiveType::I8)
        ) {
            // A 16-bit index is the common way to arrive here, because `.len`
            // is a `u16` and `for i in 0..s.len` therefore types `i` as one.
            // Indexed addressing goes through an 8-bit register, so the index
            // genuinely has to narrow — but say where the cast goes rather than
            // leaving the reader to work out which of two operands is wrong.
            if matches!(
                index_ty,
                Type::Primitive(PrimitiveType::U16 | PrimitiveType::I16)
            ) {
                return Err(SemaError::Custom {
                    message: format!(
                        "index must be `u8` or `i8`, found `{}`\n  = help: indexed \
                         addressing uses an 8-bit register, so a 16-bit index has to \
                         narrow — write `[i as u8]`\n  = note: `.len` is a `u16`, so \
                         `for i in 0..s.len` gives `i` that type; binding the bound \
                         first (`let n: u8 = s.len as u8;`) types the loop variable \
                         `u8` instead",
                        index_ty.display_name()
                    ),
                    span: index.span,
                });
            }
            return Err(SemaError::TypeMismatch {
                expected: "u8 or i8".to_string(),
                found: index_ty.display_name(),
                span: index.span,
            });
        }

        // Type check the object being indexed
        let object_ty = self.check_expr(object)?;
        self.expected_type = saved_expected;

        // Extract element type from array or string type
        match &object_ty {
            Type::Array(element_ty, array_size) => {
                // COMPILE-TIME BOUNDS CHECK
                // Try to evaluate index as a constant expression
                if let Ok(const_val) = eval_const_expr_with_env(index, &self.const_env)
                    && let Some(index_value) = const_val.as_integer()
                {
                    // Check for negative indices (only possible with i8)
                    if index_value < 0 {
                        return Err(SemaError::ArrayIndexOutOfBounds {
                            index: index_value,
                            array_size: *array_size,
                            span: index.span,
                        });
                    }

                    // Check if index >= array_size
                    let index_usize = index_value as usize;
                    if index_usize >= *array_size {
                        return Err(SemaError::ArrayIndexOutOfBounds {
                            index: index_value,
                            array_size: *array_size,
                            span: index.span,
                        });
                    }
                    // Index is valid at compile-time
                }
                // If evaluation fails or not an integer, index is not constant - skip check

                // Return the element type
                Ok((**element_ty).clone())
            }
            Type::Slice(element_ty) => {
                // Slice indexing returns the element type. Length is a runtime
                // value, so no compile-time bounds check.
                Ok((**element_ty).clone())
            }
            Type::Pointer(element_ty) => {
                // `p[i]` is the i-th element from the pointer, scaled by the
                // element width. A pointer carries no length, so there is
                // nothing to bounds-check against; that is what slices are for.
                Ok((**element_ty).clone())
            }
            Type::String => {
                // For a `str<N>` buffer we know the capacity, so a constant
                // index past it is a compile-time error (a plain `str` carries
                // no capacity in its type, so it is left unchecked as before).
                if let Expr::Variable(name) = &object.node
                    && let Some(cap) = self
                        .table
                        .lookup(name)
                        .and_then(|sym| sym.decl_span)
                        .and_then(|ds| self.string_buffers.get(&ds))
                        .map(|buf| buf.size.saturating_sub(1) as usize)
                    && let Ok(const_val) = eval_const_expr_with_env(index, &self.const_env)
                    && let Some(index_value) = const_val.as_integer()
                    && (index_value < 0 || index_value as usize >= cap)
                {
                    return Err(SemaError::ArrayIndexOutOfBounds {
                        index: index_value,
                        array_size: cap,
                        span: index.span,
                    });
                }
                // A string is semantically an array of chars, so indexing
                // yields a `char`. Cast with `as u8` for the raw byte.
                Ok(Type::Primitive(PrimitiveType::Char))
            }
            _ => Err(SemaError::TypeMismatch {
                expected: "array, slice, pointer, or string".to_string(),
                found: object_ty.display_name(),
                span: object.span,
            }),
        }
    }

    /// `x.bit(n)` / `x.set_bit(n)` / `x.clear_bit(n)` / `x.toggle_bit(n)`. The
    /// bit index is a compile-time constant bounded by the value's width; a read
    /// yields a `bool`, a mutation writes back and yields nothing.
    fn check_bitop(
        &mut self,
        object: &Spanned<Expr>,
        kind: crate::ast::BitOpKind,
        bit: &Spanned<Expr>,
        span: crate::ast::Span,
    ) -> Result<Type, SemaError> {
        let obj_ty = self.check_expr(object)?;
        let width: i64 = match &obj_ty {
            Type::Primitive(
                PrimitiveType::U8
                | PrimitiveType::I8
                | PrimitiveType::Bool
                | PrimitiveType::Char
                | PrimitiveType::B8
                | PrimitiveType::Addr,
            ) => 8,
            Type::Primitive(PrimitiveType::U16 | PrimitiveType::I16 | PrimitiveType::B16) => 16,
            _ => {
                return Err(SemaError::TypeMismatch {
                    expected: "an integer".to_string(),
                    found: obj_ty.display_name(),
                    span: object.span,
                });
            }
        };

        // A runtime bit index has no single-instruction lowering; require a
        // constant and point at the std functions for the dynamic case.
        self.check_expr(bit)?;
        let Some(n) = eval_const_expr_with_env(bit, &self.const_env)
            .ok()
            .and_then(|v| v.as_integer())
        else {
            return Err(SemaError::Custom {
                message: "bit index must be a compile-time constant; use std/math.wr's \
                          set_bit/clear_bit/test_bit for a runtime index"
                    .to_string(),
                span: bit.span,
            });
        };
        if !(0..width).contains(&n) {
            return Err(SemaError::Custom {
                message: format!(
                    "bit index {} is out of range for a {}-bit value (valid: 0..{})",
                    n, width, width
                ),
                span: bit.span,
            });
        }

        if !kind.is_mutation() {
            return Ok(Type::Primitive(PrimitiveType::Bool));
        }

        // A mutation writes back, so the target must be assignable. When the
        // chain is rooted at a named lvalue (a variable, static, or addr
        // register), that root is what has to be mutable. `lvalue_root` returns
        // None where the chain passes through a pointer or a by-reference
        // parameter — a mutation through a pointer is always to mutable memory
        // (`&` is rejected on a `const` or an `addr`), so it needs no root check
        // and codegen reaches it with an indirect read-modify-write.
        let Some(root) = self.lvalue_root(object).cloned() else {
            return Ok(Type::Void);
        };
        if let Some(info) = self.table.lookup(&root) {
            if info.kind == SymbolKind::Constant {
                return Err(SemaError::Custom {
                    message: format!(
                        "cannot modify a bit of '{}': a const lives in ROM, so the write \
                         would do nothing on real hardware",
                        root
                    ),
                    span,
                });
            }
            if info.kind == SymbolKind::Address
                && matches!(info.access_mode, Some(crate::ast::AccessMode::Read))
            {
                return Err(SemaError::ReadOnlyWrite { name: root, span });
            }
            if !info.mutable {
                return Err(SemaError::ImmutableAssignment { symbol: root, span });
            }
        }
        Ok(Type::Void)
    }

    fn check_slice(
        &mut self,
        object: &Spanned<Expr>,
        start: &Spanned<Expr>,
        end: &Spanned<Expr>,
        inclusive: bool,
        span: crate::ast::Span,
    ) -> Result<Type, SemaError> {
        // A slice is a base and a length over *contiguous* elements, which is
        // exactly what an SoA array does not have. Refused here as well as at
        // the index arm, because a slice reaches its elements without ever
        // forming an index node for one.
        if let Some(e) = self.refuse_soa_element(object, span) {
            return Err(e);
        }

        // Slice bounds are integers. u8/i8 cover the common case; u16/i16 are
        // accepted so slices longer than 255 elements can be formed (e.g. with
        // constant bounds). Runtime u16 bounds are rejected later in codegen.
        let is_int_bound = |t: &Type| {
            matches!(
                t,
                Type::Primitive(
                    PrimitiveType::U8 | PrimitiveType::I8 | PrimitiveType::U16 | PrimitiveType::I16
                )
            )
        };
        let start_ty = self.check_expr(start)?;
        if !is_int_bound(&start_ty) {
            return Err(SemaError::TypeMismatch {
                expected: "integer index".to_string(),
                found: start_ty.display_name(),
                span: start.span,
            });
        }
        let end_ty = self.check_expr(end)?;
        if !is_int_bound(&end_ty) {
            return Err(SemaError::TypeMismatch {
                expected: "integer index".to_string(),
                found: end_ty.display_name(),
                span: end.span,
            });
        }

        // Type check the object being sliced
        let object_ty = self.check_expr(object)?;

        match &object_ty {
            Type::Array(_element_ty, array_size) => {
                // COMPILE-TIME BOUNDS CHECK
                // Try to evaluate both bounds as constant expressions
                if let (Ok(start_val), Ok(end_val)) = (
                    eval_const_expr_with_env(start, &self.const_env),
                    eval_const_expr_with_env(end, &self.const_env),
                ) && let (Some(s), Some(e)) = (start_val.as_integer(), end_val.as_integer())
                {
                    let actual_end = if inclusive { e + 1 } else { e };

                    // Check for negative indices
                    if s < 0 {
                        return Err(SemaError::ArrayIndexOutOfBounds {
                            index: s,
                            array_size: *array_size,
                            span: start.span,
                        });
                    }

                    // Check if start > end
                    if s > actual_end {
                        return Err(SemaError::Custom {
                            message: format!(
                                "slice start ({}) is greater than end ({})",
                                s, actual_end
                            ),
                            span,
                        });
                    }

                    // Check if end exceeds array size
                    if actual_end as usize > *array_size {
                        return Err(SemaError::ArrayIndexOutOfBounds {
                            index: actual_end - 1,
                            array_size: *array_size,
                            span: end.span,
                        });
                    }
                }

                // As an assignment target (`arr[a..b] = [...]`) the slice keeps
                // the array type so length/element checks line up. As a value
                // (`let s: &[u8] = arr[a..b]`) it is a slice of the element type.
                if self.checking_assignment_target {
                    Ok(object_ty.clone())
                } else {
                    Ok(Type::Slice(_element_ty.clone()))
                }
            }
            Type::String => {
                // COMPILE-TIME STRING SLICING
                // For strings, slices must be compile-time evaluable
                // Extract substring and create a new string constant

                // Try to evaluate the string and bounds as constants
                let string_val = eval_const_expr_with_env(object, &self.const_env)
                    .ok()
                    .and_then(|v| match v {
                        crate::sema::const_eval::ConstValue::String(s) => Some(s),
                        _ => None,
                    });

                let start_val = eval_const_expr_with_env(start, &self.const_env)
                    .ok()
                    .and_then(|v| v.as_integer());

                let end_val = eval_const_expr_with_env(end, &self.const_env)
                    .ok()
                    .and_then(|v| v.as_integer());

                match (string_val, start_val, end_val) {
                    (Some(s), Some(start_idx), Some(end_idx)) => {
                        let actual_end = if inclusive { end_idx + 1 } else { end_idx };

                        // Validate bounds
                        if start_idx < 0 {
                            return Err(SemaError::Custom {
                                message: format!(
                                    "string slice start cannot be negative: {}",
                                    start_idx
                                ),
                                span: start.span,
                            });
                        }

                        if start_idx > actual_end {
                            return Err(SemaError::Custom {
                                message: format!(
                                    "string slice start ({}) is greater than end ({})",
                                    start_idx, actual_end
                                ),
                                span,
                            });
                        }

                        let start_usize = start_idx as usize;
                        let end_usize = actual_end as usize;

                        if end_usize > s.len() {
                            return Err(SemaError::Custom {
                                message: format!(
                                    "string slice end ({}) exceeds string length ({})",
                                    actual_end,
                                    s.len()
                                ),
                                span: end.span,
                            });
                        }

                        // Empty slice check
                        if start_usize == end_usize {
                            return Err(SemaError::Custom {
                                message: "string slice cannot be empty".to_string(),
                                span,
                            });
                        }

                        // Bounds are byte offsets, but the value is held as a
                        // Rust String: slicing inside a multi-byte character
                        // would panic on the non-boundary index. Reject it.
                        if !s.is_char_boundary(start_usize) || !s.is_char_boundary(end_usize) {
                            return Err(SemaError::Custom {
                                message: format!(
                                    "string slice {}..{} falls inside a multi-byte character; \
                                     slice at character boundaries",
                                    start_idx, actual_end
                                ),
                                span,
                            });
                        }

                        // Extract substring
                        let result = &s[start_usize..end_usize];

                        // Validate 255-byte limit
                        if result.len() > 255 {
                            return Err(SemaError::Custom {
                                message: format!(
                                    "string slice result exceeds 255 byte limit: {} bytes",
                                    result.len()
                                ),
                                span,
                            });
                        }

                        // Store the folded constant
                        self.folded_constants.insert(
                            span,
                            crate::sema::const_eval::ConstValue::String(result.to_string()),
                        );

                        Ok(Type::String)
                    }
                    _ => {
                        // Cannot evaluate at compile time
                        Err(SemaError::Custom {
                            message: "string slices must use constant expressions".to_string(),
                            span,
                        })
                    }
                }
            }
            Type::Slice(element_ty) => {
                // Slicing a slice yields another slice of the same element type.
                // The length is a runtime value, so bounds are unchecked. As an
                // assignment target this form is not supported.
                if self.checking_assignment_target {
                    return Err(SemaError::Custom {
                        message: "cannot assign through a slice-of-slice".to_string(),
                        span,
                    });
                }
                Ok(Type::Slice(element_ty.clone()))
            }
            _ => Err(SemaError::TypeMismatch {
                expected: "array, slice, or string".to_string(),
                found: object_ty.display_name(),
                span: object.span,
            }),
        }
    }
}
