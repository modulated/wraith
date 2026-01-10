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
                PrimitiveType::B8 => "b8".to_string(),
                PrimitiveType::B16 => "b16".to_string(),
                PrimitiveType::Addr => "addr".to_string(),
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

    /// Check if `from` type can be implicitly converted to `to` type
    /// This implements automatic type promotion/widening for:
    /// - Integer widening: u8 → u16, i8 → i16
    /// - Bool to u8 (legacy compatibility)
    /// - Single-element array to any-size array (shorthand fill syntax)
    /// - BCD types require explicit casts (no implicit conversion)
    pub fn is_implicitly_convertible_to(&self, to: &Type) -> bool {
        // Exact match is always ok
        if self == to {
            return true;
        }

        // BCD types: NO implicit conversion (require explicit casts)
        match (self, to) {
            (Type::Primitive(src), Type::Primitive(dst)) if src.is_bcd() || dst.is_bcd() => {
                return false;  // Force explicit casts for all BCD conversions
            }
            _ => {}
        }

        match (self, to) {
            // Integer widening: smaller unsigned to larger unsigned
            (Type::Primitive(PrimitiveType::U8), Type::Primitive(PrimitiveType::U16)) => true,
            // Integer widening: smaller signed to larger signed
            (Type::Primitive(PrimitiveType::I8), Type::Primitive(PrimitiveType::I16)) => true,
            // Bool to u8 (for compatibility)
            (Type::Primitive(PrimitiveType::Bool), Type::Primitive(PrimitiveType::U8)) => true,
            // Single-element array to any-size array of same element type
            // This enables shorthand syntax: [value] expands to fill the array
            (Type::Array(from_elem, 1), Type::Array(to_elem, _to_size)) => {
                from_elem.is_implicitly_convertible_to(to_elem)
            }
            _ => false,
        }
    }
}
