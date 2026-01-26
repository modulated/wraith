//! Item parsing for the Wraith parser

use crate::ast::{
    AccessMode, AddressDecl, Enum, EnumVariant, FnAttribute, FnParam, Function, Import, Item,
    SourceFile, Spanned, Static, Struct, StructField, TypeExpr,
};
use crate::lexer::Token;

use super::Parser;
use super::error::{ParseError, ParseResult};

impl Parser<'_> {
    /// Parse a complete source file
    pub fn parse_source_file(&mut self) -> ParseResult<SourceFile> {
        let mut items = Vec::new();

        while self.peek().is_some() {
            let pos_before = self.position();

            match self.parse_item() {
                Ok(item) => items.push(item),
                Err(err) => {
                    // Record error
                    self.record_error(err);

                    // Ensure we make progress to avoid infinite loops
                    if self.position() == pos_before {
                        // Parser didn't advance, manually skip to next potential item start
                        self.synchronize();

                        // If still stuck, forcefully advance one token
                        if self.position() == pos_before && self.peek().is_some() {
                            self.advance();
                        }
                    } else {
                        // Parser did advance, just synchronize
                        self.synchronize();
                    }

                    // If at EOF after synchronization, stop
                    if self.peek().is_none() {
                        break;
                    }
                }
            }
        }

        // If we collected any errors, return them all
        if self.has_errors() {
            return Err(ParseError::multiple(self.errors.clone()));
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

        // Parse optional 'pub' keyword
        let is_pub = if self.check(&Token::Pub) {
            self.advance();
            true
        } else {
            false
        };

        match self.peek().cloned() {
            Some(Token::Import) => {
                let import = self.parse_import()?;
                let span = start.merge(self.previous_span());
                Ok(Spanned::new(Item::Import(import), span))
            }

            Some(Token::Fn) => {
                let func = self.parse_function(attributes, is_pub)?;
                let span = start.merge(self.previous_span());
                Ok(Spanned::new(Item::Function(Box::new(func)), span))
            }

            Some(Token::Struct) => {
                let s = self.parse_struct(attributes, is_pub)?;
                let span = start.merge(self.previous_span());
                Ok(Spanned::new(Item::Struct(s), span))
            }

            Some(Token::Enum) => {
                let e = self.parse_enum(is_pub)?;
                let span = start.merge(self.previous_span());
                Ok(Spanned::new(Item::Enum(e), span))
            }

            // Static/const/address: const NAME: [read|write] type = value;
            Some(Token::Const) => {
                self.expect(&Token::Const)?;
                let name = self.expect_ident()?;
                self.expect(&Token::Colon)?;

                // Check for optional access modifier (read/write) before type
                let access = if self.check(&Token::Read) {
                    self.advance();
                    AccessMode::Read
                } else if self.check(&Token::Write) {
                    self.advance();
                    AccessMode::Write
                } else {
                    AccessMode::ReadWrite
                };

                let ty = self.parse_type()?;
                self.expect(&Token::Eq)?;
                let init = self.parse_expr()?;
                self.expect(&Token::Semi)?;

                let span = start.merge(self.previous_span());

                // Check if this is an address declaration (type is addr)
                if matches!(
                    ty.node,
                    TypeExpr::Primitive(crate::ast::PrimitiveType::Addr)
                ) {
                    Ok(Spanned::new(
                        Item::Address(AddressDecl {
                            name,
                            address: init,
                            access,
                            is_pub,
                        }),
                        span,
                    ))
                } else {
                    // Access modifiers are only valid for addr types
                    if access != AccessMode::ReadWrite {
                        return Err(ParseError::custom(
                            ty.span,
                            "access modifiers (read/write) are only valid for addr types"
                                .to_string(),
                        ));
                    }
                    Ok(Spanned::new(
                        Item::Static(Static {
                            name,
                            ty,
                            init,
                            mutable: false,
                            is_pub,
                        }),
                        span,
                    ))
                }
            }

            // Detect 'let' at global scope and provide helpful error message
            Some(Token::Let) => {
                let err_span = self.current_span();
                // Advance past 'let'
                self.advance();

                // Consume tokens until we find a semicolon to prevent cascading errors
                loop {
                    match self.peek() {
                        Some(Token::Semi) => {
                            self.advance(); // consume the semicolon
                            break;
                        }
                        None => break,       // EOF
                        _ => self.advance(), // keep consuming
                    }
                }

                Err(ParseError::custom_detailed(
                    err_span,
                    "cannot use 'let' at global scope",
                    Some("Note: 'let' is only for local variables inside functions".to_string()),
                    Some("Help: Use 'const' for global constants and addresses.".to_string()),
                ))
            }

            Some(tok) => Err(ParseError::unexpected_token(start, "item", Some(tok))),
            None => Err(ParseError::unexpected_eof(start, "item")),
        }
    }

    /// Parse import statement: import {sym1, sym2} from 'path.wr';
    fn parse_import(&mut self) -> ParseResult<Import> {
        self.expect(&Token::Import)?;
        self.expect(&Token::LBrace)?;

        // Parse comma-separated list of symbols
        let mut symbols = Vec::new();
        loop {
            let sym = self.expect_ident()?;
            symbols.push(sym);

            if !self.check(&Token::Comma) {
                break;
            }
            self.advance(); // consume comma
        }

        self.expect(&Token::RBrace)?;
        self.expect(&Token::From)?;

        // Parse path as string literal
        let path = self.expect_string()?;

        self.expect(&Token::Semi)?;

        Ok(Import { symbols, path })
    }

    /// Parse an attribute: #[name] or #[name(value)]
    fn parse_attribute(&mut self) -> ParseResult<FnAttribute> {
        self.expect(&Token::Hash)?;
        self.expect(&Token::LBracket)?;

        // Handle identifiers as attribute names
        let attr = match self.peek().cloned() {
            Some(Token::Ident(name)) => {
                let name_span = self.current_span();
                self.advance();
                match name.as_str() {
                    "inline" => FnAttribute::Inline,
                    "noreturn" => FnAttribute::NoReturn,
                    "interrupt" => FnAttribute::Interrupt,
                    "nmi" => FnAttribute::Nmi,
                    "irq" => FnAttribute::Irq,
                    "reset" => FnAttribute::Reset,
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
                    "section" => {
                        self.expect(&Token::LParen)?;
                        let section_name = match self.peek().cloned() {
                            Some(Token::String(s)) => {
                                self.advance();
                                s
                            }
                            tok => {
                                return Err(ParseError::unexpected_token(
                                    self.current_span(),
                                    "section name (string)",
                                    tok,
                                ));
                            }
                        };
                        self.expect(&Token::RParen)?;
                        FnAttribute::Section(section_name)
                    }
                    other => {
                        return Err(ParseError::custom(
                            name_span,
                            format!("unknown attribute: {}", other),
                        ));
                    }
                }
            }
            tok => {
                return Err(ParseError::unexpected_token(
                    self.current_span(),
                    "attribute name",
                    tok,
                ));
            }
        };

        self.expect(&Token::RBracket)?;
        Ok(attr)
    }

    /// Parse a function definition
    fn parse_function(
        &mut self,
        attributes: Vec<FnAttribute>,
        is_pub: bool,
    ) -> ParseResult<Function> {
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
            is_pub,
        })
    }

    /// Parse a struct definition
    fn parse_struct(&mut self, attributes: Vec<FnAttribute>, is_pub: bool) -> ParseResult<Struct> {
        self.expect(&Token::Struct)?;

        let name = self.expect_ident()?;
        self.expect(&Token::LBrace)?;

        let mut fields = Vec::new();

        while !self.check(&Token::RBrace) {
            let field_name = self.expect_ident()?;
            self.expect(&Token::Colon)?;
            let ty = self.parse_type()?;

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

        // Convert function attributes to struct attributes (currently no supported attributes)
        let struct_attrs = Vec::new();
        // Avoid unused variable warning
        let _ = attributes;

        Ok(Struct {
            name,
            fields,
            attributes: struct_attrs,
            is_pub,
        })
    }

    /// Parse an enum definition
    fn parse_enum(&mut self, is_pub: bool) -> ParseResult<Enum> {
        self.expect(&Token::Enum)?;

        let name = self.expect_ident()?;
        self.expect(&Token::LBrace)?;

        let mut variants = Vec::new();

        while !self.check(&Token::RBrace) {
            let variant_name = self.expect_ident()?;

            let variant = if self.check(&Token::LBrace) {
                // Struct variant: Variant { field: type, ... }
                self.advance();
                let mut fields = Vec::new();

                while !self.check(&Token::RBrace) {
                    let field_name = self.expect_ident()?;
                    self.expect(&Token::Colon)?;
                    let ty = self.parse_type()?;
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
                // Only set explicit value if `= number` syntax is used
                // Otherwise let semantic analysis assign sequential tags
                let value = if self.check(&Token::Eq) {
                    self.advance();
                    match self.peek().cloned() {
                        Some(Token::Integer(n)) => {
                            self.advance();
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
                    // No explicit value - let semantic analyzer assign tag
                    None
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

        Ok(Enum {
            name,
            variants,
            is_pub,
        })
    }
}
