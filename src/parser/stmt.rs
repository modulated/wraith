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
            // Variable declaration with modifiers: zp name: type = expr;
            Some(Token::Zp) => self.parse_var_decl(),

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

            // Identifier - could be variable declaration (name: type = ...) or expression/assignment
            Some(Token::Ident(_)) => {
                // Lookahead to check if this is a variable declaration
                if self.peek_ahead(1) == Some(&Token::Colon) {
                    self.parse_var_decl()
                } else {
                    self.parse_expr_or_assign_stmt()
                }
            }

            // Expression statement or assignment
            _ => self.parse_expr_or_assign_stmt(),
        }
    }

    /// Parse variable declaration: name: type = expr; or mut name: type = expr;
    fn parse_var_decl(&mut self) -> ParseResult<Spanned<Stmt>> {
        let start = self.current_span();

        // Parse optional zero page modifier
        let zero_page = self.check(&Token::Zp);
        if zero_page {
            self.advance();
        }

        // Variables are mutable by default
        let mutable = true;

        // Parse name
        let name = self.expect_ident()?;

        // Expect colon
        self.expect(&Token::Colon)?;

        // Parse type
        let ty = self.parse_type()?;

        // Parse initializer
        self.expect(&Token::Eq)?;
        let init = self.parse_expr()?;
        self.expect(&Token::Semi)?;

        let span = start.merge(self.previous_span());

        Ok(Spanned::new(
            Stmt::var_decl(name, ty, init, mutable, zero_page),
            span,
        ))
    }

    /// Parse if statement
    fn parse_if_stmt(&mut self) -> ParseResult<Spanned<Stmt>> {
        let start = self.current_span();
        self.expect(&Token::If)?;

        let condition = self.parse_expr()?;
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

        let condition = self.parse_expr()?;
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
    fn parse_for_stmt(&mut self) -> ParseResult<Spanned<Stmt>> {
        let start = self.current_span();
        self.expect(&Token::For)?;

        // Parse loop variable name
        let var_name = self.expect_ident()?;

        // Check for optional type annotation: for i: u8 in ...
        let var_type = if self.check(&Token::Colon) {
            self.advance();
            Some(self.parse_type()?)
        } else {
            None
        };

        self.expect(&Token::In)?;

        // Check if iterating over a range or a slice
        let first_expr = self.parse_expr()?;

        // Check for range syntax: start..end or start..=end
        if self.check(&Token::DotDot) || self.check(&Token::DotDotEq) {
            let inclusive = self.check(&Token::DotDotEq);
            self.advance();

            let end = self.parse_expr()?;
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
            // Iterating over slice/array
            let body = Box::new(self.parse_block()?);
            let span = start.merge(self.previous_span());

            Ok(Spanned::new(
                Stmt::ForEach {
                    var_name,
                    var_type,
                    iterable: first_expr,
                    body,
                },
                span,
            ))
        }
    }

    /// Parse match statement
    fn parse_match_stmt(&mut self) -> ParseResult<Spanned<Stmt>> {
        let start = self.current_span();
        self.expect(&Token::Match)?;

        let expr = self.parse_expr()?;
        self.expect(&Token::LBrace)?;

        let mut arms = Vec::new();
        while !self.check(&Token::RBrace) {
            let pattern = self.parse_pattern()?;
            self.expect(&Token::FatArrow)?;
            let body = Box::new(self.parse_block()?);

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
    fn parse_pattern(&mut self) -> ParseResult<Spanned<Pattern>> {
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
                        let mut bindings = Vec::new();
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
                        let mut bindings = Vec::new();
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

            // Integer literal pattern
            Some(Token::Integer(_)) => {
                let expr = self.parse_expr()?;

                // Check for range pattern
                if self.check(&Token::DotDotEq) {
                    self.advance();
                    let end = self.parse_expr()?;
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

        let mut lines = Vec::new();

        while !self.check(&Token::RBrace) {
            if let Some(Token::String(s)) = self.peek().cloned() {
                self.advance();
                lines.push(AsmLine { instruction: s });
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

        let mut stmts = Vec::new();
        while !self.check(&Token::RBrace) {
            stmts.push(self.parse_stmt()?);
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
