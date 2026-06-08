use super::*;

impl<'a> Parser<'a> {
    pub(super) fn parse_module_decl(&mut self) -> Option<ModuleDecl> {
        let start = self.previous_span();
        let name = self.parse_path_string()?;
        let end = self.last_non_newline_span(start);
        Some(ModuleDecl {
            name,
            span: start.cover(end),
        })
    }

    pub(super) fn parse_import_decl(&mut self) -> Option<ImportDecl> {
        let start = self.previous_span();
        let (segments, mut span) = self.parse_import_segments("expected import path")?;
        let mut import = ImportDecl {
            path: String::new(),
            object_name: None,
            wildcard: false,
            symbols: Vec::new(),
            span: start,
        };

        match self.current_kind() {
            TokenKind::Slash => {
                self.advance();
                match self.current_kind() {
                    TokenKind::Star => {
                        let end = self.current_span();
                        self.advance();
                        import.path = segments.join("/");
                        import.wildcard = true;
                        span = span.cover(end);
                    }
                    TokenKind::LBrace => {
                        self.advance();
                        import.path = segments.join("/");
                        let (symbols, symbols_span) = self.parse_import_symbol_list()?;
                        import.symbols = symbols;
                        span = span.cover(symbols_span);
                    }
                    TokenKind::Identifier => {
                        let (name, name_span) = self
                            .expect_identifier("expected import symbol, '*', or '{' after '/'")?;
                        if self.match_token(TokenKind::Slash) {
                            import.path = segments.join("/");
                            import.object_name = Some(name);
                            if self.match_token(TokenKind::Star) {
                                import.wildcard = true;
                                span = span.cover(self.previous_span());
                            } else if self.match_token(TokenKind::LBrace) {
                                let (symbols, symbols_span) = self.parse_import_symbol_list()?;
                                import.symbols = symbols;
                                span = span.cover(symbols_span);
                            } else {
                                self.error_at_current(
                                    "unexpected_token",
                                    "expected object member import '*', or '{'",
                                );
                                return None;
                            }
                            span = span.cover(name_span);
                        } else {
                            import.path = segments.join("/");
                            let mut symbol = ImportSymbol {
                                name,
                                alias: None,
                                span: name_span,
                            };
                            if self.match_keyword(Keyword::As) {
                                let (alias, alias_span) =
                                    self.expect_identifier("expected alias after 'as'")?;
                                symbol.alias = Some(alias);
                                symbol.span = symbol.span.cover(alias_span);
                            }
                            span = span.cover(symbol.span);
                            import.symbols.push(symbol);
                        }
                    }
                    _ => {
                        self.error_at_current(
                            "unexpected_token",
                            "expected import symbol, '*', or '{' after '/'",
                        );
                        return None;
                    }
                }
            }
            _ => {
                import.path = segments.join("/");
            }
        }

        import.span = start.cover(span);
        Some(import)
    }

