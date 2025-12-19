//! Constant Expression Evaluation
//!
//! Evaluates constant expressions at compile time for optimization.

use crate::ast::{BinaryOp, Expr, Literal, Spanned, UnaryOp};
use crate::sema::SemaError;

/// Result of constant evaluation
#[derive(Debug, Clone, PartialEq)]
pub enum ConstValue {
    Integer(i64),
    Bool(bool),
    String(String),
}

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
    match &expr.node {
        Expr::Literal(lit) => eval_literal(lit),
        Expr::Binary { left, op, right } => eval_binary(left, *op, right, expr.span),
        Expr::Unary { op, operand } => eval_unary(*op, operand, expr.span),
        Expr::Paren(inner) => eval_const_expr(inner),
        Expr::Cast { expr: inner, .. } => {
            // For now, just evaluate the inner expression
            // TODO: Handle actual type conversions
            eval_const_expr(inner)
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

fn eval_binary(
    left: &Spanned<Expr>,
    op: BinaryOp,
    right: &Spanned<Expr>,
    span: crate::ast::Span,
) -> Result<ConstValue, SemaError> {
    let left_val = eval_const_expr(left)?;
    let right_val = eval_const_expr(right)?;

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

fn eval_unary(
    op: UnaryOp,
    operand: &Spanned<Expr>,
    span: crate::ast::Span,
) -> Result<ConstValue, SemaError> {
    let val = eval_const_expr(operand)?;

    match op {
        UnaryOp::Neg => {
            if let Some(n) = val.as_integer() {
                Ok(ConstValue::Integer(
                    -n.checked_neg().ok_or_else(|| SemaError::Custom {
                        message: "negation overflow in constant expression".to_string(),
                        span,
                    })?,
                ))
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
        _ => Err(SemaError::Custom {
            message: "unsupported unary operation in constant expression".to_string(),
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
