//! Expression parsing for the Wraith parser

use crate::ast::{BinaryOp, Expr, FieldInit, Literal, Spanned, TypeExpr, UnaryOp, VariantData};
use crate::lexer::Token;

use super::Parser;
use super::error::{ParseError, ParseResult};

impl Parser<'_> {
    /// Parse an expression
    pub fn parse_expr(&mut self) -> ParseResult<Spanned<Expr>> {
        self.parse_expr_bp(0)
    }

    /// Parse expression with Pratt parsing (binding power)
    fn parse_expr_bp(&mut self, min_bp: u8) -> ParseResult<Spanned<Expr>> {
        let mut lhs = self.parse_prefix_expr()?;

        loop {
            // Handle postfix operations first: as, field access, indexing
            lhs = self.parse_postfix(lhs)?;

            let op = match self.peek() {
                Some(tok) => tok.clone(),
                None => break,
            };

            // Try to get infix operator binding power
            let (l_bp, r_bp) = match infix_binding_power(&op) {
                Some(bp) => bp,
                None => break,
            };

            if l_bp < min_bp {
                break;
            }

            // Consume the operator
            self.advance();

            // Parse right-hand side
            let rhs = self.parse_expr_bp(r_bp)?;

            let span = lhs.span.merge(rhs.span);
            let bin_op = token_to_binary_op(&op).expect("already checked");

            lhs = Spanned::new(Expr::binary(lhs, bin_op, rhs), span);
        }

        Ok(lhs)
    }

    /// Parse postfix operations: as, field access, indexing
    fn parse_postfix(&mut self, mut expr: Spanned<Expr>) -> ParseResult<Spanned<Expr>> {
        loop {
            if self.check(&Token::Dot) {
                self.advance();
                let field_name = self.expect_ident()?;
                let span = expr.span.merge(self.previous_span());

                // Check for special built-in accessors
                if field_name.node == "len" {
                    expr = Spanned::new(Expr::SliceLen(Box::new(expr)), span);
                } else if field_name.node == "low" {
                    expr = Spanned::new(Expr::U16Low(Box::new(expr)), span);
                } else if field_name.node == "high" {
                    expr = Spanned::new(Expr::U16High(Box::new(expr)), span);
                } else {
                    expr = Spanned::new(
                        Expr::Field {
                            object: Box::new(expr),
                            field: field_name,
                        },
                        span,
                    );
                }
            } else if self.check(&Token::LBracket) {
                self.advance();
                let first = self.parse_expr()?;

                // Check for slice syntax: arr[start..end] or arr[start..=end]
                if self.check(&Token::DotDot) || self.check(&Token::DotDotEq) {
                    let inclusive = self.check(&Token::DotDotEq);
                    self.advance();
                    let end = self.parse_expr()?;
                    self.expect(&Token::RBracket)?;
                    let span = expr.span.merge(self.previous_span());
                    expr = Spanned::new(
                        Expr::Slice {
                            object: Box::new(expr),
                            start: Box::new(first),
                            end: Box::new(end),
                            inclusive,
                        },
                        span,
                    );
                } else {
                    // Normal single index
                    self.expect(&Token::RBracket)?;
                    let span = expr.span.merge(self.previous_span());
                    expr = Spanned::new(
                        Expr::Index {
                            object: Box::new(expr),
                            index: Box::new(first),
                        },
                        span,
                    );
                }
            } else if self.check(&Token::As) {
                self.advance();
                let target_type = self.parse_type()?;
                let span = expr.span.merge(target_type.span);
                expr = Spanned::new(
                    Expr::Cast {
                        expr: Box::new(expr),
                        target_type,
                    },
                    span,
                );
            } else {
                break;
            }
        }
        Ok(expr)
    }

    /// Parse prefix expressions (unary ops, literals, etc.)
    fn parse_prefix_expr(&mut self) -> ParseResult<Spanned<Expr>> {
        let start = self.current_span();

        match self.peek().cloned() {
            // Unary operators
            Some(Token::Minus) => {
                self.advance();
                let operand = self.parse_prefix_expr()?;
                let span = start.merge(operand.span);
                Ok(Spanned::new(Expr::unary(UnaryOp::Neg, operand), span))
            }
            Some(Token::Bang) => {
                self.advance();
                let operand = self.parse_prefix_expr()?;
                let span = start.merge(operand.span);
                Ok(Spanned::new(Expr::unary(UnaryOp::Not, operand), span))
            }
            Some(Token::Tilde) => {
                self.advance();
                let operand = self.parse_prefix_expr()?;
                let span = start.merge(operand.span);
                Ok(Spanned::new(Expr::unary(UnaryOp::BitNot, operand), span))
            }

            // Parenthesized expression
            Some(Token::LParen) => {
                self.advance();
                let inner = self.parse_expr()?;
                self.expect(&Token::RParen)?;
                let span = start.merge(self.previous_span());
                Ok(Spanned::new(Expr::Paren(Box::new(inner)), span))
            }

            // Array literal
            Some(Token::LBracket) => self.parse_array_literal(),

            // Anonymous struct init: { field: value, ... }
            // Distinguished from block by lookahead: { ident: ... }
            Some(Token::LBrace) => {
                let is_anon_struct = match self.peek_ahead(1) {
                    Some(Token::RBrace) => false, // Empty block {}
                    Some(Token::Ident(_)) => {
                        // Check if ident is followed by colon: { field: ... }
                        matches!(self.peek_ahead(2), Some(Token::Colon))
                    }
                    _ => false,
                };

                if is_anon_struct {
                    self.parse_anon_struct_init()
                } else {
                    Err(ParseError::unexpected_token(start, "expression", Some(Token::LBrace)))
                }
            }

            // Literals
            Some(Token::Integer(n)) => {
                self.advance();
                Ok(Spanned::new(Expr::int(n), start))
            }
            Some(Token::True) => {
                self.advance();
                Ok(Spanned::new(Expr::bool(true), start))
            }
            Some(Token::False) => {
                self.advance();
                Ok(Spanned::new(Expr::bool(false), start))
            }

            // CPU status flags (read-only)
            Some(Token::Carry) => {
                self.advance();
                Ok(Spanned::new(Expr::CpuFlagCarry, start))
            }
            Some(Token::Zero) => {
                self.advance();
                Ok(Spanned::new(Expr::CpuFlagZero, start))
            }
            Some(Token::Overflow) => {
                self.advance();
                Ok(Spanned::new(Expr::CpuFlagOverflow, start))
            }
            Some(Token::Negative) => {
                self.advance();
                Ok(Spanned::new(Expr::CpuFlagNegative, start))
            }

            Some(Token::String(s)) => {
                self.advance();
                Ok(Spanned::new(Expr::Literal(Literal::String(s)), start))
            }

            // Identifier (variable, function call, struct init, enum variant)
            Some(Token::Ident(name)) => {
                self.advance();
                self.parse_ident_expr(name, start)
            }

            Some(tok) => Err(ParseError::unexpected_token(start, "expression", Some(tok))),
            None => Err(ParseError::unexpected_eof(start, "expression")),
        }
    }

    /// Parse array literal: [1, 2, 3] or [0; 10]
    fn parse_array_literal(&mut self) -> ParseResult<Spanned<Expr>> {
        let start = self.current_span();
        self.expect(&Token::LBracket)?;

        // Check for empty array
        if self.check(&Token::RBracket) {
            self.advance();
            return Ok(Spanned::new(
                Expr::Literal(Literal::Array(vec![])),
                start.merge(self.previous_span()),
            ));
        }

        let first = self.parse_expr()?;

        // Check if this is array fill syntax: [expr; count]
        if self.check(&Token::Semi) {
            self.advance();
            let count_tok = self.peek().cloned();
            if let Some(Token::Integer(count)) = count_tok {
                self.advance();
                self.expect(&Token::RBracket)?;
                let span = start.merge(self.previous_span());
                return Ok(Spanned::new(
                    Expr::Literal(Literal::ArrayFill {
                        value: Box::new(first),
                        count: count as usize,
                    }),
                    span,
                ));
            } else {
                return Err(ParseError::unexpected_token(
                    self.current_span(),
                    "array size",
                    count_tok,
                ));
            }
        }

        // Regular array literal: [expr, expr, ...]
        let mut elements = vec![first];
        while self.check(&Token::Comma) {
            self.advance();
            if self.check(&Token::RBracket) {
                break; // trailing comma
            }
            elements.push(self.parse_expr()?);
        }

        self.expect(&Token::RBracket)?;
        let span = start.merge(self.previous_span());
        Ok(Spanned::new(Expr::Literal(Literal::Array(elements)), span))
    }

    /// Parse identifier expression (variable, call, struct init, enum variant)
    fn parse_ident_expr(
        &mut self,
        name: String,
        start: crate::ast::Span,
    ) -> ParseResult<Spanned<Expr>> {
        // Check for enum variant: Name::Variant
        if self.check(&Token::ColonColon) {
            return self.parse_enum_variant_expr(name, start);
        }

        // Check for struct init or function call
        // Only parse as struct init if { is followed by ident: or }
        // This avoids incorrectly parsing `match x {` as struct init
        if self.check(&Token::LBrace) {
            // Lookahead to see if this is really a struct init
            let is_struct_init = match self.peek_ahead(1) {
                Some(Token::RBrace) => true, // Empty struct: x {}
                Some(Token::Ident(_)) => {
                    // Check if ident is followed by colon: x { field: ... }
                    matches!(self.peek_ahead(2), Some(Token::Colon))
                }
                _ => false,
            };

            if is_struct_init {
                return self.parse_struct_init(name, start);
            }
        }

        if self.check(&Token::LParen) {
            return self.parse_call_expr(name, start);
        }

        // Just a variable - postfix operations handled by parse_postfix
        Ok(Spanned::new(Expr::var(name), start))
    }

    /// Parse struct initialization: Name { field: value, ... }
    fn parse_struct_init(
        &mut self,
        name: String,
        start: crate::ast::Span,
    ) -> ParseResult<Spanned<Expr>> {
        self.expect(&Token::LBrace)?;

        let mut fields = Vec::new();

        while !self.check(&Token::RBrace) {
            let field_name = self.expect_ident()?;
            self.expect(&Token::Colon)?;
            let value = self.parse_expr()?;

            fields.push(FieldInit {
                name: field_name,
                value,
            });

            if !self.check(&Token::Comma) {
                break;
            }
            self.advance();
        }

        self.expect(&Token::RBrace)?;
        let span = start.merge(self.previous_span());

        Ok(Spanned::new(
            Expr::StructInit {
                name: Spanned::new(name, start),
                fields,
            },
            span,
        ))
    }

    /// Parse anonymous struct initialization: { field: value, ... }
    /// Struct name is inferred from context (e.g., variable type annotation)
    fn parse_anon_struct_init(&mut self) -> ParseResult<Spanned<Expr>> {
        let start = self.current_span();
        self.expect(&Token::LBrace)?;

        let mut fields = Vec::new();

        while !self.check(&Token::RBrace) {
            let field_name = self.expect_ident()?;
            self.expect(&Token::Colon)?;
            let value = self.parse_expr()?;

            fields.push(FieldInit {
                name: field_name,
                value,
            });

            if !self.check(&Token::Comma) {
                break;
            }
            self.advance();
        }

        self.expect(&Token::RBrace)?;
        let span = start.merge(self.previous_span());

        Ok(Spanned::new(Expr::AnonStructInit { fields }, span))
    }

    /// Parse function call: name(args...)
    fn parse_call_expr(
        &mut self,
        name: String,
        start: crate::ast::Span,
    ) -> ParseResult<Spanned<Expr>> {
        self.expect(&Token::LParen)?;

        let mut args = Vec::new();

        while !self.check(&Token::RParen) {
            args.push(self.parse_expr()?);
            if !self.check(&Token::Comma) {
                break;
            }
            self.advance();
        }

        self.expect(&Token::RParen)?;
        let span = start.merge(self.previous_span());

        Ok(Spanned::new(
            Expr::Call {
                function: Spanned::new(name, start),
                args,
            },
            span,
        ))
    }

    /// Parse enum variant: Enum::Variant or Enum::Variant { ... } or Enum::Variant(...)
    fn parse_enum_variant_expr(
        &mut self,
        enum_name: String,
        start: crate::ast::Span,
    ) -> ParseResult<Spanned<Expr>> {
        self.expect(&Token::ColonColon)?;

        let variant_name = self.expect_ident()?;

        let data = if self.check(&Token::LBrace) {
            // Struct variant: Enum::Variant { x: 1, y: 2 }
            // Use lookahead to ensure this is actually a struct variant
            // and not something like `match Status::On { ... }`
            let is_struct_variant = match self.peek_ahead(1) {
                Some(Token::RBrace) => true, // Empty struct variant
                Some(Token::Ident(_)) => {
                    matches!(self.peek_ahead(2), Some(Token::Colon))
                }
                _ => false,
            };

            if !is_struct_variant {
                // Not a struct variant, just a plain enum variant
                VariantData::Unit
            } else {
                self.expect(&Token::LBrace)?;
            let mut fields = Vec::new();

            while !self.check(&Token::RBrace) {
                let field_name = self.expect_ident()?;
                self.expect(&Token::Colon)?;
                let value = self.parse_expr()?;
                fields.push(FieldInit {
                    name: field_name,
                    value,
                });
                if !self.check(&Token::Comma) {
                    break;
                }
                self.advance();
            }
            self.expect(&Token::RBrace)?;

            VariantData::Struct(fields)
            }
        } else if self.check(&Token::LParen) {
            // Tuple variant: Enum::Variant(a, b)
            self.expect(&Token::LParen)?;
            let mut args = Vec::new();

            while !self.check(&Token::RParen) {
                args.push(self.parse_expr()?);
                if !self.check(&Token::Comma) {
                    break;
                }
                self.advance();
            }
            self.expect(&Token::RParen)?;

            VariantData::Tuple(args)
        } else {
            VariantData::Unit
        };

        let span = start.merge(self.previous_span());

        Ok(Spanned::new(
            Expr::EnumVariant {
                enum_name: Spanned::new(enum_name, start),
                variant: variant_name,
                data,
            },
            span,
        ))
    }

    /// Parse a type expression
    pub fn parse_type(&mut self) -> ParseResult<Spanned<TypeExpr>> {
        let start = self.current_span();

        match self.peek().cloned() {
            // Slice type: &[T] (all slices are mutable)
            Some(Token::Amp) => {
                self.advance();
                self.expect(&Token::LBracket)?;
                let element = self.parse_type()?;
                self.expect(&Token::RBracket)?;
                let span = start.merge(self.previous_span());
                // All slices are mutable (no mut keyword in language)
                Ok(Spanned::new(TypeExpr::slice(element, true), span))
            }

            // Array type: [T; N]
            Some(Token::LBracket) => {
                self.advance();
                let element = self.parse_type()?;
                self.expect(&Token::Semi)?;
                let size = match self.peek().cloned() {
                    Some(Token::Integer(n)) => {
                        self.advance();
                        n as usize
                    }
                    tok => {
                        return Err(ParseError::unexpected_token(
                            self.current_span(),
                            "array size",
                            tok,
                        ));
                    }
                };
                self.expect(&Token::RBracket)?;
                let span = start.merge(self.previous_span());
                Ok(Spanned::new(TypeExpr::array(element, size), span))
            }

            // Primitive types
            Some(Token::U8) => {
                self.advance();
                Ok(Spanned::new(
                    TypeExpr::primitive(crate::ast::PrimitiveType::U8),
                    start,
                ))
            }
            Some(Token::I8) => {
                self.advance();
                Ok(Spanned::new(
                    TypeExpr::primitive(crate::ast::PrimitiveType::I8),
                    start,
                ))
            }
            Some(Token::U16) => {
                self.advance();
                Ok(Spanned::new(
                    TypeExpr::primitive(crate::ast::PrimitiveType::U16),
                    start,
                ))
            }
            Some(Token::I16) => {
                self.advance();
                Ok(Spanned::new(
                    TypeExpr::primitive(crate::ast::PrimitiveType::I16),
                    start,
                ))
            }
            Some(Token::Bool) => {
                self.advance();
                Ok(Spanned::new(
                    TypeExpr::primitive(crate::ast::PrimitiveType::Bool),
                    start,
                ))
            }
            Some(Token::B8) => {
                self.advance();
                Ok(Spanned::new(
                    TypeExpr::primitive(crate::ast::PrimitiveType::B8),
                    start,
                ))
            }
            Some(Token::B16) => {
                self.advance();
                Ok(Spanned::new(
                    TypeExpr::primitive(crate::ast::PrimitiveType::B16),
                    start,
                ))
            }
            Some(Token::Addr) => {
                self.advance();
                Ok(Spanned::new(
                    TypeExpr::primitive(crate::ast::PrimitiveType::Addr),
                    start,
                ))
            }
            Some(Token::Str) => {
                self.advance();
                // Use named type that semantic analyzer will recognize
                Ok(Spanned::new(TypeExpr::named("str"), start))
            }

            // Named type
            Some(Token::Ident(name)) => {
                self.advance();
                Ok(Spanned::new(TypeExpr::named(name), start))
            }

            Some(tok) => Err(ParseError::unexpected_token(start, "type", Some(tok))),
            None => Err(ParseError::unexpected_eof(start, "type")),
        }
    }
}

