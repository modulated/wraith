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
        // The byte accessors fold through their operand: `.low`/`.high` of a
        // constant u16 are the constant's bytes (without these, `C.low` on a
        // `const C: u16` never folded and codegen read the Absolute(0)
        // sentinel — $0000 — for it).
        Expr::U16Low(operand) => match eval_const_expr_with_env(operand, env)? {
            ConstValue::Integer(v) => Ok(ConstValue::Integer(v & 0xFF)),
            _ => Err(SemaError::Custom {
                message: ".low requires a constant integer".to_string(),
                span: expr.span,
            }),
        },
        Expr::U16High(operand) => match eval_const_expr_with_env(operand, env)? {
            ConstValue::Integer(v) => Ok(ConstValue::Integer((v >> 8) & 0xFF)),
            _ => Err(SemaError::Custom {
                message: ".high requires a constant integer".to_string(),
                span: expr.span,
            }),
        },
        Expr::SliceLen(operand) => match eval_const_expr_with_env(operand, env)? {
            ConstValue::String(s) => Ok(ConstValue::Integer(s.len() as i64)),
            _ => Err(SemaError::Custom {
                message: "only a constant string's length folds at compile time".to_string(),
                span: expr.span,
            }),
        },
        Expr::Cast {
            expr: inner,
            target_type,
        } => {
            // Evaluate the inner expression
            let value = eval_const_expr_with_env(inner, env)?;

            // Perform type conversion based on target type
            apply_type_cast(value, target_type, expr.span)
        }
        Expr::Slice {
            object,
            start,
            end,
            inclusive,
        } => {
            // Evaluate slice on string constants at compile time
            let object_val = eval_const_expr_with_env(object, env)?;
            let start_val = eval_const_expr_with_env(start, env)?;
            let end_val = eval_const_expr_with_env(end, env)?;

            match (object_val, start_val, end_val) {
                (
                    ConstValue::String(s),
                    ConstValue::Integer(start_idx),
                    ConstValue::Integer(end_idx),
                ) => {
                    let actual_end = if *inclusive { end_idx + 1 } else { end_idx };

                    // Validate bounds
                    if start_idx < 0 {
                        return Err(SemaError::Custom {
                            message: format!("slice start cannot be negative: {}", start_idx),
                            span: start.span,
                        });
                    }

                    if start_idx > actual_end {
                        return Err(SemaError::Custom {
                            message: format!(
                                "slice start ({}) is greater than end ({})",
                                start_idx, actual_end
                            ),
                            span: expr.span,
                        });
                    }

                    let start_usize = start_idx as usize;
                    let end_usize = actual_end as usize;

                    if end_usize > s.len() {
                        return Err(SemaError::Custom {
                            message: format!(
                                "slice end ({}) exceeds string length ({})",
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
                            span: expr.span,
                        });
                    }

                    // Bounds are byte offsets, but the value is held as a Rust
                    // String: slicing inside a multi-byte character would panic
                    // on the non-boundary index (and couldn't be represented at
                    // all). Reject it rather than crash.
                    if !s.is_char_boundary(start_usize) || !s.is_char_boundary(end_usize) {
                        return Err(SemaError::Custom {
                            message: format!(
                                "string slice {}..{} falls inside a multi-byte character; \
                                 slice at character boundaries",
                                start_idx, actual_end
                            ),
                            span: expr.span,
                        });
                    }

                    // Extract substring
                    let result = s[start_usize..end_usize].to_string();

                    // Validate 255-byte limit
                    if result.len() > 255 {
                        return Err(SemaError::Custom {
                            message: format!(
                                "string slice result exceeds 255 byte limit: {} bytes",
                                result.len()
                            ),
                            span: expr.span,
                        });
                    }

                    Ok(ConstValue::String(result))
                }
                _ => Err(SemaError::Custom {
                    message: "slice operations are only supported on strings with constant bounds"
                        .to_string(),
                    span: expr.span,
                }),
            }
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
        Literal::Char(c) => Ok(ConstValue::Integer(i64::from(*c))),
        Literal::String(s) => Ok(ConstValue::String(s.clone())),
        _ => Err(SemaError::Custom {
            message: "literal cannot be evaluated as constant".to_string(),
            span: crate::ast::Span::dummy(),
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
    } else if let (ConstValue::String(l), ConstValue::String(r)) = (&left_val, &right_val) {
        // String concatenation: "hello" + "world"
        match op {
            BinaryOp::Add => {
                let result = format!("{}{}", l, r);
                // Validate 255-byte limit
                if result.len() > 255 {
                    return Err(SemaError::Custom {
                        message: format!(
                            "string concatenation exceeds 255 byte limit: {} bytes",
                            result.len()
                        ),
                        span,
                    });
                }
                Ok(ConstValue::String(result))
            }
            _ => Err(SemaError::Custom {
                message: format!(
                    "cannot apply '{:?}' operator to strings (only '+' is supported)",
                    op
                ),
                span,
            }),
        }
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
        // An address is a link-time property, not a constant expression, and a
        // dereference is a runtime read. Folding either would be worse than
        // useless: `check_expr` folds *before* dispatching, so a folded `&x`
        // would replace the address with whatever the operand happened to
        // evaluate to.
        UnaryOp::AddrOf | UnaryOp::Deref => Err(SemaError::Custom {
            message: "pointer operations are not constant expressions".to_string(),
            span,
        }),
        UnaryOp::Neg => {
            if let Some(n) = val.as_integer() {
                // `n.checked_neg()` already yields -n; a leading `-` here would
                // double-negate and fold `-5` back to `5`.
                Ok(ConstValue::Integer(n.checked_neg().ok_or_else(|| {
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

    match (value.as_integer(), target_prim) {
        (Some(n), PrimitiveType::B8) if decimal_to_bcd(n, 2).is_none() => Err(SemaError::Custom {
            message: format!(
                "value {} is out of range for BCD type b8 (valid range: 0-99)",
                n
            ),
            span,
        }),
        (Some(n), PrimitiveType::B16) if decimal_to_bcd(n, 4).is_none() => Err(SemaError::Custom {
            message: format!(
                "value {} is out of range for BCD type b16 (valid range: 0-9999)",
                n
            ),
            span,
        }),
        _ => Ok(()),
    }
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
            PrimitiveType::Char => {
                // Truncate to an 8-bit ASCII byte (unchecked, like every other
                // narrowing cast in the language).
                if let Some(n) = value.as_integer() {
                    Ok(ConstValue::Integer(i64::from(n as u8)))
                } else {
                    Err(SemaError::Custom {
                        message: "cannot cast to char".to_string(),
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
        // A pointer is a 16-bit address, so `0xD012 as &u8` folds to the number
        // itself. This is how a fixed hardware location is named without an
        // `addr` declaration, and it is what lets a pointer `static` carry a
        // real initial value rather than silently starting at $0000.
        TypeExpr::Pointer { .. } => {
            if let Some(n) = value.as_integer() {
                Ok(ConstValue::Integer((n as u16) as i64))
            } else {
                Err(SemaError::Custom {
                    message: "only an integer can be cast to a pointer".to_string(),
                    span,
                })
            }
        }
        _ => Err(SemaError::Custom {
            message: "unsupported type cast in constant expression".to_string(),
            span,
        }),
    }
}

/// Fold `expr` the way the *generated code* would compute it: wrapping at
/// `bits` after every operation, rather than in full precision with one
/// truncation at the end.
///
/// The difference is observable. `(94 << 6) >> 3` on a `u8` wraps the shift to
/// 128 and yields 16; evaluated in `i64` it is 6016 >> 3 = 752, which truncates
/// to 240. The same expression written with a variable runs at u8 width and
/// gives 16, so folding it to 240 made a constant disagree with the identical
/// runtime computation. Multiplication has the same shape: `(200 * 2) / 4` is
/// 36 at u8 width and 100 in full precision.
///
/// Comparisons and non-integer results are left to the ordinary evaluator; only
/// integer arithmetic needs narrowing. A `Cast` deliberately changes width, so
/// it is evaluated by the ordinary path and then narrowed to the *outer* width
/// like any other leaf.
pub fn eval_const_expr_wrapping(
    expr: &Spanned<Expr>,
    env: &ConstEnv,
    bits: u32,
    signed: bool,
) -> Result<ConstValue, SemaError> {
    let narrowed = |v: i64| ConstValue::Integer(narrow(v, bits, signed));

    match &expr.node {
        Expr::Paren(inner) => eval_const_expr_wrapping(inner, env, bits, signed),

        Expr::Binary { left, op, right } => {
            // Only arithmetic narrows; a comparison yields a bool, and the
            // logical operators take bools.
            if !matches!(
                op,
                BinaryOp::Add
                    | BinaryOp::Sub
                    | BinaryOp::Mul
                    | BinaryOp::Div
                    | BinaryOp::Mod
                    | BinaryOp::BitAnd
                    | BinaryOp::BitOr
                    | BinaryOp::BitXor
                    | BinaryOp::Shl
                    | BinaryOp::Shr
            ) {
                return eval_const_expr_with_env(expr, env);
            }

            let l = eval_const_expr_wrapping(left, env, bits, signed)?;
            let r = eval_const_expr_wrapping(right, env, bits, signed)?;
            let (Some(a), Some(b)) = (l.as_integer(), r.as_integer()) else {
                return eval_const_expr_with_env(expr, env);
            };

            let v = match op {
                BinaryOp::Add => a.wrapping_add(b),
                BinaryOp::Sub => a.wrapping_sub(b),
                BinaryOp::Mul => a.wrapping_mul(b),
                BinaryOp::Div => {
                    if b == 0 {
                        return Err(SemaError::Custom {
                            message: "division by zero in constant expression".to_string(),
                            span: expr.span,
                        });
                    }
                    a.wrapping_div(b)
                }
                BinaryOp::Mod => {
                    if b == 0 {
                        return Err(SemaError::Custom {
                            message: "modulo by zero in constant expression".to_string(),
                            span: expr.span,
                        });
                    }
                    a.wrapping_rem(b)
                }
                BinaryOp::BitAnd => a & b,
                BinaryOp::BitOr => a | b,
                BinaryOp::BitXor => a ^ b,
                // A shift of the width or more clears the value, which is what
                // the emitted code does; `checked_shl` would instead give None
                // and abandon the fold.
                BinaryOp::Shl => {
                    if !(0..i64::from(bits)).contains(&b) {
                        0
                    } else {
                        a.wrapping_shl(b as u32)
                    }
                }
                BinaryOp::Shr => {
                    if !(0..i64::from(bits)).contains(&b) {
                        0
                    } else {
                        // Narrow first so an unsigned shift does not drag down
                        // sign bits that the value does not have at this width.
                        narrow(a, bits, signed).wrapping_shr(b as u32)
                    }
                }
                _ => unreachable!("filtered above"),
            };
            Ok(narrowed(v))
        }

        // Everything else is a leaf as far as width is concerned: evaluate it
        // normally and narrow the result to this expression's width.
        _ => match eval_const_expr_with_env(expr, env)? {
            ConstValue::Integer(v) => Ok(narrowed(v)),
            other => Ok(other),
        },
    }
}

/// Truncate `v` to `bits`, sign-extending when the type is signed.
fn narrow(v: i64, bits: u32, signed: bool) -> i64 {
    if bits == 0 || bits >= 64 {
        return v;
    }
    let mask = (1i64 << bits) - 1;
    let t = v & mask;
    if signed && (t >> (bits - 1)) & 1 == 1 {
        t - (1i64 << bits)
    } else {
        t
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::Span;

    fn make_int(n: i64) -> Spanned<Expr> {
        Spanned {
            node: Expr::Literal(Literal::Integer(n)),
            span: Span::dummy(),
        }
    }

    fn make_binary(left: Spanned<Expr>, op: BinaryOp, right: Spanned<Expr>) -> Spanned<Expr> {
        Spanned {
            node: Expr::Binary {
                left: Box::new(left),
                op,
                right: Box::new(right),
            },
            span: Span::dummy(),
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
