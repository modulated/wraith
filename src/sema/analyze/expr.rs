//! Expression Type Checking
//!
//! Type checking for all expression variants in the AST.

use crate::ast::{BinaryOp, Expr, PrimitiveType, Spanned};
use crate::sema::const_eval::eval_const_expr_with_env;
use crate::sema::table::SymbolKind;
use crate::sema::types::Type;
use crate::sema::SemaError;

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
        if !contains_addr_ref
            && let Ok(const_val) = eval_const_expr_with_env(expr, &self.const_env)
        {
            self.folded_constants.insert(expr.span, const_val);
        }

        let result_ty = match &expr.node {
            Expr::Literal(lit) => self.check_literal(lit, expr.span)?,

            Expr::Variable(name) => self.check_variable(name, expr)?,

            Expr::Binary { left, op, right } => self.check_binary(left, op, right, expr.span)?,

            Expr::Call { function, args } => self.check_call(function, args, expr.span)?,

            Expr::Unary { op, operand } => self.check_unary(op, operand, expr.span)?,

            Expr::Paren(inner) => self.check_expr(inner)?,

            Expr::Cast { expr: inner, target_type } => {
                // Check that the inner expression is valid
                self.check_expr(inner)?;
                // Return the target type
                self.resolve_type(&target_type.node)?
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

            Expr::AnonStructInit { fields } => {
                self.check_anon_struct_init(fields, expr.span)?
            }

            Expr::EnumVariant { enum_name, variant, data } => {
                self.check_enum_variant(enum_name, variant, data, expr.span)?
            }

            Expr::Field { object, field } => {
                self.check_field_access(object, field)?
            }

            Expr::Index { object, index } => {
                self.check_index(object, index, expr.span)?
            }

            Expr::Slice { object, start, end, inclusive } => {
                self.check_slice(object, start, end, *inclusive, expr.span)?
            }

            Expr::SliceLen(slice_expr) => {
                // Verify the expression is actually a slice, array, or string
                let slice_ty = self.check_expr(slice_expr)?;

                // Check if it's a type that has a length
                match &slice_ty {
                    Type::Pointer(..) | Type::Array(_, _) | Type::String => {
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
                    Type::Primitive(PrimitiveType::U16)
                    | Type::Primitive(PrimitiveType::I16) => {
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
                    Type::Primitive(PrimitiveType::U16)
                    | Type::Primitive(PrimitiveType::I16) => {
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
            Expr::CpuFlagCarry | Expr::CpuFlagZero | Expr::CpuFlagOverflow | Expr::CpuFlagNegative => {
                Type::Primitive(PrimitiveType::Bool)
            }
        };

        // Store the resolved type for this expression so codegen can access it
        self.resolved_types.insert(expr.span, result_ty.clone());

        Ok(result_ty)
    }

    fn check_literal(&mut self, lit: &crate::ast::Literal, span: crate::ast::Span) -> Result<Type, SemaError> {
        match lit {
            crate::ast::Literal::Integer(val) => {
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
                            message: format!("integer literal {} is too large (max 65535 for u16)", val),
                            span,
                        })
                    }
                }
            }
            crate::ast::Literal::Bool(_) => {
                Ok(Type::Primitive(PrimitiveType::Bool))
            }
            crate::ast::Literal::String(_) => {
                Ok(Type::String)
            }
            crate::ast::Literal::Array(elements) => {
                if elements.is_empty() {
                    // Empty array - need type context to determine element type
                    // For now, default to [u8; 0]
                    return Ok(Type::Array(
                        Box::new(Type::Primitive(PrimitiveType::U8)),
                        0,
                    ));
                }

                // Infer element type from first element
                let element_ty = self.check_expr(&elements[0])?;

                // Check that all elements have the same type
                for elem in &elements[1..] {
                    let elem_ty = self.check_expr(elem)?;
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
                let element_ty = self.check_expr(value)?;
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

        // Mark variable as used (for unused variable/parameter warnings)
        self.used_variables.insert(name.to_string());
        // Also track in all_used_symbols (for unused import warnings)
        self.all_used_symbols.insert(name.to_string());

        Ok(info.ty)
    }

    fn check_binary(
        &mut self,
        left: &Spanned<Expr>,
        op: &BinaryOp,
        right: &Spanned<Expr>,
        span: crate::ast::Span,
    ) -> Result<Type, SemaError> {
        let left_ty = self.check_expr(left)?;
        let right_ty = self.check_expr(right)?;

        // Special handling for pointer arithmetic
        match (&left_ty, op, &right_ty) {
            // Pointer + Integer or Integer + Pointer
            (Type::Pointer(..), BinaryOp::Add, Type::Primitive(_))
            | (Type::Primitive(_), BinaryOp::Add, Type::Pointer(..)) => {
                // Return the pointer type
                if matches!(left_ty, Type::Pointer(..)) {
                    return Ok(left_ty);
                } else {
                    return Ok(right_ty);
                }
            }
            // Pointer - Integer
            (Type::Pointer(..), BinaryOp::Sub, Type::Primitive(_)) => {
                return Ok(left_ty);
            }
            // Pointer - Pointer (returns offset as integer)
            (Type::Pointer(..), BinaryOp::Sub, Type::Pointer(..)) => {
                return Ok(Type::Primitive(PrimitiveType::U16));
            }
            _ => {}
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
                BinaryOp::Add | BinaryOp::Sub => {}  // Hardware supported
                BinaryOp::Eq | BinaryOp::Ne | BinaryOp::Lt |
                BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge => {}  // Comparisons work

                BinaryOp::Mul | BinaryOp::Div | BinaryOp::Mod => {
                    return Err(SemaError::InvalidBinaryOp {
                        op: format!("{:?} (not supported on BCD, convert to binary first)", op),
                        left_ty: left_ty.display_name(),
                        right_ty: right_ty.display_name(),
                        span,
                    });
                }

                BinaryOp::BitAnd | BinaryOp::BitOr | BinaryOp::BitXor |
                BinaryOp::Shl | BinaryOp::Shr => {
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
            // Allow same-type shifts (u8 >> u8, u16 >> u16, etc.)
            // Or allow larger type to be shifted by u8 (u16 >> u8)
            left_ty == right_ty
                || (matches!(left_ty, Type::Primitive(PrimitiveType::U16))
                    && matches!(right_ty, Type::Primitive(PrimitiveType::U8)))
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
            BinaryOp::Eq | BinaryOp::Ne | BinaryOp::Lt | BinaryOp::Le
            | BinaryOp::Gt | BinaryOp::Ge | BinaryOp::And | BinaryOp::Or => {
                Ok(Type::Primitive(PrimitiveType::Bool))
            }
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
        for (arg, param_ty) in args.iter().zip(param_types.iter()) {
            let arg_ty = self.check_expr(arg)?;
            // Check if argument type can be implicitly converted to parameter type
            if !arg_ty.is_implicitly_convertible_to(param_ty) {
                return Err(SemaError::TypeMismatch {
                    expected: param_ty.display_name(),
                    found: arg_ty.display_name(),
                    span: arg.span,
                });
            }
        }
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
                // Negation works on numeric types
                if operand_ty.is_primitive() {
                    Ok(operand_ty)
                } else {
                    Err(SemaError::InvalidUnaryOp {
                        op: "-".to_string(),
                        operand_ty: operand_ty.display_name(),
                        span,
                    })
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
            _ => Ok(operand_ty), // For other operators, preserve type
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
        let enum_def = self.type_registry.get_enum(&enum_name.node)
            .ok_or_else(|| SemaError::UndefinedSymbol {
                name: enum_name.node.clone(),
                span: enum_name.span,
            })?;

        // Verify the variant exists
        let variant_info = enum_def.get_variant(&variant.node)
            .ok_or_else(|| SemaError::Custom {
                message: format!("variant '{}' not found in enum '{}'", variant.node, enum_name.node),
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
                    let field_info = field_info_vec.iter()
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
        let struct_def = self.type_registry.get_struct(struct_name)
            .ok_or_else(|| SemaError::Custom {
                message: format!("struct '{}' not found", struct_name),
                span: object.span,
            })?;

        // Find the field and return its type
        let field_info = struct_def.get_field(&field.node)
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
        if !matches!(index_ty, Type::Primitive(PrimitiveType::U8 | PrimitiveType::I8)) {
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
                    && let Some(index_value) = const_val.as_integer() {
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
            Type::String => {
                // String indexing returns u8 (a single byte)
                Ok(Type::Primitive(PrimitiveType::U8))
            }
            _ => {
                Err(SemaError::TypeMismatch {
                    expected: "array or string".to_string(),
                    found: object_ty.display_name(),
                    span: object.span,
                })
            }
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
        // Type check start bound (must be u8)
        let start_ty = self.check_expr(start)?;
        if !matches!(start_ty, Type::Primitive(PrimitiveType::U8 | PrimitiveType::I8)) {
            return Err(SemaError::TypeMismatch {
                expected: "u8 or i8".to_string(),
                found: start_ty.display_name(),
                span: start.span,
            });
        }

        // Type check end bound (must be u8)
        let end_ty = self.check_expr(end)?;
        if !matches!(end_ty, Type::Primitive(PrimitiveType::U8 | PrimitiveType::I8)) {
            return Err(SemaError::TypeMismatch {
                expected: "u8 or i8".to_string(),
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
                    eval_const_expr_with_env(end, &self.const_env)
                )
                    && let (Some(s), Some(e)) = (start_val.as_integer(), end_val.as_integer()) {
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
                                message: format!("slice start ({}) is greater than end ({})", s, actual_end),
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

                // Slices return the same array type (for assignment compatibility)
                Ok(object_ty.clone())
            }
            _ => {
                Err(SemaError::TypeMismatch {
                    expected: "array".to_string(),
                    found: object_ty.display_name(),
                    span: object.span,
                })
            }
        }
    }
}
