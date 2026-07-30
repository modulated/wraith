//! Statement parsing for the Wraith parser

use crate::ast::{AsmLine, MatchArm, Pattern, PatternBinding, Range, Spanned, Stmt};
use crate::lexer::Token;

use super::Parser;
use super::error::{ParseError, ParseResult};

impl Parser<'_> {
    /// Parse a statement
    pub fn parse_stmt(&mut self) -> ParseResult<Spanned<Stmt>> {
        let start = self.current_span();

        match self.peek().cloned() {
            // Variable declaration: let name: type = expr;
            Some(Token::Let) => self.parse_var_decl(),

            // Control flow
            Some(Token::If) => self.parse_if_stmt(),
            Some(Token::While) => self.parse_while_stmt(),
            Some(Token::Loop) => self.parse_loop_stmt(),
            Some(Token::For) => self.parse_for_stmt(),
            Some(Token::Match) => self.parse_match_stmt(),

            // Jump statements
            Some(Token::Return) => self.parse_return_stmt(),
            Some(Token::Break) => {
                self.advance();
                self.expect(&Token::Semi)?;
                Ok(Spanned::new(Stmt::Break, start.merge(self.previous_span())))
            }
            Some(Token::Continue) => {
                self.advance();
                self.expect(&Token::Semi)?;
                Ok(Spanned::new(
                    Stmt::Continue,
                    start.merge(self.previous_span()),
                ))
            }

            // Inline assembly
            Some(Token::Asm) => self.parse_asm_stmt(),

            // Block
            Some(Token::LBrace) => self.parse_block(),

            // Expression statement or assignment (no more implicit variable declarations)
            _ => self.parse_expr_or_assign_stmt(),
        }
    }

    /// Parse variable declaration: let name: type = expr;
    fn parse_var_decl(&mut self) -> ParseResult<Spanned<Stmt>> {
        let start = self.current_span();

        // Require 'let' keyword for variable declarations
        self.expect(&Token::Let)?;

        // Variables are mutable by default
        let mutable = true;

        // Parse name
        let name = self.expect_ident()?;

        // Expect colon for type annotation (required)
        self.expect(&Token::Colon)?;

        // Parse type (required)
        let ty = self.parse_type()?;

        // Parse initializer (required)
        self.expect(&Token::Eq)?;
        let init = self.parse_expr()?;
        self.expect(&Token::Semi)?;

        let span = start.merge(self.previous_span());

        Ok(Spanned::new(Stmt::var_decl(name, ty, init, mutable), span))
    }

    /// Parse if statement
    fn parse_if_stmt(&mut self) -> ParseResult<Spanned<Stmt>> {
        let start = self.current_span();
        self.expect(&Token::If)?;

        let condition = self.parse_condition_expr()?;
        let then_branch = Box::new(self.parse_block()?);

        let else_branch = if self.check(&Token::Else) {
            self.advance();
            if self.check(&Token::If) {
                Some(Box::new(self.parse_if_stmt()?))
            } else {
                Some(Box::new(self.parse_block()?))
            }
        } else {
            None
        };

        let span = start.merge(self.previous_span());

        Ok(Spanned::new(
            Stmt::If {
                condition,
                then_branch,
                else_branch,
            },
            span,
        ))
    }

    /// Parse while statement
    fn parse_while_stmt(&mut self) -> ParseResult<Spanned<Stmt>> {
        let start = self.current_span();
        self.expect(&Token::While)?;

        let condition = self.parse_condition_expr()?;
        let body = Box::new(self.parse_block()?);

        let span = start.merge(self.previous_span());

        Ok(Spanned::new(Stmt::While { condition, body }, span))
    }

    /// Parse loop statement
    fn parse_loop_stmt(&mut self) -> ParseResult<Spanned<Stmt>> {
        let start = self.current_span();
        self.expect(&Token::Loop)?;

        let body = Box::new(self.parse_block()?);
        let span = start.merge(self.previous_span());

        Ok(Spanned::new(Stmt::Loop { body }, span))
    }

    /// Parse for statement
    /// Supports:
    ///   - for i in 0..10 { }  (range loop)
    ///   - for item in slice { }  (for-each)
    ///   - for (i, item) in slice { }  (for-each with index)
    fn parse_for_stmt(&mut self) -> ParseResult<Spanned<Stmt>> {
        let start = self.current_span();
        self.expect(&Token::For)?;

        // Check for tuple destructuring syntax: for (index, value) in ...
        let (index_var, var_name, var_type) = if self.check(&Token::LParen) {
            self.advance(); // consume '('
            let idx_var = self.expect_ident()?;
            self.expect(&Token::Comma)?;
            let val_var = self.expect_ident()?;
            self.expect(&Token::RParen)?;

            // Type annotation comes after the tuple: for (i, c): u8 in ...
            let var_type = if self.check(&Token::Colon) {
                self.advance();
                Some(self.parse_type()?)
            } else {
                None
            };

            (Some(idx_var), val_var, var_type)
        } else {
            // Simple variable: for i in ...
            let name = self.expect_ident()?;

            // Check for optional type annotation: for i: u8 in ...
            let var_type = if self.check(&Token::Colon) {
                self.advance();
                Some(self.parse_type()?)
            } else {
                None
            };

            (None, name, var_type)
        };

        self.expect(&Token::In)?;

        // Check if iterating over a range or a slice
        let first_expr = self.parse_condition_expr()?;

        // Check for range syntax: start..end or start..=end
        if self.check(&Token::DotDot) || self.check(&Token::DotDotEq) {
            let inclusive = self.check(&Token::DotDotEq);
            self.advance();

            let end = self.parse_condition_expr()?;
            let body = Box::new(self.parse_block()?);
            let span = start.merge(self.previous_span());

            Ok(Spanned::new(
                Stmt::For {
                    var_name,
                    var_type,
                    range: Range {
                        start: first_expr,
                        end,
                        inclusive,
                    },
                    body,
                },
                span,
            ))
        } else {
            // Iterating over slice/array/string
            let body = Box::new(self.parse_block()?);
            let span = start.merge(self.previous_span());

            Ok(Spanned::new(
                Stmt::ForEach {
                    var_name,
                    var_type,
                    iterable: first_expr,
                    body,
                    index_var,
                },
                span,
            ))
        }
    }

    /// Parse match statement
    fn parse_match_stmt(&mut self) -> ParseResult<Spanned<Stmt>> {
        let start = self.current_span();
        self.expect(&Token::Match)?;

        let expr = self.parse_condition_expr()?;
        self.expect(&Token::LBrace)?;

        let mut arms = Vec::with_capacity(4);
        while !self.check(&Token::RBrace) {
            let pattern = self.parse_pattern()?;
            self.expect(&Token::FatArrow)?;

            // Parse body - either a block or a single expression
            let body = if self.check(&Token::LBrace) {
                // Full block: { statements }
                Box::new(self.parse_block()?)
            } else {
                // Single expression - wrap in a block
                let expr_start = self.current_span();
                let expr = self.parse_expr()?;
                let expr_span = expr_start.merge(self.previous_span());
                let expr_stmt = Spanned::new(Stmt::Expr(expr), expr_span);
                Box::new(Spanned::new(Stmt::block(vec![expr_stmt]), expr_span))
            };

            arms.push(MatchArm { pattern, body });

            // Optional comma between arms
            if self.check(&Token::Comma) {
                self.advance();
            }
        }

        self.expect(&Token::RBrace)?;
        let span = start.merge(self.previous_span());

        Ok(Spanned::new(Stmt::Match { expr, arms }, span))
    }

    /// Parse a pattern
    pub fn parse_pattern(&mut self) -> ParseResult<Spanned<Pattern>> {
        let start = self.current_span();

        match self.peek().cloned() {
            // Wildcard
            Some(Token::Ident(ref name)) if name == "_" => {
                self.advance();
                Ok(Spanned::new(Pattern::Wildcard, start))
            }

            // Enum variant or variable
            Some(Token::Ident(name)) => {
                self.advance();

                if self.check(&Token::ColonColon) {
                    // Enum variant pattern
                    self.advance();
                    let variant = self.expect_ident()?;

                    let bindings = if self.check(&Token::LBrace) {
                        self.advance();
                        let mut bindings = Vec::with_capacity(4);
                        while !self.check(&Token::RBrace) {
                            bindings.push(PatternBinding {
                                name: self.expect_ident()?,
                            });
                            if !self.check(&Token::Comma) {
                                break;
                            }
                            self.advance();
                        }
                        self.expect(&Token::RBrace)?;
                        bindings
                    } else if self.check(&Token::LParen) {
                        self.advance();
                        let mut bindings = Vec::with_capacity(4);
                        while !self.check(&Token::RParen) {
                            bindings.push(PatternBinding {
                                name: self.expect_ident()?,
                            });
                            if !self.check(&Token::Comma) {
                                break;
                            }
                            self.advance();
                        }
                        self.expect(&Token::RParen)?;
                        bindings
                    } else {
                        Vec::new()
                    };

                    let span = start.merge(self.previous_span());
                    Ok(Spanned::new(
                        Pattern::EnumVariant {
                            enum_name: Spanned::new(name, start),
                            variant,
                            bindings,
                        },
                        span,
                    ))
                } else {
                    // Variable binding
                    Ok(Spanned::new(Pattern::Variable(name), start))
                }
            }

            // Integer literal pattern, possibly negative (e.g. `-128..=-1`).
            Some(Token::Integer(_)) | Some(Token::Minus) => {
                let expr = self.parse_pattern_bound()?;

                // Check for range pattern
                if self.check(&Token::DotDotEq) {
                    self.advance();
                    let end = self.parse_pattern_bound()?;
                    let span = start.merge(end.span);
                    Ok(Spanned::new(
                        Pattern::Range {
                            start: expr,
                            end,
                            inclusive: true,
                        },
                        span,
                    ))
                } else {
                    Ok(Spanned::new(Pattern::Literal(expr), start))
                }
            }

            Some(tok) => Err(ParseError::unexpected_token(start, "pattern", Some(tok))),
            None => Err(ParseError::unexpected_eof(start, "pattern")),
        }
    }

    /// Parse a numeric bound of a literal/range pattern, folding an optional
    /// leading `-` into a negative integer literal. Downstream match codegen
    /// pattern-matches on `Expr::Literal(Integer(_))`, so a negative bound must
    /// arrive already folded rather than as `Unary(Neg, ..)`.
    fn parse_pattern_bound(&mut self) -> ParseResult<Spanned<crate::ast::Expr>> {
        let start = self.current_span();
        let negative = if self.check(&Token::Minus) {
            self.advance();
            true
        } else {
            false
        };
        match self.peek().cloned() {
            Some(Token::Integer(n)) => {
                self.advance();
                let val = if negative { -n } else { n };
                let span = start.merge(self.previous_span());
                Ok(Spanned::new(
                    crate::ast::Expr::Literal(crate::ast::Literal::Integer(val)),
                    span,
                ))
            }
            other => Err(ParseError::unexpected_token(
                start,
                "integer in pattern",
                other,
            )),
        }
    }

    /// Parse return statement
    fn parse_return_stmt(&mut self) -> ParseResult<Spanned<Stmt>> {
        let start = self.current_span();
        self.expect(&Token::Return)?;

        let value = if !self.check(&Token::Semi) {
            Some(self.parse_expr()?)
        } else {
            None
        };

        self.expect(&Token::Semi)?;
        let span = start.merge(self.previous_span());

        Ok(Spanned::new(Stmt::Return(value), span))
    }

    /// Parse inline assembly block
    fn parse_asm_stmt(&mut self) -> ParseResult<Spanned<Stmt>> {
        let start = self.current_span();
        self.expect(&Token::Asm)?;
        self.expect(&Token::LBrace)?;

        let mut lines = Vec::with_capacity(8);

        while !self.check(&Token::RBrace) {
            if let Some(Token::String(s)) = self.peek().cloned() {
                self.advance();
                lines.push(AsmLine { instruction: s });

                // Require comma after each assembly line
                // Allow optional trailing comma before closing brace
                if !self.check(&Token::RBrace) {
                    self.expect(&Token::Comma)?;
                } else if self.check(&Token::Comma) {
                    // Consume optional trailing comma
                    self.advance();
                }
            } else {
                return Err(ParseError::unexpected_token(
                    self.current_span(),
                    "assembly string",
                    self.peek().cloned(),
                ));
            }
        }

        self.expect(&Token::RBrace)?;
        let span = start.merge(self.previous_span());

        Ok(Spanned::new(Stmt::Asm { lines }, span))
    }

    /// Parse a block of statements
    pub fn parse_block(&mut self) -> ParseResult<Spanned<Stmt>> {
        let start = self.current_span();
        self.expect(&Token::LBrace)?;

        // Statement-level error recovery: a bad statement is recorded and
        // skipped to the next recovery point, so one typo doesn't cascade
        // into a bogus error per remaining token (the item-level recovery
        // alone synchronized *into* the body and treated each statement as
        // a malformed item).
        let mut stmts = Vec::with_capacity(8);
        while !self.check(&Token::RBrace) {
            if self.peek().is_none() {
                break; // let the expect(RBrace) below report the EOF
            }
            if self.error_limit_reached() {
                return Err(ParseError::custom(
                    self.current_span(),
                    "too many parse errors; giving up on this block",
                ));
            }
            let pos_before = self.position();
            match self.parse_stmt() {
                Ok(stmt) => stmts.push(stmt),
                Err(err) => {
                    self.record_error(err);
                    // Ensure progress, mirroring the item-level recovery.
                    if self.position() == pos_before {
                        self.synchronize();
                        if self.position() == pos_before
                            && self.peek().is_some()
                            && !self.check(&Token::RBrace)
                        {
                            self.advance();
                        }
                    } else {
                        self.synchronize();
                    }
                }
            }
        }

        self.expect(&Token::RBrace)?;
        let span = start.merge(self.previous_span());

        Ok(Spanned::new(Stmt::block(stmts), span))
    }

    /// Parse expression statement or assignment
    fn parse_expr_or_assign_stmt(&mut self) -> ParseResult<Spanned<Stmt>> {
        let start = self.current_span();
        let expr = self.parse_expr()?;

        // Check for assignment operators
        if let Some(op_token) = self.peek().cloned() {
            let compound_op = match op_token {
                Token::Eq => None,
                Token::PlusEq => Some(crate::ast::BinaryOp::Add),
                Token::MinusEq => Some(crate::ast::BinaryOp::Sub),
                Token::StarEq => Some(crate::ast::BinaryOp::Mul),
                Token::SlashEq => Some(crate::ast::BinaryOp::Div),
                Token::PercentEq => Some(crate::ast::BinaryOp::Mod),
                Token::AmpEq => Some(crate::ast::BinaryOp::BitAnd),
                Token::PipeEq => Some(crate::ast::BinaryOp::BitOr),
                Token::CaretEq => Some(crate::ast::BinaryOp::BitXor),
                Token::ShlEq => Some(crate::ast::BinaryOp::Shl),
                Token::ShrEq => Some(crate::ast::BinaryOp::Shr),
                _ => None,
            };

            if compound_op.is_some() || matches!(op_token, Token::Eq) {
                self.advance();
                let right_value = self.parse_expr()?;
                self.expect(&Token::Semi)?;
                let span = start.merge(self.previous_span());

                // Expand compound assignments: x += y becomes x = x + y
                let value = if let Some(op) = compound_op {
                    // The desugar evaluates the target twice. For a pure
                    // target (a variable, an index chain of pure
                    // expressions) that is only a few wasted cycles, but a
                    // call in the target runs its side effects twice —
                    // `arr[idx()] += 5` called idx() three times in the
                    // emitted code. Reject that shape loudly instead of
                    // building evaluate-once machinery for it.
                    if let Some(call_span) = find_call_span(&expr) {
                        return Err(ParseError::custom(
                            call_span,
                            "compound assignment would evaluate this call twice; \
                             bind it first, e.g. `let _i = idx(); arr[_i] = arr[_i] + v;`",
                        ));
                    }
                    let left_clone = expr.clone();
                    Spanned::new(
                        crate::ast::Expr::Binary {
                            left: Box::new(left_clone),
                            op,
                            right: Box::new(right_value),
                        },
                        span,
                    )
                } else {
                    right_value
                };

                return Ok(Spanned::new(
                    Stmt::Assign {
                        target: expr,
                        value,
                    },
                    span,
                ));
            }
        }

        self.expect(&Token::Semi)?;
        let span = start.merge(self.previous_span());
        Ok(Spanned::new(Stmt::expr(expr), span))
    }
}

/// The span of the first call (direct or indirect) anywhere in an lvalue
/// chain, if there is one. Used to reject compound assignments whose desugar
/// would run the call twice.
fn find_call_span(expr: &crate::ast::Spanned<crate::ast::Expr>) -> Option<crate::ast::Span> {
    use crate::ast::Expr;
    match &expr.node {
        Expr::Call { .. } | Expr::CallIndirect { .. } => Some(expr.span),
        Expr::Binary { left, right, .. } => find_call_span(left).or_else(|| find_call_span(right)),
        Expr::Unary { operand, .. } => find_call_span(operand),
        Expr::Paren(inner) => find_call_span(inner),
        Expr::Cast { expr: inner, .. } => find_call_span(inner),
        Expr::Index { object, index } => find_call_span(object).or_else(|| find_call_span(index)),
        Expr::Field { object, .. } => find_call_span(object),
        Expr::Slice {
            object, start, end, ..
        } => find_call_span(object)
            .or_else(|| find_call_span(start))
            .or_else(|| find_call_span(end)),
        Expr::SliceLen(inner) | Expr::U16Low(inner) | Expr::U16High(inner) => find_call_span(inner),
        _ => None,
    }
}