    pub(super) fn parse_item(&mut self) -> Option<Item> {
        let annotations = self.parse_annotations()?;
        let visibility = self.parse_visibility();
        match self.current_kind() {
            TokenKind::Keyword(Keyword::Def) => {
                let function = self.parse_function_decl(annotations, visibility)?;
                Some(Item::Function(function))
            }
            TokenKind::Keyword(Keyword::Class)
            | TokenKind::Keyword(Keyword::Object)
            | TokenKind::Keyword(Keyword::Interface)
            | TokenKind::Keyword(Keyword::Enum) => {
                if visibility == Visibility::Public {
                    self.error_at_current(
                        "unexpected_visibility",
                        "'public' is only supported for top-level functions and immutable bindings",
                    );
                    return None;
                }
                let decl = self.parse_type_decl(annotations, visibility)?;
                Some(Item::Type(decl))
            }
            TokenKind::Keyword(Keyword::Record) => {
                self.error_at_current(
                    "unexpected_record_decl",
                    "named 'record' declarations were removed; use 'class'",
                );
                None
            }
            TokenKind::Keyword(Keyword::Impl) => {
                if !annotations.is_empty() {
                    self.error_at_current(
                        "unexpected_annotation",
                        "impl blocks do not accept annotations",
                    );
                    return None;
                }
                if visibility != Visibility::Default {
                    self.error_at_current(
                        "unexpected_visibility",
                        "impl blocks do not accept visibility modifiers",
                    );
                    return None;
                }
                let block = self.parse_impl_block()?;
                Some(Item::Impl(block))
            }
            _ => {
                if visibility == Visibility::Public && self.at_keyword(Keyword::Var) {
                    self.error_at_current(
                        "unexpected_visibility",
                        "'public' is only supported for immutable named top-level bindings",
                    );
                    return None;
                }
                if self.is_binding_start() {
                    if !annotations.is_empty() {
                        self.error_at_current(
                            "unexpected_annotation",
                            "annotations are not supported on bindings yet",
                        );
                        return None;
                    }
                    let mut stmt = self.try_parse_binding_stmt()?;
                    if visibility == Visibility::Public
                        && stmt
                            .bindings
                            .iter()
                            .any(|binding| binding.name == "_" || binding.mutable)
                    {
                        self.error_at_current(
                            "unexpected_visibility",
                            "'public' is only supported for immutable named top-level bindings",
                        );
                        return None;
                    }
                    stmt.visibility = visibility;
                    return Some(Item::Statement(Stmt::Binding(stmt)));
                }
                if !annotations.is_empty() {
                    self.error_at_current(
                        "unexpected_annotation",
                        "annotations are only valid on declarations",
                    );
                    return None;
                }
                if visibility != Visibility::Default {
                    self.error_at_current(
                        "unexpected_visibility",
                        if visibility == Visibility::Public {
                            "'public' is only supported for top-level functions and immutable bindings"
                        } else {
                            "visibility modifiers are only valid on declarations"
                        },
                    );
                    return None;
                }
                self.parse_stmt().map(Item::Statement)
            }
        }
    }

    pub(super) fn parse_visibility(&mut self) -> Visibility {
        if self.match_keyword(Keyword::Public) {
            Visibility::Public
        } else if self.match_keyword(Keyword::Hidden) {
            Visibility::Hidden
        } else {
            Visibility::Default
        }
    }

    pub(super) fn parse_annotations(&mut self) -> Option<Vec<Annotation>> {
        let mut annotations = Vec::new();
        loop {
            self.skip_newlines();
            if !self.match_token(TokenKind::At) {
                break;
            }
            let start = self.previous_span();
            let value = self.parse_expr()?;
            annotations.push(Annotation {
                span: start.cover(value.span()),
                value,
            });
        }
        Some(annotations)
    }

    pub(super) fn parse_import_segments(
        &mut self,
        message: &'static str,
    ) -> Option<(Vec<String>, Span)> {
        let (first, start) = self.expect_identifier(message)?;
        let mut segments = vec![first];
        let mut span = start;
        while self.at(TokenKind::Slash)
            && self
                .tokens
                .get(self.index + 1)
                .is_some_and(|token| token.kind == TokenKind::Identifier)
            && self
                .tokens
                .get(self.index + 1)
                .is_some_and(|token| starts_lower(&token.lexeme))
        {
            self.advance();
            let (segment, next_span) = self.expect_identifier("expected path segment after '/'")?;
            segments.push(segment);
            span = span.cover(next_span);
        }
        Some((segments, span))
    }

    pub(super) fn parse_import_symbol_list(&mut self) -> Option<(Vec<ImportSymbol>, Span)> {
        let open = self.previous_span();
        let mut symbols = Vec::new();
        loop {
            let (name, name_span) = self.expect_identifier("expected import symbol")?;
            let mut symbol = ImportSymbol {
                name,
                alias: None,
                span: name_span,
            };
            if self.match_keyword(Keyword::As) {
                let (alias, alias_span) = self.expect_identifier("expected alias after 'as'")?;
                symbol.alias = Some(alias);
                symbol.span = symbol.span.cover(alias_span);
            }
            symbols.push(symbol);
            if !self.match_token(TokenKind::Comma) {
                break;
            }
        }
        let end = self.consume(TokenKind::RBrace, "expected '}' after import symbol list")?;
        Some((symbols, open.cover(end)))
    }

    pub(super) fn parse_function_decl(
        &mut self,
        annotations: Vec<Annotation>,
        visibility: Visibility,
    ) -> Option<FunctionDecl> {
        let start = self.consume_keyword(Keyword::Def, "expected 'def'")?;
        let (name, _) = self.parse_callable_name("expected function name")?;
        let type_params = self.parse_type_params()?;
        let params = self.parse_param_list()?;
        let return_type = self.parse_optional_return_type();
        let body = self.parse_callable_body()?;
        let end = body.span();
        Some(FunctionDecl {
            annotations,
            visibility,
            name,
            type_params,
            params,
            return_type,
            body,
            span: start.cover(end),
        })
    }

