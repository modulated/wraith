//! Semantic Analysis Logic
//!
//! Traverses the AST to populate the symbol table and perform type checking.

use crate::ast::{Expr, Function, Item, SourceFile, Spanned, Stmt, TypeExpr};
use crate::sema::const_eval::{eval_const_expr, ConstValue};
use crate::sema::table::{SymbolInfo, SymbolKind, SymbolLocation, SymbolTable};
use crate::sema::types::Type;
use crate::sema::{FunctionMetadata, ProgramInfo, SemaError};

use crate::ast::Span;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

pub struct SemanticAnalyzer {
    pub table: SymbolTable,
    pub errors: Vec<SemaError>,
    current_return_type: Option<Type>,
    resolved_symbols: HashMap<Span, SymbolInfo>,
    function_metadata: HashMap<String, FunctionMetadata>,
    folded_constants: HashMap<Span, ConstValue>,
    base_path: Option<PathBuf>,
    imported_files: HashSet<PathBuf>,
    zp_allocator: ZeroPageAllocator,
}

/// Zero page memory allocator
/// Manages allocation of zero page addresses ($00-$FF)
#[allow(dead_code)]
struct ZeroPageAllocator {
    /// Next available address
    next_addr: u8,
    /// Reserved ranges (start, end) that cannot be allocated
    reserved: Vec<(u8, u8)>,
}

impl ZeroPageAllocator {
    fn new() -> Self {
        Self {
            next_addr: 0x40, // Start after commonly used system locations
            reserved: vec![
                (0x00, 0x1F), // System reserved
                (0x20, 0x2F), // Temporary storage for codegen
                (0x30, 0x3F), // Pointer operations
            ],
        }
    }

    /// Allocate a single byte in zero page
    fn allocate(&mut self) -> Result<u8, SemaError> {
        // Find next available address
        loop {
            let addr = self.next_addr;

            // Check if this address is reserved
            let is_reserved = self.reserved.iter().any(|(start, end)| addr >= *start && addr <= *end);

            if !is_reserved && addr != 0xFF {
                self.next_addr = addr + 1;
                return Ok(addr);
            }

            // Try next address
            self.next_addr += 1;

            if self.next_addr == 0 {
                // Wrapped around - out of zero page
                return Err(SemaError::OutOfZeroPage {
                    span: Span { start: 0, end: 0 }, // No span context in allocator
                });
            }
        }
    }

    /// Allocate multiple consecutive bytes
    #[allow(dead_code)]
    fn allocate_range(&mut self, count: u8) -> Result<u8, SemaError> {
        let start = self.next_addr;

        // Check if we have enough space
        if start as usize + count as usize > 0x100 {
            return Err(SemaError::OutOfZeroPage {
                span: Span { start: 0, end: 0 }, // No span context in allocator
            });
        }

        // Allocate each byte
        for _ in 0..count {
            self.allocate()?;
        }

        Ok(start)
    }

    /// Reset allocator (for new scope/function)
    #[allow(dead_code)]
    fn reset(&mut self) {
        self.next_addr = 0x40;
    }
}

impl Default for SemanticAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

impl SemanticAnalyzer {
    pub fn new() -> Self {
        Self {
            table: SymbolTable::new(),
            errors: Vec::new(),
            current_return_type: None,
            resolved_symbols: HashMap::new(),
            function_metadata: HashMap::new(),
            folded_constants: HashMap::new(),
            base_path: None,
            imported_files: HashSet::new(),
            zp_allocator: ZeroPageAllocator::new(),
        }
    }

