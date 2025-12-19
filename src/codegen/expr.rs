//! Expression Code Generation
//!
//! Compiles expressions into assembly instructions.
//! Result is typically left in the Accumulator (A).

use crate::ast::{Expr, Spanned};
use crate::codegen::{CodegenError, Emitter};
use crate::sema::ProgramInfo;
use crate::sema::table::SymbolLocation;

const TEMP_REG: u8 = 0x20;

pub fn generate_expr(
    expr: &Spanned<Expr>,
    emitter: &mut Emitter,
    info: &ProgramInfo,
) -> Result<(), CodegenError> {
    match &expr.node {
        Expr::Literal(lit) => generate_literal(lit, emitter),
        Expr::Variable(name) => generate_variable(name, expr.span, emitter, info),
        Expr::Binary { left, op, right } => generate_binary(left, *op, right, emitter, info),
        Expr::Call { function, args } => generate_call(function, args, emitter, info),
        _ => Ok(()), // TODO: Implement other expressions
    }
}

fn generate_call(
    function: &Spanned<String>,
    args: &[Spanned<Expr>],
    emitter: &mut Emitter,
    info: &ProgramInfo,
) -> Result<(), CodegenError> {
    // 1. Push arguments to stack (in reverse order usually, but for 6502 custom ABI we can do forward)
    // Let's do forward for simplicity for now, assuming callee pops them or uses stack offset
    // Actually, standard 6502 C compilers often use a software stack.
    // For this simple implementation, let's assume arguments are passed in registers or fixed locations if simple,
    // but since we support arbitrary args, we should push them.

    for arg in args {
        generate_expr(arg, emitter, info)?;
        emitter.emit_inst("PHA", "");
    }

    // 2. JSR to function
    emitter.emit_inst("JSR", &function.node);

    // 3. Clean up stack (add to SP)
    // 6502 doesn't have easy stack pointer manipulation.
    // Usually we just pop X times or use TSX/TXS.
    // For now, we'll just emit PLAs to discard args
    for _ in args {
        emitter.emit_inst("PLA", "");
    }

    Ok(())
}

fn generate_binary(
    left: &Spanned<Expr>,
    op: crate::ast::BinaryOp,
    right: &Spanned<Expr>,
    emitter: &mut Emitter,
    info: &ProgramInfo,
) -> Result<(), CodegenError> {
    // 1. Generate left operand -> A
    generate_expr(left, emitter, info)?;

    // 2. Push A to stack to save it
    emitter.emit_inst("PHA", "");

    // 3. Generate right operand -> A
    generate_expr(right, emitter, info)?;

    // 4. Store right operand in TEMP
    emitter.emit_inst("STA", &format!("${:02X}", TEMP_REG));

    // 5. Restore left operand -> A
    emitter.emit_inst("PLA", "");

    // 6. Perform operation
    match op {
        crate::ast::BinaryOp::Add => {
            emitter.emit_inst("CLC", "");
            emitter.emit_inst("ADC", &format!("${:02X}", TEMP_REG));
        }
        crate::ast::BinaryOp::Sub => {
            emitter.emit_inst("SEC", "");
            emitter.emit_inst("SBC", &format!("${:02X}", TEMP_REG));
        }
        // TODO: Implement other ops (Mul, Div, Bitwise, etc.)
        _ => return Err(CodegenError::Unknown),
    }

    Ok(())
}

fn generate_literal(lit: &crate::ast::Literal, emitter: &mut Emitter) -> Result<(), CodegenError> {
    match lit {
        crate::ast::Literal::Integer(val) => {
            // TODO: Handle values > 255
            emitter.emit_inst("LDA", &format!("#${:02X}", val));
            Ok(())
        }
        crate::ast::Literal::Bool(val) => {
            let v = if *val { 1 } else { 0 };
            emitter.emit_inst("LDA", &format!("#${:02X}", v));
            Ok(())
        }
        _ => Err(CodegenError::Unknown),
    }
}

fn generate_variable(
    name: &str,
    span: crate::ast::Span,
    emitter: &mut Emitter,
    info: &ProgramInfo,
) -> Result<(), CodegenError> {
    if let Some(sym) = info.resolved_symbols.get(&span) {
        match sym.location {
            SymbolLocation::Absolute(addr) => {
                emitter.emit_inst("LDA", &format!("${:04X}", addr));
                Ok(())
            }
            SymbolLocation::ZeroPage(addr) => {
                emitter.emit_inst("LDA", &format!("${:02X}", addr));
                Ok(())
            }
            SymbolLocation::Stack(offset) => {
                // Stack relative addressing: LDA $offset, X (assuming X is frame pointer)
                // For now, we don't have a frame pointer setup, so this is a placeholder
                // TODO: Implement stack frame access
                emitter.emit_comment(&format!("Load local {} (offset {})", name, offset));
                Ok(())
            }
            _ => Err(CodegenError::Unknown),
        }
    } else {
        // Fallback to global lookup if not found in resolved (shouldn't happen if analyzed correctly)
        if let Some(sym) = info.table.lookup(name) {
            match sym.location {
                SymbolLocation::Absolute(addr) => {
                    emitter.emit_inst("LDA", &format!("${:04X}", addr));
                    Ok(())
                }
                _ => Err(CodegenError::Unknown),
            }
        } else {
            Err(CodegenError::Unknown)
        }
    }
}
