use super::*;

impl<'a> Parser<'a> {
    pub(super) fn parse_type_params(&mut self) -> Option<Vec<TypeParam>> {
        self.parse_generic_clause().map(|clause| clause.params)
    }

    pub(super) fn parse_generic_clause(&mut self) -> Option<ParsedGenericClause> {
        if !self.match_token(TokenKind::LBracket) {
            return Some(ParsedGenericClause::default());
        }
        let mut params = Vec::new();
        let mut conditions = Vec::new();
        self.skip_newlines();
        while !self.at(TokenKind::RBracket) && !self.at(TokenKind::Eof) {
            if self.match_keyword(Keyword::When) {
                self.skip_newlines();
                if self.at(TokenKind::RBracket) {
                    self.error_at_current(
                        "missing_generic_condition",
                        "expected a bound or equality condition after 'when'",
                    );
                    return None;
                }
                loop {
                    if self.match_keyword(Keyword::When) {
                        self.diagnostics.push(Diagnostic::error(
                            "duplicate_when_clause",
                            "a generic clause may contain only one 'when'; separate conditions with commas",
                            self.previous_span(),
                        ));
                    }
                    let left = self.parse_type_ref()?;
                    let start = left.span();
                    if self.match_keyword(Keyword::With) {
                        let bound = self.parse_type_ref()?;
                        let span = start.cover(bound.span());
                        conditions.push(GenericCondition::Bound {
                            subject: left,
                            bound,
                            span,
                        });
                    } else if self.match_token(TokenKind::Eq) {
                        let right = self.parse_type_ref()?;
                        let span = start.cover(right.span());
                        conditions.push(GenericCondition::Equal { left, right, span });
                    } else {
                        self.error_at_current(
                            "invalid_generic_condition",
                            "generic conditions use 'Type with Interface' or 'Left = Right'",
                        );
                        return None;
                    }
                    self.skip_newlines();
                    if !self.match_token(TokenKind::Comma) {
                        break;
                    }
                    self.skip_newlines();
                    if self.at(TokenKind::RBracket) {
                        break;
                    }
                }
                break;
            } else {
                let reified = self.match_keyword(Keyword::Reified);
                let reified_span = reified.then(|| self.previous_span());
                let (name, start) = self.expect_identifier("expected type parameter name")?;
                let mut bounds = Vec::new();
                while self.match_keyword(Keyword::With) {
                    bounds.push(self.parse_type_ref()?);
                }
                let end = bounds.last().map(TypeRef::span).unwrap_or(start);
                params.push(TypeParam {
                    name,
                    reified,
                    bounds,
                    span: reified_span.unwrap_or(start).cover(end),
                });
                self.skip_newlines();
                if self.at_keyword(Keyword::When) {
                    continue;
                }
                if !self.match_token(TokenKind::Comma) {
                    break;
                }
                self.skip_newlines();
            }
        }
        self.consume(TokenKind::RBracket, "expected ']' after type parameters")?;
        Some(ParsedGenericClause { params, conditions })
    }