    pub fn with_base_path(base_path: PathBuf) -> Self {
        Self {
            table: SymbolTable::new(),
            errors: Vec::new(),
            current_return_type: None,
            resolved_symbols: HashMap::new(),
            function_metadata: HashMap::new(),
            folded_constants: HashMap::new(),
            base_path: Some(base_path),
            imported_files: HashSet::new(),
            zp_allocator: ZeroPageAllocator::new(),
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
            function_metadata: self.function_metadata.clone(),
            folded_constants: self.folded_constants.clone(),
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
                self.table.insert(name.clone(), info);

                // Extract org attribute if present
                let org_address = func.attributes.iter().find_map(|attr| {
                    if let crate::ast::FnAttribute::Org(addr) = attr {
                        Some(*addr)
                    } else {
                        None
                    }
                });

                self.function_metadata.insert(
                    name,
                    FunctionMetadata { org_address },
                );
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

                // Evaluate the address expression to get the actual address
                let address = if let Expr::Literal(crate::ast::Literal::Integer(val)) = &addr.address.node {
                    *val as u16
                } else {
                    // For now, default to 0 for non-literal addresses
                    // TODO: Support constant expressions
                    0
                };

                let info = SymbolInfo {
                    name: name.clone(),
                    kind: SymbolKind::Variable,
                    ty: Type::Primitive(crate::ast::PrimitiveType::U8),
                    location: SymbolLocation::Absolute(address),
                    mutable: true,
                };
                self.table.insert(name, info);
            }
            Item::Import(import) => {
                self.process_import(import)?;
            }
            _ => {}
        }
        Ok(())
    }

    fn process_import(&mut self, import: &crate::ast::Import) -> Result<(), SemaError> {
        // Resolve the import path relative to the base path
        let import_path = if let Some(base) = &self.base_path {
            base.parent().unwrap_or(base).join(&import.path.node)
        } else {
            PathBuf::from(&import.path.node)
        };

        // Check if we've already imported this file to avoid circular imports
        if self.imported_files.contains(&import_path) {
            return Ok(());
        }
        self.imported_files.insert(import_path.clone());

        // Load and parse the imported file
        let source = std::fs::read_to_string(&import_path)
            .map_err(|e| SemaError::ImportError {
                path: import.path.node.clone(),
                reason: format!("failed to read file: {}", e),
                span: import.path.span,
            })?;

        let tokens = crate::lex(&source)
            .map_err(|e| SemaError::ImportError {
                path: import.path.node.clone(),
                reason: format!("lexer error: {:?}", e),
                span: import.path.span,
            })?;

        let ast = crate::Parser::parse(&tokens)
            .map_err(|e| SemaError::ImportError {
                path: import.path.node.clone(),
                reason: format!("parser error: {:?}", e),
                span: import.path.span,
            })?;

        // Analyze the imported file
        let mut imported_analyzer = SemanticAnalyzer::with_base_path(import_path.clone());
        imported_analyzer.imported_files = self.imported_files.clone();
        let imported_info = imported_analyzer.analyze(&ast)?;

        // Import the requested symbols into our table
        for symbol_name in &import.symbols {
            let name = &symbol_name.node;
            if let Some(symbol) = imported_info.table.lookup(name) {
                self.table.insert(name.clone(), symbol.clone());
            } else {
                return Err(SemaError::ImportError {
                    path: import.path.node.clone(),
                    reason: format!("symbol '{}' not found in imported file", name),
                    span: symbol_name.span,
                });
            }
        }

        // Merge the imported files set
        self.imported_files.extend(imported_analyzer.imported_files);

        Ok(())
    }

    fn analyze_item(&mut self, item: &Spanned<Item>) -> Result<(), SemaError> {
        if let Item::Function(func) = &item.node {
            self.table.enter_scope();

            // Set current return type for checking return statements
            let return_type = if let Some(ret) = &func.return_type {
                self.resolve_type(&ret.node)?
            } else {
                Type::Void
            };
            self.current_return_type = Some(return_type);

            // Register parameters
            // Allocate parameters in zero page using allocator
            for param in &func.params {
                let name = param.name.node.clone();
                let addr = self.zp_allocator.allocate()?;
                let location = SymbolLocation::ZeroPage(addr);
                let info = SymbolInfo {
                    name: name.clone(),
                    kind: SymbolKind::Variable,
                    ty: self.resolve_type(&param.ty.node)?,
                    location,
                    mutable: false,
                };
                self.table.insert(name, info);
            }

            // Analyze body
            self.analyze_stmt(&func.body)?;

            self.current_return_type = None;
            self.table.exit_scope();
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
                zero_page: _,
            } => {
                let declared_ty = self.resolve_type(&ty.node)?;
                let init_ty = self.check_expr(init)?;

                // Allow Bool to U8 assignment (booleans are 0/1 bytes in 6502)
                let types_compatible = declared_ty == init_ty
                    || (matches!(declared_ty, Type::Primitive(crate::ast::PrimitiveType::U8))
                        && matches!(init_ty, Type::Primitive(crate::ast::PrimitiveType::Bool)));

                if !types_compatible {
                    return Err(SemaError::TypeMismatch {
                        expected: declared_ty.display_name(),
                        found: init_ty.display_name(),
                        span: init.span,
                    });
                }

                // Allocate in zero page using allocator
                let addr = self.zp_allocator.allocate()?;
                let location = SymbolLocation::ZeroPage(addr);

                let info = SymbolInfo {
                    name: name.node.clone(),
                    kind: SymbolKind::Variable,
                    ty: declared_ty,
                    location,
                    mutable: *mutable,
                };
                self.table.insert(name.node.clone(), info.clone());
                // Also add to resolved_symbols so codegen can find it
                self.resolved_symbols.insert(name.span, info);
            }
            Stmt::Assign { target, value } => {
                let target_ty = self.check_expr(target)?;
                let value_ty = self.check_expr(value)?;

                // Allow Bool to U8 assignment (booleans are 0/1 bytes in 6502)
                let types_compatible = target_ty == value_ty
                    || (matches!(target_ty, Type::Primitive(crate::ast::PrimitiveType::U8))
                        && matches!(value_ty, Type::Primitive(crate::ast::PrimitiveType::Bool)));

                if !types_compatible {
                    return Err(SemaError::TypeMismatch {
                        expected: target_ty.display_name(),
                        found: value_ty.display_name(),
                        span: value.span,
                    });
                }

                // Check mutability
                if let Expr::Variable(name) = &target.node {
                    if let Some(info) = self.table.lookup(name) {
                        if !info.mutable {
                            return Err(SemaError::ImmutableAssignment {
                                symbol: name.clone(),
                                span: target.span,
                            });
                        }
                    }
                }
            }
            Stmt::Return(expr) => {
                let expr_ty = if let Some(e) = expr {
                    self.check_expr(e)?
                } else {
                    Type::Void
                };

                if let Some(ret_ty) = &self.current_return_type {
                    if &expr_ty != ret_ty {
                        return Err(SemaError::ReturnTypeMismatch {
                            expected: ret_ty.display_name(),
                            found: expr_ty.display_name(),
                            span: expr.as_ref().map(|e| e.span).unwrap_or(stmt.span),
                        });
                    }
                }
            }
            Stmt::If {
                condition,
                then_branch,
                else_branch,
            } => {
                let cond_ty = self.check_expr(condition)?;
                if cond_ty != Type::Primitive(crate::ast::PrimitiveType::Bool) {
                    return Err(SemaError::TypeMismatch {
                        expected: "bool".to_string(),
                        found: cond_ty.display_name(),
                        span: condition.span,
                    });
                }
                self.analyze_stmt(then_branch)?;
                if let Some(else_b) = else_branch {
                    self.analyze_stmt(else_b)?;
                }
            }
            Stmt::While { condition, body } => {
                let cond_ty = self.check_expr(condition)?;
                if cond_ty != Type::Primitive(crate::ast::PrimitiveType::Bool) {
                    return Err(SemaError::TypeMismatch {
                        expected: "bool".to_string(),
                        found: cond_ty.display_name(),
                        span: condition.span,
                    });
                }
                self.analyze_stmt(body)?;
            }
            Stmt::For {
                var_name,
                var_type,
                range,
                body,
            } => {
                // Create a new scope for the loop variable
                self.table.enter_scope();

                // Register the loop variable
                let var_ty = self.resolve_type(&var_type.node)?;
                let addr = self.zp_allocator.allocate()?;
                let info = SymbolInfo {
                    name: var_name.node.clone(),
                    kind: SymbolKind::Variable,
                    ty: var_ty,
                    location: SymbolLocation::ZeroPage(addr),
                    mutable: true,
                };
                self.table.insert(var_name.node.clone(), info);

                // Check range bounds
                self.check_expr(&range.start)?;
                self.check_expr(&range.end)?;

                // Analyze body
                self.analyze_stmt(body)?;

                self.table.exit_scope();
            }
            Stmt::Loop { body } => {
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
        // Try to fold the expression if it's constant
        if let Ok(const_val) = eval_const_expr(expr) {
            self.folded_constants.insert(expr.span, const_val);
        }

        match &expr.node {
            Expr::Literal(lit) => match lit {
                crate::ast::Literal::Integer(_) => {
                    Ok(Type::Primitive(crate::ast::PrimitiveType::U8))
                } // Default to u8, need better inference
                crate::ast::Literal::Bool(_) => {
                    Ok(Type::Primitive(crate::ast::PrimitiveType::Bool))
                }
                crate::ast::Literal::String(_) => {
                    Ok(Type::String)
                }
                _ => Ok(Type::Void),
            },
            Expr::Variable(name) => {
                let info = if let Some(info) = self.table.lookup(name) {
                    info.clone()
                } else {
                    return Err(SemaError::UndefinedSymbol {
                        name: name.clone(),
                        span: expr.span,
                    });
                };

                self.resolved_symbols.insert(expr.span, info.clone());
                Ok(info.ty)
            }
            Expr::Binary { left, op, right } => {
                let left_ty = self.check_expr(left)?;
                let right_ty = self.check_expr(right)?;

                if left_ty != right_ty {
                    return Err(SemaError::InvalidBinaryOp {
                        op: format!("{:?}", op),
                        left_ty: left_ty.display_name(),
                        right_ty: right_ty.display_name(),
                        span: expr.span,
                    });
                }

                // Comparison and logical operators return Bool
                use crate::ast::BinaryOp;
                match op {
                    BinaryOp::Eq | BinaryOp::Ne | BinaryOp::Lt | BinaryOp::Le
                    | BinaryOp::Gt | BinaryOp::Ge | BinaryOp::And | BinaryOp::Or => {
                        Ok(Type::Primitive(crate::ast::PrimitiveType::Bool))
                    }
                    // Arithmetic and bitwise operators return the operand type
                    _ => Ok(left_ty),
                }
            }
            Expr::Call { function, args } => {
                // TODO: Check function signature
                let (param_types, ret_type) = if let Some(info) = self.table.lookup(&function.node)
                {
                    if let Type::Function(param_types, ret_type) = &info.ty {
                        (param_types.clone(), ret_type.clone())
                    } else {
                        return Err(SemaError::TypeMismatch {
                            expected: "function".to_string(),
                            found: info.ty.display_name(),
                            span: function.span,
                        });
                    }
                } else {
                    return Err(SemaError::UndefinedSymbol {
                        name: function.node.clone(),
                        span: function.span,
                    });
                };

                if args.len() != param_types.len() {
                    return Err(SemaError::ArityMismatch {
                        expected: param_types.len(),
                        found: args.len(),
                        span: expr.span,
                    });
                }
                for (arg, param_ty) in args.iter().zip(param_types.iter()) {
                    let arg_ty = self.check_expr(arg)?;
                    if &arg_ty != param_ty {
                        return Err(SemaError::TypeMismatch {
                            expected: param_ty.display_name(),
                            found: arg_ty.display_name(),
                            span: arg.span,
                        });
                    }
                }
                Ok(*ret_type)
            }
            Expr::Unary { op, operand } => {
                let operand_ty = self.check_expr(operand)?;

                // Check type compatibility with the operator
                match op {
                    crate::ast::UnaryOp::Neg => {
                        // Negation works on numeric types
                        if operand_ty.is_primitive() {
                            Ok(operand_ty)
                        } else {
                            Err(SemaError::InvalidUnaryOp {
                                op: "-".to_string(),
                                operand_ty: operand_ty.display_name(),
                                span: expr.span,
                            })
                        }
                    }
                    crate::ast::UnaryOp::BitNot => {
                        // Bitwise NOT works on integer types
                        if operand_ty.is_primitive() {
                            Ok(operand_ty)
                        } else {
                            Err(SemaError::InvalidUnaryOp {
                                op: "~".to_string(),
                                operand_ty: operand_ty.display_name(),
                                span: expr.span,
                            })
                        }
                    }
                    crate::ast::UnaryOp::Not => {
                        // Logical NOT returns bool
                        Ok(Type::Primitive(crate::ast::PrimitiveType::Bool))
                    }
                    _ => Ok(operand_ty), // For other operators, preserve type
                }
            }
            Expr::Paren(inner) => self.check_expr(inner),
            Expr::Cast { expr: inner, target_type } => {
                // Check that the inner expression is valid
                self.check_expr(inner)?;
                // Return the target type
                self.resolve_type(&target_type.node)
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
