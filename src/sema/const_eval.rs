//! Constant Expression Evaluation
//!
//! Evaluates constant expressions at compile time for optimization.

use crate::ast::{BinaryOp, Expr, Literal, Spanned, UnaryOp};
use crate::sema::SemaError;
use rustc_hash::FxHashMap as HashMap;

/// Result of constant evaluation
#[derive(Debug, Clone, PartialEq)]
pub enum ConstValue {
    Integer(i64),
    Bool(bool),
    String(String),
}

/// Environment for constant evaluation (maps names to constant values)
pub type ConstEnv = HashMap<String, ConstValue>;

impl ConstValue {
    pub fn as_integer(&self) -> Option<i64> {
        match self {
            ConstValue::Integer(n) => Some(*n),
            ConstValue::Bool(b) => Some(if *b { 1 } else { 0 }),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            ConstValue::Bool(b) => Some(*b),
            ConstValue::Integer(n) => Some(*n != 0),
            _ => None,
        }
    }

    pub fn to_literal(&self) -> Literal {
        match self {
            ConstValue::Integer(n) => Literal::Integer(*n),
            ConstValue::Bool(b) => Literal::Bool(*b),
            ConstValue::String(s) => Literal::String(s.clone()),
        }
    }
}

/// Evaluates a constant expression at compile time
pub fn eval_const_expr(expr: &Spanned<Expr>) -> Result<ConstValue, SemaError> {
    eval_const_expr_with_env(expr, &ConstEnv::default())
}

/// Evaluates a constant expression with an environment of named constants
pub fn eval_const_expr_with_env(
    expr: &Spanned<Expr>,
    env: &ConstEnv,
) -> Result<ConstValue, SemaError> {
    match &expr.node {
        Expr::Literal(lit) => eval_literal(lit),
        Expr::Variable(name) => env.get(name).cloned().ok_or_else(|| SemaError::Custom {
            message: format!("constant '{}' not found in this scope", name),
            span: expr.span,
        }),
        Expr::Binary { left, op, right } => eval_binary_with_env(left, *op, right, expr.span, env),
        Expr::Unary { op, operand } => eval_unary_with_env(*op, operand, expr.span, env),
        Expr::Paren(inner) => eval_const_expr_with_env(inner, env),
        Expr::Cast {
            expr: inner,
            target_type,
        } => {
            // Evaluate the inner expression
            let value = eval_const_expr_with_env(inner, env)?;

            // Perform type conversion based on target type
            apply_type_cast(value, target_type, expr.span)
        }
        _ => Err(SemaError::Custom {
            message: "expression is not constant".to_string(),
            span: expr.span,
        }),
    }
}

fn eval_literal(lit: &Literal) -> Result<ConstValue, SemaError> {
    match lit {
        Literal::Integer(n) => Ok(ConstValue::Integer(*n)),
        Literal::Bool(b) => Ok(ConstValue::Bool(*b)),
        Literal::String(s) => Ok(ConstValue::String(s.clone())),
        _ => Err(SemaError::Custom {
            message: "literal cannot be evaluated as constant".to_string(),
            span: crate::ast::Span { start: 0, end: 0 },
        }),
    }
}

