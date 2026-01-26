//! Statement AST nodes for the Wraith language

use super::expr::Expr;
use super::span::Spanned;
use super::types::TypeExpr;

/// A range expression for for loops
#[derive(Debug, Clone, PartialEq)]
pub struct Range {
    pub start: Spanned<Expr>,
    pub end: Spanned<Expr>,
    /// If true, range is inclusive (0..=10), otherwise exclusive (0..10)
    pub inclusive: bool,
}

/// A match arm
#[derive(Debug, Clone, PartialEq)]
pub struct MatchArm {
    pub pattern: Spanned<Pattern>,
    pub body: Box<Spanned<Stmt>>,
}

/// A pattern for match statements
#[derive(Debug, Clone, PartialEq)]
pub enum Pattern {
    /// Literal pattern: 0, 1, true
    Literal(Spanned<Expr>),

    /// Range pattern: 1..=10
    Range {
        start: Spanned<Expr>,
        end: Spanned<Expr>,
        inclusive: bool,
    },

    /// Wildcard pattern: _
    Wildcard,

    /// Enum variant pattern: Direction::North
    EnumVariant {
        enum_name: Spanned<String>,
        variant: Spanned<String>,
        bindings: Vec<PatternBinding>,
    },

    /// Variable binding: x
    Variable(String),
}

/// Binding in an enum pattern
#[derive(Debug, Clone, PartialEq)]
pub struct PatternBinding {
    pub name: Spanned<String>,
}

/// An inline assembly line
#[derive(Debug, Clone, PartialEq)]
pub struct AsmLine {
    /// The assembly instruction with optional {var} substitutions
    pub instruction: String,
}

/// A statement in the language
#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    /// Variable declaration: let x: u8 = 42;
    VarDecl {
        name: Spanned<String>,
        ty: Spanned<TypeExpr>,
        init: Spanned<Expr>,
        mutable: bool,
    },

    /// Assignment: x = 42; or compound: x += 1;
    Assign {
        target: Spanned<Expr>,
        value: Spanned<Expr>,
    },

    /// Expression statement: foo();
    Expr(Spanned<Expr>),

    /// Return statement: return x;
    Return(Option<Spanned<Expr>>),

    /// If statement
    If {
        condition: Spanned<Expr>,
        then_branch: Box<Spanned<Stmt>>,
        else_branch: Option<Box<Spanned<Stmt>>>,
    },

    /// While loop
    While {
        condition: Spanned<Expr>,
        body: Box<Spanned<Stmt>>,
    },

    /// Infinite loop
    Loop { body: Box<Spanned<Stmt>> },

    /// For loop: for i in 0..10 { } or for i: u8 in 0..10 { }
    For {
        var_name: Spanned<String>,
        var_type: Option<Spanned<TypeExpr>>,
        range: Range,
        body: Box<Spanned<Stmt>>,
    },

    /// For-each over slice: for item in data { } or for item: u8 in data { }
    ForEach {
        var_name: Spanned<String>,
        var_type: Option<Spanned<TypeExpr>>,
        iterable: Spanned<Expr>,
        body: Box<Spanned<Stmt>>,
    },

    /// Match statement
    Match {
        expr: Spanned<Expr>,
        arms: Vec<MatchArm>,
    },

    /// Break statement
    Break,

    /// Continue statement
    Continue,

    /// Block of statements: { stmt1; stmt2; }
    Block(Vec<Spanned<Stmt>>),

    /// Inline assembly block
    Asm { lines: Vec<AsmLine> },
}

impl Stmt {
    /// Create a variable declaration statement
    pub fn var_decl(
        name: Spanned<String>,
        ty: Spanned<TypeExpr>,
        init: Spanned<Expr>,
        mutable: bool,
    ) -> Self {
        Stmt::VarDecl {
            name,
            ty,
            init,
            mutable,
        }
    }

    /// Create an expression statement
    pub fn expr(e: Spanned<Expr>) -> Self {
        Stmt::Expr(e)
    }

    /// Create a block statement
    pub fn block(stmts: Vec<Spanned<Stmt>>) -> Self {
        Stmt::Block(stmts)
    }
}
