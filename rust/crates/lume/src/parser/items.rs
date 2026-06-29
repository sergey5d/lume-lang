use super::*;

#[derive(Clone, Copy, PartialEq, Eq)]
enum TypeBodyOrder {
    Storage,
    Constructor,
    Method,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ImplBodyOrder {
    Constructor,
    Method,
}

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
        let (segments, mut span) = self.parse_import_segments("expected use path")?;
        let mut import = ImportDecl {
            path: String::new(),
            single_name: None,
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
                        let (name, name_span) =
                            self.expect_identifier("expected use symbol, '*', or '{' after '/'")?;
                        if self.match_token(TokenKind::Slash) {
                            import.path = segments.join("/");
                            import.single_name = Some(name);
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
                                    "expected single member use '*', or '{'",
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
                            "expected use symbol, '*', or '{' after '/'",
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
            TokenKind::Keyword(Keyword::Annotation)
            | TokenKind::Keyword(Keyword::Class)
            | TokenKind::Keyword(Keyword::Shape)
            | TokenKind::Keyword(Keyword::Single)
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
                if self.at_keyword(Keyword::Var) {
                    if !annotations.is_empty() {
                        self.error_at_current(
                            "unexpected_annotation",
                            "annotations are not supported on bindings yet",
                        );
                        return None;
                    }
                    if visibility != Visibility::Default {
                        self.error_at_current(
                            "unexpected_visibility",
                            "visibility modifiers are only valid on declarations",
                        );
                        return None;
                    }
                    let stmt = self.parse_binding_stmt_after_var()?;
                    return Some(Item::Statement(Stmt::Binding(stmt)));
                }
                if self.is_binding_start() {
                    if !annotations.is_empty() {
                        self.error_at_current(
                            "unexpected_annotation",
                            "annotations are not supported on bindings yet",
                        );
                        return None;
                    }
                    if let Some(mut stmt) = self.try_parse_binding_stmt() {
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
                self.error_at_current(
                    "unexpected_top_level_statement",
                    "top-level statements are not allowed; move executable code into a function such as 'def main() { ... }'",
                );
                None
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
            let (name, name_span) = self.expect_identifier("expected use symbol")?;
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
        let end = self.consume(TokenKind::RBrace, "expected '}' after use symbol list")?;
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
        let return_type = if self.callable_body_starts_here() {
            None
        } else {
            self.parse_optional_return_type()
        };
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
            TokenKind::Keyword(Keyword::Annotation) => {
                let span = self.current_span();
                self.advance();
                (TypeKind::Annotation, span)
            }
            TokenKind::Keyword(Keyword::Class) => {
                let span = self.current_span();
                self.advance();
                (TypeKind::Class, span)
            }
            TokenKind::Keyword(Keyword::Shape) => {
                let span = self.current_span();
                self.advance();
                (TypeKind::Record, span)
            }
            TokenKind::Keyword(Keyword::Single) => {
                let span = self.current_span();
                self.advance();
                (TypeKind::Single, span)
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
                    "expected annotation, class, shape, single, interface, or enum",
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
        let mut body_order = TypeBodyOrder::Storage;
        let mut misplaced_constructors = Vec::new();
        while !self.at(TokenKind::RBrace) && !self.at(TokenKind::Eof) {
            self.skip_newlines();
            if self.at(TokenKind::RBrace) {
                break;
            }

            let member_annotations = self.parse_annotations()?;

            if kind == TypeKind::Enum && self.match_keyword(Keyword::Case) {
                if let Some(case_decl) =
                    self.parse_enum_case(member_annotations, self.previous_span(), &name)
                {
                    match body_order {
                        TypeBodyOrder::Storage => {}
                        TypeBodyOrder::Constructor => {
                            self.diagnostics.push(Diagnostic::error(
                                "invalid_member_order",
                                format!(
                                    "enum cases must appear before constructors in enum '{}'; move case '{}' above constructor declarations",
                                    name, case_decl.name
                                ),
                                case_decl.span,
                            ));
                        }
                        TypeBodyOrder::Method => {
                            self.diagnostics.push(Diagnostic::error(
                                "invalid_member_order",
                                format!(
                                    "enum cases must appear before methods in enum '{}'; move case '{}' above method declarations",
                                    name, case_decl.name
                                ),
                                case_decl.span,
                            ));
                        }
                    }
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
                    TypeKind::Annotation => {
                        "public is not allowed on annotation fields; annotation fields are public by default"
                    }
                    TypeKind::Interface => "public is not allowed inside interfaces",
                    TypeKind::Enum => "public is not allowed on enum members",
                    TypeKind::Class => "public is not allowed on class members",
                    TypeKind::Single => "public is not allowed on single members",
                    TypeKind::Record => "public is not allowed on shape members",
                };
                self.error_at_current("unexpected_visibility", message);
                return None;
            }
            match self.current_kind() {
                TokenKind::Identifier if self.current().lexeme == "new" => {
                    let constructor =
                        self.parse_constructor_decl(member_annotations, member_visibility)?;
                    if body_order == TypeBodyOrder::Method {
                        self.diagnostics.push(Diagnostic::error(
                            "invalid_member_order",
                            format!(
                                "constructors must appear before methods in {} '{}'; move 'new' above method declarations",
                                type_kind_name(kind),
                                name
                            ),
                            constructor.span,
                        ));
                    }
                    misplaced_constructors.push(constructor.span);
                    if body_order != TypeBodyOrder::Method {
                        body_order = TypeBodyOrder::Constructor;
                    }
                    members.push(TypeMember::Method(constructor));
                }
                TokenKind::Keyword(Keyword::Def) => {
                    let body_method_error = match kind {
                        TypeKind::Annotation => Some(format!(
                            "annotation '{}' cannot declare methods; annotations are data-only metadata shapes",
                            name
                        )),
                        TypeKind::Record => Some(format!(
                            "shape '{}' cannot declare methods in its body; use 'impl {}'",
                            name, name
                        )),
                        _ => None,
                    };
                    if let Some(message) = body_method_error {
                        self.error_at_current("unexpected_method_decl", message);
                        return None;
                    }
                    let method = self.parse_method_decl(
                        member_annotations,
                        member_visibility,
                        kind == TypeKind::Interface,
                    )?;
                    body_order = TypeBodyOrder::Method;
                    members.push(TypeMember::Method(method));
                }
                _ => {
                    let field = self.parse_field_decl(member_annotations, member_visibility)?;
                    if matches!(
                        kind,
                        TypeKind::Class | TypeKind::Record | TypeKind::Enum | TypeKind::Single
                    ) {
                        match body_order {
                            TypeBodyOrder::Storage => {}
                            TypeBodyOrder::Constructor => {
                                self.diagnostics.push(Diagnostic::error(
                                    "invalid_member_order",
                                    format!(
                                        "storage fields must appear before constructors in {} '{}'; move field '{}' above constructor declarations",
                                        type_kind_name(kind),
                                        name,
                                        field.name
                                    ),
                                    field.span,
                                ));
                            }
                            TypeBodyOrder::Method => {
                                self.diagnostics.push(Diagnostic::error(
                                    "invalid_member_order",
                                    format!(
                                        "storage fields must appear before methods in {} '{}'; move field '{}' above method declarations",
                                        type_kind_name(kind),
                                        name,
                                        field.name
                                    ),
                                    field.span,
                                ));
                            }
                        }
                    }
                    members.push(TypeMember::Field(field));
                }
            }
            self.skip_newlines();
        }

        for span in misplaced_constructors {
            self.diagnostics.push(Diagnostic::error(
                "unexpected_constructor_decl",
                type_body_constructor_message(kind, &name),
                span,
            ));
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
        enum_name: &str,
    ) -> Option<EnumCaseDecl> {
        let (name, name_span) = self.expect_identifier("expected enum case name")?;
        let mut fields = Vec::new();
        self.skip_newlines();
        let mut end = name_span;
        if self.match_keyword(Keyword::With) {
            end = self.report_enum_case_interface_bound(enum_name, &name)?;
            self.skip_newlines();
        }
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
            span: case_span.cover(end),
        })
    }

    fn report_enum_case_interface_bound(
        &mut self,
        enum_name: &str,
        case_name: &str,
    ) -> Option<Span> {
        let with_span = self.previous_span();
        let bounds = self.parse_type_ref_list()?;
        let end = bounds.last().map(TypeRef::span).unwrap_or(with_span);
        self.diagnostics.push(Diagnostic::error(
            "invalid_enum_case_interface",
            format!(
                "enum case '{}.{}' cannot implement interfaces; put 'with ...' on enum '{}'",
                enum_name, case_name, enum_name
            ),
            with_span.cover(end),
        ));
        Some(end)
    }

    pub(super) fn parse_impl_block(&mut self) -> Option<ImplBlock> {
        let start = self.consume_keyword(Keyword::Impl, "expected 'impl'")?;
        let target_kind = if self.match_keyword(Keyword::Single) {
            ImplTargetKind::Single
        } else {
            ImplTargetKind::Instance
        };
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
        let mut body_order = ImplBodyOrder::Constructor;
        while !self.at(TokenKind::RBrace) && !self.at(TokenKind::Eof) {
            self.skip_newlines();
            if self.at(TokenKind::RBrace) {
                break;
            }
            let annotations = self.parse_annotations()?;
            let visibility = self.parse_visibility();
            let method = if self.at(TokenKind::Identifier) && self.current().lexeme == "new" {
                let constructor = self.parse_constructor_decl(annotations, visibility)?;
                if body_order == ImplBodyOrder::Method {
                    self.diagnostics.push(Diagnostic::error(
                        "invalid_member_order",
                        format!(
                            "constructors must appear before methods in impl '{}'; move 'new' above method declarations",
                            impl_target_name(&target)
                        ),
                        constructor.span,
                    ));
                }
                constructor
            } else {
                let method = self.parse_method_decl(annotations, visibility, false)?;
                if method.name == "new" {
                    self.diagnostics.push(Diagnostic::error(
                        "old_constructor_syntax",
                        "constructors use `new { params } { body }`; replace `def new(...)` with `new { ... } { ... }`",
                        method.span,
                    ));
                }
                body_order = ImplBodyOrder::Method;
                method
            };
            methods.push(method);
            self.skip_newlines();
        }
        let end = self.consume(TokenKind::RBrace, "expected '}' after impl body")?;
        Some(ImplBlock {
            target_kind,
            target,
            methods,
            span: start.cover(end),
        })
    }

    pub(super) fn parse_constructor_decl(
        &mut self,
        annotations: Vec<Annotation>,
        visibility: Visibility,
    ) -> Option<MethodDecl> {
        let (name, start) = self.expect_identifier("expected constructor name")?;
        if name != "new" {
            self.error_at_current("expected_constructor", "expected 'new'");
            return None;
        }
        let params = self.parse_constructor_param_block()?;
        let body = self.parse_callable_body()?;
        let end = body.span();
        Some(MethodDecl {
            annotations,
            visibility,
            name,
            type_params: Vec::new(),
            params,
            return_type: None,
            body: Some(body),
            span: start.cover(end),
        })
    }

    fn parse_constructor_param_block(&mut self) -> Option<Vec<Param>> {
        self.consume(
            TokenKind::LBrace,
            "expected '{' before constructor parameters",
        )?;
        let mut params = Vec::new();
        self.skip_newlines();
        while !self.at(TokenKind::RBrace) && !self.at(TokenKind::Eof) {
            let (name, start) = self.expect_identifier("expected constructor parameter name")?;
            let ty = if self.can_start_type_ref() {
                Some(self.parse_type_ref()?)
            } else {
                self.error_at_current("expected_type", "expected constructor parameter type");
                return None;
            };
            let variadic = self.match_keyword(Keyword::Vararg);
            let initializer = if self.match_token(TokenKind::Eq) {
                Some(self.parse_expr()?)
            } else {
                None
            };
            let end = initializer
                .as_ref()
                .map(Expr::span)
                .or_else(|| variadic.then(|| self.previous_span()))
                .or_else(|| ty.as_ref().map(TypeRef::span))
                .unwrap_or(start);
            params.push(Param {
                name,
                ty,
                initializer,
                variadic,
                span: start.cover(end),
            });
            self.skip_newlines();
            if self.match_token(TokenKind::Comma) {
                self.skip_newlines();
            }
        }
        self.consume(
            TokenKind::RBrace,
            "expected '}' after constructor parameters",
        )?;
        Some(params)
    }

    pub(super) fn parse_method_decl(
        &mut self,
        annotations: Vec<Annotation>,
        visibility: Visibility,
        allow_signature_only: bool,
    ) -> Option<MethodDecl> {
        let start = self.consume_keyword(Keyword::Def, "expected 'def'")?;
        let (name, _) = if self.at(TokenKind::Keyword(Keyword::Expect)) {
            let token = self.current().clone();
            self.advance();
            (token.lexeme, token.span)
        } else {
            self.parse_callable_name("expected method name")?
        };
        let type_params = self.parse_type_params()?;
        let params = self.parse_param_list()?;
        let return_type = if self.callable_body_starts_here() {
            None
        } else {
            self.parse_optional_return_type()
        };
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

        let (initializer, end) = if self.match_token(TokenKind::Eq) {
            let assign_span = self.previous_span();
            let expr = self.parse_expr()?;
            let end = expr.span();
            (Some(expr), assign_span.cover(end))
        } else if self.match_token(TokenKind::ColonAssign) {
            let assign_span = self.previous_span();
            let _ = self.parse_expr();
            self.diagnostics.push(Diagnostic::error(
                "unexpected_token",
                "use '=' for field initializers; ':=' is only for reassignment statements",
                assign_span,
            ));
            return None;
        } else {
            (None, ty.as_ref().map(TypeRef::span).unwrap_or(start))
        };

        Some(FieldDecl {
            annotations,
            visibility,
            mutable,
            name,
            ty,
            initializer,
            span: start.cover(end),
        })
    }

    pub(super) fn parse_callable_body(&mut self) -> Option<CallableBody> {
        self.skip_newlines();
        if self.at(TokenKind::LBrace) {
            return self.parse_block().map(CallableBody::Block);
        }
        self.consume(TokenKind::Eq, "expected '=' or '{' before callable body")?;
        self.skip_newlines();
        if self.at(TokenKind::LBrace) && !self.looks_like_brace_record_literal(false) {
            let block = self.parse_block()?;
            self.diagnostics.push(Diagnostic::error(
                "invalid_callable_body",
                "block callable bodies omit '='; use 'def name(...) { ... }'",
                block.span,
            ));
            return Some(CallableBody::Block(block));
        }
        let expr = self.parse_expr()?;
        Some(CallableBody::Expr(expr))
    }

    fn callable_body_starts_here(&self) -> bool {
        match self.next_significant_token().kind {
            TokenKind::Eq => true,
            TokenKind::LBrace => {
                let mut parser = Parser {
                    tokens: self.tokens,
                    index: self.index,
                    diagnostics: Vec::new(),
                    allow_trailing_block_call: self.allow_trailing_block_call,
                };
                if parser.parse_type_ref().is_some() {
                    parser.skip_newlines();
                    return !matches!(parser.current_kind(), TokenKind::Eq | TokenKind::LBrace);
                }
                true
            }
            _ => false,
        }
    }
}

fn type_kind_name(kind: TypeKind) -> &'static str {
    match kind {
        TypeKind::Annotation => "annotation",
        TypeKind::Class => "class",
        TypeKind::Record => "shape",
        TypeKind::Single => "single",
        TypeKind::Interface => "interface",
        TypeKind::Enum => "enum",
    }
}

fn impl_target_name(target: &TypeRef) -> &str {
    match target {
        TypeRef::Named { name, .. } => name,
        _ => "target",
    }
}

fn type_body_constructor_message(kind: TypeKind, name: &str) -> String {
    match kind {
        TypeKind::Annotation => {
            format!(
                "annotation '{name}' cannot declare constructors; annotations are data-only metadata shapes"
            )
        }
        TypeKind::Class => {
            format!(
                "class '{name}' constructors are declared in impl blocks; move 'new' into impl {name}"
            )
        }
        TypeKind::Record => {
            format!(
                "shape '{name}' cannot declare custom constructors; use structural brace construction"
            )
        }
        TypeKind::Single => {
            format!(
                "single '{name}' cannot declare custom constructors; reference '{name}' directly"
            )
        }
        TypeKind::Interface => {
            format!("interface '{name}' cannot declare constructors")
        }
        TypeKind::Enum => {
            format!("enum '{name}' cannot declare constructors; enum cases define values")
        }
    }
}

fn starts_lower(value: &str) -> bool {
    value
        .chars()
        .next()
        .is_some_and(|ch| ch.is_ascii_lowercase())
}
