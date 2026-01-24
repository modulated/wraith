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
pub use expr::{BinaryOp, Expr, ExprMatchArm, FieldInit, Literal, UnaryOp, VariantData};
pub use item::{
    AccessMode, AddressDecl, Enum, EnumVariant, FnAttribute, FnParam, Function, Import, Item,
    SourceFile, Static, Struct, StructAttribute, StructField,
};
pub use span::{LineCol, Span, Spanned};
pub use stmt::{AsmLine, MatchArm, Pattern, PatternBinding, Range, Stmt};
pub use types::{PrimitiveType, TypeExpr};