fn eval_binary_with_env(
    left: &Spanned<Expr>,
    op: BinaryOp,
    right: &Spanned<Expr>,
    span: crate::ast::Span,
    env: &ConstEnv,
) -> Result<ConstValue, SemaError> {
    let left_val = eval_const_expr_with_env(left, env)?;
    let right_val = eval_const_expr_with_env(right, env)?;

    // Try integer operations first
    if let (Some(l), Some(r)) = (left_val.as_integer(), right_val.as_integer()) {
        let result = match op {
            BinaryOp::Add => l.checked_add(r),
            BinaryOp::Sub => l.checked_sub(r),
            BinaryOp::Mul => l.checked_mul(r),
            BinaryOp::Div => {
                if r == 0 {
                    return Err(SemaError::Custom {
                        message: "division by zero in constant expression".to_string(),
                        span,
                    });
                }
                l.checked_div(r)
            }
            BinaryOp::Mod => {
                if r == 0 {
                    return Err(SemaError::Custom {
                        message: "modulo by zero in constant expression".to_string(),
                        span,
                    });
                }
                l.checked_rem(r)
            }
            BinaryOp::BitAnd => Some(l & r),
            BinaryOp::BitOr => Some(l | r),
            BinaryOp::BitXor => Some(l ^ r),
            BinaryOp::Shl => {
                if !(0..=63).contains(&r) {
                    return Err(SemaError::Custom {
                        message: "shift amount out of range in constant expression".to_string(),
                        span,
                    });
                }
                l.checked_shl(r as u32)
            }
            BinaryOp::Shr => {
                if !(0..=63).contains(&r) {
                    return Err(SemaError::Custom {
                        message: "shift amount out of range in constant expression".to_string(),
                        span,
                    });
                }
                l.checked_shr(r as u32)
            }
            // Comparison operators return bool
            BinaryOp::Eq => return Ok(ConstValue::Bool(l == r)),
            BinaryOp::Ne => return Ok(ConstValue::Bool(l != r)),
            BinaryOp::Lt => return Ok(ConstValue::Bool(l < r)),
            BinaryOp::Le => return Ok(ConstValue::Bool(l <= r)),
            BinaryOp::Gt => return Ok(ConstValue::Bool(l > r)),
            BinaryOp::Ge => return Ok(ConstValue::Bool(l >= r)),
            // Logical operators need bool operands
            BinaryOp::And | BinaryOp::Or => {
                return eval_logical_binary(left_val, op, right_val, span);
            }
        };

        if let Some(val) = result {
            Ok(ConstValue::Integer(val))
        } else {
            Err(SemaError::Custom {
                message: "arithmetic overflow in constant expression".to_string(),
                span,
            })
        }
    } else if matches!(op, BinaryOp::And | BinaryOp::Or) {
        eval_logical_binary(left_val, op, right_val, span)
    } else {
        Err(SemaError::Custom {
            message: "cannot evaluate binary operation on non-integer constants".to_string(),
            span,
        })
    }
}

fn eval_logical_binary(
    left: ConstValue,
    op: BinaryOp,
    right: ConstValue,
    span: crate::ast::Span,
) -> Result<ConstValue, SemaError> {
    let l = left.as_bool().ok_or_else(|| SemaError::Custom {
        message: "logical operation requires boolean operands".to_string(),
        span,
    })?;
    let r = right.as_bool().ok_or_else(|| SemaError::Custom {
        message: "logical operation requires boolean operands".to_string(),
        span,
    })?;

    let result = match op {
        BinaryOp::And => l && r,
        BinaryOp::Or => l || r,
        _ => unreachable!(),
    };

    Ok(ConstValue::Bool(result))
}

fn eval_unary_with_env(
    op: UnaryOp,
    operand: &Spanned<Expr>,
    span: crate::ast::Span,
    env: &ConstEnv,
) -> Result<ConstValue, SemaError> {
    let val = eval_const_expr_with_env(operand, env)?;

    match op {
        UnaryOp::Neg => {
            if let Some(n) = val.as_integer() {
                Ok(ConstValue::Integer(-n.checked_neg().ok_or_else(|| {
                    SemaError::Custom {
                        message: "negation overflow in constant expression".to_string(),
                        span,
                    }
                })?))
            } else {
                Err(SemaError::Custom {
                    message: "cannot negate non-integer constant".to_string(),
                    span,
                })
            }
        }
        UnaryOp::BitNot => {
            if let Some(n) = val.as_integer() {
                Ok(ConstValue::Integer(!n))
            } else {
                Err(SemaError::Custom {
                    message: "cannot apply bitwise NOT to non-integer constant".to_string(),
                    span,
                })
            }
        }
        UnaryOp::Not => {
            if let Some(b) = val.as_bool() {
                Ok(ConstValue::Bool(!b))
            } else {
                Err(SemaError::Custom {
                    message: "cannot apply logical NOT to non-boolean constant".to_string(),
                    span,
                })
            }
        }
    }
}