    pub(super) fn parse_type_decl(
        &mut self,
        annotations: Vec<Annotation>,
        visibility: Visibility,
    ) -> Option<TypeDecl> {
        let (kind, start) = match self.current_kind() {
            TokenKind::Keyword(Keyword::Class) => {
                let span = self.current_span();
                self.advance();
                (TypeKind::Class, span)
            }
            TokenKind::Keyword(Keyword::Object) => {
                let span = self.current_span();
                self.advance();
                (TypeKind::Object, span)
            }
            TokenKind::Keyword(Keyword::Interface) => {
                let span = self.current_span();
                self.advance();
                (TypeKind::Interface, span)
            }
            TokenKind::Keyword(Keyword::Enum) => {
                let span = self.current_span();
                self.advance();
                (TypeKind::Enum, span)
            }
            _ => {
                self.error_at_current(
                    "expected_type_decl",
                    "expected class, object, interface, or enum",
                );
                return None;
            }
        };

        let (name, _) = self.expect_identifier("expected type name")?;
        let type_params = self.parse_type_params()?;
        let with_bounds = if self.match_keyword(Keyword::With) {
            self.parse_type_ref_list()?
        } else {
            Vec::new()
        };
        self.skip_newlines();
        self.consume(TokenKind::LBrace, "expected '{' after type declaration")?;
        self.skip_newlines();

        let mut members = Vec::new();
        while !self.at(TokenKind::RBrace) && !self.at(TokenKind::Eof) {
            self.skip_newlines();
            if self.at(TokenKind::RBrace) {
                break;
            }

            let member_annotations = self.parse_annotations()?;

            if kind == TypeKind::Enum && self.match_keyword(Keyword::Case) {
                if let Some(case_decl) =
                    self.parse_enum_case(member_annotations, self.previous_span())
                {
                    members.push(TypeMember::Case(case_decl));
                } else {
                    self.synchronize_member();
                }
                self.skip_newlines();
                continue;
            }

            let member_visibility = self.parse_visibility();
            if member_visibility == Visibility::Public {
                let message = match kind {
                    TypeKind::Interface => "public is not allowed inside interfaces",
                    TypeKind::Enum => "public is not allowed on enum members",
                    TypeKind::Class => "public is not allowed on class members",
                    TypeKind::Object => "public is not allowed on object members",
                    TypeKind::Record => unreachable!("named records are no longer parsed"),
                };
                self.error_at_current("unexpected_visibility", message);
                return None;
            }
            match self.current_kind() {
                TokenKind::Keyword(Keyword::Def) => {
                    let method = self.parse_method_decl(
                        member_annotations,
                        member_visibility,
                        kind == TypeKind::Interface,
                    )?;
                    members.push(TypeMember::Method(method));
                }
                _ => {
                    let field = self.parse_field_decl(member_annotations, member_visibility)?;
                    members.push(TypeMember::Field(field));
                }
            }
            self.skip_newlines();
        }

        let end = self.consume(TokenKind::RBrace, "expected '}' after type body")?;
        Some(TypeDecl {
            annotations,
            visibility,
            kind,
            name,
            type_params,
            with_bounds,
            members,
            span: start.cover(end),
        })
    }

    pub(super) fn parse_enum_case(
        &mut self,
        annotations: Vec<Annotation>,
        case_span: Span,
    ) -> Option<EnumCaseDecl> {
        let (name, name_span) = self.expect_identifier("expected enum case name")?;
        let mut fields = Vec::new();
        self.skip_newlines();
        if self.match_token(TokenKind::LBrace) {
            self.skip_newlines();
            while !self.at(TokenKind::RBrace) && !self.at(TokenKind::Eof) {
                let field_annotations = self.parse_annotations()?;
                let field_visibility = self.parse_visibility();
                let field = self.parse_field_decl(field_annotations, field_visibility)?;
                fields.push(field);
                self.skip_newlines();
            }
            let end = self.consume(TokenKind::RBrace, "expected '}' after enum case body")?;
            return Some(EnumCaseDecl {
                annotations,
                name,
                fields,
                span: case_span.cover(end),
            });
        }
        Some(EnumCaseDecl {
            annotations,
            name,
            fields,
            span: case_span.cover(name_span),
        })
    }