    pub(super) fn parse_param_list(&mut self) -> Option<Vec<Param>> {
        self.consume(TokenKind::LParen, "expected '(' before parameter list")?;
        let mut params = Vec::new();
        self.skip_newlines();
        if !self.at(TokenKind::RParen) {
            loop {
                let prefix_vararg = if self.match_keyword(Keyword::Vararg) {
                    let span = self.previous_span();
                    self.diagnostics.push(Diagnostic::error(
                        "invalid_variadic_param",
                        "vararg must follow the parameter type, like 'args [T] vararg'",
                        span,
                    ));
                    Some(span)
                } else {
                    None
                };
                let (name, start) = self.expect_identifier("expected parameter name")?;
                let (lazy, ty) = if self.match_token(TokenKind::FatArrow) {
                    (true, Some(self.parse_type_ref()?))
                } else if self.can_start_type_ref() {
                    (false, Some(self.parse_type_ref()?))
                } else {
                    (false, None)
                };
                let postfix_vararg = self.match_keyword(Keyword::Vararg);
                let postfix_span = postfix_vararg.then(|| self.previous_span());
                let variadic = prefix_vararg.is_some() || postfix_vararg;
                let initializer = if self.match_token(TokenKind::Eq) {
                    Some(self.parse_expr()?)
                } else {
                    None
                };
                let end = initializer
                    .as_ref()
                    .map(Expr::span)
                    .or(postfix_span)
                    .or_else(|| ty.as_ref().map(TypeRef::span))
                    .unwrap_or(start);
                let span = prefix_vararg.unwrap_or(start).cover(end);
                params.push(Param {
                    name,
                    ty,
                    initializer,
                    variadic,
                    lazy,
                    span,
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

    pub(super) fn parse_interface_ref_list_after_with(&mut self) -> Option<Vec<TypeRef>> {
        let mut refs = self.parse_type_ref_list()?;
        while self.match_keyword(Keyword::With) {
            self.diagnostics.push(Diagnostic::error(
                "repeated_interface_with",
                "interface lists use one 'with' followed by comma-separated types",
                self.previous_span(),
            ));
            refs.extend(self.parse_type_ref_list()?);
        }
        Some(refs)
    }

    pub(super) fn parse_type_ref(&mut self) -> Option<TypeRef> {
        self.skip_newlines();
        let left = if self.at_keyword(Keyword::Fn) {
            self.parse_function_type_ref()?
        } else if self.at(TokenKind::LParen) {
            self.parse_parenthesized_or_function_type_ref()?
        } else {
            self.parse_primary_type_ref()?
        };
        let left = self.parse_optional_type_suffix(left);
        if self.at(TokenKind::FatArrow) || self.at(TokenKind::Arrow) {
            self.error_at_current(
                "invalid_function_type",
                "function types use 'fn(...) T'; write 'fn(T) U'",
            );
            self.advance();
            let ret = self.parse_type_ref()?;
            let span = left.span().cover(ret.span());
            return Some(TypeRef::Function {
                params: vec![left],
                ret: Box::new(ret),
                span,
            });
        }
        Some(left)
    }

    pub(super) fn parse_pattern_type_ref(&mut self) -> Option<TypeRef> {
        self.skip_newlines();
        let ty = if self.at_keyword(Keyword::Fn) {
            self.parse_function_type_ref()?
        } else if self.at(TokenKind::LParen) {
            self.parse_parenthesized_or_function_type_ref()?
        } else {
            self.parse_primary_type_ref()?
        };
        Some(self.parse_optional_type_suffix(ty))
    }

    fn parse_optional_type_suffix(&mut self, ty: TypeRef) -> TypeRef {
        if self.match_token(TokenKind::QuestionQuestion) {
            self.diagnostics.push(Diagnostic::error(
                "double_optional_type",
                "'??' is the extract-or-fallback operator and cannot form an optional type; write 'Option[Int?]' for a nested optional type",
                ty.span().cover(self.previous_span()),
            ));
            return ty;
        }
        if !self.match_token(TokenKind::Question) {
            return ty;
        }

        let span = ty.span().cover(self.previous_span());
        let optional = TypeRef::Named {
            name: "Option".to_string(),
            args: vec![ty],
            span,
        };
        if self.match_token(TokenKind::Question) || self.match_token(TokenKind::QuestionQuestion) {
            self.diagnostics.push(Diagnostic::error(
                "double_optional_type",
                "optional shorthand may be used only once; write 'Option[Int?]' for a nested optional type",
                span.cover(self.previous_span()),
            ));
        }
        optional
    }

    fn parse_function_type_ref(&mut self) -> Option<TypeRef> {
        self.match_keyword(Keyword::Fn);
        let start = self.previous_span();
        if !self.at(TokenKind::LParen) {
            self.error_at_current(
                "invalid_function_type",
                "expected '(' after 'fn', as in 'fn(Int) Str'",
            );
            return None;
        }
        self.advance();
        self.skip_newlines();

        let mut params = Vec::new();
        if !self.at(TokenKind::RParen) {
            loop {
                params.push(self.parse_type_ref()?);
                self.skip_newlines();
                if !self.match_token(TokenKind::Comma) {
                    break;
                }
                self.skip_newlines();
                if self.at(TokenKind::RParen) {
                    break;
                }
            }
        }
        self.consume(
            TokenKind::RParen,
            "expected ')' after function parameter types",
        )?;
        self.skip_newlines();

        if self.match_token(TokenKind::FatArrow) || self.match_token(TokenKind::Arrow) {
            self.diagnostics.push(Diagnostic::error(
                "invalid_function_type",
                "function types do not use an arrow; write 'fn(...) T'",
                self.previous_span(),
            ));
        }
        let ret = self.parse_type_ref()?;
        let span = start.cover(ret.span());
        Some(TypeRef::Function {
            params,
            ret: Box::new(ret),
            span,
        })
    }

    fn parse_parenthesized_or_function_type_ref(&mut self) -> Option<TypeRef> {
        let start = self.consume(TokenKind::LParen, "expected '('")?;
        self.skip_newlines();
        let mut fields = Vec::new();
        let mut singleton_comma = None;
        if !self.at(TokenKind::RParen) {
            fields.push(self.parse_tuple_type_field()?);
            while self.match_token(TokenKind::Comma) {
                let comma = self.previous_span();
                self.skip_newlines();
                if self.at(TokenKind::RParen) {
                    if fields.len() == 1 {
                        singleton_comma = Some(comma);
                    }
                    break;
                }
                fields.push(self.parse_tuple_type_field()?);
            }
        }
        let end = self.consume(TokenKind::RParen, "expected ')' after tuple type")?;

        if self.match_token(TokenKind::FatArrow) || self.match_token(TokenKind::Arrow) {
            self.diagnostics.push(Diagnostic::error(
                "removed_function_type_syntax",
                "function types use 'fn(...) T'; replace the parenthesized parameter-type form",
                start.cover(self.previous_span()),
            ));
            let ret = self.parse_type_ref()?;
            return Some(TypeRef::Function {
                params: fields.into_iter().map(|field| field.ty).collect(),
                ret: Box::new(ret.clone()),
                span: start.cover(ret.span()),
            });
        }

        if fields.len() == 1 {
            if let Some(comma) = singleton_comma {
                self.diagnostics.push(Diagnostic::error(
                    "singleton_tuple",
                    "singleton tuple types are not supported; remove the trailing comma to use the element type directly",
                    comma,
                ));
            }
            return Some(fields.into_iter().next()?.ty);
        }
        Some(TypeRef::Tuple {
            fields,
            span: start.cover(end),
        })
    }

    pub(super) fn parse_primary_type_ref(&mut self) -> Option<TypeRef> {
        self.skip_newlines();
        if self.at_keyword(Keyword::Fn) {
            return self.parse_function_type_ref();
        }
        if self.match_token(TokenKind::LBracket) {
            let start = self.previous_span();
            let first = self.parse_type_ref()?;
            if self.match_token(TokenKind::Colon) {
                let value = self.parse_type_ref()?;
                let end = self.consume(TokenKind::RBracket, "expected ']' after map type")?;
                return Some(TypeRef::Named {
                    name: "Map".to_string(),
                    args: vec![first, value],
                    span: start.cover(end),
                });
            }
            let end = self.consume(TokenKind::RBracket, "expected ']' after vector type")?;
            return Some(TypeRef::Named {
                name: "Vector".to_string(),
                args: vec![first],
                span: start.cover(end),
            });
        }
        if self.match_token(TokenKind::LBrace) {
            let start = self.previous_span();
            self.skip_newlines();
            let mut fields = Vec::new();
            if !self.at(TokenKind::RBrace) {
                loop {
                    let (name, name_span) = self.expect_identifier("expected shape field name")?;
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
            let end = self.consume(TokenKind::RBrace, "expected '}' after anonymous shape type")?;
            return Some(TypeRef::Record {
                fields,
                span: start.cover(end),
            });
        }
        if self.at(TokenKind::LParen) {
            return self.parse_parenthesized_or_function_type_ref();
        }
        if self.is_placeholder_identifier() {
            let span = self.current_span();
            self.advance();
            return Some(TypeRef::Wildcard { span });
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
        if self.at(TokenKind::Identifier) && self.at_next(TokenKind::Identifier) {
            let name_span = self.current_span();
            self.error_at_current(
                "invalid_tuple_type_field",
                "tuple types are positional; use anonymous shape type '{ name Type }' for named fields",
            );
            self.advance();
            let ty = self.parse_type_ref()?;
            let span = name_span.cover(ty.span());
            return Some(TupleTypeField { ty, span });
        }
        let ty = self.parse_type_ref()?;
        let span = ty.span();
        Some(TupleTypeField { ty, span })
    }

    pub(super) fn can_start_type_ref(&self) -> bool {
        matches!(
            self.current_kind(),
            TokenKind::Identifier
                | TokenKind::Keyword(Keyword::Fn)
                | TokenKind::LParen
                | TokenKind::LBrace
                | TokenKind::LBracket
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

#[derive(Debug, Default)]
pub(super) struct ParsedGenericClause {
    pub params: Vec<TypeParam>,
    pub conditions: Vec<GenericCondition>,
}
