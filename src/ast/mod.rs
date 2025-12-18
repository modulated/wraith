//! Abstract Syntax Tree (AST) for the Wraith programming language
//!
//! This module contains all the AST node types used to represent
//! parsed Wraith source code.

mod expr;
mod item;
mod span;
mod stmt;
mod types;

// Re-export all public types
pub use expr::{BinaryOp, Expr, FieldInit, Literal, UnaryOp, VariantData};
pub use item::{
    Enum, EnumVariant, FnAttribute, FnParam, Function, Item, SourceFile, Static, Struct,
    StructAttribute, StructField,
};
pub use span::{Span, Spanned};
pub use stmt::{AsmLine, MatchArm, Pattern, PatternBinding, Range, Stmt};
pub use types::{PrimitiveType, TypeExpr};
