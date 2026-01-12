//! Expression AST nodes for the Wraith language

use super::span::Spanned;
use super::types::TypeExpr;

/// Binary operators
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    // Arithmetic
    Add,
    Sub,
    Mul,
    Div,
    Mod,

    // Bitwise
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,

    // Comparison
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,

    // Logical
    And,
    Or,
}

/// Unary operators
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    /// Arithmetic negation: -x
    Neg,
    /// Bitwise NOT: ~x
    BitNot,
    /// Logical NOT: !x
    Not,
    /// Dereference: *ptr
    Deref,
    /// Address-of: &x
    AddrOf,
    /// Mutable address-of: &mut x
    AddrOfMut,
}

/// A literal value
#[derive(Debug, Clone, PartialEq)]
pub enum Literal {
    /// Integer literal (value stored as i64 to handle all sizes)
    Integer(i64),
    /// Boolean literal
    Bool(bool),
    /// String literal: "Hello, world!"
    /// Stored as length-prefixed byte array (u16 little-endian length + data)
    String(String),
    /// Array literal: [1, 2, 3, 4, 5]
    Array(Vec<Spanned<Expr>>),
    /// Array fill literal: [0; 10]
    ArrayFill {
        value: Box<Spanned<Expr>>,
        count: usize,
    },
}

/// Field initializer for struct construction
#[derive(Debug, Clone, PartialEq)]
pub struct FieldInit {
    pub name: Spanned<String>,
    pub value: Spanned<Expr>,
}

/// Enum variant construction data
#[derive(Debug, Clone, PartialEq)]
pub enum VariantData {
    /// Unit variant: Enum::Variant
    Unit,
    /// Tuple variant: Enum::Variant(a, b)
    Tuple(Vec<Spanned<Expr>>),
    /// Struct variant: Enum::Variant { x: 1, y: 2 }
    Struct(Vec<FieldInit>),
}

/// An expression in the language
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    /// A literal value
    Literal(Literal),

    /// Variable reference: x
    Variable(String),

    /// Binary operation: a + b
    Binary {
        left: Box<Spanned<Expr>>,
        op: BinaryOp,
        right: Box<Spanned<Expr>>,
    },

    /// Unary operation: -x, !flag, *ptr, &var
    Unary {
        op: UnaryOp,
        operand: Box<Spanned<Expr>>,
    },

    /// Type cast: x as u16
    Cast {
        expr: Box<Spanned<Expr>>,
        target_type: Spanned<TypeExpr>,
    },

    /// Field access: point.x
    Field {
        object: Box<Spanned<Expr>>,
        field: Spanned<String>,
    },

    /// Array/slice index: arr[i]
    Index {
        object: Box<Spanned<Expr>>,
        index: Box<Spanned<Expr>>,
    },

    /// Array slice: arr[start..end] or arr[start..=end]
    Slice {
        object: Box<Spanned<Expr>>,
        start: Box<Spanned<Expr>>,
        end: Box<Spanned<Expr>>,
        inclusive: bool,
    },

    /// Function call: foo(a, b)
    Call {
        function: Spanned<String>,
        args: Vec<Spanned<Expr>>,
    },

    /// Struct construction: Point { x: 10, y: 20 }
    StructInit {
        name: Spanned<String>,
        fields: Vec<FieldInit>,
    },

    /// Anonymous struct construction: { x: 10, y: 20 }
    /// Struct name is inferred from context (e.g., variable type annotation)
    AnonStructInit {
        fields: Vec<FieldInit>,
    },

    /// Enum variant construction: Direction::North or Message::Move { x: 1, y: 2 }
    EnumVariant {
        enum_name: Spanned<String>,
        variant: Spanned<String>,
        data: VariantData,
    },

    /// Slice length access: slice.len
    SliceLen(Box<Spanned<Expr>>),

    /// Low byte access: value.low
    /// Returns the low byte of a u16/i16 value
    U16Low(Box<Spanned<Expr>>),

    /// High byte access: value.high
    /// Returns the high byte of a u16/i16 value
    U16High(Box<Spanned<Expr>>),

    /// CPU status flag access: carry
    /// Returns true if carry flag is set
    CpuFlagCarry,

    /// CPU status flag access: zero
    /// Returns true if zero flag is set
    CpuFlagZero,

    /// CPU status flag access: overflow
    /// Returns true if overflow flag is set
    CpuFlagOverflow,

    /// CPU status flag access: negative
    /// Returns true if negative flag is set
    CpuFlagNegative,

    /// Parenthesized expression (for preserving source structure)
    Paren(Box<Spanned<Expr>>),
}

impl Expr {
    /// Create an integer literal expression
    pub fn int(value: i64) -> Self {
        Expr::Literal(Literal::Integer(value))
    }

    /// Create a boolean literal expression
    pub fn bool(value: bool) -> Self {
        Expr::Literal(Literal::Bool(value))
    }

    /// Create a variable reference expression
    pub fn var(name: impl Into<String>) -> Self {
        Expr::Variable(name.into())
    }

    /// Create a binary expression
    pub fn binary(left: Spanned<Expr>, op: BinaryOp, right: Spanned<Expr>) -> Self {
        Expr::Binary {
            left: Box::new(left),
            op,
            right: Box::new(right),
        }
    }

    /// Create a unary expression
    pub fn unary(op: UnaryOp, operand: Spanned<Expr>) -> Self {
        Expr::Unary {
            op,
            operand: Box::new(operand),
        }
    }
}
