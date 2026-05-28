use crate::{
    ast::*,
    diagnostic::Diagnostic,
    lexer::{Keyword, Token, TokenKind},
    source::Span,
};

pub struct ParseResult {
    pub program: Option<Program>,
    pub diagnostics: Vec<Diagnostic>,
}

pub fn parse_program(tokens: &[Token]) -> ParseResult {
    Parser::new(tokens).parse_program()
}

struct Parser<'a> {
    tokens: &'a [Token],
    index: usize,
    diagnostics: Vec<Diagnostic>,
    allow_trailing_block_call: bool,
}

#[derive(Clone, Copy)]
struct Checkpoint {
    index: usize,
    diagnostics_len: usize,
    allow_trailing_block_call: bool,
}

impl<'a> Parser<'a> {
    fn new(tokens: &'a [Token]) -> Self {
        Self {
            tokens,
            index: 0,
            diagnostics: Vec::new(),
            allow_trailing_block_call: true,
        }
    }

    fn parse_program(mut self) -> ParseResult {
        self.skip_newlines();
        let start_span = self.current_span();

        let module = if self.match_keyword(Keyword::Module) {
            self.parse_module_decl()
        } else {
            None
        };

        self.skip_newlines();
        let mut imports = Vec::new();
        while self.match_keyword(Keyword::Import) {
            if let Some(import) = self.parse_import_decl() {
                imports.push(import);
            }
            self.skip_newlines();
        }

        let mut items = Vec::new();
        while !self.at(TokenKind::Eof) {
            self.skip_newlines();
            if self.at(TokenKind::Eof) {
                break;
            }

            let before = self.index;
            if let Some(item) = self.parse_item() {
                items.push(item);
            } else {
                self.synchronize_item();
                if self.index == before && !self.at(TokenKind::Eof) {
                    self.advance();
                }
            }
            self.skip_newlines();
        }

        let end_span = if self.index > 0 {
            self.tokens[self.index.saturating_sub(1)].span
        } else {
            start_span
        };

        let program = Program {
            module,
            imports,
            items,
            span: Some(start_span.cover(end_span)),
        };

        ParseResult {
            program: Some(program),
            diagnostics: self.diagnostics,
        }
    }

    fn parse_module_decl(&mut self) -> Option<ModuleDecl> {
        let start = self.previous_span();
        let name = self.parse_path_string()?;
        let end = self.last_non_newline_span(start);
        Some(ModuleDecl {
            name,
            span: start.cover(end),
        })
    }

