//! Statement Code Generation
//!
//! Compiles statements into assembly instructions.

use crate::ast::{Spanned, Stmt};
use crate::codegen::expr::generate_expr;
use crate::codegen::{CodegenError, Emitter};
use crate::sema::ProgramInfo;

pub fn generate_stmt(
    stmt: &Spanned<Stmt>,
    emitter: &mut Emitter,
    info: &ProgramInfo,
) -> Result<(), CodegenError> {
    match &stmt.node {
        Stmt::Block(stmts) => {
            for s in stmts {
                generate_stmt(s, emitter, info)?;
            }
            Ok(())
        }
        Stmt::Return(expr) => {
            if let Some(e) = expr {
                generate_expr(e, emitter, info)?;
            }
            // RTS is handled by function epilogue, but we might need a jump to end
            // For now, assuming return is last statement or we just emit RTS here
            // Note: Multiple returns require jumping to epilogue
            emitter.emit_inst("RTS", "");
            Ok(())
        }
        Stmt::Assign { target, value } => {
            // 1. Generate code for value (result in A)
            generate_expr(value, emitter, info)?;

            // 2. Store A into target
            // We need a helper to generate store instructions based on target
            match &target.node {
                crate::ast::Expr::Variable(name) => {
                    if let Some(sym) = info.table.lookup(name) {
                        match sym.location {
                            crate::sema::table::SymbolLocation::Absolute(addr) => {
                                emitter.emit_inst("STA", &format!("${:04X}", addr));
                            }
                            crate::sema::table::SymbolLocation::ZeroPage(addr) => {
                                emitter.emit_inst("STA", &format!("${:02X}", addr));
                            }
                            crate::sema::table::SymbolLocation::Stack(offset) => {
                                // Placeholder for stack store
                                emitter.emit_comment(&format!(
                                    "Store local {} (offset {})",
                                    name, offset
                                ));
                            }
                            _ => return Err(CodegenError::Unknown),
                        }
                    } else {
                        return Err(CodegenError::Unknown);
                    }
                }
                _ => return Err(CodegenError::Unknown), // Only variable assignment supported for now
            }
            Ok(())
        }
        Stmt::Expr(expr) => {
            generate_expr(expr, emitter, info)?;
            Ok(())
        }
        Stmt::If {
            condition,
            then_branch,
            else_branch,
        } => {
            let else_label = emitter.next_label("else");
            let end_label = emitter.next_label("end");

            // Condition
            generate_expr(condition, emitter, info)?;
            emitter.emit_inst("CMP", "#$00");
            emitter.emit_inst("BEQ", &else_label);

            // Then
            generate_stmt(then_branch, emitter, info)?;
            emitter.emit_inst("JMP", &end_label);

            // Else
            emitter.emit_label(&else_label);
            if let Some(else_b) = else_branch {
                generate_stmt(else_b, emitter, info)?;
            }

            // End
            emitter.emit_label(&end_label);
            Ok(())
        }
        Stmt::While { condition, body } => {
            let start_label = emitter.next_label("while_start");
            let end_label = emitter.next_label("while_end");

            emitter.emit_label(&start_label);

            // Condition
            generate_expr(condition, emitter, info)?;
            emitter.emit_inst("CMP", "#$00");
            emitter.emit_inst("BEQ", &end_label);

            // Body
            generate_stmt(body, emitter, info)?;
            emitter.emit_inst("JMP", &start_label);

            emitter.emit_label(&end_label);
            Ok(())
        }
        _ => Ok(()), // TODO: Implement other statements
    }
}