    pub(super) fn parse_impl_block(&mut self) -> Option<ImplBlock> {
        let start = self.consume_keyword(Keyword::Impl, "expected 'impl'")?;
        let target = self.parse_type_ref()?;
        if self.match_token(TokenKind::Dot) {
            let _ = self.expect_identifier("expected enum case name after '.'")?;
            let owner = match &target {
                TypeRef::Named { name, .. } => name.as_str(),
                _ => "enum",
            };
            self.error_at_current(
                "unexpected_impl_target",
                format!(
                    "enum cases cannot declare methods; move methods to enum '{}'",
                    owner
                ),
            );
            return None;
        }
        self.skip_newlines();
        self.consume(TokenKind::LBrace, "expected '{' after impl target")?;
        self.skip_newlines();

        let mut methods = Vec::new();
        while !self.at(TokenKind::RBrace) && !self.at(TokenKind::Eof) {
            self.skip_newlines();
            if self.at(TokenKind::RBrace) {
                break;
            }
            let annotations = self.parse_annotations()?;
            let visibility = self.parse_visibility();
            let method = self.parse_method_decl(annotations, visibility, false)?;
            methods.push(method);
            self.skip_newlines();
        }
        let end = self.consume(TokenKind::RBrace, "expected '}' after impl body")?;
        Some(ImplBlock {
            target,
            methods,
            span: start.cover(end),
        })
    }

    pub(super) fn parse_method_decl(
        &mut self,
        annotations: Vec<Annotation>,
        visibility: Visibility,
        allow_signature_only: bool,
    ) -> Option<MethodDecl> {
        let start = self.consume_keyword(Keyword::Def, "expected 'def'")?;
        let (name, _) = self.parse_callable_name("expected method name")?;
        let type_params = self.parse_type_params()?;
        let params = self.parse_param_list()?;
        let return_type = self.parse_optional_return_type();
        let body = if self.at(TokenKind::LBrace) || self.at(TokenKind::Eq) {
            Some(self.parse_callable_body()?)
        } else if allow_signature_only {
            None
        } else {
            self.error_at_current("expected_method_body", "expected method body");
            return None;
        };
        let end = body
            .as_ref()
            .map(CallableBody::span)
            .or_else(|| return_type.as_ref().map(TypeRef::span))
            .unwrap_or(start);
        Some(MethodDecl {
            annotations,
            visibility,
            name,
            type_params,
            params,
            return_type,
            body,
            span: start.cover(end),
        })
    }

    pub(super) fn parse_field_decl(
        &mut self,
        annotations: Vec<Annotation>,
        visibility: Visibility,
    ) -> Option<FieldDecl> {
        let start = self.current_span();
        let mutable = self.match_keyword(Keyword::Var);
        let (name, _) = self.expect_binding_name("expected field name")?;

        let ty = if self.can_start_type_ref() {
            Some(self.parse_type_ref()?)
        } else {
            None
        };

        let (initializer, deferred, end) =
            if self.match_token(TokenKind::Eq) || self.match_token(TokenKind::ColonAssign) {
                let assign_span = self.previous_span();
                if self.match_token(TokenKind::Question) {
                    (None, true, self.previous_span())
                } else {
                    let expr = self.parse_expr()?;
                    let end = expr.span();
                    (Some(expr), false, assign_span.cover(end))
                }
            } else {
                (None, false, ty.as_ref().map(TypeRef::span).unwrap_or(start))
            };

        Some(FieldDecl {
            annotations,
            visibility,
            mutable,
            name,
            ty,
            initializer,
            deferred,
            span: start.cover(end),
        })
    }

    pub(super) fn parse_callable_body(&mut self) -> Option<CallableBody> {
        self.skip_newlines();
        if self.at(TokenKind::LBrace) {
            return self.parse_block().map(CallableBody::Block);
        }
        self.consume(TokenKind::Eq, "expected '=' or '{' before callable body")?;
        let expr = self.parse_expr()?;
        Some(CallableBody::Expr(expr))
    }
}

fn starts_lower(value: &str) -> bool {
    value
        .chars()
        .next()
        .is_some_and(|ch| ch.is_ascii_lowercase())
}
