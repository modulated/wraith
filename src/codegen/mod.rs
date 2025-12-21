pub mod emitter;
pub mod expr;
pub mod item;
pub mod memory_layout;
pub mod stmt;

use crate::ast::SourceFile;
use crate::sema::ProgramInfo;
use emitter::Emitter;
use item::generate_item;

#[derive(Debug, Clone)]
pub enum CodegenError {
    Unknown,
    UnsupportedOperation(String),
    SymbolNotFound(String),
}

pub fn generate(ast: &SourceFile, program: &ProgramInfo) -> Result<String, CodegenError> {
    let mut emitter = Emitter::new();

    for item in &ast.items {
        generate_item(item, &mut emitter, program)?;
    }

    Ok(emitter.finish())
}