    fn parse_import_decl(&mut self) -> Option<ImportDecl> {
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

    fn parse_item(&mut self) -> Option<Item> {
        let annotations = self.parse_annotations()?;
        let visibility = self.parse_visibility();
        match self.current_kind() {
            TokenKind::Keyword(Keyword::Def) => {
                let function = self.parse_function_decl(annotations, visibility)?;
                Some(Item::Function(function))
            }
            TokenKind::Keyword(Keyword::Class)
            | TokenKind::Keyword(Keyword::Record)
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

    fn parse_visibility(&mut self) -> Visibility {
        if self.match_keyword(Keyword::Public) {
            Visibility::Public
        } else if self.match_keyword(Keyword::Hidden) {
            Visibility::Hidden
        } else {
            Visibility::Default
        }
    }

    fn parse_annotations(&mut self) -> Option<Vec<Annotation>> {
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

    fn parse_import_segments(&mut self, message: &'static str) -> Option<(Vec<String>, Span)> {
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

    fn parse_import_symbol_list(&mut self) -> Option<(Vec<ImportSymbol>, Span)> {
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

    fn parse_function_decl(
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

    fn parse_type_decl(
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
            TokenKind::Keyword(Keyword::Record) => {
                let span = self.current_span();
                self.advance();
                (TypeKind::Record, span)
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
                    "expected class, record, object, interface, or enum",
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
                    TypeKind::Record => "public is not allowed on record members",
                    TypeKind::Object => "public is not allowed on object members",
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

    fn parse_enum_case(
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

    fn parse_impl_block(&mut self) -> Option<ImplBlock> {
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

    fn parse_method_decl(
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

    fn parse_field_decl(
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

    fn parse_callable_body(&mut self) -> Option<CallableBody> {
        self.skip_newlines();
        if self.at(TokenKind::LBrace) {
            return self.parse_block().map(CallableBody::Block);
        }
        self.consume(TokenKind::Eq, "expected '=' or '{' before callable body")?;
        let expr = self.parse_expr()?;
        Some(CallableBody::Expr(expr))
    }

    fn parse_block(&mut self) -> Option<Block> {
        let start = self.consume(TokenKind::LBrace, "expected '{'")?;
        self.skip_newlines();
        let mut statements = Vec::new();
        while !self.at(TokenKind::RBrace) && !self.at(TokenKind::Eof) {
            if let Some(stmt) = self.parse_stmt() {
                statements.push(stmt);
            } else {
                self.synchronize_stmt();
            }
            self.skip_newlines();
        }
        let end = self.consume(TokenKind::RBrace, "expected '}' after block")?;
        Some(Block {
            statements,
            span: start.cover(end),
        })
    }

    fn parse_stmt(&mut self) -> Option<Stmt> {
        self.skip_newlines();
        match self.current_kind() {
            TokenKind::Keyword(Keyword::Def) => {
                let function = self.parse_function_decl(Vec::new(), Visibility::Default)?;
                Some(Stmt::LocalFunction(function))
            }
            TokenKind::Keyword(Keyword::Match) => self.parse_match_stmt(false).map(Stmt::Match),
            TokenKind::Keyword(Keyword::Partial) => self.parse_match_stmt(true).map(Stmt::Match),
            TokenKind::Keyword(Keyword::Unwrap) => self.parse_unwrap_stmt(),
            TokenKind::Keyword(Keyword::Let) => self.parse_let_stmt(),
            TokenKind::Keyword(Keyword::Var) => {
                let stmt = self.parse_binding_stmt_after_var()?;
                Some(Stmt::Binding(stmt))
            }
            TokenKind::Keyword(Keyword::If) => self.parse_if_stmt().map(Stmt::If),
            TokenKind::Keyword(Keyword::While) => self.parse_while_stmt().map(Stmt::While),
            TokenKind::Keyword(Keyword::For) => {
                if self.is_for_yield_start() {
                    let expr = self.parse_expr()?;
                    let span = expr.span();
                    Some(Stmt::Expr(ExprStmt { expr, span }))
                } else {
                    self.parse_for_stmt().map(Stmt::For)
                }
            }
            TokenKind::Keyword(Keyword::Return) => self.parse_return_stmt().map(Stmt::Return),
            TokenKind::Keyword(Keyword::Break) => self.parse_break_stmt().map(Stmt::Break),
            _ => {
                if let Some(binding) = self.try_parse_binding_stmt() {
                    return Some(Stmt::Binding(binding));
                }
                if let Some(assignment) = self.try_parse_assignment_stmt() {
                    return Some(Stmt::Assignment(assignment));
                }
                let expr = self.parse_expr()?;
                let span = expr.span();
                Some(Stmt::Expr(ExprStmt { expr, span }))
            }
        }
    }

    fn parse_binding_stmt_after_var(&mut self) -> Option<BindingStmt> {
        let start = self.consume_keyword(Keyword::Var, "expected 'var'")?;
        let bindings = self.parse_binding_list(true)?;
        self.consume(TokenKind::Eq, "expected '=' after bindings")?;
        if self.at(TokenKind::Newline) {
            self.error_at_current(
                "expected_expression",
                "expected expression on same line after \"=\"",
            );
            return None;
        }
        let values = self.parse_expr_list()?;
        if bindings.len() > 1 && values.len() == 1 {
            self.error_at_current(
                "unexpected_token",
                "destructuring bindings require 'let (...) = value'",
            );
            return None;
        }
        let end = values.last().map(Expr::span).unwrap_or(start);
        Some(BindingStmt {
            visibility: Visibility::Default,
            bindings,
            values,
            span: start.cover(end),
        })
    }

    fn parse_let_stmt(&mut self) -> Option<Stmt> {
        let start = self.consume_keyword(Keyword::Let, "expected 'let'")?;

        if self.at(TokenKind::LBrace) {
            let (clauses, clauses_end) = self.parse_refutable_clause_block("let")?;
            self.consume_keyword(Keyword::Else, "expected 'else' after let clause block")?;
            let else_block = self.parse_block_or_inline_stmt_body("let else")?;
            let end = else_block.span;
            return Some(Stmt::LetElse(LetElseStmt {
                clauses,
                pattern: Pattern::Wildcard { span: clauses_end },
                value: Expr::Unit { span: clauses_end },
                else_block,
                span: start.cover(end),
            }));
        }

        if self.match_token(TokenKind::LParen) {
            let bindings = self.parse_binding_list(false)?;
            self.consume(
                TokenKind::RParen,
                "expected ')' after destructuring bindings",
            )?;
            self.consume(TokenKind::Eq, "expected '=' after destructuring bindings")?;
            if self.at(TokenKind::Newline) {
                self.error_at_current(
                    "expected_expression",
                    "expected expression on same line after \"=\"",
                );
                return None;
            }
            let values = self.parse_expr_list()?;
            if values.len() != 1 {
                self.error_at_current(
                    "unexpected_token",
                    "destructuring bindings require a single initializer expression",
                );
                return None;
            }
            let end = values.last().map(Expr::span).unwrap_or(start);
            return Some(Stmt::Binding(BindingStmt {
                visibility: Visibility::Default,
                bindings,
                values,
                span: start.cover(end),
            }));
        }

        let checkpoint = self.checkpoint();
        if self.is_binding_start()
            && !(self.at(TokenKind::Identifier)
                && (self.at_next(TokenKind::LParen) || self.at_next(TokenKind::Dot)))
        {
            if let Some(bindings) = self.parse_binding_list(false) {
                if self.match_token(TokenKind::Eq) {
                    if self.at(TokenKind::Newline) {
                        self.error_at_current(
                            "expected_expression",
                            "expected expression on same line after \"=\"",
                        );
                        return None;
                    }
                    let values = self.parse_expr_list()?;
                    if self.match_keyword(Keyword::Else) {
                        self.error_at_current(
                            "unexpected_token",
                            "plain 'let name = value' bindings do not support 'else'; use a refutable pattern like 'let Some(name) = value else { ... }'",
                        );
                        return None;
                    }
                    if bindings.len() > 1 && values.len() == 1 {
                        self.error_at_current(
                            "unexpected_token",
                            "destructuring bindings require 'let (...) = value'",
                        );
                        return None;
                    }
                    let end = values.last().map(Expr::span).unwrap_or(start);
                    return Some(Stmt::Binding(BindingStmt {
                        visibility: Visibility::Default,
                        bindings,
                        values,
                        span: start.cover(end),
                    }));
                }
            }
        }
        self.restore(checkpoint);

        let pattern = self.parse_pattern()?;
        self.consume(TokenKind::Eq, "expected '=' after let pattern")?;
        if self.at(TokenKind::Newline) {
            self.error_at_current(
                "expected_expression",
                "expected expression on same line after \"=\"",
            );
            return None;
        }
        let value = self.parse_expr()?;
        self.consume_keyword(Keyword::Else, "expected 'else' after let pattern")?;
        let else_block = self.parse_block_or_inline_stmt_body("let else")?;
        let end = else_block.span;
        Some(Stmt::LetElse(LetElseStmt {
            clauses: Vec::new(),
            pattern,
            value,
            else_block,
            span: start.cover(end),
        }))
    }

    fn try_parse_binding_stmt(&mut self) -> Option<BindingStmt> {
        if !self.is_binding_start() {
            return None;
        }
        let checkpoint = self.checkpoint();
        let Some(bindings) = self.parse_binding_list(false) else {
            self.restore(checkpoint);
            return None;
        };
        if !self.match_token(TokenKind::Eq) {
            self.restore(checkpoint);
            return None;
        }
        if self.at(TokenKind::Newline) {
            self.error_at_current(
                "expected_expression",
                "expected expression on same line after \"=\"",
            );
            return None;
        }
        let Some(values) = self.parse_expr_list() else {
            return None;
        };
        if bindings.len() > 1 && values.len() == 1 {
            self.error_at_current(
                "unexpected_token",
                "destructuring bindings require 'let (...) = value'",
            );
            return None;
        }
        let start = bindings[0].span;
        let end = values.last().map(Expr::span).unwrap_or(start);
        Some(BindingStmt {
            visibility: Visibility::Default,
            bindings,
            values,
            span: start.cover(end),
        })
    }

    fn parse_binding(&mut self, mutable: bool) -> Option<Binding> {
        let (name, start) = self.expect_binding_name("expected binding name")?;
        let ty = if self.binding_type_starts_on_same_line(start) && self.can_start_type_ref() {
            Some(self.parse_type_ref()?)
        } else {
            None
        };
        let span = ty.as_ref().map(TypeRef::span).unwrap_or(start);
        Some(Binding {
            name,
            ty,
            mutable,
            deferred: false,
            span: start.cover(span),
        })
    }

    fn parse_binding_list(&mut self, mutable: bool) -> Option<Vec<Binding>> {
        let mut bindings = vec![self.parse_binding(mutable)?];
        while self.match_token(TokenKind::Comma) {
            bindings.push(self.parse_binding(mutable)?);
        }
        Some(bindings)
    }

    fn is_binding_start(&self) -> bool {
        self.at(TokenKind::Identifier)
    }

    fn try_parse_assignment_stmt(&mut self) -> Option<AssignmentStmt> {
        let checkpoint = self.checkpoint();
        let Some(targets) = self.parse_expr_list() else {
            self.restore(checkpoint);
            return None;
        };
        let operator =
            if self.match_token(TokenKind::Eq) || self.match_token(TokenKind::ColonAssign) {
                AssignOp::Reassign
            } else if self.match_token(TokenKind::PlusEq) {
                AssignOp::AddAssign
            } else if self.match_token(TokenKind::MinusEq) {
                AssignOp::SubAssign
            } else if self.match_token(TokenKind::StarEq) {
                AssignOp::MulAssign
            } else if self.match_token(TokenKind::SlashEq) {
                AssignOp::DivAssign
            } else {
                self.restore(checkpoint);
                return None;
            };
        if self.at(TokenKind::Newline) {
            self.error_at_current(
                "expected_expression",
                format!(
                    "expected expression on same line after \"{}\"",
                    self.tokens[self.index.saturating_sub(1)].lexeme
                ),
            );
            return None;
        }
        let Some(values) = self.parse_expr_list() else {
            self.restore(checkpoint);
            return None;
        };
        let start = targets
            .first()
            .map(Expr::span)
            .unwrap_or(self.previous_span());
        let end = values
            .last()
            .map(Expr::span)
            .unwrap_or(self.previous_span());
        Some(AssignmentStmt {
            targets,
            operator,
            values,
            span: start.cover(end),
        })
    }

    fn parse_if_stmt(&mut self) -> Option<IfStmt> {
        let start = self.consume_keyword(Keyword::If, "expected 'if'")?;
        let (condition, pattern, pattern_value, pattern_clauses, bindings, binding_value) =
            if self.match_keyword(Keyword::Let) {
                if self.at(TokenKind::LBrace) {
                    let (clauses, _) = self.parse_refutable_clause_block("if let")?;
                    (None, None, None, clauses, Vec::new(), None)
                } else {
                    let pattern = self.parse_pattern()?;
                    self.consume(TokenKind::Eq, "expected '=' after if pattern")?;
                    if self.at(TokenKind::Newline) {
                        self.error_at_current(
                            "expected_expression",
                            "expected expression on same line after \"=\"",
                        );
                        return None;
                    }
                    let value = self.parse_expr_without_trailing_block_call()?;
                    (
                        None,
                        Some(pattern),
                        Some(value),
                        Vec::new(),
                        Vec::new(),
                        None,
                    )
                }
            } else if self.pattern_followed_by_eq(self.index) {
                self.error_at_current(
                    "unexpected_token",
                    "pattern matches in 'if' require 'let'; use 'if let Pattern = value { ... }'",
                );
                return None;
            } else {
                (
                    Some(self.parse_expr_without_trailing_block_call()?),
                    None,
                    None,
                    Vec::new(),
                    Vec::new(),
                    None,
                )
            };
        let then_block = self.parse_then_stmt_body_block("if")?;
        let else_branch = if self.match_keyword(Keyword::Else) {
            if self.at(TokenKind::Newline) {
                self.error_at_current(
                    "unexpected_token",
                    "else body must stay on the same line unless it uses '{ ... }'",
                );
                return None;
            }
            if self.at_keyword(Keyword::If) {
                Some(ElseBranch::If(Box::new(self.parse_if_stmt()?)))
            } else {
                Some(ElseBranch::Block(
                    self.parse_block_or_inline_stmt_body("else")?,
                ))
            }
        } else {
            None
        };
        let end = else_branch
            .as_ref()
            .map(|branch| match branch {
                ElseBranch::If(if_stmt) => if_stmt.span,
                ElseBranch::Block(block) => block.span,
            })
            .unwrap_or(then_block.span);
        Some(IfStmt {
            condition,
            pattern,
            pattern_value,
            pattern_clauses,
            bindings,
            binding_value,
            then_block,
            else_branch,
            span: start.cover(end),
        })
    }

    fn parse_refutable_clause(&mut self, owner: &'static str) -> Option<RefutableClause> {
        let pattern = self.parse_pattern()?;
        let eq_message = match owner {
            "if let" => "expected '=' after if pattern",
            "let" => "expected '=' after let pattern",
            _ => "expected '=' after pattern",
        };
        self.consume(TokenKind::Eq, eq_message)?;
        if self.at(TokenKind::Newline) {
            self.error_at_current(
                "expected_expression",
                "expected expression on same line after \"=\"",
            );
            return None;
        }
        let value = self.parse_expr_without_trailing_block_call()?;
        let span = pattern.span().cover(value.span());
        Some(RefutableClause {
            pattern,
            value,
            span,
        })
    }

    fn parse_refutable_clause_block(
        &mut self,
        owner: &'static str,
    ) -> Option<(Vec<RefutableClause>, Span)> {
        let open_message = match owner {
            "if let" => "expected '{' after if let",
            "let" => "expected '{' after let",
            _ => "expected '{' before clause block",
        };
        let open = self.consume(TokenKind::LBrace, open_message)?;
        self.skip_newlines();
        let mut clauses = Vec::new();
        while !self.at(TokenKind::RBrace) && !self.at(TokenKind::Eof) {
            clauses.push(self.parse_refutable_clause(owner)?);
            self.skip_newlines();
        }
        if clauses.is_empty() {
            self.error_at_current(
                "unexpected_token",
                format!("{owner} clause block must contain at least one 'PATTERN = value' clause"),
            );
            return None;
        }
        let close_message = match owner {
            "if let" => "expected '}' after if let clause block",
            "let" => "expected '}' after let clause block",
            _ => "expected '}' after clause block",
        };
        let close = self.consume(TokenKind::RBrace, close_message)?;
        Some((clauses, open.cover(close)))
    }

    fn parse_while_stmt(&mut self) -> Option<WhileStmt> {
        let start = self.consume_keyword(Keyword::While, "expected 'while'")?;
        let condition = self.parse_expr_without_trailing_block_call()?;
        let body = self.parse_block()?;
        Some(WhileStmt {
            condition,
            body: body.clone(),
            span: start.cover(body.span),
        })
    }

    fn parse_for_stmt(&mut self) -> Option<ForStmt> {
        let start = self.consume_keyword(Keyword::For, "expected 'for'")?;
        let bindings = self.parse_binding_list(false)?;
        self.consume(TokenKind::LeftArrow, "expected '<-' in for loop")?;
        if self.at(TokenKind::Newline) {
            self.error_at_current(
                "expected_expression",
                "expected expression on same line after \"<-\"",
            );
            return None;
        }
        let iterable = self.parse_expr_without_trailing_block_call()?;
        if !self.at(TokenKind::LBrace) {
            self.error_at_current(
                "unexpected_token",
                "for requires a '{ ... }' block body; one-line for forms are not supported",
            );
            return None;
        }
        let body = self.parse_block()?;
        Some(ForStmt {
            bindings: vec![ForBinding {
                span: bindings
                    .first()
                    .map(|binding| binding.span)
                    .unwrap_or(start)
                    .cover(iterable.span()),
                bindings,
                iterable: Some(iterable),
                values: Vec::new(),
            }],
            body: body.clone(),
            span: start.cover(body.span),
        })
    }

    fn parse_unwrap_stmt(&mut self) -> Option<Stmt> {
        let start = self.consume_keyword(Keyword::Unwrap, "expected 'unwrap'")?;
        if self.match_token(TokenKind::LBrace) {
            return self.parse_unwrap_block_stmt(start).map(Stmt::UnwrapBlock);
        }
        let bindings = self.parse_binding_list(false)?;
        self.consume(TokenKind::LeftArrow, "expected '<-' after unwrap bindings")?;
        if self.at(TokenKind::Newline) {
            self.error_at_current(
                "expected_expression",
                "expected expression on same line after \"<-\"",
            );
            return None;
        }
        let value = self.parse_expr()?;
        let else_block = if self.match_keyword(Keyword::Else) {
            Some(self.parse_block_or_inline_stmt_body("unwrap else")?)
        } else {
            None
        };
        let end = else_block
            .as_ref()
            .map(|block| block.span)
            .unwrap_or_else(|| value.span());
        Some(Stmt::Unwrap(UnwrapStmt {
            bindings,
            value,
            else_block,
            span: start.cover(end),
        }))
    }

    fn parse_unwrap_block_stmt(&mut self, start: Span) -> Option<UnwrapBlockStmt> {
        self.skip_newlines();
        let mut clauses = Vec::new();
        while !self.at(TokenKind::RBrace) && !self.at(TokenKind::Eof) {
            let bindings = self.parse_binding_list(false)?;
            self.consume(TokenKind::LeftArrow, "expected '<-' in unwrap block")?;
            if self.at(TokenKind::Newline) {
                self.error_at_current(
                    "expected_expression",
                    "expected expression on same line after \"<-\"",
                );
                return None;
            }
            let value = self.parse_expr()?;
            let span = bindings
                .first()
                .map(|binding| binding.span)
                .unwrap_or(value.span())
                .cover(value.span());
            clauses.push(UnwrapStmt {
                bindings,
                value,
                else_block: None,
                span,
            });
            self.skip_newlines();
        }
        let close = self.consume(TokenKind::RBrace, "expected '}' after unwrap block")?;
        let else_block = if self.match_keyword(Keyword::Else) {
            Some(self.parse_block_or_inline_stmt_body("unwrap else")?)
        } else {
            None
        };
        let end = else_block.as_ref().map(|block| block.span).unwrap_or(close);
        Some(UnwrapBlockStmt {
            clauses,
            else_block,
            span: start.cover(end),
        })
    }

    fn parse_match_stmt(&mut self, partial: bool) -> Option<MatchStmt> {
        let start = if partial {
            self.consume_keyword(Keyword::Partial, "expected 'partial'")?
        } else {
            self.consume_keyword(Keyword::Match, "expected 'match'")?
        };
        let value = if self.at(TokenKind::LBrace) {
            Expr::Placeholder { span: start }
        } else {
            self.parse_expr_without_trailing_block_call()?
        };
        let (cases, end) = self.parse_match_cases()?;
        Some(MatchStmt {
            partial,
            value,
            cases,
            span: start.cover(end),
        })
    }

    fn parse_match_cases(&mut self) -> Option<(Vec<MatchCase>, Span)> {
        if !self.at(TokenKind::LBrace) {
            self.error_at_current(
                "unexpected_token",
                format!(
                    "expected end of expression, got {}",
                    self.next_significant_token_string()
                ),
            );
            return None;
        }
        self.consume(TokenKind::LBrace, "expected '{' after match value")?;
        self.skip_newlines();
        let mut cases = Vec::new();
        while !self.at(TokenKind::RBrace) && !self.at(TokenKind::Eof) {
            if !self.at_keyword(Keyword::Case) {
                self.error_at_current(
                    "unexpected_token",
                    format!(
                        "expected 'case' before match pattern, got {}",
                        self.next_significant_token_string()
                    ),
                );
                return None;
            }
            self.consume_keyword(Keyword::Case, "expected 'case' before match pattern")?;
            let pattern = self.parse_pattern()?;
            let guard = if self.match_keyword(Keyword::If) {
                Some(self.parse_expr()?)
            } else {
                None
            };
            self.consume(TokenKind::FatArrow, "expected '=>' after match pattern")?;
            let body = if self.at(TokenKind::LBrace) {
                MatchCaseBody::Block(self.parse_block()?)
            } else {
                MatchCaseBody::Expr(self.parse_expr()?)
            };
            let end = match &body {
                MatchCaseBody::Block(block) => block.span,
                MatchCaseBody::Expr(expr) => expr.span(),
            };
            cases.push(MatchCase {
                pattern: pattern.clone(),
                guard,
                body,
                span: pattern.span().cover(end),
            });
            self.skip_newlines();
        }
        let end = self.consume(TokenKind::RBrace, "expected '}' after match cases")?;
        Some((cases, end))
    }

    fn parse_pattern(&mut self) -> Option<Pattern> {
        self.parse_pattern_at_depth(0)
    }

    fn parse_pattern_at_depth(&mut self, depth: usize) -> Option<Pattern> {
        self.skip_newlines();
        match self.current_kind() {
            TokenKind::Identifier if self.is_placeholder_identifier() => {
                let start = self.current_span();
                self.advance();
                if depth == 0
                    && self.binding_type_starts_on_same_line(start)
                    && matches!(
                        self.current_kind(),
                        TokenKind::Identifier | TokenKind::LBrace
                    )
                {
                    let target = self.parse_type_ref()?;
                    return Some(Pattern::Type {
                        name: None,
                        span: start.cover(target.span()),
                        target,
                    });
                }
                Some(Pattern::Wildcard { span: start })
            }
            TokenKind::Integer => {
                let token = self.current().clone();
                self.advance();
                Some(Pattern::Literal {
                    span: token.span,
                    value: Expr::Integer {
                        raw: token.lexeme,
                        span: token.span,
                    },
                })
            }
            TokenKind::Float => {
                let token = self.current().clone();
                self.advance();
                Some(Pattern::Literal {
                    span: token.span,
                    value: Expr::Float {
                        raw: token.lexeme,
                        span: token.span,
                    },
                })
            }
            TokenKind::String => {
                let token = self.current().clone();
                self.advance();
                Some(Pattern::Literal {
                    span: token.span,
                    value: Expr::String {
                        raw: token.lexeme,
                        span: token.span,
                    },
                })
            }
            TokenKind::Keyword(Keyword::True) | TokenKind::Keyword(Keyword::False) => {
                let span = self.current_span();
                let value = self.at_keyword(Keyword::True);
                self.advance();
                Some(Pattern::Literal {
                    span,
                    value: Expr::Bool { value, span },
                })
            }
            TokenKind::LParen => {
                let start = self.consume(TokenKind::LParen, "expected '('")?;
                if self.match_token(TokenKind::RParen) {
                    let end = self.previous_span();
                    let span = start.cover(end);
                    return Some(Pattern::Literal {
                        span,
                        value: Expr::Unit { span },
                    });
                }
                let first = self.parse_pattern_at_depth(depth + 1)?;
                if !self.match_token(TokenKind::Comma) {
                    self.consume(TokenKind::RParen, "expected ')' after pattern")?;
                    return Some(first);
                }
                let mut elements = vec![first];
                loop {
                    elements.push(self.parse_pattern_at_depth(depth + 1)?);
                    if !self.match_token(TokenKind::Comma) {
                        break;
                    }
                }
                let end = self.consume(TokenKind::RParen, "expected ')' after tuple pattern")?;
                Some(Pattern::Tuple {
                    elements,
                    span: start.cover(end),
                })
            }
            TokenKind::Identifier => {
                let (name, start) = self.expect_identifier("expected match pattern")?;
                if depth == 0
                    && self.binding_type_starts_on_same_line(start)
                    && matches!(
                        self.current_kind(),
                        TokenKind::Identifier | TokenKind::LBrace
                    )
                {
                    let target = self.parse_type_ref()?;
                    return Some(Pattern::Type {
                        name: Some(name),
                        span: start.cover(target.span()),
                        target,
                    });
                }
                let mut path = vec![name.clone()];
                let mut end = start;
                while self.match_token(TokenKind::Dot) {
                    let (segment, segment_span) =
                        self.expect_identifier("expected identifier after '.'")?;
                    path.push(segment);
                    end = end.cover(segment_span);
                }
                if self.match_token(TokenKind::LParen) {
                    let mut args = Vec::new();
                    if !self.at(TokenKind::RParen) {
                        loop {
                            args.push(self.parse_pattern_at_depth(depth + 1)?);
                            if !self.match_token(TokenKind::Comma) {
                                break;
                            }
                        }
                    }
                    let close =
                        self.consume(TokenKind::RParen, "expected ')' after constructor pattern")?;
                    return Some(Pattern::Constructor {
                        path,
                        args,
                        span: start.cover(close),
                    });
                }
                if path.len() == 1 {
                    Some(Pattern::Binding { name, span: start })
                } else {
                    Some(Pattern::Constructor {
                        path,
                        args: Vec::new(),
                        span: start.cover(end),
                    })
                }
            }
            _ => {
                self.error_at_current("expected_pattern", "expected match pattern");
                None
            }
        }
    }

    fn parse_return_stmt(&mut self) -> Option<ReturnStmt> {
        let start = self.consume_keyword(Keyword::Return, "expected 'return'")?;
        if self.at(TokenKind::Newline) || self.at(TokenKind::RBrace) || self.at(TokenKind::Eof) {
            return Some(ReturnStmt {
                value: None,
                span: start,
            });
        }
        let value = self.parse_expr()?;
        let end = value.span();
        Some(ReturnStmt {
            value: Some(value),
            span: start.cover(end),
        })
    }

    fn parse_break_stmt(&mut self) -> Option<BreakStmt> {
        let span = self.consume_keyword(Keyword::Break, "expected 'break'")?;
        Some(BreakStmt { span })
    }

    fn parse_expr(&mut self) -> Option<Expr> {
        self.skip_newlines();
        if let Some(lambda) = self.try_parse_lambda_expr() {
            return Some(lambda);
        }
        if self.match_keyword(Keyword::If) {
            return self.parse_if_expr(self.previous_span());
        }
        if self.at_keyword(Keyword::Match) {
            let start = self.consume_keyword(Keyword::Match, "expected 'match'")?;
            return self.parse_match_expr_after_keyword(start, false);
        }
        if self.at_keyword(Keyword::Partial) {
            let start = self.consume_keyword(Keyword::Partial, "expected 'partial'")?;
            return self.parse_match_expr_after_keyword(start, true);
        }
        if self.at_keyword(Keyword::For) {
            let start = self.consume_keyword(Keyword::For, "expected 'for'")?;
            return self.parse_for_yield_expr_after_start(start);
        }
        self.parse_colon_expr()
    }

    fn parse_expr_without_trailing_block_call(&mut self) -> Option<Expr> {
        let previous = self.allow_trailing_block_call;
        self.allow_trailing_block_call = false;
        let result = self.parse_expr();
        self.allow_trailing_block_call = previous;
        result
    }

    fn try_parse_lambda_expr(&mut self) -> Option<Expr> {
        let checkpoint = self.checkpoint();
        if self.at(TokenKind::Identifier) {
            if let Some((name, start)) = self.expect_identifier("expected lambda parameter") {
                let mut ty = None;
                let ty_checkpoint = self.checkpoint();
                if self.can_start_type_ref() {
                    if let Some(primary) = self.parse_primary_type_ref() {
                        if self.at(TokenKind::Arrow) {
                            ty = Some(primary);
                        } else {
                            self.restore(ty_checkpoint);
                            ty = self.parse_type_ref();
                        }
                    } else {
                        self.restore(ty_checkpoint);
                    }
                }
                if self.match_token(TokenKind::Arrow) {
                    let end = ty.as_ref().map(TypeRef::span).unwrap_or(start);
                    let param = LambdaParam {
                        name,
                        ty,
                        span: start.cover(end),
                    };
                    let body = match self.parse_lambda_body() {
                        Some(body) => body,
                        None => {
                            self.restore(checkpoint);
                            return None;
                        }
                    };
                    let end = body.span();
                    return Some(Expr::Lambda {
                        params: vec![param],
                        body,
                        span: start.cover(end),
                    });
                }
            }
            self.restore(checkpoint);
            return None;
        }
        if !self.match_token(TokenKind::LParen) {
            return None;
        }
        let start = self.previous_span();
        let mut params = Vec::new();
        self.skip_newlines();
        if !self.at(TokenKind::RParen) {
            let Some(param) = self.parse_lambda_param() else {
                self.restore(checkpoint);
                return None;
            };
            params.push(param);
            while self.match_token(TokenKind::Comma) {
                let Some(param) = self.parse_lambda_param() else {
                    self.restore(checkpoint);
                    return None;
                };
                params.push(param);
            }
        }
        self.skip_newlines();
        if self
            .consume(TokenKind::RParen, "expected ')' after lambda parameters")
            .is_none()
        {
            self.restore(checkpoint);
            return None;
        }
        if !self.match_token(TokenKind::Arrow) {
            self.restore(checkpoint);
            return None;
        }
        let Some(body) = self.parse_lambda_body() else {
            self.restore(checkpoint);
            return None;
        };
        let end = body.span();
        Some(Expr::Lambda {
            params,
            body,
            span: start.cover(end),
        })
    }

    fn parse_lambda_param(&mut self) -> Option<LambdaParam> {
        let (name, start) = self.expect_identifier("expected lambda parameter")?;
        let ty = if self.can_start_type_ref() {
            Some(self.parse_type_ref()?)
        } else {
            None
        };
        let end = ty.as_ref().map(TypeRef::span).unwrap_or(start);
        Some(LambdaParam {
            name,
            ty,
            span: start.cover(end),
        })
    }

    fn parse_lambda_body(&mut self) -> Option<LambdaBody> {
        if self.at(TokenKind::LBrace) {
            return self.parse_block().map(LambdaBody::Block);
        }
        self.parse_expr()
            .map(|expr| LambdaBody::Expr(Box::new(expr)))
    }

    fn parse_if_expr(&mut self, start: Span) -> Option<Expr> {
        let condition = self.parse_expr_without_trailing_block_call()?;
        let then_block = self.parse_then_expr_body_block("if")?;
        self.consume_keyword(Keyword::Else, "expected 'else' in if expression")?;
        if self.at(TokenKind::Newline) {
            self.error_at_current(
                "unexpected_token",
                "else body must stay on the same line unless it uses '{ ... }'",
            );
            return None;
        }
        let else_branch = if self.at_keyword(Keyword::If) {
            let else_start = self.consume_keyword(Keyword::If, "expected 'if'")?;
            let else_if = self.parse_if_expr(else_start)?;
            ElseExprBranch::If(Box::new(else_if))
        } else {
            ElseExprBranch::Block(self.parse_block_or_inline_expr_body("else")?)
        };
        let end = match &else_branch {
            ElseExprBranch::If(expr) => expr.span(),
            ElseExprBranch::Block(block) => block.span,
        };
        Some(Expr::If {
            condition: Box::new(condition),
            then_block,
            else_branch: Box::new(else_branch),
            span: start.cover(end),
        })
    }

    fn parse_match_expr_after_keyword(&mut self, start: Span, partial: bool) -> Option<Expr> {
        let value = if self.at(TokenKind::LBrace) {
            Expr::Placeholder { span: start }
        } else {
            self.parse_expr_without_trailing_block_call()?
        };
        let (cases, end) = self.parse_match_cases()?;
        Some(Expr::Match {
            partial,
            value: Box::new(value),
            cases,
            span: start.cover(end),
        })
    }

    fn parse_for_yield_expr_after_start(&mut self, start: Span) -> Option<Expr> {
        let bindings = if self.match_token(TokenKind::LBrace) {
            self.parse_for_binding_block()?
        } else {
            let bindings = self.parse_binding_list(false)?;
            self.consume(TokenKind::LeftArrow, "expected '<-' after for bindings")?;
            let iterable = self.parse_expr_without_trailing_block_call()?;
            vec![ForBinding {
                span: bindings
                    .first()
                    .map(|binding| binding.span)
                    .unwrap_or(iterable.span())
                    .cover(iterable.span()),
                bindings,
                iterable: Some(iterable),
                values: Vec::new(),
            }]
        };
        self.consume_keyword(Keyword::Yield, "expected 'yield' after for bindings")?;
        let yield_body = self.parse_yield_body_block()?;
        Some(Expr::ForYield {
            span: start.cover(yield_body.span),
            bindings,
            yield_body,
        })
    }

    fn parse_for_binding_block(&mut self) -> Option<Vec<ForBinding>> {
        self.skip_newlines();
        let mut bindings = Vec::new();
        while !self.at(TokenKind::RBrace) && !self.at(TokenKind::Eof) {
            let mutable = self.match_keyword(Keyword::Var);
            let clause_bindings = self.parse_binding_list(mutable)?;
            if self.match_token(TokenKind::LeftArrow) {
                let iterable = self.parse_expr_without_trailing_block_call()?;
                let span = clause_bindings
                    .first()
                    .map(|binding| binding.span)
                    .unwrap_or(iterable.span())
                    .cover(iterable.span());
                bindings.push(ForBinding {
                    bindings: clause_bindings,
                    iterable: Some(iterable),
                    values: Vec::new(),
                    span,
                });
            } else {
                self.consume(TokenKind::Eq, "expected '=' or '<-' in for binding block")?;
                let values = self.parse_expr_list()?;
                let start = clause_bindings
                    .first()
                    .map(|binding| binding.span)
                    .unwrap_or_else(|| values.last().map(Expr::span).unwrap());
                let end = values
                    .last()
                    .map(Expr::span)
                    .unwrap_or_else(|| clause_bindings.last().map(|binding| binding.span).unwrap());
                bindings.push(ForBinding {
                    bindings: clause_bindings,
                    iterable: None,
                    values,
                    span: start.cover(end),
                });
            }
            self.skip_newlines();
        }
        self.consume(TokenKind::RBrace, "expected '}' after for bindings")?;
        Some(bindings)
    }

    fn parse_yield_body_block(&mut self) -> Option<Block> {
        if self.at(TokenKind::LBrace) {
            self.parse_block()
        } else {
            if self.at(TokenKind::Newline) {
                self.error_at_current(
                    "unexpected_token",
                    "yield body must stay on the same line unless it uses '{ ... }'",
                );
                return None;
            }
            let expr = self.parse_expr()?;
            let span = expr.span();
            Some(Block {
                statements: vec![Stmt::Expr(ExprStmt { expr, span })],
                span,
            })
        }
    }

    fn parse_record_literal_expr(&mut self, start: Span) -> Option<Expr> {
        if self.match_token(TokenKind::LBrace) {
            return self.finish_brace_record_literal_expr(start);
        }
        self.consume(TokenKind::LParen, "expected '{' or '(' after 'record'")?;
        let mut values = Vec::new();
        if !self.at(TokenKind::RParen) {
            values = self.parse_expr_list()?;
        }
        let end = self.consume(TokenKind::RParen, "expected ')' after record literal")?;
        Some(Expr::RecordLiteral {
            fields: Vec::new(),
            values,
            span: start.cover(end),
        })
    }

    fn parse_brace_record_literal_expr(&mut self) -> Option<Expr> {
        let start = self.consume(TokenKind::LBrace, "expected '{'")?;
        self.finish_brace_record_literal_expr(start)
    }

    fn finish_brace_record_literal_expr(&mut self, start: Span) -> Option<Expr> {
        let mut fields = Vec::new();
        self.skip_newlines();
        while !self.at(TokenKind::RBrace) && !self.at(TokenKind::Eof) {
            let (name, name_span) = self.expect_identifier("expected record field name")?;
            self.consume(TokenKind::Eq, "expected '=' after record field name")?;
            let value = self.parse_expr()?;
            fields.push(CallArg {
                name: Some(name),
                span: name_span.cover(value.span()),
                value,
            });
            self.skip_newlines();
            if !self.match_token(TokenKind::Comma) && self.at(TokenKind::Identifier) {
                continue;
            }
            self.skip_newlines();
        }
        let end = self.consume(TokenKind::RBrace, "expected '}' after record literal")?;
        Some(Expr::RecordLiteral {
            fields,
            values: Vec::new(),
            span: start.cover(end),
        })
    }

    fn is_anonymous_interface_expr_start(&self) -> bool {
        if !self.can_start_type_ref() {
            return false;
        }
        let mut parser = Parser {
            tokens: self.tokens,
            index: self.index,
            diagnostics: Vec::new(),
            allow_trailing_block_call: self.allow_trailing_block_call,
        };
        let Some(_) = parser.parse_type_ref() else {
            return false;
        };
        while parser.match_keyword(Keyword::With) {
            if parser.parse_type_ref().is_none() {
                return false;
            }
        }
        if !parser.at(TokenKind::LBrace) {
            return false;
        }
        if parser
            .tokens
            .get(parser.index + 2)
            .is_some_and(|token| token.kind == TokenKind::Eq)
        {
            return false;
        }
        let mut lookahead = parser.index + 1;
        while parser
            .tokens
            .get(lookahead)
            .is_some_and(|token| token.kind == TokenKind::Newline)
        {
            lookahead += 1;
        }
        matches!(
            parser.tokens.get(lookahead).map(|token| token.kind),
            Some(TokenKind::At)
                | Some(TokenKind::Keyword(Keyword::Hidden))
                | Some(TokenKind::Keyword(Keyword::Public))
                | Some(TokenKind::Keyword(Keyword::Def))
        )
    }

    fn parse_anonymous_interface_expr(&mut self) -> Option<Expr> {
        let first = self.parse_type_ref()?;
        let start = first.span();
        let mut interfaces = vec![first];
        while self.match_keyword(Keyword::With) {
            interfaces.push(self.parse_type_ref()?);
        }
        self.consume(
            TokenKind::LBrace,
            "expected '{' after anonymous interface list",
        )?;
        self.skip_newlines();
        let mut methods = Vec::new();
        while !self.at(TokenKind::RBrace) && !self.at(TokenKind::Eof) {
            let annotations = self.parse_annotations()?;
            let visibility = self.parse_visibility();
            if !self.at_keyword(Keyword::Def) {
                self.error_at_current("unexpected_token", "expected anonymous interface member");
                return None;
            }
            methods.push(self.parse_method_decl(annotations, visibility, false)?);
            self.skip_newlines();
        }
        let end = self.consume(
            TokenKind::RBrace,
            "expected '}' after anonymous interface body",
        )?;
        Some(Expr::AnonymousInterface {
            interfaces,
            methods,
            span: start.cover(end),
        })
    }

    fn parse_record_update_args(&mut self) -> Option<(Vec<CallArg>, Span)> {
        let start = self.consume(TokenKind::LBrace, "expected '{' after 'with'")?;
        self.skip_newlines();
        let mut updates = Vec::new();
        while !self.at(TokenKind::RBrace) && !self.at(TokenKind::Eof) {
            let (name, name_span) = self.expect_identifier("expected record field name")?;
            self.consume(TokenKind::Eq, "expected '=' after record field name")?;
            let value = self.parse_expr()?;
            updates.push(CallArg {
                name: Some(name),
                span: name_span.cover(value.span()),
                value,
            });
            self.skip_newlines();
            if !self.match_token(TokenKind::Comma) {
                break;
            }
            self.skip_newlines();
        }
        let end = self.consume(TokenKind::RBrace, "expected '}' after record update")?;
        Some((updates, start.cover(end)))
    }

    fn parse_then_stmt_body_block(&mut self, owner: &'static str) -> Option<Block> {
        if self.at(TokenKind::LBrace) {
            return self.parse_block();
        }
        if !self.at_keyword(Keyword::Then) {
            self.error_at_current(
                "unexpected_token",
                format!(
                    "expected end of expression, got {}",
                    self.next_significant_token_string()
                ),
            );
            return None;
        }
        self.consume_keyword(Keyword::Then, "expected 'then'")?;
        if self.at(TokenKind::Newline) {
            self.error_at_current(
                "unexpected_token",
                format!("{owner} then-body must stay on the same line unless it uses '{{ ... }}'"),
            );
            return None;
        }
        self.parse_block_or_inline_stmt_body(owner)
    }

    fn parse_block_or_inline_stmt_body(&mut self, owner: &'static str) -> Option<Block> {
        if self.at(TokenKind::LBrace) {
            return self.parse_block();
        }
        if self.at(TokenKind::Newline) {
            self.error_at_current(
                "unexpected_token",
                format!("{owner} body must stay on the same line unless it uses '{{ ... }}'"),
            );
            return None;
        }
        if self.at(TokenKind::Eof) {
            self.error_at_current(
                "expected_statement",
                format!("expected statement after {owner}"),
            );
            return None;
        }
        let stmt = self.parse_stmt()?;
        let span = stmt.span();
        Some(Block {
            statements: vec![stmt],
            span,
        })
    }

    fn parse_then_expr_body_block(&mut self, owner: &'static str) -> Option<Block> {
        if self.at(TokenKind::LBrace) {
            return self.parse_block();
        }
        if !self.at_keyword(Keyword::Then) {
            self.error_at_current(
                "unexpected_token",
                format!(
                    "expected end of expression, got {}",
                    self.next_significant_token_string()
                ),
            );
            return None;
        }
        self.consume_keyword(Keyword::Then, "expected 'then'")?;
        if self.at(TokenKind::Newline) {
            self.error_at_current(
                "unexpected_token",
                format!("{owner} then-body must stay on the same line unless it uses '{{ ... }}'"),
            );
            return None;
        }
        self.parse_block_or_inline_expr_body(owner)
    }

    fn parse_block_or_inline_expr_body(&mut self, owner: &'static str) -> Option<Block> {
        if self.at(TokenKind::LBrace) {
            return self.parse_block();
        }
        if self.at(TokenKind::Newline) {
            self.error_at_current(
                "unexpected_token",
                format!("{owner} body must stay on the same line unless it uses '{{ ... }}'"),
            );
            return None;
        }
        if self.at(TokenKind::Eof) {
            self.error_at_current(
                "expected_expression",
                format!("expected expression after {owner}"),
            );
            return None;
        }
        let expr = self.parse_expr()?;
        let span = expr.span();
        Some(Block {
            statements: vec![Stmt::Expr(ExprStmt { expr, span })],
            span,
        })
    }

    fn parse_colon_expr(&mut self) -> Option<Expr> {
        let mut expr = self.parse_or_expr()?;
        loop {
            self.skip_newlines();
            if !self.match_token(TokenKind::Colon) {
                break;
            }
            let right = self.parse_or_expr()?;
            let span = expr.span().cover(right.span());
            expr = Expr::Binary {
                left: Box::new(expr),
                op: BinaryOp::Colon,
                right: Box::new(right),
                span,
            };
        }
        Some(expr)
    }

    fn parse_or_expr(&mut self) -> Option<Expr> {
        self.parse_left_assoc(
            |parser| parser.parse_bit_or_expr(),
            &[(TokenKind::OrOr, BinaryOp::Or)],
        )
    }

    fn parse_bit_or_expr(&mut self) -> Option<Expr> {
        self.parse_left_assoc(
            |parser| parser.parse_and_expr(),
            &[(TokenKind::Pipe, BinaryOp::BitOr)],
        )
    }

    fn parse_and_expr(&mut self) -> Option<Expr> {
        self.parse_left_assoc(
            |parser| parser.parse_bit_and_expr(),
            &[(TokenKind::AndAnd, BinaryOp::And)],
        )
    }

    fn parse_bit_and_expr(&mut self) -> Option<Expr> {
        self.parse_left_assoc(
            |parser| parser.parse_equality_expr(),
            &[(TokenKind::Ampersand, BinaryOp::BitAnd)],
        )
    }

    fn parse_equality_expr(&mut self) -> Option<Expr> {
        let mut expr = self.parse_comparison_expr()?;
        loop {
            self.skip_newlines();
            if self.match_token(TokenKind::EqEq) {
                let right = self.parse_comparison_expr()?;
                let span = expr.span().cover(right.span());
                expr = Expr::Binary {
                    left: Box::new(expr),
                    op: BinaryOp::Eq,
                    right: Box::new(right),
                    span,
                };
                continue;
            }
            if self.match_token(TokenKind::NotEq) {
                let right = self.parse_comparison_expr()?;
                let span = expr.span().cover(right.span());
                expr = Expr::Binary {
                    left: Box::new(expr),
                    op: BinaryOp::NotEq,
                    right: Box::new(right),
                    span,
                };
                continue;
            }
            if self.match_keyword(Keyword::Is) {
                let target = self.parse_type_ref()?;
                let span = expr.span().cover(target.span());
                expr = Expr::Is {
                    left: Box::new(expr),
                    target,
                    span,
                };
                continue;
            }
            break;
        }
        Some(expr)
    }

    fn parse_comparison_expr(&mut self) -> Option<Expr> {
        self.parse_left_assoc(
            |parser| parser.parse_term_expr(),
            &[
                (TokenKind::Less, BinaryOp::Less),
                (TokenKind::LessEq, BinaryOp::LessEq),
                (TokenKind::Greater, BinaryOp::Greater),
                (TokenKind::GreaterEq, BinaryOp::GreaterEq),
            ],
        )
    }

    fn parse_term_expr(&mut self) -> Option<Expr> {
        self.parse_left_assoc(
            |parser| parser.parse_factor_expr(),
            &[
                (TokenKind::Plus, BinaryOp::Add),
                (TokenKind::Minus, BinaryOp::Sub),
                (TokenKind::PlusPlus, BinaryOp::Concat),
                (TokenKind::MinusMinus, BinaryOp::Remove),
                (TokenKind::ColonPlus, BinaryOp::Append),
                (TokenKind::ColonMinus, BinaryOp::Prepend),
            ],
        )
    }

    fn parse_factor_expr(&mut self) -> Option<Expr> {
        self.parse_left_assoc(
            |parser| parser.parse_unary_expr(),
            &[
                (TokenKind::Star, BinaryOp::Mul),
                (TokenKind::Slash, BinaryOp::Div),
                (TokenKind::Percent, BinaryOp::Mod),
            ],
        )
    }

    fn parse_left_assoc<F>(
        &mut self,
        mut parse_operand: F,
        operators: &[(TokenKind, BinaryOp)],
    ) -> Option<Expr>
    where
        F: FnMut(&mut Self) -> Option<Expr>,
    {
        let mut expr = parse_operand(self)?;
        loop {
            self.skip_newlines();
            let mut matched = None;
            for (kind, op) in operators {
                if self.match_token(*kind) {
                    matched = Some(*op);
                    break;
                }
            }
            let Some(op) = matched else {
                break;
            };
            let right = parse_operand(self)?;
            let span = expr.span().cover(right.span());
            expr = Expr::Binary {
                left: Box::new(expr),
                op,
                right: Box::new(right),
                span,
            };
        }
        Some(expr)
    }

    fn parse_unary_expr(&mut self) -> Option<Expr> {
        if self.match_keyword(Keyword::Try) {
            let start = self.previous_span();
            let value = self.parse_unary_expr()?;
            let span = start.cover(value.span());
            return Some(Expr::Try {
                value: Box::new(value),
                span,
            });
        }
        if self.match_token(TokenKind::Bang) {
            let start = self.previous_span();
            let expr = self.parse_unary_expr()?;
            let span = start.cover(expr.span());
            return Some(Expr::Unary {
                op: UnaryOp::Not,
                expr: Box::new(expr),
                span,
            });
        }
        if self.match_token(TokenKind::Minus) {
            let start = self.previous_span();
            let expr = self.parse_unary_expr()?;
            let span = start.cover(expr.span());
            return Some(Expr::Unary {
                op: UnaryOp::Neg,
                expr: Box::new(expr),
                span,
            });
        }
        self.parse_postfix_expr()
    }

    fn parse_postfix_expr(&mut self) -> Option<Expr> {
        let mut expr = self.parse_primary_expr()?;
        loop {
            if self.match_token(TokenKind::LParen) {
                let start = expr.span();
                let args = self.parse_call_args()?;
                let end = self.consume(TokenKind::RParen, "expected ')' after arguments")?;
                expr = Expr::Call {
                    callee: Box::new(expr),
                    args,
                    span: start.cover(end),
                };
                continue;
            }
            if self.match_token(TokenKind::Dot) {
                self.skip_newlines();
                let (name, end) = self.expect_identifier("expected member name after '.'")?;
                let start = expr.span();
                expr = Expr::Member {
                    receiver: Box::new(expr),
                    name,
                    span: start.cover(end),
                };
                continue;
            }
            if self.match_token(TokenKind::LBracket) {
                let start = expr.span();
                let index = self.parse_expr()?;
                let end = self.consume(TokenKind::RBracket, "expected ']' after index")?;
                expr = Expr::Index {
                    receiver: Box::new(expr),
                    index: Box::new(index),
                    span: start.cover(end),
                };
                continue;
            }
            if self.match_keyword(Keyword::With) {
                let start = expr.span();
                let (updates, end) = self.parse_record_update_args()?;
                expr = Expr::RecordUpdate {
                    receiver: Box::new(expr),
                    updates,
                    span: start.cover(end),
                };
                continue;
            }
            if self.match_keyword(Keyword::Match) {
                let start = expr.span();
                let (cases, end) = self.parse_match_cases()?;
                expr = Expr::Match {
                    partial: false,
                    value: Box::new(expr),
                    cases,
                    span: start.cover(end),
                };
                continue;
            }
            if self.allow_trailing_block_call && self.at(TokenKind::LBrace) {
                let start = expr.span();
                let checkpoint = self.checkpoint();
                let arg = if let Some(record) = self.parse_brace_record_literal_expr() {
                    record
                } else {
                    self.restore(checkpoint);
                    let block = self.parse_block()?;
                    Expr::Block {
                        span: block.span,
                        body: block,
                    }
                };
                expr = Expr::Call {
                    callee: Box::new(expr),
                    args: vec![CallArg {
                        name: None,
                        span: arg.span(),
                        value: arg.clone(),
                    }],
                    span: start.cover(arg.span()),
                };
                continue;
            }
            break;
        }
        Some(expr)
    }

    fn parse_call_args(&mut self) -> Option<Vec<CallArg>> {
        let mut args = Vec::new();
        self.skip_newlines();
        if self.at(TokenKind::RParen) {
            return Some(args);
        }
        loop {
            let start = self.current_span();
            if self.at(TokenKind::Identifier) && self.at_next(TokenKind::Eq) {
                let (name, name_span) = self.expect_identifier("expected named argument")?;
                self.consume(TokenKind::Eq, "expected '=' after argument name")?;
                let value = self.parse_expr()?;
                let span = name_span.cover(value.span());
                args.push(CallArg {
                    name: Some(name),
                    value,
                    span,
                });
            } else {
                if self.is_bare_record_call_arg_start() {
                    self.error_at_current(
                        "unexpected_token",
                        "bare '{ ... }' record arguments are not allowed inside '(...)'; use 'Type { ... }' or 'Type(record { ... })'",
                    );
                    return None;
                }
                let value = self.parse_expr()?;
                args.push(CallArg {
                    name: None,
                    span: value.span(),
                    value,
                });
            }
            self.skip_newlines();
            if !self.match_token(TokenKind::Comma) {
                break;
            }
            self.skip_newlines();
            if self.at(TokenKind::RParen) {
                break;
            }
            let _ = start;
        }
        Some(args)
    }

    fn parse_primary_expr(&mut self) -> Option<Expr> {
        self.skip_newlines();
        if self.can_start_type_ref() && self.is_anonymous_interface_expr_start() {
            return self.parse_anonymous_interface_expr();
        }
        match self.current_kind() {
            TokenKind::Identifier => {
                let token = self.current().clone();
                self.advance();
                if token.lexeme == "_" {
                    Some(Expr::Placeholder { span: token.span })
                } else {
                    Some(Expr::Identifier {
                        name: token.lexeme,
                        span: token.span,
                    })
                }
            }
            TokenKind::Integer => {
                let token = self.current().clone();
                self.advance();
                Some(Expr::Integer {
                    raw: token.lexeme,
                    span: token.span,
                })
            }
            TokenKind::Float => {
                let token = self.current().clone();
                self.advance();
                Some(Expr::Float {
                    raw: token.lexeme,
                    span: token.span,
                })
            }
            TokenKind::String => {
                let token = self.current().clone();
                self.advance();
                self.parse_string_expr(token)
            }
            TokenKind::Keyword(Keyword::True) => {
                let span = self.current_span();
                self.advance();
                Some(Expr::Bool { value: true, span })
            }
            TokenKind::Keyword(Keyword::False) => {
                let span = self.current_span();
                self.advance();
                Some(Expr::Bool { value: false, span })
            }
            TokenKind::Keyword(Keyword::Record) => {
                let start = self.consume_keyword(Keyword::Record, "expected 'record'")?;
                self.parse_record_literal_expr(start)
            }
            TokenKind::Keyword(Keyword::Match) => {
                let start = self.consume_keyword(Keyword::Match, "expected 'match'")?;
                self.parse_match_expr_after_keyword(start, false)
            }
            TokenKind::Keyword(Keyword::Partial) => {
                let start = self.consume_keyword(Keyword::Partial, "expected 'partial'")?;
                self.parse_match_expr_after_keyword(start, true)
            }
            TokenKind::Keyword(Keyword::For) => {
                let start = self.consume_keyword(Keyword::For, "expected 'for'")?;
                self.parse_for_yield_expr_after_start(start)
            }
            TokenKind::LBracket => self.parse_list_literal(),
            TokenKind::LParen => self.parse_group_or_tuple_expr(),
            TokenKind::LBrace => {
                let block = self.parse_block()?;
                Some(Expr::Block {
                    span: block.span,
                    body: block,
                })
            }
            _ => {
                self.error_at_current("expected_expression", "expected expression");
                None
            }
        }
    }

    fn is_bare_record_call_arg_start(&self) -> bool {
        if !self.at(TokenKind::LBrace) {
            return false;
        }
        let mut lookahead = self.index + 1;
        while self
            .tokens
            .get(lookahead)
            .is_some_and(|token| token.kind == TokenKind::Newline)
        {
            lookahead += 1;
        }
        self.tokens
            .get(lookahead)
            .is_some_and(|token| token.kind == TokenKind::Identifier)
            && self
                .tokens
                .get(lookahead + 1)
                .is_some_and(|token| token.kind == TokenKind::Eq)
    }

    fn parse_list_literal(&mut self) -> Option<Expr> {
        let start = self.consume(TokenKind::LBracket, "expected '['")?;
        self.skip_newlines();
        let mut items = Vec::new();
        if !self.at(TokenKind::RBracket) {
            items.push(self.parse_expr()?);
            while self.match_token(TokenKind::Comma) {
                self.skip_newlines();
                if self.at(TokenKind::RBracket) {
                    break;
                }
                items.push(self.parse_expr()?);
            }
        }
        let end = self.consume(TokenKind::RBracket, "expected ']' after list literal")?;
        Some(Expr::ListLiteral {
            items,
            span: start.cover(end),
        })
    }

    fn parse_group_or_tuple_expr(&mut self) -> Option<Expr> {
        let start = self.consume(TokenKind::LParen, "expected '('")?;
        self.skip_newlines();
        if self.match_token(TokenKind::RParen) {
            let end = self.previous_span();
            return Some(Expr::Unit {
                span: start.cover(end),
            });
        }

        let first = self.parse_expr()?;
        self.skip_newlines();
        if self.match_token(TokenKind::Comma) {
            let mut items = vec![first];
            self.skip_newlines();
            if !self.at(TokenKind::RParen) {
                items.push(self.parse_expr()?);
                while self.match_token(TokenKind::Comma) {
                    self.skip_newlines();
                    if self.at(TokenKind::RParen) {
                        break;
                    }
                    items.push(self.parse_expr()?);
                }
            }
            let end = self.consume(TokenKind::RParen, "expected ')' after tuple literal")?;
            return Some(Expr::TupleLiteral {
                items,
                span: start.cover(end),
            });
        }
        let end = self.consume(TokenKind::RParen, "expected ')' after grouped expression")?;
        Some(Expr::Group {
            inner: Box::new(first),
            span: start.cover(end),
        })
    }

    fn parse_expr_list(&mut self) -> Option<Vec<Expr>> {
        let mut exprs = vec![self.parse_expr()?];
        while self.match_token(TokenKind::Comma) {
            exprs.push(self.parse_expr()?);
        }
        Some(exprs)
    }

    fn parse_type_params(&mut self) -> Option<Vec<TypeParam>> {
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

    fn parse_param_list(&mut self) -> Option<Vec<Param>> {
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

    fn parse_optional_return_type(&mut self) -> Option<TypeRef> {
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

    fn parse_type_ref_list(&mut self) -> Option<Vec<TypeRef>> {
        let mut refs = vec![self.parse_type_ref()?];
        while self.match_token(TokenKind::Comma) {
            refs.push(self.parse_type_ref()?);
        }
        Some(refs)
    }

    fn parse_type_ref(&mut self) -> Option<TypeRef> {
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

    fn parse_primary_type_ref(&mut self) -> Option<TypeRef> {
        self.skip_newlines();
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

    fn parse_tuple_type_field(&mut self) -> Option<TupleTypeField> {
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

    fn can_start_type_ref(&self) -> bool {
        matches!(
            self.current_kind(),
            TokenKind::Identifier | TokenKind::LParen | TokenKind::LBrace
        )
    }

    fn parse_path_string(&mut self) -> Option<String> {
        let (first, _) = self.expect_identifier("expected path segment")?;
        let mut path = first;
        while self.match_token(TokenKind::Slash) {
            let (segment, _) = self.expect_identifier("expected path segment after '/'")?;
            path.push('/');
            path.push_str(&segment);
        }
        Some(path)
    }

    fn synchronize_item(&mut self) {
        while !self.at(TokenKind::Eof) {
            if self.at(TokenKind::Newline) {
                self.advance();
                return;
            }
            match self.current_kind() {
                TokenKind::Keyword(Keyword::Def)
                | TokenKind::Keyword(Keyword::Class)
                | TokenKind::Keyword(Keyword::Record)
                | TokenKind::Keyword(Keyword::Object)
                | TokenKind::Keyword(Keyword::Interface)
                | TokenKind::Keyword(Keyword::Enum)
                | TokenKind::Keyword(Keyword::Impl) => return,
                _ => self.advance(),
            }
        }
    }

    fn synchronize_member(&mut self) {
        while !self.at(TokenKind::Eof) && !self.at(TokenKind::RBrace) {
            if self.at(TokenKind::Newline) {
                self.advance();
                return;
            }
            self.advance();
        }
    }

    fn synchronize_stmt(&mut self) {
        while !self.at(TokenKind::Eof) && !self.at(TokenKind::RBrace) {
            if self.at(TokenKind::Newline) {
                self.advance();
                return;
            }
            self.advance();
        }
    }

    fn checkpoint(&self) -> Checkpoint {
        Checkpoint {
            index: self.index,
            diagnostics_len: self.diagnostics.len(),
            allow_trailing_block_call: self.allow_trailing_block_call,
        }
    }

    fn restore(&mut self, checkpoint: Checkpoint) {
        self.index = checkpoint.index;
        self.diagnostics.truncate(checkpoint.diagnostics_len);
        self.allow_trailing_block_call = checkpoint.allow_trailing_block_call;
    }

    fn skip_newlines(&mut self) {
        while self.match_token(TokenKind::Newline) {}
    }

    fn consume(&mut self, kind: TokenKind, message: &'static str) -> Option<Span> {
        if self.match_token(kind) {
            Some(self.previous_span())
        } else {
            self.error_at_current("unexpected_token", message);
            None
        }
    }

    fn consume_keyword(&mut self, keyword: Keyword, message: &'static str) -> Option<Span> {
        if self.match_keyword(keyword) {
            Some(self.previous_span())
        } else {
            self.error_at_current("unexpected_token", message);
            None
        }
    }

    fn expect_identifier(&mut self, message: &'static str) -> Option<(String, Span)> {
        if self.at(TokenKind::Identifier) {
            let token = self.current().clone();
            self.advance();
            Some((token.lexeme, token.span))
        } else {
            self.error_at_current("expected_identifier", message);
            None
        }
    }

    fn expect_binding_name(&mut self, message: &'static str) -> Option<(String, Span)> {
        self.expect_identifier(message)
    }

    fn parse_callable_name(&mut self, message: &'static str) -> Option<(String, Span)> {
        if self.at(TokenKind::Identifier) {
            return self.expect_identifier(message);
        }
        if self.match_token(TokenKind::LBracket) {
            let start = self.previous_span();
            let end = self.consume(TokenKind::RBracket, "expected ']' in operator name")?;
            return Some(("[]".to_string(), start.cover(end)));
        }
        let token = self.current().clone();
        let name = match token.kind {
            TokenKind::Plus => "+",
            TokenKind::Minus => "-",
            TokenKind::Star => "*",
            TokenKind::Slash => "/",
            TokenKind::Percent => "%",
            TokenKind::PlusPlus => "++",
            TokenKind::MinusMinus => "--",
            TokenKind::ColonPlus => ":+",
            TokenKind::ColonMinus => ":-",
            _ => {
                self.error_at_current("expected_identifier", message);
                return None;
            }
        };
        self.advance();
        Some((name.to_string(), token.span))
    }

    fn match_keyword(&mut self, keyword: Keyword) -> bool {
        if self.at_keyword(keyword) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn at_keyword(&self, keyword: Keyword) -> bool {
        matches!(self.current_kind(), TokenKind::Keyword(k) if k == keyword)
    }

    fn match_token(&mut self, kind: TokenKind) -> bool {
        if self.at(kind) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn at(&self, kind: TokenKind) -> bool {
        self.current_kind() == kind
    }

    fn at_next(&self, kind: TokenKind) -> bool {
        self.tokens
            .get(self.index + 1)
            .map(|token| token.kind == kind)
            .unwrap_or(false)
    }

    fn binding_type_starts_on_same_line(&self, name_span: Span) -> bool {
        self.current_span().start_pos.line == name_span.end_pos.line
    }

    fn binding_list_followed_by_left_arrow(&self, start: usize) -> bool {
        let mut i = start;
        loop {
            let Some(token) = self.tokens.get(i) else {
                return false;
            };
            if token.kind != TokenKind::Identifier {
                return false;
            }
            i += 1;
            while let Some(token) = self.tokens.get(i) {
                if token.span.start_pos.line != self.tokens[i - 1].span.end_pos.line {
                    break;
                }
                if !matches!(
                    token.kind,
                    TokenKind::Identifier
                        | TokenKind::LParen
                        | TokenKind::LBrace
                        | TokenKind::LBracket
                ) {
                    break;
                }
                if !self.scan_potential_type_ref(i) {
                    break;
                }
                i = self.scan_type_ref_end(i);
            }
            match self.tokens.get(i).map(|token| token.kind) {
                Some(TokenKind::LeftArrow) => return true,
                Some(TokenKind::Comma) => i += 1,
                _ => return false,
            }
        }
    }

    fn pattern_followed_by_eq(&self, start: usize) -> bool {
        let mut parser = Parser {
            tokens: self.tokens,
            index: start,
            diagnostics: Vec::new(),
            allow_trailing_block_call: self.allow_trailing_block_call,
        };
        if parser.parse_pattern().is_none() {
            return false;
        }
        parser.at(TokenKind::Eq)
    }

    fn scan_potential_type_ref(&self, start: usize) -> bool {
        self.tokens.get(start).is_some_and(|token| {
            matches!(
                token.kind,
                TokenKind::Identifier | TokenKind::LParen | TokenKind::LBrace
            )
        })
    }

    fn scan_type_ref_end(&self, start: usize) -> usize {
        let mut i = start;
        let mut paren_depth = 0isize;
        let mut brace_depth = 0isize;
        let mut bracket_depth = 0isize;
        while let Some(token) = self.tokens.get(i) {
            match token.kind {
                TokenKind::LParen => paren_depth += 1,
                TokenKind::RParen => {
                    if paren_depth == 0 {
                        break;
                    }
                    paren_depth -= 1;
                }
                TokenKind::LBrace => brace_depth += 1,
                TokenKind::RBrace => {
                    if brace_depth == 0 {
                        break;
                    }
                    brace_depth -= 1;
                }
                TokenKind::LBracket => bracket_depth += 1,
                TokenKind::RBracket => {
                    if bracket_depth == 0 {
                        break;
                    }
                    bracket_depth -= 1;
                }
                TokenKind::Arrow
                | TokenKind::Comma
                | TokenKind::LeftArrow
                | TokenKind::Eq
                | TokenKind::FatArrow
                | TokenKind::Keyword(Keyword::Then)
                | TokenKind::Keyword(Keyword::Else)
                    if paren_depth == 0 && brace_depth == 0 && bracket_depth == 0 =>
                {
                    break;
                }
                _ => {}
            }
            i += 1;
        }
        i
    }

    fn is_placeholder_identifier(&self) -> bool {
        self.at(TokenKind::Identifier) && self.current().lexeme == "_"
    }

    fn is_for_yield_start(&self) -> bool {
        if !self.at_keyword(Keyword::For) {
            return false;
        }
        if self
            .tokens
            .get(self.index + 1)
            .is_some_and(|token| token.kind == TokenKind::LBrace)
        {
            return true;
        }
        let mut i = self.index + 1;
        while let Some(token) = self.tokens.get(i) {
            match token.kind {
                TokenKind::Keyword(Keyword::Yield) => return true,
                TokenKind::LBrace => return false,
                TokenKind::Newline | TokenKind::Eof => return false,
                _ => i += 1,
            }
        }
        false
    }

    fn current(&self) -> &Token {
        &self.tokens[self.index.min(self.tokens.len().saturating_sub(1))]
    }

    fn current_kind(&self) -> TokenKind {
        self.current().kind
    }

    fn current_span(&self) -> Span {
        self.current().span
    }

    fn previous_span(&self) -> Span {
        self.tokens
            .get(self.index.saturating_sub(1))
            .map(|token| token.span)
            .unwrap_or_else(|| self.current_span())
    }

    fn last_non_newline_span(&self, fallback: Span) -> Span {
        for token in self.tokens[..self.index].iter().rev() {
            if token.kind != TokenKind::Newline {
                return token.span;
            }
        }
        fallback
    }

    fn next_significant_token(&self) -> &Token {
        let mut index = self.index;
        while let Some(token) = self.tokens.get(index) {
            if token.kind != TokenKind::Newline {
                return token;
            }
            index += 1;
        }
        self.current()
    }

    fn next_significant_token_string(&self) -> String {
        self.format_token_like(self.next_significant_token())
    }

    fn format_token_like(&self, token: &Token) -> String {
        format!(
            "{}(\"{}\" @ {}:{})",
            self.token_kind_label(token.kind),
            token.lexeme,
            token.span.start_pos.line,
            token.span.start_pos.column
        )
    }

    fn token_kind_label(&self, kind: TokenKind) -> &'static str {
        match kind {
            TokenKind::Identifier => "IDENT",
            TokenKind::Integer => "INT",
            TokenKind::Float => "FLOAT",
            TokenKind::String => "STRING",
            TokenKind::Keyword(Keyword::Case) => "CASE",
            TokenKind::Keyword(Keyword::If) => "IF",
            TokenKind::Keyword(Keyword::Then) => "THEN",
            TokenKind::Keyword(Keyword::Else) => "ELSE",
            TokenKind::Keyword(Keyword::Match) => "MATCH",
            TokenKind::Keyword(Keyword::Partial) => "PARTIAL",
            TokenKind::Keyword(Keyword::For) => "FOR",
            TokenKind::Keyword(Keyword::Yield) => "YIELD",
            TokenKind::Keyword(Keyword::Unwrap) => "UNWRAP",
            TokenKind::Keyword(Keyword::Def) => "DEF",
            TokenKind::Keyword(Keyword::Class) => "CLASS",
            TokenKind::Keyword(Keyword::Record) => "RECORD",
            TokenKind::Keyword(Keyword::Object) => "OBJECT",
            TokenKind::Keyword(Keyword::Interface) => "INTERFACE",
            TokenKind::Keyword(Keyword::Enum) => "ENUM",
            TokenKind::Keyword(Keyword::Public) => "PUB",
            TokenKind::Keyword(Keyword::Hidden) => "PRIVATE",
            TokenKind::Keyword(Keyword::Var) => "VAR",
            TokenKind::LBrace => "{",
            TokenKind::RBrace => "}",
            TokenKind::LParen => "(",
            TokenKind::RParen => ")",
            TokenKind::LBracket => "[",
            TokenKind::RBracket => "]",
            TokenKind::Eq => "=",
            TokenKind::FatArrow => "=>",
            TokenKind::LeftArrow => "<-",
            TokenKind::Newline => "NEWLINE",
            TokenKind::Eof => "EOF",
            _ => "TOKEN",
        }
    }

    fn advance(&mut self) {
        if !self.at(TokenKind::Eof) {
            self.index += 1;
        }
    }

    fn error_at_current(&mut self, code: &'static str, message: impl Into<String>) {
        let span = self.current_span();
        self.diagnostics
            .push(Diagnostic::error(code, message, span));
    }

    fn parse_string_expr(&mut self, token: Token) -> Option<Expr> {
        if is_multiline_string(&token.lexeme) {
            return Some(Expr::String {
                raw: token.lexeme,
                span: token.span,
            });
        }

        if !string_has_interpolation(&token.lexeme) {
            return Some(Expr::String {
                raw: token.lexeme,
                span: token.span,
            });
        }

        let parts = match parse_interpolated_string_parts(&token.lexeme) {
            Ok(parts) => parts,
            Err(message) => {
                self.diagnostics.push(Diagnostic::error(
                    "invalid_string_interpolation",
                    format!("invalid string interpolation: {message}"),
                    token.span,
                ));
                return None;
            }
        };

        let mut exprs = Vec::with_capacity(parts.len() * 2 + 1);
        if parts.first().is_none_or(|part| !part.is_literal) {
            exprs.push(Expr::String {
                raw: encode_string_literal(""),
                span: token.span,
            });
        }

        for part in parts {
            if part.is_literal {
                let decoded = match decode_string_contents(&part.text) {
                    Ok(decoded) => decoded,
                    Err(message) => {
                        self.diagnostics.push(Diagnostic::error(
                            "invalid_string_literal",
                            format!("invalid string literal: {message}"),
                            token.span,
                        ));
                        return None;
                    }
                };
                exprs.push(Expr::String {
                    raw: encode_string_literal(&decoded),
                    span: token.span,
                });
                continue;
            }

            let expr = match parse_embedded_expr(&part.text) {
                Ok(expr) => expr,
                Err(message) => {
                    self.diagnostics.push(Diagnostic::error(
                        "invalid_string_interpolation",
                        format!("invalid string interpolation: {message}"),
                        token.span,
                    ));
                    return None;
                }
            };
            exprs.push(expr);
        }

        let mut iter = exprs.into_iter();
        let mut left = iter.next()?;
        for right in iter {
            left = Expr::Binary {
                left: Box::new(left),
                op: BinaryOp::Add,
                right: Box::new(right),
                span: token.span,
            };
        }
        Some(left)
    }
}

fn starts_lower(value: &str) -> bool {
    value
        .chars()
        .next()
        .is_some_and(|ch| ch.is_ascii_lowercase())
}

#[derive(Debug, Clone)]
struct InterpolatedStringPart {
    is_literal: bool,
    text: String,
}

fn is_multiline_string(raw: &str) -> bool {
    raw.starts_with("\"\"\"") && raw.ends_with("\"\"\"")
}

fn string_has_interpolation(raw: &str) -> bool {
    let mut chars = raw.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '$' {
            continue;
        }
        if chars.peek().is_some() {
            return true;
        }
    }
    false
}

fn decode_string_contents(raw: &str) -> Result<String, String> {
    let mut out = String::new();
    let mut chars = raw.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        let Some(next) = chars.next() else {
            return Err("dangling escape".to_string());
        };
        match next {
            'n' => out.push('\n'),
            't' => out.push('\t'),
            'r' => out.push('\r'),
            '\\' => out.push('\\'),
            '"' => out.push('"'),
            '$' => out.push('$'),
            other => return Err(format!("unsupported escape \\{other}")),
        }
    }
    Ok(out)
}

fn encode_string_literal(value: &str) -> String {
    let mut raw = String::from("\"");
    for ch in value.chars() {
        match ch {
            '\n' => raw.push_str("\\n"),
            '\t' => raw.push_str("\\t"),
            '\r' => raw.push_str("\\r"),
            '\\' => raw.push_str("\\\\"),
            '"' => raw.push_str("\\\""),
            other => raw.push(other),
        }
    }
    raw.push('"');
    raw
}

fn parse_interpolated_string_parts(raw: &str) -> Result<Vec<InterpolatedStringPart>, String> {
    let body = raw
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .ok_or_else(|| "expected regular string literal".to_string())?;

    let runes: Vec<char> = body.chars().collect();
    let mut parts = Vec::new();
    let mut literal = String::new();
    let flush_literal = |parts: &mut Vec<InterpolatedStringPart>, literal: &mut String| {
        if literal.is_empty() {
            return;
        }
        parts.push(InterpolatedStringPart {
            is_literal: true,
            text: std::mem::take(literal),
        });
    };

    let mut i = 0usize;
    while i < runes.len() {
        match runes[i] {
            '\\' if i + 1 < runes.len() && runes[i + 1] == '$' => {
                literal.push('$');
                i += 2;
            }
            '\\' => {
                literal.push('\\');
                i += 1;
            }
            '$' => {
                if i + 1 >= runes.len() {
                    return Err("dangling '$'".to_string());
                }
                flush_literal(&mut parts, &mut literal);
                if runes[i + 1] == '{' {
                    let start = i + 2;
                    let end = find_interpolated_expr_end(&runes, start)?;
                    let expr: String = runes[start..end].iter().collect();
                    if expr.is_empty() {
                        return Err("empty interpolation".to_string());
                    }
                    parts.push(InterpolatedStringPart {
                        is_literal: false,
                        text: expr,
                    });
                    i = end + 1;
                    continue;
                }
                if !runes[i + 1].is_ascii_alphabetic() {
                    return Err("expected identifier or '{' after '$'".to_string());
                }
                let start = i + 1;
                let mut end = start + 1;
                while end < runes.len() && runes[end].is_ascii_alphanumeric() {
                    end += 1;
                }
                parts.push(InterpolatedStringPart {
                    is_literal: false,
                    text: runes[start..end].iter().collect(),
                });
                i = end;
            }
            ch => {
                literal.push(ch);
                i += 1;
            }
        }
    }

    flush_literal(&mut parts, &mut literal);
    Ok(parts)
}

fn find_interpolated_expr_end(runes: &[char], start: usize) -> Result<usize, String> {
    let mut depth = 1usize;
    let mut i = start;
    while i < runes.len() {
        match runes[i] {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Ok(i);
                }
            }
            '"' => {
                i += 1;
                while i < runes.len() {
                    if runes[i] == '\\' {
                        i += 2;
                        continue;
                    }
                    if runes[i] == '"' {
                        break;
                    }
                    i += 1;
                }
                if i >= runes.len() {
                    return Err("unterminated string inside interpolation".to_string());
                }
            }
            '\'' => {
                i += 1;
                while i < runes.len() {
                    if runes[i] == '\\' {
                        i += 2;
                        continue;
                    }
                    if runes[i] == '\'' {
                        break;
                    }
                    i += 1;
                }
                if i >= runes.len() {
                    return Err("unterminated rune inside interpolation".to_string());
                }
            }
            _ => {}
        }
        i += 1;
    }
    Err("unterminated '${...}'".to_string())
}

fn parse_embedded_expr(source: &str) -> Result<Expr, String> {
    let file = crate::source::SourceFile::new("<interpolation>", source);
    let lexed = crate::lexer::lex(&file);
    if let Some(diag) = lexed.diagnostics.first() {
        return Err(diag.message.clone());
    }

    let mut parser = Parser::new(&lexed.tokens);
    let Some(expr) = parser.parse_expr() else {
        return Err("empty interpolation".to_string());
    };
    parser.skip_newlines();
    if !parser.at(TokenKind::Eof) {
        return Err("unexpected trailing tokens in interpolation".to_string());
    }
    if let Some(diag) = parser.diagnostics.first() {
        return Err(diag.message.clone());
    }
    Ok(expr)
}

impl CallableBody {
    fn span(&self) -> Span {
        match self {
            CallableBody::Block(block) => block.span,
            CallableBody::Expr(expr) => expr.span(),
        }
    }
}

impl LambdaBody {
    fn span(&self) -> Span {
        match self {
            LambdaBody::Expr(expr) => expr.span(),
            LambdaBody::Block(block) => block.span,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{lexer::lex, source::SourceFile};
    use std::{
        fs,
        path::{Path, PathBuf},
    };

    fn parse(src: &str) -> ParseResult {
        let file = SourceFile::new("test.lum", src);
        let lexed = lex(&file);
        assert!(
            lexed.diagnostics.is_empty(),
            "lexer diagnostics: {:#?}",
            lexed.diagnostics
        );
        parse_program(&lexed.tokens)
    }

    fn workspace_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../..")
            .canonicalize()
            .expect("workspace root")
    }

    fn collect_lum_files(dir: &Path, out: &mut Vec<PathBuf>) {
        let entries = fs::read_dir(dir).expect("read dir");
        for entry in entries {
            let entry = entry.expect("dir entry");
            let path = entry.path();
            if path.is_dir() {
                if path.file_name().is_some_and(|name| name == "failures") {
                    continue;
                }
                collect_lum_files(&path, out);
            } else if path.extension().is_some_and(|ext| ext == "lum") {
                out.push(path);
            }
        }
    }

    #[test]
    fn parses_function_with_range_loop() {
        let result = parse(
            r#"
def run(limit Int) Int {
    var total Int = 0
    for i <- Range(0, limit) {
        total += i
    }
    return total
}
"#,
        );
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
        let program = result.program.expect("program");
        assert_eq!(program.items.len(), 1);
        match &program.items[0] {
            Item::Function(function) => {
                assert_eq!(function.name, "run");
                match &function.body {
                    CallableBody::Block(block) => {
                        assert_eq!(block.statements.len(), 3);
                    }
                    other => panic!("expected block body, got {other:#?}"),
                }
            }
            other => panic!("expected function, got {other:#?}"),
        }
    }

    #[test]
    fn parses_class_and_impl() {
        let result = parse(
            r#"
class Counter {
    hidden var count Int
}

impl Counter {
    def init(count Int) {
        this.count = count
    }

    def bump(delta Int) Int = this.count + delta
}
"#,
        );
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
        let program = result.program.expect("program");
        assert_eq!(program.items.len(), 2);
        assert!(matches!(program.items[0], Item::Type(_)));
        assert!(matches!(program.items[1], Item::Impl(_)));
    }

    #[test]
    fn parses_single_expression_function_body() {
        let result = parse("def zero() Int = 0\n");
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
        let program = result.program.expect("program");
        match &program.items[0] {
            Item::Function(function) => match &function.body {
                CallableBody::Expr(Expr::Integer { raw, .. }) => assert_eq!(raw, "0"),
                other => panic!("expected integer expr body, got {other:#?}"),
            },
            other => panic!("expected function, got {other:#?}"),
        }
    }

    #[test]
    fn parses_if_expression_and_calls() {
        let result = parse(
            r#"
def run(flag Bool) Int {
    value Int = if flag {
        foo(1)
    } else {
        bar(2)
    }
    return value
}
"#,
        );
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
        let program = result.program.expect("program");
        match &program.items[0] {
            Item::Function(function) => match &function.body {
                CallableBody::Block(block) => match &block.statements[0] {
                    Stmt::Binding(binding) => {
                        assert_eq!(binding.bindings[0].name, "value");
                    }
                    other => panic!("expected binding, got {other:#?}"),
                },
                other => panic!("expected block body, got {other:#?}"),
            },
            other => panic!("expected function, got {other:#?}"),
        }
    }

    #[test]
    fn parses_lambda_expression() {
        let result = parse("def make() Unit = values.map((x, y) -> x + y)\n");
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn parses_single_param_typed_lambda_without_parens() {
        let result = parse(
            r#"
def main() Unit {
    value = item Int -> item + 1
}
"#,
        );
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn parses_string_interpolation_as_binary_concat() {
        let result = parse(
            r#"
def run(name Str, count Int) Str {
    return "hello $name ${count + 1} \$done"
}
"#,
        );
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
        let program = result.program.expect("program");
        match &program.items[0] {
            Item::Function(function) => match &function.body {
                CallableBody::Block(block) => match &block.statements[0] {
                    Stmt::Return(ret) => {
                        assert!(matches!(ret.value, Some(Expr::Binary { .. })));
                    }
                    other => panic!("expected return statement, got {other:#?}"),
                },
                other => panic!("expected block body, got {other:#?}"),
            },
            other => panic!("expected function, got {other:#?}"),
        }
    }

    #[test]
    fn keeps_multiline_string_as_literal() {
        let result = parse(
            r#"
def run() Str {
    return """
hello
$name
\n
"""
}
"#,
        );
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
        let program = result.program.expect("program");
        match &program.items[0] {
            Item::Function(function) => match &function.body {
                CallableBody::Block(block) => match &block.statements[0] {
                    Stmt::Return(ret) => {
                        assert!(matches!(ret.value, Some(Expr::String { .. })));
                    }
                    other => panic!("expected return statement, got {other:#?}"),
                },
                other => panic!("expected block body, got {other:#?}"),
            },
            other => panic!("expected function, got {other:#?}"),
        }
    }

    #[test]
    fn parses_repo_sources_except_skipped_and_failures() {
        let root = workspace_root();
        let mut files = Vec::new();
        collect_lum_files(&root.join("stdlib"), &mut files);
        collect_lum_files(&root.join("examples"), &mut files);
        files.sort();

        let mut failures = Vec::new();
        for path in files {
            let text = fs::read_to_string(&path).expect("source text");
            if text
                .lines()
                .next()
                .is_some_and(|line| line.trim() == "# SKIP")
            {
                continue;
            }
            let file = SourceFile::new(path.display().to_string(), text);
            let lexed = lex(&file);
            if !lexed.diagnostics.is_empty() {
                failures.push(format!(
                    "lex {}: {:#?}",
                    path.strip_prefix(&root).unwrap_or(&path).display(),
                    lexed.diagnostics
                ));
                continue;
            }
            let parsed = parse_program(&lexed.tokens);
            if !parsed.diagnostics.is_empty() {
                failures.push(format!(
                    "parse {}: {:#?}",
                    path.strip_prefix(&root).unwrap_or(&path).display(),
                    parsed.diagnostics
                ));
            }
        }

        assert!(
            failures.is_empty(),
            "repo parse failures:\n{}",
            failures.join("\n\n")
        );
    }
}
