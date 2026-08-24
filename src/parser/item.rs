//! Item parsing for the Wraith parser

use crate::ast::{
    AccessMode, AddressDecl, Enum, EnumVariant, FnAttribute, FnParam, Function, Import, Item,
    SourceFile, Span, Spanned, Static, Struct, StructField, TypeExpr,
};
use crate::lexer::Token;

use super::Parser;
use super::error::{ParseError, ParseResult};

/// How an attribute is written, for a diagnostic that names it.
fn attribute_name(attr: &FnAttribute) -> &'static str {
    match attr {
        FnAttribute::Inline => "#[inline]",
        FnAttribute::NoReturn => "#[noreturn]",
        FnAttribute::Interrupt => "#[interrupt]",
        FnAttribute::Nmi => "#[nmi]",
        FnAttribute::Irq => "#[irq]",
        FnAttribute::Reset => "#[reset]",
        FnAttribute::Org(_) => "#[org]",
        FnAttribute::Section(_) => "#[section]",
        FnAttribute::Soa => "#[soa]",
    }
}

impl Parser<'_> {
    /// Parse a complete source file
    pub fn parse_source_file(&mut self) -> ParseResult<SourceFile> {
        let mut items = Vec::with_capacity(self.tokens.len() / 10);

        while self.peek().is_some() {
            let pos_before = self.position();

            match self.parse_item() {
                Ok(item) => items.push(item),
                Err(err) => {
                    // Record error
                    self.record_error(err);

                    if self.error_limit_reached() {
                        break;
                    }

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

        // Parse optional attributes, keeping each one's span: a refusal wants
        // to point at the attribute rather than at the whole declaration.
        let mut attributes = Vec::with_capacity(4);
        let mut attr_spans = Vec::with_capacity(4);
        while self.check(&Token::Hash) {
            let at = self.current_span();
            attributes.push(self.parse_attribute()?);
            attr_spans.push(at.merge(self.previous_span()));
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
                Self::reject_attributes(&attributes, &attr_spans, "an import")?;
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
                Self::reject_attributes(&attributes, &attr_spans, "a struct")?;
                let s = self.parse_struct(attributes, is_pub)?;
                let span = start.merge(self.previous_span());
                Ok(Spanned::new(Item::Struct(s), span))
            }

            Some(Token::Enum) => {
                Self::reject_attributes(&attributes, &attr_spans, "an enum")?;
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
                    // An `addr` names a fixed hardware location; there is no
                    // storage here to lay out, so no attribute applies — not
                    // even the one the other two arms accept.
                    if let Some(at) =
                        Self::storage_attributes(&attributes, &attr_spans, "an addr declaration")?
                    {
                        return Err(ParseError::custom(
                            at,
                            "an addr declaration cannot take #[soa]: it names a fixed location \
                             rather than storage the compiler lays out"
                                .to_string(),
                        ));
                    }
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
                    let soa = Self::storage_attributes(&attributes, &attr_spans, "a const")?;
                    Ok(Spanned::new(
                        Item::Static(Static {
                            name,
                            ty,
                            init,
                            mutable: false,
                            is_pub,
                            soa,
                        }),
                        span,
                    ))
                }
            }

            // Mutable global in RAM: static NAME: T = init;
            // Unlike `const` (immutable ROM data), a static is writable and is
            // shared between functions and interrupt handlers.
            Some(Token::Static) => {
                self.expect(&Token::Static)?;
                let name = self.expect_ident()?;
                self.expect(&Token::Colon)?;
                let ty = self.parse_type()?;
                self.expect(&Token::Eq)?;
                let init = self.parse_expr()?;
                self.expect(&Token::Semi)?;
                let span = start.merge(self.previous_span());

                // `addr` denotes a fixed hardware location, not storage we
                // allocate; it must be declared with `const`.
                if matches!(
                    ty.node,
                    TypeExpr::Primitive(crate::ast::PrimitiveType::Addr)
                ) {
                    return Err(ParseError::custom_detailed(
                        ty.span,
                        "'static' cannot have type 'addr'",
                        Some("Note: an addr names a fixed memory-mapped location".to_string()),
                        Some("Help: declare it with 'const NAME: addr = 0x...;'".to_string()),
                    ));
                }

                let soa = Self::storage_attributes(&attributes, &attr_spans, "a static")?;
                Ok(Spanned::new(
                    Item::Static(Static {
                        name,
                        ty,
                        init,
                        mutable: true,
                        is_pub,
                        soa,
                    }),
                    span,
                ))
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

    /// Parse an import statement.
    ///
    /// ```text
    /// import { sym1, sym2 } from "path.wr";   // named
    /// import { * } from "path.wr";            // glob: every pub item
    /// import * from "path.wr";                // glob, braces optional
    /// import { a, * } from "path.wr";         // both (the names are redundant)
    /// ```
    fn parse_import(&mut self) -> ParseResult<Import> {
        self.expect(&Token::Import)?;

        let mut symbols = Vec::with_capacity(4);
        let mut glob = None;

        // The braces are optional around a bare `*`, matching how `use foo::*`
        // reads in Rust without a brace group.
        if self.check(&Token::Star) {
            let span = self.current_span();
            self.advance();
            glob = Some(span);
        } else {
            self.expect(&Token::LBrace)?;
            loop {
                if self.check(&Token::Star) {
                    // Later `*`s are redundant but harmless; keep the first span.
                    let span = self.current_span();
                    self.advance();
                    glob.get_or_insert(span);
                } else {
                    symbols.push(self.expect_ident()?);
                }

                if !self.check(&Token::Comma) {
                    break;
                }
                self.advance(); // consume comma
            }
            self.expect(&Token::RBrace)?;
        }

        self.expect(&Token::From)?;

        // Parse path as string literal
        let path = self.expect_string()?;

        self.expect(&Token::Semi)?;

        Ok(Import {
            symbols,
            glob,
            path,
        })
    }

    /// The one attribute a `static` or `const` accepts, and a refusal for the
    /// rest.
    ///
    /// Every attribute used to be dropped on the floor here, so `#[inline]
    /// static X: u8 = 1;` compiled and did nothing — the reader had asked for
    /// something and been silently ignored. An attribute that does not apply
    /// is now an error at the attribute.
    fn storage_attributes(
        attributes: &[FnAttribute],
        spans: &[Span],
        what: &str,
    ) -> ParseResult<Option<Span>> {
        let mut soa = None;
        for (attr, span) in attributes.iter().zip(spans) {
            match attr {
                FnAttribute::Soa => {
                    if soa.is_some() {
                        return Err(ParseError::custom(
                            *span,
                            "#[soa] is already on this declaration".to_string(),
                        ));
                    }
                    soa = Some(*span);
                }
                other => {
                    return Err(ParseError::custom(
                        *span,
                        format!("{} cannot take {}", what, attribute_name(other)),
                    ));
                }
            }
        }
        Ok(soa)
    }

    /// No attribute applies to this declaration kind, so reject any that appear.
    ///
    /// `enum`, `import` and `struct` used to discard their attributes the way
    /// `static` did before the storage arms were fixed: `#[inline] enum E {…}`
    /// compiled and did nothing. An attribute is a request, and a request the
    /// compiler cannot honour is an error at the attribute, not a silence.
    fn reject_attributes(
        attributes: &[FnAttribute],
        spans: &[Span],
        what: &str,
    ) -> ParseResult<()> {
        if let Some((attr, span)) = attributes.iter().zip(spans).next() {
            return Err(ParseError::custom(
                *span,
                format!("{} cannot take {}", what, attribute_name(attr)),
            ));
        }
        Ok(())
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
                    "soa" => FnAttribute::Soa,
                    "noreturn" => FnAttribute::NoReturn,
                    "interrupt" => FnAttribute::Interrupt,
                    "nmi" => FnAttribute::Nmi,
                    "irq" => FnAttribute::Irq,
                    "reset" => FnAttribute::Reset,
                    "org" => {
                        self.expect(&Token::LParen)?;
                        let addr = match self.peek().cloned() {
                            Some(Token::Integer(n)) => {
                                let span = self.current_span();
                                self.advance();
                                // `n as u16` truncated: #[org(0x10000)] became
                                // $0000 and failed later for the wrong reason.
                                if !(0..=0xFFFF).contains(&n) {
                                    return Err(ParseError::custom(
                                        span,
                                        format!(
                                            "#[org] address {:#X} is out of range ($0000-$FFFF)",
                                            n
                                        ),
                                    ));
                                }
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
        let mut params = Vec::with_capacity(8);

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

        let mut fields = Vec::with_capacity(8);

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
        let struct_attrs = Vec::with_capacity(0);
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

        let mut variants = Vec::with_capacity(8);

        while !self.check(&Token::RBrace) {
            let variant_name = self.expect_ident()?;

            let variant = if self.check(&Token::LBrace) {
                // Struct variant: Variant { field: type, ... }
                self.advance();
                let mut fields = Vec::with_capacity(4);

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
                let mut fields = Vec::with_capacity(4);

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