/// Convert decimal integer to BCD (Binary Coded Decimal)
/// Each nibble represents a decimal digit 0-9
fn decimal_to_bcd(decimal: i64, max_digits: usize) -> Option<i64> {
    if decimal < 0 {
        return None; // BCD is unsigned
    }

    let mut result = 0i64;
    let mut value = decimal;
    let max_value = 10i64.pow(max_digits as u32) - 1;

    if value > max_value {
        return None; // Value too large for BCD range
    }

    for digit_pos in 0..max_digits {
        let digit = value % 10;
        if digit > 9 {
            return None; // Invalid digit
        }
        result |= digit << (digit_pos * 4);
        value /= 10;
    }

    Some(result)
}

/// Validate that a value can be safely cast to a BCD type
pub fn validate_bcd_cast(
    value: ConstValue,
    target_prim: &crate::ast::PrimitiveType,
    span: crate::ast::Span,
) -> Result<(), SemaError> {
    use crate::ast::PrimitiveType;

    if let Some(n) = value.as_integer() {
        match target_prim {
            PrimitiveType::B8 => {
                if decimal_to_bcd(n, 2).is_none() {
                    return Err(SemaError::Custom {
                        message: format!(
                            "value {} is out of range for BCD type b8 (valid range: 0-99)",
                            n
                        ),
                        span,
                    });
                }
            }
            PrimitiveType::B16 => {
                if decimal_to_bcd(n, 4).is_none() {
                    return Err(SemaError::Custom {
                        message: format!(
                            "value {} is out of range for BCD type b16 (valid range: 0-9999)",
                            n
                        ),
                        span,
                    });
                }
            }
            _ => {}
        }
    }

    Ok(())
}

