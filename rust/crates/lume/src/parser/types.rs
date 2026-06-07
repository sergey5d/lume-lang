use super::*;

impl<'a> Parser<'a> {
    pub(super) fn parse_type_params(&mut self) -> Option<Vec<TypeParam>> {
        if !self.match_token(TokenKind::LBracket) {
            return Some(Vec::new());
        }
        let mut params = Vec::new();
        self.skip_newlines();
        if !self.at(TokenKind::RBracket) {
            loop {
                let (name, start) = self.expect_identifier("expected type parameter name")?;
                let bounds = if self.match_keyword(Keyword::With) {
                    self.parse_type_ref_list()?
                } else {
                    Vec::new()
                };
                let end = bounds.last().map(TypeRef::span).unwrap_or(start);
                params.push(TypeParam {
                    name,
                    bounds,
                    span: start.cover(end),
                });
                self.skip_newlines();
                if !self.match_token(TokenKind::Comma) {
                    break;
                }
                self.skip_newlines();
            }
        }
        self.consume(TokenKind::RBracket, "expected ']' after type parameters")?;
        Some(params)
    }

    pub(super) fn parse_param_list(&mut self) -> Option<Vec<Param>> {
        self.consume(TokenKind::LParen, "expected '(' before parameter list")?;
        let mut params = Vec::new();
        self.skip_newlines();
        if !self.at(TokenKind::RParen) {
            loop {
                let (name, start) = self.expect_identifier("expected parameter name")?;
                let ty = if self.can_start_type_ref() {
                    Some(self.parse_type_ref()?)
                } else {
                    None
                };
                let variadic = self.match_token(TokenKind::Ellipsis);
                let end = if variadic {
                    self.previous_span()
                } else {
                    ty.as_ref().map(TypeRef::span).unwrap_or(start)
                };
                params.push(Param {
                    name,
                    ty,
                    variadic,
                    span: start.cover(end),
                });
                self.skip_newlines();
                if !self.match_token(TokenKind::Comma) {
                    break;
                }
                self.skip_newlines();
            }
        }
        self.consume(TokenKind::RParen, "expected ')' after parameters")?;
        Some(params)
    }

    pub(super) fn parse_optional_return_type(&mut self) -> Option<TypeRef> {
        self.skip_newlines();
        if self.can_start_type_ref() {
            let checkpoint = self.checkpoint();
            let ty = self.parse_type_ref();
            if ty.is_none() {
                self.restore(checkpoint);
            }
            ty
        } else {
            None
        }
    }

    pub(super) fn parse_type_ref_list(&mut self) -> Option<Vec<TypeRef>> {
        let mut refs = vec![self.parse_type_ref()?];
        while self.match_token(TokenKind::Comma) {
            refs.push(self.parse_type_ref()?);
        }
        Some(refs)
    }

    pub(super) fn parse_type_ref(&mut self) -> Option<TypeRef> {
        let left = self.parse_primary_type_ref()?;
        if self.match_token(TokenKind::Arrow) {
            let ret = self.parse_type_ref()?;
            let params = match left {
                TypeRef::Tuple { fields, .. } => fields.into_iter().map(|field| field.ty).collect(),
                other => vec![other],
            };
            let span = params
                .first()
                .map(TypeRef::span)
                .unwrap_or(ret.span())
                .cover(ret.span());
            return Some(TypeRef::Function {
                params,
                ret: Box::new(ret),
                span,
            });
        }
        Some(left)
    }

    pub(super) fn parse_primary_type_ref(&mut self) -> Option<TypeRef> {
        self.skip_newlines();
        if self.match_token(TokenKind::LBracket) {
            let start = self.previous_span();
            let inner = self.parse_type_ref()?;
            let end = self.consume(TokenKind::RBracket, "expected ']' after list type")?;
            return Some(TypeRef::Named {
                name: "List".to_string(),
                args: vec![inner],
                span: start.cover(end),
            });
        }
        if self.match_token(TokenKind::LBrace) {
            let start = self.previous_span();
            self.skip_newlines();
            let mut fields = Vec::new();
            if !self.at(TokenKind::RBrace) {
                loop {
                    let (name, name_span) = self.expect_identifier("expected record field name")?;
                    let ty = self.parse_type_ref()?;
                    let span = name_span.cover(ty.span());
                    fields.push(RecordTypeField { name, ty, span });
                    self.skip_newlines();
                    if !self.match_token(TokenKind::Comma) {
                        break;
                    }
                    self.skip_newlines();
                }
            }
            let end = self.consume(TokenKind::RBrace, "expected '}' after record type")?;
            return Some(TypeRef::Record {
                fields,
                span: start.cover(end),
            });
        }
        if self.match_token(TokenKind::LParen) {
            let start = self.previous_span();
            self.skip_newlines();
            let mut fields = Vec::new();
            if !self.at(TokenKind::RParen) {
                fields.push(self.parse_tuple_type_field()?);
                while self.match_token(TokenKind::Comma) {
                    self.skip_newlines();
                    if self.at(TokenKind::RParen) {
                        break;
                    }
                    fields.push(self.parse_tuple_type_field()?);
                }
            }
            let end = self.consume(TokenKind::RParen, "expected ')' after tuple type")?;
            if fields.len() == 1 && fields[0].name.is_none() {
                return Some(fields.into_iter().next()?.ty);
            }
            return Some(TypeRef::Tuple {
                fields,
                span: start.cover(end),
            });
        }

        let (name, start) = self.expect_identifier("expected type name")?;
        let mut args = Vec::new();
        if self.match_token(TokenKind::LBracket) {
            self.skip_newlines();
            if !self.at(TokenKind::RBracket) {
                args.push(self.parse_type_ref()?);
                while self.match_token(TokenKind::Comma) {
                    args.push(self.parse_type_ref()?);
                }
            }
            self.consume(TokenKind::RBracket, "expected ']' after type arguments")?;
        }
        let end = if let Some(last) = args.last() {
            last.span()
        } else {
            start
        };
        Some(TypeRef::Named {
            name,
            args,
            span: start.cover(end),
        })
    }

    pub(super) fn parse_tuple_type_field(&mut self) -> Option<TupleTypeField> {
        let checkpoint = self.checkpoint();
        if self.at(TokenKind::Identifier) && self.at_next(TokenKind::Identifier) {
            let (name, name_span) = self.expect_identifier("expected tuple field name")?;
            let ty = self.parse_type_ref()?;
            let span = name_span.cover(ty.span());
            return Some(TupleTypeField {
                name: Some(name),
                ty,
                span,
            });
        }
        self.restore(checkpoint);
        let ty = self.parse_type_ref()?;
        let span = ty.span();
        Some(TupleTypeField {
            name: None,
            ty,
            span,
        })
    }

    pub(super) fn can_start_type_ref(&self) -> bool {
        matches!(
            self.current_kind(),
            TokenKind::Identifier | TokenKind::LParen | TokenKind::LBrace | TokenKind::LBracket
        )
    }

    pub(super) fn parse_path_string(&mut self) -> Option<String> {
        let (first, _) = self.expect_identifier("expected path segment")?;
        let mut path = first;
        while self.match_token(TokenKind::Slash) {
            let (segment, _) = self.expect_identifier("expected path segment after '/'")?;
            path.push('/');
            path.push_str(&segment);
        }
        Some(path)
    }
}
