//! Comment Generation Utilities
//!
//! Helper functions for generating contextual assembly comments

use crate::ast::*;
use crate::sema::types::Type;

/// Describe what's in a register after an operation
pub fn describe_register_content(operation: &str) -> String {
    match operation {
        "result" => "operation result".to_string(),
        "pointer_low" => "pointer low byte".to_string(),
        "pointer_high" => "pointer high byte".to_string(),
        "index" => "array/string index".to_string(),
        _ => operation.to_string(),
    }
}

/// Describe a condition for branching
pub fn describe_condition(expr: &Spanned<Expr>) -> String {
    match &expr.node {
        Expr::Binary { op, left, right } => {
            let left_str = simple_expr_name(left);
            let right_str = simple_expr_name(right);
            match op {
                BinaryOp::Eq => format!("{} == {}", left_str, right_str),
                BinaryOp::Ne => format!("{} != {}", left_str, right_str),
                BinaryOp::Lt => format!("{} < {}", left_str, right_str),
                BinaryOp::Le => format!("{} <= {}", left_str, right_str),
                BinaryOp::Gt => format!("{} > {}", left_str, right_str),
                BinaryOp::Ge => format!("{} >= {}", left_str, right_str),
                _ => "condition".to_string(),
            }
        }
        Expr::Unary {
            op: UnaryOp::Not, ..
        } => "not condition".to_string(),
        _ => "condition".to_string(),
    }
}

/// Get simple name for expression (for comments)
fn simple_expr_name(expr: &Spanned<Expr>) -> String {
    match &expr.node {
        Expr::Variable(name) => name.clone(),
        Expr::Literal(lit) => match lit {
            Literal::Integer(n) => n.to_string(),
            Literal::Bool(b) => b.to_string(),
            _ => "value".to_string(),
        },
        _ => "value".to_string(),
    }
}

/// Format register state comment
pub fn format_register_state(a: &str, x: &str, y: &str) -> String {
    format!("A={}, X={}, Y={}", a, x, y)
}

/// Describe a type cast operation
pub fn describe_cast(from_type: &Type, to_type: &Type) -> String {
    use crate::ast::PrimitiveType;

    match (from_type, to_type) {
        (Type::Primitive(PrimitiveType::U16), Type::Primitive(PrimitiveType::U8)) => {
            "Extract low byte (discard high byte)".to_string()
        }
        (Type::Primitive(PrimitiveType::U8), Type::Primitive(PrimitiveType::U16)) => {
            "Zero-extend u8 to u16 (high byte = 0)".to_string()
        }
        _ => format!("Cast from {:?} to {:?}", from_type, to_type),
    }
}

/// Describe a shift operation with its mathematical meaning
pub fn describe_shift(direction: &str, count: u8) -> String {
    match direction {
        "left" => {
            if count == 8 {
                "Shift left 8 bits (multiply by 256, move to high byte)".to_string()
            } else {
                format!("Shift left {} bits (multiply by {})", count, 1u16 << count)
            }
        }
        "right" => {
            if count == 8 {
                "Shift right 8 bits (divide by 256, extract high byte)".to_string()
            } else {
                format!("Shift right {} bits (divide by {})", count, 1u16 << count)
            }
        }
        _ => format!("Shift {} {} bits", direction, count),
    }
}

/// Describe a binary operation
pub fn describe_binary_op(op: &BinaryOp) -> String {
    match op {
        BinaryOp::Add => "Addition".to_string(),
        BinaryOp::Sub => "Subtraction".to_string(),
        BinaryOp::Mul => "Multiplication".to_string(),
        BinaryOp::Div => "Division".to_string(),
        BinaryOp::Mod => "Modulo".to_string(),
        BinaryOp::BitAnd => "Bitwise AND".to_string(),
        BinaryOp::BitOr => "Bitwise OR".to_string(),
        BinaryOp::BitXor => "Bitwise XOR".to_string(),
        BinaryOp::Shl => "Shift left".to_string(),
        BinaryOp::Shr => "Shift right".to_string(),
        BinaryOp::Eq => "Equality test".to_string(),
        BinaryOp::Ne => "Inequality test".to_string(),
        BinaryOp::Lt => "Less than test".to_string(),
        BinaryOp::Le => "Less than or equal test".to_string(),
        BinaryOp::Gt => "Greater than test".to_string(),
        BinaryOp::Ge => "Greater than or equal test".to_string(),
        BinaryOp::And => "Logical AND".to_string(),
        BinaryOp::Or => "Logical OR".to_string(),
    }
}
