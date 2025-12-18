//! Item parsing for the Wraith parser

use crate::ast::{
    AccessMode, AddressDecl, Enum, EnumVariant, FnAttribute, FnParam, Function, Item, SourceFile,
    Spanned, Static, Struct, StructAttribute, StructField,
};
use crate::lexer::Token;

use super::Parser;
use super::error::{ParseError, ParseResult};

impl<'a> Parser<'a> {
    /// Parse a complete source file
    pub fn parse_source_file(&mut self) -> ParseResult<SourceFile> {
        let mut items = Vec::new();

        while self.peek().is_some() {
            items.push(self.parse_item()?);
        }

        Ok(SourceFile::with_items(items))
    }

    /// Parse a top-level item
    pub fn parse_item(&mut self) -> ParseResult<Spanned<Item>> {
        let start = self.current_span();

        // Parse optional attributes
        let mut attributes = Vec::new();
        while self.check(&Token::Hash) {
            attributes.push(self.parse_attribute()?);
        }

        match self.peek().cloned() {
            Some(Token::Fn) | Some(Token::Inline) => {
                let func = self.parse_function(attributes)?;
                let span = start.merge(self.previous_span());
                Ok(Spanned::new(Item::Function(Box::new(func)), span))
            }

            Some(Token::Addr) => {
                let a = self.parse_address_decl()?;
                let span = start.merge(self.previous_span());
                Ok(Spanned::new(Item::Address(a), span))
            }

            Some(Token::Struct) => {
                let s = self.parse_struct(attributes)?;
                let span = start.merge(self.previous_span());
                Ok(Spanned::new(Item::Struct(s), span))
            }

            Some(Token::Enum) => {
                let e = self.parse_enum()?;
                let span = start.merge(self.previous_span());
                Ok(Spanned::new(Item::Enum(e), span))
            }

            // Static/const: type name = value;
            Some(Token::Mut)
            | Some(Token::Zp)
            | Some(Token::U8)
            | Some(Token::I8)
            | Some(Token::U16)
            | Some(Token::I16)
            | Some(Token::Bool)
            | Some(Token::Star)
            | Some(Token::LBracket) => {
                let s = self.parse_static()?;
                let span = start.merge(self.previous_span());
                Ok(Spanned::new(Item::Static(s), span))
            }

            Some(tok) => Err(ParseError::unexpected_token(start, "item", Some(tok))),
            None => Err(ParseError::unexpected_eof(start, "item")),
        }
    }

    /// Parse memory-mapped I/O address declaration
    fn parse_address_decl(&mut self) -> ParseResult<AddressDecl> {
        self.expect(&Token::Addr)?;

        let access = if self.check(&Token::Read) {
            self.advance();
            AccessMode::Read
        } else if self.check(&Token::Write) {
            self.advance();
            AccessMode::Write
        } else {
            AccessMode::ReadWrite
        };

        let name = self.expect_ident()?;

        self.expect(&Token::Eq)?;
        let address = self.parse_expr()?;
        self.expect(&Token::Semi)?;

        Ok(AddressDecl {
            name,
            address,
            access,
        })
    }

    /// Parse an attribute: #[name] or #[name(value)]
    fn parse_attribute(&mut self) -> ParseResult<FnAttribute> {
        self.expect(&Token::Hash)?;
        self.expect(&Token::LBracket)?;

        let name = self.expect_ident()?;

        let attr = match name.node.as_str() {
            "inline" => FnAttribute::Inline,
            "noreturn" => FnAttribute::NoReturn,
            "interrupt" => FnAttribute::Interrupt,
            "org" => {
                self.expect(&Token::LParen)?;
                let addr = match self.peek().cloned() {
                    Some(Token::Integer(n)) => {
                        self.advance();
                        n as u16
                    }
                    tok => {
                        return Err(ParseError::unexpected_token(
                            self.current_span(),
                            "address",
                            tok,
                        ));
                    }
                };
                self.expect(&Token::RParen)?;
                FnAttribute::Org(addr)
            }
            "zp_section" => {
                // This is a struct attribute, but we return it as NoReturn for now
                // In a real impl, we'd have separate attribute types
                FnAttribute::NoReturn
            }
            other => {
                return Err(ParseError::custom(
                    name.span,
                    format!("unknown attribute: {}", other),
                ));
            }
        };

        self.expect(&Token::RBracket)?;
        Ok(attr)
    }

