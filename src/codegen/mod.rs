pub mod emitter;
pub mod expr;
pub mod item;
pub mod memory_layout;
pub mod regstate;
pub mod section_allocator;
pub mod stmt;

use crate::ast::SourceFile;
use crate::sema::ProgramInfo;
use emitter::Emitter;
use item::generate_item;
use section_allocator::SectionAllocator;

#[derive(Debug, Clone)]
pub enum CodegenError {
    Unknown,
    UnsupportedOperation(String),
    SymbolNotFound(String),
    SectionError(String),
}

pub fn generate(ast: &SourceFile, program: &ProgramInfo) -> Result<String, CodegenError> {
    let mut emitter = Emitter::new();
    let mut section_alloc = SectionAllocator::default();

    for item in &ast.items {
        generate_item(item, &mut emitter, program, &mut section_alloc)?;
    }

    Ok(emitter.finish())
}
