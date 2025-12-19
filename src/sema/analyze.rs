//! Semantic Analysis Logic
//!
//! Traverses the AST to populate the symbol table and perform type checking.

use crate::ast::{Expr, Function, Item, SourceFile, Spanned, Stmt, TypeExpr};
use crate::sema::table::{SymbolInfo, SymbolKind, SymbolLocation, SymbolTable};
use crate::sema::types::Type;
use crate::sema::{ProgramInfo, SemaError};

use crate::ast::Span;
use std::collections::HashMap;

pub struct SemanticAnalyzer {
    pub table: SymbolTable,
    pub errors: Vec<SemaError>,
    current_return_type: Option<Type>,
    resolved_symbols: HashMap<Span, SymbolInfo>,
}

impl SemanticAnalyzer {
    pub fn new() -> Self {
        Self {
            table: SymbolTable::new(),
            errors: Vec::new(),
            current_return_type: None,
            resolved_symbols: HashMap::new(),
        }
    }

    pub fn analyze(&mut self, source: &SourceFile) -> Result<ProgramInfo, SemaError> {
        // First pass: Register all global items (functions, statics, structs)
        for item in &source.items {
            self.register_item(item)?;
        }

        // Second pass: Analyze function bodies
        for item in &source.items {
            self.analyze_item(item)?;
        }

        if !self.errors.is_empty() {
            return Err(self.errors[0].clone());
        }

        Ok(ProgramInfo {
            table: self.table.clone(),
            resolved_symbols: self.resolved_symbols.clone(),
        })
    }

    fn register_item(&mut self, item: &Spanned<Item>) -> Result<(), SemaError> {
        match &item.node {
            Item::Function(func) => {
                let name = func.name.node.clone();
                let info = SymbolInfo {
                    name: name.clone(),
                    kind: SymbolKind::Function,
                    ty: self.resolve_function_type(func)?,
                    location: SymbolLocation::Absolute(0),
                    mutable: false,
                };
                self.table.insert(name, info);
            }
            Item::Static(stat) => {
                let name = stat.name.node.clone();
                let info = SymbolInfo {
                    name: name.clone(),
                    kind: SymbolKind::Constant,
                    ty: self.resolve_type(&stat.ty.node)?,
                    location: SymbolLocation::Absolute(0),
                    mutable: stat.mutable,
                };
                self.table.insert(name, info);
            }
            Item::Address(addr) => {
                let name = addr.name.node.clone();
                let info = SymbolInfo {
                    name: name.clone(),
                    kind: SymbolKind::Variable,
                    ty: Type::Primitive(crate::ast::PrimitiveType::U8),
                    location: SymbolLocation::Absolute(0),
                    mutable: true,
                };
                self.table.insert(name, info);
            }
            _ => {}
        }
        Ok(())
    }

    fn analyze_item(&mut self, item: &Spanned<Item>) -> Result<(), SemaError> {
        match &item.node {
            Item::Function(func) => {
                self.table.enter_scope();

                // Set current return type for checking return statements
                let return_type = if let Some(ret) = &func.return_type {
                    self.resolve_type(&ret.node)?
                } else {
                    Type::Void
                };
                self.current_return_type = Some(return_type);

                // Register parameters
                for param in &func.params {
                    let name = param.name.node.clone();
                    let info = SymbolInfo {
                        name: name.clone(),
                        kind: SymbolKind::Variable,
                        ty: self.resolve_type(&param.ty.node)?,
                        location: SymbolLocation::Stack(0),
                        mutable: false,
                    };
                    self.table.insert(name, info);
                }

                // Analyze body
                self.analyze_stmt(&func.body)?;

                self.current_return_type = None;
                self.table.exit_scope();
            }
            _ => {}
        }
        Ok(())
    }