/// Apply type cast to a constant value
fn apply_type_cast(
    value: ConstValue,
    target_type: &Spanned<crate::ast::TypeExpr>,
    span: crate::ast::Span,
) -> Result<ConstValue, SemaError> {
    use crate::ast::{PrimitiveType, TypeExpr};

    match &target_type.node {
        TypeExpr::Primitive(prim) => match prim {
            PrimitiveType::Bool => {
                // Convert to boolean: 0 = false, non-zero = true
                if let Some(b) = value.as_bool() {
                    Ok(ConstValue::Bool(b))
                } else {
                    Err(SemaError::Custom {
                        message: "cannot cast to bool".to_string(),
                        span,
                    })
                }
            }
            PrimitiveType::U8 => {
                // Truncate to 8-bit unsigned
                if let Some(n) = value.as_integer() {
                    Ok(ConstValue::Integer((n as u8) as i64))
                } else {
                    Err(SemaError::Custom {
                        message: "cannot cast to u8".to_string(),
                        span,
                    })
                }
            }
            PrimitiveType::I8 => {
                // Truncate to 8-bit signed
                if let Some(n) = value.as_integer() {
                    Ok(ConstValue::Integer((n as i8) as i64))
                } else {
                    Err(SemaError::Custom {
                        message: "cannot cast to i8".to_string(),
                        span,
                    })
                }
            }
            PrimitiveType::U16 => {
                // Truncate/extend to 16-bit unsigned
                if let Some(n) = value.as_integer() {
                    Ok(ConstValue::Integer((n as u16) as i64))
                } else {
                    Err(SemaError::Custom {
                        message: "cannot cast to u16".to_string(),
                        span,
                    })
                }
            }
            PrimitiveType::I16 => {
                // Truncate/extend to 16-bit signed
                if let Some(n) = value.as_integer() {
                    Ok(ConstValue::Integer((n as i16) as i64))
                } else {
                    Err(SemaError::Custom {
                        message: "cannot cast to i16".to_string(),
                        span,
                    })
                }
            }
            PrimitiveType::B8 => {
                // BCD 8-bit: convert decimal to BCD format (0-99)
                if let Some(n) = value.as_integer() {
                    if let Some(bcd) = decimal_to_bcd(n, 2) {
                        Ok(ConstValue::Integer(bcd))
                    } else {
                        Err(SemaError::Custom {
                            message: format!("value {} out of range for b8 (0-99)", n),
                            span,
                        })
                    }
                } else {
                    Err(SemaError::Custom {
                        message: "cannot cast to b8".to_string(),
                        span,
                    })
                }
            }
            PrimitiveType::B16 => {
                // BCD 16-bit: convert decimal to BCD format (0-9999)
                if let Some(n) = value.as_integer() {
                    if let Some(bcd) = decimal_to_bcd(n, 4) {
                        Ok(ConstValue::Integer(bcd))
                    } else {
                        Err(SemaError::Custom {
                            message: format!("value {} out of range for b16 (0-9999)", n),
                            span,
                        })
                    }
                } else {
                    Err(SemaError::Custom {
                        message: "cannot cast to b16".to_string(),
                        span,
                    })
                }
            }
            PrimitiveType::Addr => {
                // Address type: treat as 16-bit unsigned
                if let Some(n) = value.as_integer() {
                    Ok(ConstValue::Integer((n as u16) as i64))
                } else {
                    Err(SemaError::Custom {
                        message: "cannot cast to addr".to_string(),
                        span,
                    })
                }
            }
        },
        _ => Err(SemaError::Custom {
            message: "unsupported type cast in constant expression".to_string(),
            span,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::Span;

    fn make_int(n: i64) -> Spanned<Expr> {
        Spanned {
            node: Expr::Literal(Literal::Integer(n)),
            span: Span { start: 0, end: 0 },
        }
    }

    fn make_binary(left: Spanned<Expr>, op: BinaryOp, right: Spanned<Expr>) -> Spanned<Expr> {
        Spanned {
            node: Expr::Binary {
                left: Box::new(left),
                op,
                right: Box::new(right),
            },
            span: Span { start: 0, end: 0 },
        }
    }

    #[test]
    fn test_simple_addition() {
        let expr = make_binary(make_int(2), BinaryOp::Add, make_int(3));
        let result = eval_const_expr(&expr).unwrap();
        assert_eq!(result, ConstValue::Integer(5));
    }

    #[test]
    fn test_multiplication() {
        let expr = make_binary(make_int(4), BinaryOp::Mul, make_int(5));
        let result = eval_const_expr(&expr).unwrap();
        assert_eq!(result, ConstValue::Integer(20));
    }

    #[test]
    fn test_nested_expression() {
        // (2 + 3) * 4 = 20
        let inner = make_binary(make_int(2), BinaryOp::Add, make_int(3));
        let expr = make_binary(inner, BinaryOp::Mul, make_int(4));
        let result = eval_const_expr(&expr).unwrap();
        assert_eq!(result, ConstValue::Integer(20));
    }

    #[test]
    fn test_bitwise_operations() {
        let expr = make_binary(make_int(0xF0), BinaryOp::BitAnd, make_int(0x0F));
        let result = eval_const_expr(&expr).unwrap();
        assert_eq!(result, ConstValue::Integer(0));

        let expr = make_binary(make_int(0xF0), BinaryOp::BitOr, make_int(0x0F));
        let result = eval_const_expr(&expr).unwrap();
        assert_eq!(result, ConstValue::Integer(0xFF));
    }

    #[test]
    fn test_division_by_zero() {
        let expr = make_binary(make_int(10), BinaryOp::Div, make_int(0));
        assert!(eval_const_expr(&expr).is_err());
    }
}
