//! Type representations for the Wraith language

use super::span::Spanned;

/// Primitive types supported by the language
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrimitiveType {
    /// 8-bit unsigned integer (0 to 255)
    U8,
    /// 8-bit signed integer (-128 to 127)
    I8,
    /// 16-bit unsigned integer (0 to 65535)
    U16,
    /// 16-bit signed integer (-32768 to 32767)
    I16,
    /// Boolean (actually u8: 0 or 1)
    Bool,
    /// Binary Coded Decimal: 8-bit (0-99, packed two digits)
    B8,
    /// Binary Coded Decimal: 16-bit (0-9999, packed four digits)
    B16,
}

impl PrimitiveType {
    /// Returns the size in bytes of this primitive type
    pub fn size_bytes(&self) -> usize {
        match self {
            PrimitiveType::U8 | PrimitiveType::I8 | PrimitiveType::Bool | PrimitiveType::B8 => 1,
            PrimitiveType::U16 | PrimitiveType::I16 | PrimitiveType::B16 => 2,
        }
    }

    /// Returns true if this is a BCD type
    pub fn is_bcd(&self) -> bool {
        matches!(self, PrimitiveType::B8 | PrimitiveType::B16)
    }
}

/// A type expression in the language
#[derive(Debug, Clone, PartialEq)]
pub enum TypeExpr {
    /// A primitive type (u8, i8, u16, i16, bool)
    Primitive(PrimitiveType),

    /// A named type (struct or enum name)
    Named(String),

    /// Pointer type: *T or *mut T
    Pointer {
        pointee: Box<Spanned<TypeExpr>>,
        mutable: bool,
    },

    /// Fixed-size array: [T; N]
    Array {
        element: Box<Spanned<TypeExpr>>,
        size: usize,
    },

    /// Slice type: &[T] or &[mut T]
    Slice {
        element: Box<Spanned<TypeExpr>>,
        mutable: bool,
    },
}

impl TypeExpr {
    /// Create a primitive type
    pub fn primitive(prim: PrimitiveType) -> Self {
        TypeExpr::Primitive(prim)
    }

    /// Create a named type
    pub fn named(name: impl Into<String>) -> Self {
        TypeExpr::Named(name.into())
    }

    /// Create a pointer type
    pub fn pointer(pointee: Spanned<TypeExpr>, mutable: bool) -> Self {
        TypeExpr::Pointer {
            pointee: Box::new(pointee),
            mutable,
        }
    }

    /// Create an array type
    pub fn array(element: Spanned<TypeExpr>, size: usize) -> Self {
        TypeExpr::Array {
            element: Box::new(element),
            size,
        }
    }

    /// Create a slice type
    pub fn slice(element: Spanned<TypeExpr>, mutable: bool) -> Self {
        TypeExpr::Slice {
            element: Box::new(element),
            mutable,
        }
    }
}
