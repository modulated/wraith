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
    /// ASCII character: an 8-bit value holding a single ASCII codepoint
    /// (0-127). Distinct from u8 for type safety; cast with `as u8` / `as char`.
    Char,
    /// Binary Coded Decimal: 8-bit (0-99, packed two digits)
    B8,
    /// Binary Coded Decimal: 16-bit (0-9999, packed four digits)
    B16,
    /// Signed 16-bit fixed-point with 8 fraction bits (Q8.8). The value is a
    /// two's-complement `i16` scaled by 256, so `1.5` is stored as `0x0180`.
    /// Add and subtract are plain 16-bit two's-complement arithmetic (cheaper
    /// than BCD — no decimal mode); the fraction only matters for multiply,
    /// divide, and conversion to and from the integer types.
    Q8_8,
    /// Memory-mapped I/O address (stores u8 values)
    Addr,
}

impl PrimitiveType {
    /// Returns the size in bytes of this primitive type
    pub fn size_bytes(&self) -> usize {
        match self {
            PrimitiveType::U8
            | PrimitiveType::I8
            | PrimitiveType::Bool
            | PrimitiveType::Char
            | PrimitiveType::B8
            | PrimitiveType::Addr => 1,
            PrimitiveType::U16 | PrimitiveType::I16 | PrimitiveType::B16 | PrimitiveType::Q8_8 => 2,
        }
    }

    /// Returns true if this is a BCD type
    pub fn is_bcd(&self) -> bool {
        matches!(self, PrimitiveType::B8 | PrimitiveType::B16)
    }

    /// Returns true if this is a fixed-point type.
    pub fn is_fixed(&self) -> bool {
        matches!(self, PrimitiveType::Q8_8)
    }

    /// The number of fraction bits a fixed-point type carries, or 0 for a type
    /// that is not fixed-point. `q8.8` has 8, so a value is scaled by `1 << 8`.
    pub fn frac_bits(&self) -> u32 {
        match self {
            PrimitiveType::Q8_8 => 8,
            _ => 0,
        }
    }

    /// Returns true if this is a signed integer type (i8 or i16).
    ///
    /// Only i8/i16 are signed; u8/u16/bool/BCD/addr are all unsigned. This is
    /// the primitive codegen consults to choose signed vs unsigned comparison,
    /// shift, and division sequences.
    pub fn is_signed(&self) -> bool {
        matches!(
            self,
            PrimitiveType::I8 | PrimitiveType::I16 | PrimitiveType::Q8_8
        )
    }
}

/// A type expression in the language
#[derive(Debug, Clone, PartialEq)]
pub enum TypeExpr {
    /// A primitive type (u8, i8, u16, i16, bool)
    Primitive(PrimitiveType),

    /// A named type (struct or enum name)
    Named(String),

    /// A fixed-capacity, mutable string buffer: `str<N>`. At runtime it is a
    /// `str` (a 2-byte pointer to `[u8 len][bytes…]`), but it owns `N` bytes of
    /// RAM so its contents can be edited in place. `N` is the maximum content
    /// length (0-255); the backing block is `1 + N` bytes. Resolves to
    /// `Type::String`, so every `str` read operation applies unchanged.
    StringBuf { capacity: usize },

    /// Fixed-size array: [T; N]
    Array {
        element: Box<Spanned<TypeExpr>>,
        size: usize,
    },

    /// Slice type: `&[T]`. A read-only view — base address plus length — over
    /// storage that lives somewhere else. There is no mutable counterpart yet
    /// (see the roadmap): the descriptor cannot know whether its target is
    /// writable, so `s[i] = v` is rejected everywhere rather than depending on
    /// a declaration elsewhere.
    Slice { element: Box<Spanned<TypeExpr>> },

    /// Pointer type: &T. A 16-bit address of a single value.
    ///
    /// Spelled with the same `&` as the slice type, which is also an unchecked
    /// reference in this language; a slice additionally carries a length.
    Pointer { pointee: Box<Spanned<TypeExpr>> },

    /// Function pointer type: fn(params) -> ret (ret omitted = void).
    /// Stored as a 16-bit code address.
    Function {
        params: Vec<Spanned<TypeExpr>>,
        ret: Option<Box<Spanned<TypeExpr>>>,
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

    /// Create an array type
    pub fn array(element: Spanned<TypeExpr>, size: usize) -> Self {
        TypeExpr::Array {
            element: Box::new(element),
            size,
        }
    }

    /// Create a slice type
    pub fn slice(element: Spanned<TypeExpr>) -> Self {
        TypeExpr::Slice {
            element: Box::new(element),
        }
    }

    /// Create a pointer type
    pub fn pointer(pointee: Spanned<TypeExpr>) -> Self {
        TypeExpr::Pointer {
            pointee: Box::new(pointee),
        }
    }
}
