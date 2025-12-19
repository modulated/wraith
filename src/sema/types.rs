//! Semantic Types
//!
//! Represents the canonical types used during semantic analysis.
//! These are resolved from the AST `TypeExpr`s.

use crate::ast::PrimitiveType;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Type {
    /// Primitive types (u8, i8, etc.)
    Primitive(PrimitiveType),
    /// Pointer to another type
    Pointer(Box<Type>, bool), // (pointee, is_mutable)
    /// Array type [T; N]
    Array(Box<Type>, usize),
    /// Function type (params, return_type)
    Function(Vec<Type>, Box<Type>),
    /// Void/Unit type (for functions with no return)
    Void,
    /// User-defined type (Struct/Enum) - stored by name
    /// We store the name here, and look up the definition in the symbol table
    Named(String),
}

impl Type {
    pub fn is_primitive(&self) -> bool {
        matches!(self, Type::Primitive(_))
    }

    pub fn is_pointer(&self) -> bool {
        matches!(self, Type::Pointer(_, _))
    }

    pub fn size(&self) -> usize {
        match self {
            Type::Primitive(prim) => prim.size_bytes(),
            Type::Pointer(_, _) => 2, // Pointers are 16-bit
            Type::Array(ty, len) => ty.size() * len,
            Type::Function(_, _) => 2, // Function pointer is 16-bit address
            Type::Void => 0,
            Type::Named(_) => 0, // Size depends on definition, needs lookup
        }
    }
}
