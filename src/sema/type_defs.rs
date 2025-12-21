//! Type Definitions
//!
//! Stores struct and enum definitions for semantic analysis and codegen.

use crate::sema::types::Type;
use std::collections::HashMap;

/// Information about a struct field with computed offset
#[derive(Debug, Clone)]
pub struct FieldInfo {
    pub name: String,
    pub ty: Type,
    pub offset: usize, // Byte offset from struct base
}

/// Definition of a struct type
#[derive(Debug, Clone)]
pub struct StructDef {
    pub name: String,
    pub fields: Vec<FieldInfo>,
    pub total_size: usize,
    pub zero_page: bool, // Whether this struct should be in zero page
}

impl StructDef {
    /// Get field by name
    pub fn get_field(&self, name: &str) -> Option<&FieldInfo> {
        self.fields.iter().find(|f| f.name == name)
    }

    /// Calculate total size of the struct
    pub fn calculate_size(fields: &[FieldInfo]) -> usize {
        fields.iter().map(|f| f.ty.size()).sum()
    }
}

/// Information about an enum variant with computed tag and data
#[derive(Debug, Clone)]
pub struct VariantInfo {
    pub name: String,
    pub tag: u8, // Discriminant value
    pub data: VariantData,
}

/// Data associated with an enum variant
#[derive(Debug, Clone)]
pub enum VariantData {
    /// Unit variant (no data)
    Unit,
    /// Tuple variant with field types
    Tuple(Vec<Type>),
    /// Struct variant with named fields
    Struct(Vec<FieldInfo>),
}

/// Definition of an enum type
#[derive(Debug, Clone)]
pub struct EnumDef {
    pub name: String,
    pub variants: Vec<VariantInfo>,
    pub total_size: usize, // Size = tag (1 byte) + max variant data size
}

impl EnumDef {
    /// Get variant by name
    pub fn get_variant(&self, name: &str) -> Option<&VariantInfo> {
        self.variants.iter().find(|v| v.name == name)
    }

    /// Calculate total size of the enum (tag + largest variant)
    pub fn calculate_size(variants: &[VariantInfo]) -> usize {
        let max_data_size = variants
            .iter()
            .map(|v| match &v.data {
                VariantData::Unit => 0,
                VariantData::Tuple(types) => types.iter().map(|t| t.size()).sum(),
                VariantData::Struct(fields) => StructDef::calculate_size(fields),
            })
            .max()
            .unwrap_or(0);

        1 + max_data_size // 1 byte for tag + data
    }
}

/// Registry of all type definitions
#[derive(Debug, Clone, Default)]
pub struct TypeRegistry {
    pub structs: HashMap<String, StructDef>,
    pub enums: HashMap<String, EnumDef>,
}

impl TypeRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_struct(&mut self, def: StructDef) {
        self.structs.insert(def.name.clone(), def);
    }

    pub fn add_enum(&mut self, def: EnumDef) {
        self.enums.insert(def.name.clone(), def);
    }

    pub fn get_struct(&self, name: &str) -> Option<&StructDef> {
        self.structs.get(name)
    }

    pub fn get_enum(&self, name: &str) -> Option<&EnumDef> {
        self.enums.get(name)
    }

    /// Get the size of a named type
    pub fn get_type_size(&self, name: &str) -> Option<usize> {
        if let Some(struct_def) = self.get_struct(name) {
            Some(struct_def.total_size)
        } else if let Some(enum_def) = self.get_enum(name) {
            Some(enum_def.total_size)
        } else {
            None
        }
    }
}