    fn analyze_stmt(&mut self, stmt: &Spanned<Stmt>) -> Result<(), SemaError> {
        match &stmt.node {
            Stmt::Block(stmts) => {
                self.table.enter_scope();
                for s in stmts {
                    self.analyze_stmt(s)?;
                }
                self.table.exit_scope();
            }
            Stmt::VarDecl {
                name,
                ty,
                init,
                mutable,
                ..
            } => {
                let declared_ty = self.resolve_type(&ty.node)?;
                let init_ty = self.check_expr(init)?;

                if declared_ty != init_ty {
                    // TODO: Better error reporting
                    return Err(SemaError::TypeMismatch);
                }

                let info = SymbolInfo {
                    name: name.node.clone(),
                    kind: SymbolKind::Variable,
                    ty: declared_ty,
                    location: SymbolLocation::Stack(0),
                    mutable: *mutable,
                };
                self.table.insert(name.node.clone(), info);
            }
            Stmt::Assign { target, value } => {
                let target_ty = self.check_expr(target)?;
                let value_ty = self.check_expr(value)?;

                if target_ty != value_ty {
                    return Err(SemaError::TypeMismatch);
                }

                // Check mutability
                if let Expr::Variable(name) = &target.node {
                    if let Some(info) = self.table.lookup(name)
                        && !info.mutable
                    {
                        return Err(SemaError::ImmutableAssign); // Cannot assign to immutable
                    }
                }
            }
            Stmt::Return(expr) => {
                let expr_ty = if let Some(e) = expr {
                    self.check_expr(e)?
                } else {
                    Type::Void
                };

                if let Some(ret_ty) = &self.current_return_type
                    && &expr_ty != ret_ty
                {
                    return Err(SemaError::TypeMismatch);
                }
            }
            Stmt::If {
                condition,
                then_branch,
                else_branch,
            } => {
                let cond_ty = self.check_expr(condition)?;
                if cond_ty != Type::Primitive(crate::ast::PrimitiveType::Bool) {
                    return Err(SemaError::TypeMismatch);
                }
                self.analyze_stmt(then_branch)?;
                if let Some(else_b) = else_branch {
                    self.analyze_stmt(else_b)?;
                }
            }
            Stmt::While { condition, body } => {
                let cond_ty = self.check_expr(condition)?;
                if cond_ty != Type::Primitive(crate::ast::PrimitiveType::Bool) {
                    return Err(SemaError::TypeMismatch);
                }
                self.analyze_stmt(body)?;
            }
            Stmt::Expr(expr) => {
                self.check_expr(expr)?;
            }
            _ => {}
        }
        Ok(())
    }

    fn check_expr(&mut self, expr: &Spanned<Expr>) -> Result<Type, SemaError> {
        match &expr.node {
            Expr::Literal(lit) => match lit {
                crate::ast::Literal::Integer(_) => {
                    Ok(Type::Primitive(crate::ast::PrimitiveType::U8))
                } // Default to u8, need better inference
                crate::ast::Literal::Bool(_) => {
                    Ok(Type::Primitive(crate::ast::PrimitiveType::Bool))
                }
                _ => Ok(Type::Void),
            },
            Expr::Variable(name) => {
                let info = if let Some(info) = self.table.lookup(name) {
                    info.clone()
                } else {
                    return Err(SemaError::SymbolNotFound); // Symbol not found
                };

                self.resolved_symbols.insert(expr.span, info.clone());
                Ok(info.ty)
            }
            Expr::Binary { left, op: _, right } => {
                let left_ty = self.check_expr(left)?;
                let right_ty = self.check_expr(right)?;

                if left_ty != right_ty {
                    return Err(SemaError::TypeMismatch);
                }
                Ok(left_ty) // Result type depends on op, assuming same for now
            }
            Expr::Call { function, args } => {
                // TODO: Check function signature
                let (param_types, ret_type) = if let Some(info) = self.table.lookup(&function.node)
                {
                    if let Type::Function(param_types, ret_type) = &info.ty {
                        (param_types.clone(), ret_type.clone())
                    } else {
                        return Err(SemaError::TypeMismatch);
                    }
                } else {
                    return Err(SemaError::SymbolNotFound);
                };

                if args.len() != param_types.len() {
                    return Err(SemaError::ArgMismatch);
                }
                for (arg, param_ty) in args.iter().zip(param_types.iter()) {
                    let arg_ty = self.check_expr(arg)?;
                    if &arg_ty != param_ty {
                        return Err(SemaError::TypeMismatch);
                    }
                }
                Ok(*ret_type)
            }
            _ => Ok(Type::Void), // TODO: Handle other expressions
        }
    }

    fn resolve_type(&self, ty: &TypeExpr) -> Result<Type, SemaError> {
        match ty {
            TypeExpr::Primitive(p) => Ok(Type::Primitive(*p)),
            // TODO: Handle other types
            _ => Ok(Type::Void),
        }
    }

    fn resolve_function_type(&self, func: &Function) -> Result<Type, SemaError> {
        let mut param_types = Vec::new();
        for param in &func.params {
            param_types.push(self.resolve_type(&param.ty.node)?);
        }

        let return_type = if let Some(ret) = &func.return_type {
            self.resolve_type(&ret.node)?
        } else {
            Type::Void
        };

        Ok(Type::Function(param_types, Box::new(return_type)))
    }
}