/// Get the binding power for infix operators
fn infix_binding_power(token: &Token) -> Option<(u8, u8)> {
    Some(match token {
        // Logical OR (lowest precedence)
        Token::OrOr => (1, 2),
        // Logical AND
        Token::AndAnd => (3, 4),
        // Bitwise OR
        Token::Pipe => (5, 6),
        // Bitwise XOR
        Token::Caret => (7, 8),
        // Bitwise AND
        Token::Amp => (9, 10),
        // Equality
        Token::EqEq | Token::Ne => (11, 12),
        // Comparison
        Token::Lt | Token::Gt | Token::Le | Token::Ge => (13, 14),
        // Bit shifts
        Token::Shl | Token::Shr => (15, 16),
        // Addition/subtraction
        Token::Plus | Token::Minus => (17, 18),
        // Multiplication/division/modulo
        Token::Star | Token::Slash | Token::Percent => (19, 20),
        _ => return None,
    })
}

/// Convert token to binary operator
fn token_to_binary_op(token: &Token) -> Option<BinaryOp> {
    Some(match token {
        Token::Plus => BinaryOp::Add,
        Token::Minus => BinaryOp::Sub,
        Token::Star => BinaryOp::Mul,
        Token::Slash => BinaryOp::Div,
        Token::Percent => BinaryOp::Mod,
        Token::Amp => BinaryOp::BitAnd,
        Token::Pipe => BinaryOp::BitOr,
        Token::Caret => BinaryOp::BitXor,
        Token::Shl => BinaryOp::Shl,
        Token::Shr => BinaryOp::Shr,
        Token::EqEq => BinaryOp::Eq,
        Token::Ne => BinaryOp::Ne,
        Token::Lt => BinaryOp::Lt,
        Token::Gt => BinaryOp::Gt,
        Token::Le => BinaryOp::Le,
        Token::Ge => BinaryOp::Ge,
        Token::AndAnd => BinaryOp::And,
        Token::OrOr => BinaryOp::Or,
        _ => return None,
    })
}
