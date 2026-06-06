use crate::{
    ast::*,
    diagnostic::Diagnostic,
    lexer::{Keyword, Token, TokenKind},
    source::Span,
};

mod pattern;
mod strings;
mod support;
mod types;

#[cfg(test)]
mod tests;

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
        let (
            condition,
            condition_clauses,
            pattern,
            pattern_value,
            pattern_clauses,
            bindings,
            binding_value,
        ) = if self.match_keyword(Keyword::Let) {
            if self.at(TokenKind::LBrace) {
                let (clauses, _) = self.parse_refutable_clause_block("if let")?;
                if self.at(TokenKind::AndAnd) {
                    let initial = clauses.into_iter().map(IfConditionClause::Let).collect();
                    let clauses = self.parse_if_condition_clauses(initial)?;
                    (None, clauses, None, None, Vec::new(), Vec::new(), None)
                } else {
                    (None, Vec::new(), None, None, clauses, Vec::new(), None)
                }
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
                let value = self.parse_if_condition_expr()?;
                if self.at(TokenKind::AndAnd) {
                    let clauses = self.parse_if_condition_clauses(vec![IfConditionClause::Let(
                        RefutableClause {
                            span: pattern.span().cover(value.span()),
                            pattern,
                            value,
                        },
                    )])?;
                    (None, clauses, None, None, Vec::new(), Vec::new(), None)
                } else {
                    (
                        None,
                        Vec::new(),
                        Some(pattern),
                        Some(value),
                        Vec::new(),
                        Vec::new(),
                        None,
                    )
                }
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
                Vec::new(),
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
            condition_clauses,
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
        if !self.match_keyword(Keyword::Else) {
            self.error_at_current(
                "unsupported_syntax",
                "bare 'unwrap x <- y' syntax was removed; use 'value = try source' for propagation or add 'else' for an explicit fallback",
            );
            return None;
        }
        let else_block = Some(self.parse_block_or_inline_stmt_body("unwrap else")?);
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
        if !self.match_keyword(Keyword::Else) {
            self.error_at_current(
                "unsupported_syntax",
                "bare 'unwrap { ... }' syntax was removed; add 'else' or use 'let { PATTERN = value ... } else { ... }'",
            );
            return None;
        }
        let else_block = Some(self.parse_block_or_inline_stmt_body("unwrap else")?);
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
        self.error_at_current(
            "unexpected_token",
            "anonymous record literals use 'record { ... }'; 'record(...)' is not supported",
        );
        None
    }

    fn parse_brace_record_literal_expr(&mut self) -> Option<Expr> {
        let start = self.consume(TokenKind::LBrace, "expected '{'")?;
        self.finish_brace_record_literal_expr(start)
    }

    fn finish_brace_record_literal_expr(&mut self, start: Span) -> Option<Expr> {
        #[derive(Clone)]
        struct RecordEntry {
            name: Option<String>,
            value: Expr,
            span: Span,
        }

        let mut entries = Vec::new();
        self.skip_newlines();
        while !self.at(TokenKind::RBrace) && !self.at(TokenKind::Eof) {
            let entry = if self.at(TokenKind::Identifier) && self.at_next(TokenKind::Eq) {
                let (name, name_span) = self.expect_identifier("expected record field name")?;
                self.consume(TokenKind::Eq, "expected '=' after record field name")?;
                let value = self.parse_expr()?;
                RecordEntry {
                    name: Some(name),
                    span: name_span.cover(value.span()),
                    value,
                }
            } else {
                let value = self.parse_expr()?;
                RecordEntry {
                    name: None,
                    span: value.span(),
                    value,
                }
            };
            entries.push(entry.clone());
            self.skip_newlines();
            if self.match_token(TokenKind::Comma) {
                self.skip_newlines();
                continue;
            }
        }
        let end = self.consume(TokenKind::RBrace, "expected '}' after record literal")?;
        let has_named = entries.iter().any(|entry| entry.name.is_some());
        let mut fields = Vec::new();
        let mut values = Vec::new();
        if has_named {
            for entry in entries {
                if let Some(name) = entry.name {
                    fields.push(CallArg {
                        name: Some(name),
                        value: entry.value,
                        span: entry.span,
                    });
                    continue;
                }
                self.diagnostics.push(Diagnostic::error(
                    "unexpected_token",
                    "cannot mix named and positional record fields",
                    entry.span,
                ));
                return None;
            }
        } else {
            values = entries.into_iter().map(|entry| entry.value).collect();
        }
        Some(Expr::RecordLiteral {
            fields,
            values,
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
                let mut lambda_probe = self.checkpoint();
                lambda_probe.index += 1;
                while self
                    .tokens
                    .get(lambda_probe.index)
                    .is_some_and(|token| token.kind == TokenKind::Newline)
                {
                    lambda_probe.index += 1;
                }
                self.restore(lambda_probe);
                let prefers_block = self.try_parse_lambda_expr().is_some();
                self.restore(checkpoint);
                let arg = if !prefers_block {
                    if let Some(record) = self.parse_brace_record_literal_expr() {
                        record
                    } else {
                        self.restore(checkpoint);
                        let block = self.parse_block()?;
                        Expr::Block {
                            span: block.span,
                            body: block,
                        }
                    }
                } else {
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
            && (self
                .tokens
                .get(lookahead + 1)
                .is_some_and(|token| token.kind == TokenKind::Eq)
                || self.tokens.get(lookahead + 1).is_some_and(|token| {
                    token.kind == TokenKind::Comma || token.kind == TokenKind::RBrace
                })
                || self.tokens.get(lookahead + 1).is_some_and(|token| {
                    token.span.start_pos.line > self.tokens[lookahead].span.start_pos.line
                }))
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
}

fn starts_lower(value: &str) -> bool {
    value
        .chars()
        .next()
        .is_some_and(|ch| ch.is_ascii_lowercase())
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
