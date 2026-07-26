//! Expression Type Checking
//!
//! Type checking for all expression variants in the AST.

use crate::ast::{BinaryOp, Expr, PrimitiveType, Spanned};
use crate::sema::SemaError;
use crate::sema::const_eval::eval_const_expr_with_env;
use crate::sema::table::SymbolKind;
use crate::sema::types::Type;

use super::SemanticAnalyzer;

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
                self.check_expr(inner)?;

                // Validate BCD casts for constant expressions
                let target_ty = self.resolve_type(&target_type.node)?;
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
                        name: name.node.clone(),
                        span: name.span,
                    });
                }

                // Type check each field value
                for field in fields {
                    self.check_expr(&field.value)?;
                }

                Type::Named(name.node.clone())
            }

            Expr::AnonStructInit { fields } => self.check_anon_struct_init(fields, expr.span)?,

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

            Expr::U16Low(operand) => {
                let operand_ty = self.check_expr(operand)?;
                match &operand_ty {
                    Type::Primitive(PrimitiveType::U16) | Type::Primitive(PrimitiveType::I16) => {
                        Type::Primitive(PrimitiveType::U8)
                    }
                    _ => {
                        return Err(SemaError::TypeMismatch {
                            expected: "u16 or i16".to_string(),
                            found: operand_ty.display_name(),
                            span: operand.span,
                        });
                    }
                }
            }

            Expr::U16High(operand) => {
                let operand_ty = self.check_expr(operand)?;
                match &operand_ty {
                    Type::Primitive(PrimitiveType::U16) | Type::Primitive(PrimitiveType::I16) => {
                        Type::Primitive(PrimitiveType::U8)
                    }
                    _ => {
                        return Err(SemaError::TypeMismatch {
                            expected: "u16 or i16".to_string(),
                            found: operand_ty.display_name(),
                            span: operand.span,
                        });
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
                let mut arm_types = Vec::new();
                for arm in arms {
                    self.table.enter_scope();
                    self.add_pattern_bindings(&arm.pattern.node, arm.pattern.span, &match_ty)?;
                    let arm_ty = self.check_expr(&arm.body)?;
                    self.table.exit_scope();
                    arm_types.push(arm_ty);
                }

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
        };

        // Store the resolved type for this expression so codegen can access it
        self.resolved_types.insert(expr.span, result_ty.clone());

        Ok(result_ty)
    }

    /// Find a common type for two arm/branch types: identical types unify to
    /// themselves; otherwise the narrower widens to the other if implicitly
    /// convertible (e.g. u8 + u16 -> u16). Returns None if incompatible.
    fn unify_types(a: &Type, b: &Type) -> Option<Type> {
        if a == b {
            Some(a.clone())
        } else if b.is_implicitly_convertible_to(a) {
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
            crate::ast::Literal::String(s) => {
                // Validate string length (256 byte limit for 6502)
                if s.len() > 255 {
                    return Err(SemaError::Custom {
                        message: format!(
                            "string literal exceeds 256 byte limit: {} bytes",
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

    fn check_variable(&mut self, name: &str, expr: &Spanned<Expr>) -> Result<Type, SemaError> {
        let info = if let Some(info) = self.table.lookup(name) {
            info.clone()
        } else {
            return Err(SemaError::UndefinedSymbol {
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

        Ok(info.ty)
    }

    /// Is this operand a bare integer literal for width-adaptation purposes?
    /// Accepts an integer literal, a unary-negated integer literal (`-5`), and
    /// either wrapped in parentheses.
    fn is_adaptable_int_literal(expr: &Expr) -> bool {
        use crate::ast::{Literal, UnaryOp};
        match expr {
            Expr::Literal(Literal::Integer(_)) => true,
            Expr::Unary {
                op: UnaryOp::Neg,
                operand,
            } => matches!(&operand.node, Expr::Literal(Literal::Integer(_))),
            Expr::Paren(inner) => Self::is_adaptable_int_literal(&inner.node),
            _ => false,
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
        } else {
            (self.check_expr(left)?, self.check_expr(right)?)
        };

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
        } else if let Some(caller) = &self.current_function {
            // Record a call-graph edge (caller -> callee) for frame coloring and
            // recursion detection. Only for real named functions.
            self.call_edges
                .entry(caller.clone())
                .or_default()
                .insert(function.node.clone());
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
            let arg_ty = self.check_expr(arg)?;
            // Check if argument type can be implicitly converted to parameter type
            if !arg_ty.is_implicitly_convertible_to(param_ty) {
                self.expected_type = saved_expected;
                return Err(SemaError::TypeMismatch {
                    expected: param_ty.display_name(),
                    found: arg_ty.display_name(),
                    span: arg.span,
                });
            }
        }
        self.expected_type = saved_expected;
        Ok(*ret_type)
    }

    fn check_unary(
        &mut self,
        op: &crate::ast::UnaryOp,
        operand: &Spanned<Expr>,
        span: crate::ast::Span,
    ) -> Result<Type, SemaError> {
        let operand_ty = self.check_expr(operand)?;

        // Check type compatibility with the operator
        match op {
            crate::ast::UnaryOp::Neg => {
                // Negation works on numeric types and always yields a signed
                // result: `-5` is i8, not u8. `5` on its own infers as u8, so
                // without this the operand type would leak through and code like
                // `let x: i8 = -5;` would fail to type-check.
                if !operand_ty.is_primitive() {
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
                    } else {
                        Ok(Type::Primitive(PrimitiveType::I16))
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
                name: struct_name.clone(),
                span,
            });
        }

        // Type check each field value
        for field in fields {
            self.check_expr(&field.value)?;
        }

        // Store the resolved struct name for codegen
        self.resolved_struct_names.insert(span, struct_name.clone());

        Ok(Type::Named(struct_name))
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
                    let value_ty = self.check_expr(value_expr)?;
                    if &value_ty != expected_ty {
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
                    let value_ty = self.check_expr(&field_init.value)?;

                    // Find the expected type for this field
                    let field_info = field_info_vec
                        .iter()
                        .find(|f| f.name == field_init.name.node)
                        .ok_or_else(|| SemaError::FieldNotFound {
                            struct_name: enum_name.node.clone(),
                            field_name: field_init.name.node.clone(),
                            span: field_init.name.span,
                        })?;

                    if value_ty != field_info.ty {
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

    fn check_field_access(
        &mut self,
        object: &Spanned<Expr>,
        field: &Spanned<String>,
    ) -> Result<Type, SemaError> {
        // Get the type of the object
        let object_ty = self.check_expr(object)?;

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
        // Type check the index expression (should be integer)
        let index_ty = self.check_expr(index)?;
        if !matches!(
            index_ty,
            Type::Primitive(PrimitiveType::U8 | PrimitiveType::I8)
        ) {
            return Err(SemaError::TypeMismatch {
                expected: "u8 or i8".to_string(),
                found: index_ty.display_name(),
                span: index.span,
            });
        }

        // Type check the object being indexed
        let object_ty = self.check_expr(object)?;

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
            Type::String => {
                // String indexing returns u8 (a single byte)
                Ok(Type::Primitive(PrimitiveType::U8))
            }
            _ => Err(SemaError::TypeMismatch {
                expected: "array, slice, or string".to_string(),
                found: object_ty.display_name(),
                span: object.span,
            }),
        }
    }

    fn check_slice(
        &mut self,
        object: &Spanned<Expr>,
        start: &Spanned<Expr>,
        end: &Spanned<Expr>,
        inclusive: bool,
        span: crate::ast::Span,
    ) -> Result<Type, SemaError> {
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

                        // Extract substring
                        let result = &s[start_usize..end_usize];

                        // Validate 256-byte limit
                        if result.len() > 255 {
                            return Err(SemaError::Custom {
                                message: format!(
                                    "string slice result exceeds 256 byte limit: {} bytes",
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
