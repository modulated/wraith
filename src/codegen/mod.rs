//! Code Generation module
//!
//! Responsible for converting analyzed AST into 6502 assembly.

use crate::sema::ProgramInfo;

#[derive(Debug, Clone)]
pub enum CodegenError {
    // Placeholder
    Unknown,
}

pub fn generate(_program: &ProgramInfo) -> Result<String, CodegenError> {
    // TODO: Implement code generation
    Ok(String::new())
}
