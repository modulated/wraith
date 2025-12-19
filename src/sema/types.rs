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
    /// String type - represented as length-prefixed byte array
    /// In memory: [u16 length (little-endian)] [bytes...]
    /// The type points to the start of the length field
    String,
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
            Type::String => 2, // String is represented as a pointer to length-prefixed data
            Type::Function(_, _) => 2, // Function pointer is 16-bit address
            Type::Void => 0,
            Type::Named(_) => 0, // Size depends on definition, needs lookup
        }
    }

    /// Format the type in a user-friendly way for error messages
    pub fn display_name(&self) -> String {
        match self {
            Type::Primitive(p) => match p {
                PrimitiveType::U8 => "u8".to_string(),
                PrimitiveType::I8 => "i8".to_string(),
                PrimitiveType::U16 => "u16".to_string(),
                PrimitiveType::I16 => "i16".to_string(),
                PrimitiveType::Bool => "bool".to_string(),
            },
            Type::Pointer(inner, mutable) => {
                if *mutable {
                    format!("*mut {}", inner.display_name())
                } else {
                    format!("*{}", inner.display_name())
                }
            }
            Type::Array(element_ty, size) => format!("[{}; {}]", element_ty.display_name(), size),
            Type::Function(params, ret) => {
                let params_str = params.iter()
                    .map(|p| p.display_name())
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("fn({}) -> {}", params_str, ret.display_name())
            },
            Type::Named(name) => name.clone(),
            Type::String => "string".to_string(),
            Type::Void => "void".to_string(),
        }
    }
}