    /// Parse a function definition
    fn parse_function(&mut self, mut attributes: Vec<FnAttribute>) -> ParseResult<Function> {
        let is_inline = self.check(&Token::Inline);
        if is_inline {
            self.advance();
            attributes.push(FnAttribute::Inline);
        }

        self.expect(&Token::Fn)?;

        let name = self.expect_ident()?;

        // Parse parameters
        self.expect(&Token::LParen)?;
        let mut params = Vec::new();

        while !self.check(&Token::RParen) {
            let param_name = self.expect_ident()?;
            self.expect(&Token::Colon)?;
            let ty = self.parse_type()?;

            params.push(FnParam {
                name: param_name,
                ty,
            });

            if !self.check(&Token::Comma) {
                break;
            }
            self.advance();
        }

        self.expect(&Token::RParen)?;

        // Parse optional return type
        let return_type = if self.check(&Token::Arrow) {
            self.advance();
            Some(self.parse_type()?)
        } else {
            None
        };

        // Parse body
        let body = self.parse_block()?;

        Ok(Function {
            name,
            params,
            return_type,
            body,
            attributes,
            is_inline,
        })
    }

    /// Parse a struct definition
    fn parse_struct(&mut self, attributes: Vec<FnAttribute>) -> ParseResult<Struct> {
        self.expect(&Token::Struct)?;

        let name = self.expect_ident()?;
        self.expect(&Token::LBrace)?;

        let mut fields = Vec::new();

        while !self.check(&Token::RBrace) {
            let ty = self.parse_type()?;
            let field_name = self.expect_ident()?;

            fields.push(StructField {
                name: field_name,
                ty,
            });

            if !self.check(&Token::Comma) {
                break;
            }
            self.advance();
        }

        self.expect(&Token::RBrace)?;

        // Convert function attributes to struct attributes
        let struct_attrs = attributes
            .into_iter()
            .filter_map(|a| match a {
                FnAttribute::NoReturn => Some(StructAttribute::ZpSection), // Hack for zp_section
                _ => None,
            })
            .collect();

        Ok(Struct {
            name,
            fields,
            attributes: struct_attrs,
        })
    }

    /// Parse an enum definition
    fn parse_enum(&mut self) -> ParseResult<Enum> {
        self.expect(&Token::Enum)?;

        let name = self.expect_ident()?;
        self.expect(&Token::LBrace)?;

        let mut variants = Vec::new();
        let mut next_value = 0i64;

        while !self.check(&Token::RBrace) {
            let variant_name = self.expect_ident()?;

            let variant = if self.check(&Token::LBrace) {
                // Struct variant: Variant { type field, ... }
                self.advance();
                let mut fields = Vec::new();

                while !self.check(&Token::RBrace) {
                    let ty = self.parse_type()?;
                    let field_name = self.expect_ident()?;
                    fields.push(StructField {
                        name: field_name,
                        ty,
                    });
                    if !self.check(&Token::Comma) {
                        break;
                    }
                    self.advance();
                }
                self.expect(&Token::RBrace)?;

                EnumVariant::Struct {
                    name: variant_name,
                    fields,
                }
            } else if self.check(&Token::LParen) {
                // Tuple variant: Variant(type, ...)
                self.advance();
                let mut fields = Vec::new();

                while !self.check(&Token::RParen) {
                    fields.push(self.parse_type()?);
                    if !self.check(&Token::Comma) {
                        break;
                    }
                    self.advance();
                }
                self.expect(&Token::RParen)?;

                EnumVariant::Tuple {
                    name: variant_name,
                    fields,
                }
            } else {
                // Unit variant
                let value = if self.check(&Token::Eq) {
                    self.advance();
                    match self.peek().cloned() {
                        Some(Token::Integer(n)) => {
                            self.advance();
                            next_value = n + 1;
                            Some(n)
                        }
                        tok => {
                            return Err(ParseError::unexpected_token(
                                self.current_span(),
                                "integer",
                                tok,
                            ));
                        }
                    }
                } else {
                    let v = next_value;
                    next_value += 1;
                    Some(v)
                };

                EnumVariant::Unit {
                    name: variant_name,
                    value,
                }
            };

            variants.push(variant);

            if !self.check(&Token::Comma) {
                break;
            }
            self.advance();
        }

        self.expect(&Token::RBrace)?;

        Ok(Enum { name, variants })
    }

    /// Parse a static/const declaration
    fn parse_static(&mut self) -> ParseResult<Static> {
        let zero_page = self.check(&Token::Zp);
        if zero_page {
            self.advance();
        }

        let mutable = self.check(&Token::Mut);
        if mutable {
            self.advance();
        }

        let ty = self.parse_type()?;
        let name = self.expect_ident()?;

        self.expect(&Token::Eq)?;
        let init = self.parse_expr()?;
        self.expect(&Token::Semi)?;

        Ok(Static {
            name,
            ty,
            init,
            mutable,
            zero_page,
        })
    }
}
