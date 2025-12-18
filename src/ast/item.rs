//! Top-level item AST nodes for the Wraith language

use super::span::Spanned;
use super::stmt::Stmt;
use super::types::TypeExpr;

/// Function parameter
#[derive(Debug, Clone, PartialEq)]
pub struct FnParam {
    pub name: Spanned<String>,
    pub ty: Spanned<TypeExpr>,
}

/// Function attribute
#[derive(Debug, Clone, PartialEq)]
pub enum FnAttribute {
    /// Suggest inlining
    Inline,
    /// Function never returns
    NoReturn,
    /// Interrupt handler (compiler handles register save/restore)
    Interrupt,
    /// Place at specific address
    Org(u16),
}

/// A struct field definition
#[derive(Debug, Clone, PartialEq)]
pub struct StructField {
    pub name: Spanned<String>,
    pub ty: Spanned<TypeExpr>,
}

/// Struct attribute
#[derive(Debug, Clone, PartialEq)]
pub enum StructAttribute {
    /// Place struct in zero page section
    ZpSection,
    /// Pack struct tightly (no padding)
    Packed,
}

/// Enum variant definition
#[derive(Debug, Clone, PartialEq)]
pub enum EnumVariant {
    /// Simple variant: North = 0
    Unit {
        name: Spanned<String>,
        value: Option<i64>,
    },

    /// Tuple variant: Write(u8)
    Tuple {
        name: Spanned<String>,
        fields: Vec<Spanned<TypeExpr>>,
    },

    /// Struct variant: Move { u8 x, u8 y }
    Struct {
        name: Spanned<String>,
        fields: Vec<StructField>,
    },
}

impl EnumVariant {
    /// Get the name of this variant
    pub fn name(&self) -> &str {
        match self {
            EnumVariant::Unit { name, .. } => &name.node,
            EnumVariant::Tuple { name, .. } => &name.node,
            EnumVariant::Struct { name, .. } => &name.node,
        }
    }
}

/// A function definition
#[derive(Debug, Clone, PartialEq)]
pub struct Function {
    pub name: Spanned<String>,
    pub params: Vec<FnParam>,
    pub return_type: Option<Spanned<TypeExpr>>,
    pub body: Spanned<Stmt>,
    pub attributes: Vec<FnAttribute>,
    pub is_inline: bool,
}

/// A struct definition
#[derive(Debug, Clone, PartialEq)]
pub struct Struct {
    pub name: Spanned<String>,
    pub fields: Vec<StructField>,
    pub attributes: Vec<StructAttribute>,
}

/// An enum definition
#[derive(Debug, Clone, PartialEq)]
pub struct Enum {
    pub name: Spanned<String>,
    pub variants: Vec<EnumVariant>,
}

/// Static/const variable declaration (top-level)
#[derive(Debug, Clone, PartialEq)]
pub struct Static {
    pub name: Spanned<String>,
    pub ty: Spanned<TypeExpr>,
    pub init: Spanned<super::expr::Expr>,
    pub mutable: bool,
    pub zero_page: bool,
}

/// A top-level item in a source file
#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    Function(Box<Function>),
    Struct(Struct),
    Enum(Enum),
    Static(Static),
}

/// A complete source file / compilation unit
#[derive(Debug, Clone, PartialEq)]
pub struct SourceFile {
    pub items: Vec<Spanned<Item>>,
}

impl SourceFile {
    /// Create a new empty source file
    pub fn new() -> Self {
        Self { items: Vec::new() }
    }

    /// Create a source file with items
    pub fn with_items(items: Vec<Spanned<Item>>) -> Self {
        Self { items }
    }
}

impl Default for SourceFile {
    fn default() -> Self {
        Self::new()
    }
}
