use super::stmt::ForClauseTarget;
use super::*;

impl<'a> Parser<'a> {
    fn wrap_if_without_else(&self, start: Span, condition: Expr, then_block: Block) -> Expr {
        let span = start.cover(then_block.span);
        Expr::Block {
            body: Block {
                statements: vec![Stmt::If(IfStmt {
                    condition: Some(condition),
                    condition_clauses: Vec::new(),
                    pattern: None,
                    pattern_value: None,
                    pattern_clauses: Vec::new(),
                    bindings: Vec::new(),
                    binding_value: None,
                    then_block,
                    else_branch: None,
                    span,
                })],
                span,
            },
            span,
        }
    }

    pub(super) fn parse_expr(&mut self) -> Option<Expr> {
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
            return self.parse_partial_match_expr_after_partial(start);
        }
        if self.at_keyword(Keyword::For) {
            let start = self.consume_keyword(Keyword::For, "expected 'for'")?;
            return self.parse_for_yield_expr_after_start(start);
        }
        self.parse_colon_expr()
    }

    pub(super) fn parse_expr_without_trailing_block_call(&mut self) -> Option<Expr> {
        let previous = self.allow_trailing_block_call;
        self.allow_trailing_block_call = false;
        let result = self.parse_expr();
        self.allow_trailing_block_call = previous;
        result
    }

    pub(super) fn try_parse_lambda_expr(&mut self) -> Option<Expr> {
        let checkpoint = self.checkpoint();
        if self.match_keyword(Keyword::Let) {
            let start = self.previous_span();
            let Some(param) = self.parse_lambda_destructure_param(start, 0) else {
                self.restore(checkpoint);
                return None;
            };
            if !self.match_token(TokenKind::Arrow) {
                self.restore(checkpoint);
                return None;
            }
            self.diagnostics.push(Diagnostic::error(
                "invalid_lambda_params",
                "lambda parameters cannot use 'let' destructuring; name the parameter and destructure inside the lambda body",
                param.span,
            ));
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
        if self.at(TokenKind::Identifier) {
            if let Some((name, start)) = self.expect_identifier("expected lambda parameter") {
                if self.match_token(TokenKind::Arrow) {
                    let param = LambdaParam {
                        name,
                        ty: None,
                        destructure: None,
                        span: start,
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

                let ty_checkpoint = self.checkpoint();
                if self.can_start_type_ref() {
                    if let Some(ty) = self.parse_primary_type_ref() {
                        if self.match_token(TokenKind::Arrow) {
                            let ty_span = ty.span();
                            self.diagnostics.push(Diagnostic::error(
                                "invalid_lambda_params",
                                "typed single-parameter lambdas must use parentheses; write '(x T) -> ...'",
                                start.cover(ty_span),
                            ));
                            let param = LambdaParam {
                                name,
                                ty: Some(ty),
                                destructure: None,
                                span: start.cover(ty_span),
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
                    self.restore(ty_checkpoint);
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
            let Some(param) = self.parse_lambda_param(params.len()) else {
                self.restore(checkpoint);
                return None;
            };
            params.push(param);
            while self.match_token(TokenKind::Comma) {
                self.skip_newlines();
                let Some(param) = self.parse_lambda_param(params.len()) else {
                    self.restore(checkpoint);
                    return None;
                };
                params.push(param);
            }
        }
        self.skip_newlines();
        let Some(close) = self.consume(TokenKind::RParen, "expected ')' after lambda parameters")
        else {
            self.restore(checkpoint);
            return None;
        };
        if !self.match_token(TokenKind::Arrow) {
            self.restore(checkpoint);
            return None;
        }
        let simple_params = params
            .iter()
            .filter(|param| param.destructure.is_none())
            .collect::<Vec<_>>();
        let typed_count = simple_params
            .iter()
            .filter(|param| param.ty.is_some())
            .count();
        if typed_count > 0 && typed_count < simple_params.len() {
            self.diagnostics.push(Diagnostic::error(
                "invalid_lambda_params",
                "lambda parameters must be either all typed or all untyped",
                start.cover(close),
            ));
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

    pub(super) fn parse_lambda_param(&mut self, index: usize) -> Option<LambdaParam> {
        if self.match_keyword(Keyword::Let) {
            let start = self.previous_span();
            self.diagnostics.push(Diagnostic::error(
                "invalid_lambda_params",
                "lambda parameters cannot use 'let' destructuring; name the parameter and destructure inside the lambda body",
                start,
            ));
            return self.parse_lambda_destructure_param(start, index);
        }
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
            destructure: None,
            span: start.cover(end),
        })
    }

    pub(super) fn parse_lambda_destructure_param(
        &mut self,
        start: Span,
        index: usize,
    ) -> Option<LambdaParam> {
        let (kind, bindings, end) = if self.match_token(TokenKind::LParen) {
            let bindings = self.parse_binding_list(false)?;
            let end = self.consume(
                TokenKind::RParen,
                "expected ')' after lambda tuple destructuring parameter",
            )?;
            (DestructureKind::Tuple, bindings, end)
        } else if self.match_token(TokenKind::LBrace) {
            let bindings = self.parse_brace_destructure_binding_list(false)?;
            let end = self.consume(
                TokenKind::RBrace,
                "expected '}' after lambda class destructuring parameter",
            )?;
            (DestructureKind::Record, bindings, end)
        } else {
            self.error_at_current(
                "expected_lambda_param",
                "expected '(' or '{' after 'let' in lambda parameter",
            );
            return None;
        };
        Some(LambdaParam {
            name: format!("$lambda_param{index}"),
            ty: None,
            destructure: Some(LambdaParamDestructure { kind, bindings }),
            span: start.cover(end),
        })
    }

    pub(super) fn parse_lambda_body(&mut self) -> Option<LambdaBody> {
        if self.at(TokenKind::LBrace) {
            return self.parse_block().map(LambdaBody::Block);
        }
        if self.at(TokenKind::Newline) {
            return self.parse_implicit_lambda_body();
        }
        self.parse_expr()
            .map(|expr| LambdaBody::Expr(Box::new(expr)))
    }

    fn parse_implicit_lambda_body(&mut self) -> Option<LambdaBody> {
        self.skip_newlines();
        let Some(first) = self.parse_stmt() else {
            self.synchronize_stmt();
            self.error_at_current(
                "expected_expression",
                "expected expression or lambda body after '->'",
            );
            return None;
        };
        let body_indent = first.span().start_pos.column;
        let mut statements = vec![first];
        loop {
            self.skip_newlines();
            if self.at(TokenKind::RParen)
                || self.at(TokenKind::RBrace)
                || self.at(TokenKind::Comma)
                || self.at(TokenKind::Eof)
                || self.current_span().start_pos.column < body_indent
            {
                break;
            }
            if let Some(stmt) = self.parse_stmt() {
                statements.push(stmt);
            } else {
                self.synchronize_stmt();
                break;
            }
        }

        let first = statements
            .first()
            .cloned()
            .expect("lambda body first statement");
        let span = first
            .span()
            .cover(statements.last().map(Stmt::span).unwrap_or(first.span()));

        if statements.len() == 1 {
            if let Stmt::Expr(ExprStmt { expr, .. }) = statements.remove(0) {
                return Some(LambdaBody::Expr(Box::new(expr)));
            }
        }

        Some(LambdaBody::Block(Block { statements, span }))
    }

    pub(super) fn parse_if_expr(&mut self, start: Span) -> Option<Expr> {
        let condition = self.parse_expr_without_trailing_block_call()?;
        let then_block = self.parse_if_body_block()?;
        if !self.match_keyword(Keyword::Else) {
            return Some(self.wrap_if_without_else(start, condition, then_block));
        }
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

    pub(super) fn parse_match_expr_after_keyword(
        &mut self,
        start: Span,
        partial: bool,
    ) -> Option<Expr> {
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

    fn parse_partial_match_expr_after_partial(&mut self, start: Span) -> Option<Expr> {
        self.consume_keyword(Keyword::Match, "expected 'match' after 'partial'")?;
        self.parse_match_expr_after_keyword(start, true)
    }

    pub(super) fn parse_for_yield_expr_after_start(&mut self, start: Span) -> Option<Expr> {
        let bindings =
            if self.at(TokenKind::LBrace) && !self.is_for_brace_destructuring_binding_start() {
                self.consume(TokenKind::LBrace, "expected '{' after 'for'")?;
                self.parse_for_binding_block()?
            } else {
                let target = self.parse_for_clause_target(false)?;
                self.consume(TokenKind::LeftArrow, "expected '<-' after for bindings")?;
                let iterable = self.parse_expr_without_trailing_block_call()?;
                let (bindings, destructure, pattern, target_span) = match target {
                    ForClauseTarget::Bindings {
                        bindings,
                        destructure,
                    } => {
                        let target_span = bindings
                            .first()
                            .map(|binding| binding.span)
                            .unwrap_or(iterable.span());
                        (bindings, destructure, None, target_span)
                    }
                    ForClauseTarget::Pattern(pattern) => {
                        let pattern_span = pattern.span();
                        (Vec::new(), None, Some(pattern), pattern_span)
                    }
                };
                vec![ForBinding {
                    span: target_span.cover(iterable.span()),
                    bindings,
                    destructure,
                    pattern,
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

    pub(super) fn parse_for_binding_block(&mut self) -> Option<Vec<ForBinding>> {
        self.skip_newlines();
        let mut bindings = Vec::new();
        while !self.at(TokenKind::RBrace) && !self.at(TokenKind::Eof) {
            let mutable = self.match_keyword(Keyword::Var);
            let target = self.parse_for_clause_target(mutable)?;
            if self.match_token(TokenKind::LeftArrow) {
                let iterable = self.parse_expr_without_trailing_block_call()?;
                let (clause_bindings, destructure, pattern, target_span) = match target {
                    ForClauseTarget::Bindings {
                        bindings,
                        destructure,
                    } => {
                        let target_span = bindings
                            .first()
                            .map(|binding| binding.span)
                            .unwrap_or(iterable.span());
                        (bindings, destructure, None, target_span)
                    }
                    ForClauseTarget::Pattern(pattern) => {
                        let pattern_span = pattern.span();
                        (Vec::new(), None, Some(pattern), pattern_span)
                    }
                };
                let span = target_span.cover(iterable.span());
                bindings.push(ForBinding {
                    bindings: clause_bindings,
                    destructure,
                    pattern,
                    iterable: Some(iterable),
                    values: Vec::new(),
                    span,
                });
            } else {
                self.consume(TokenKind::Eq, "expected '=' or '<-' in for binding block")?;
                let values = self.parse_expr_list()?;
                let (clause_bindings, destructure, pattern, target_span) = match target {
                    ForClauseTarget::Bindings {
                        bindings,
                        destructure,
                    } => {
                        let target_span = bindings
                            .first()
                            .map(|binding| binding.span)
                            .unwrap_or_else(|| values.last().map(Expr::span).unwrap());
                        (bindings, destructure, None, target_span)
                    }
                    ForClauseTarget::Pattern(pattern) => {
                        let pattern_span = pattern.span();
                        (Vec::new(), None, Some(pattern), pattern_span)
                    }
                };
                let start = target_span;
                let end = values
                    .last()
                    .map(Expr::span)
                    .unwrap_or_else(|| clause_bindings.last().map(|binding| binding.span).unwrap());
                if (destructure.is_some() || pattern.is_some()) && values.len() != 1 {
                    self.error_at_current(
                        "unexpected_token",
                        "pattern and destructuring for-bindings require a single initializer expression",
                    );
                    return None;
                }
                bindings.push(ForBinding {
                    bindings: clause_bindings,
                    destructure,
                    pattern,
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

    pub(super) fn parse_yield_body_block(&mut self) -> Option<Block> {
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

    pub(super) fn parse_brace_record_literal_expr(&mut self) -> Option<Expr> {
        let start = self.consume(TokenKind::LBrace, "expected '{'")?;
        self.finish_brace_record_literal_expr(start)
    }

    pub(super) fn finish_brace_record_literal_expr(&mut self, start: Span) -> Option<Expr> {
        #[derive(Clone)]
        struct RecordEntry {
            name: Option<String>,
            ty: Option<TypeRef>,
            value: Expr,
            span: Span,
        }

        let mut entries = Vec::new();
        self.skip_newlines();
        while !self.at(TokenKind::RBrace) && !self.at(TokenKind::Eof) {
            let entry = if self.at(TokenKind::Identifier) {
                let checkpoint = self.checkpoint();
                let (name, name_span) = self.expect_identifier("expected record field name")?;
                if self.match_token(TokenKind::Colon) {
                    let value = self.parse_expr()?;
                    RecordEntry {
                        name: Some(name),
                        ty: None,
                        span: name_span.cover(value.span()),
                        value,
                    }
                } else if self.can_start_type_ref() {
                    let ty = self.parse_type_ref();
                    if let Some(ty) = ty {
                        if self.match_token(TokenKind::Colon) {
                            let value = self.parse_expr()?;
                            RecordEntry {
                                name: Some(name),
                                ty: Some(ty),
                                span: name_span.cover(value.span()),
                                value,
                            }
                        } else {
                            self.restore(checkpoint);
                            let value = self.parse_expr()?;
                            RecordEntry {
                                name: None,
                                ty: None,
                                span: value.span(),
                                value,
                            }
                        }
                    } else {
                        self.restore(checkpoint);
                        let value = self.parse_expr()?;
                        RecordEntry {
                            name: None,
                            ty: None,
                            span: value.span(),
                            value,
                        }
                    }
                } else {
                    self.restore(checkpoint);
                    let value = self.parse_expr()?;
                    RecordEntry {
                        name: None,
                        ty: None,
                        span: value.span(),
                        value,
                    }
                }
            } else {
                let value = self.parse_expr()?;
                RecordEntry {
                    name: None,
                    ty: None,
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
        if has_named {
            for entry in entries {
                if let Some(name) = entry.name {
                    fields.push(CallArg {
                        name: Some(name),
                        ty: entry.ty,
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
            if !entries.is_empty() {
                self.diagnostics.push(Diagnostic::error(
                    "positional_brace_construction",
                    "braces are for named fields; use 'Type(...)' for positional constructors or assign a tuple to an explicitly typed shape",
                    start.cover(end),
                ));
                return None;
            }
        }
        Some(Expr::RecordLiteral {
            fields,
            values: Vec::new(),
            span: start.cover(end),
        })
    }

    pub(super) fn is_anonymous_interface_expr_start(&self) -> bool {
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

    pub(super) fn parse_anonymous_interface_expr(&mut self) -> Option<Expr> {
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

    pub(super) fn parse_record_update_args(&mut self) -> Option<(Vec<CallArg>, Span)> {
        let start = self.consume(TokenKind::LBrace, "expected '{' after ':<'")?;
        self.skip_newlines();
        let mut updates = Vec::new();
        while !self.at(TokenKind::RBrace) && !self.at(TokenKind::Eof) {
            let (name, name_span) = self.expect_identifier("expected record field name")?;
            self.consume(
                TokenKind::Colon,
                "expected ':' after record update field name",
            )?;
            let value = self.parse_expr()?;
            updates.push(CallArg {
                name: Some(name),
                ty: None,
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

    pub(super) fn parse_block_or_inline_stmt_body(&mut self, owner: &'static str) -> Option<Block> {
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

    pub(super) fn parse_if_body_block(&mut self) -> Option<Block> {
        if self.at(TokenKind::LBrace) {
            return self.parse_block();
        }
        let message = if self.at(TokenKind::Identifier) && self.current().lexeme == "then" {
            "'then' is unsupported; use 'if condition { ... }'"
        } else {
            "expected '{' after if condition"
        };
        self.error_at_current("unexpected_token", message);
        None
    }

    pub(super) fn parse_block_or_inline_expr_body(&mut self, owner: &'static str) -> Option<Block> {
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

    pub(super) fn parse_colon_expr(&mut self) -> Option<Expr> {
        let mut expr = self.parse_or_expr()?;
        loop {
            let op = if self.match_token(TokenKind::Colon) {
                BinaryOp::Colon
            } else if self.match_token(TokenKind::ColonPlus) {
                BinaryOp::RecordMerge
            } else {
                break;
            };
            self.skip_newlines();
            let right = self.parse_or_expr()?;
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

    pub(super) fn parse_or_expr(&mut self) -> Option<Expr> {
        self.parse_left_assoc(
            |parser| parser.parse_bit_or_expr(),
            &[(TokenKind::OrOr, BinaryOp::Or)],
        )
    }

    pub(super) fn parse_bit_or_expr(&mut self) -> Option<Expr> {
        self.parse_left_assoc(
            |parser| parser.parse_and_expr(),
            &[(TokenKind::Pipe, BinaryOp::BitOr)],
        )
    }

    pub(super) fn parse_and_expr(&mut self) -> Option<Expr> {
        self.parse_left_assoc(
            |parser| parser.parse_bit_and_expr(),
            &[(TokenKind::AndAnd, BinaryOp::And)],
        )
    }

    pub(super) fn parse_bit_and_expr(&mut self) -> Option<Expr> {
        self.parse_left_assoc(
            |parser| parser.parse_equality_expr(),
            &[(TokenKind::Ampersand, BinaryOp::BitAnd)],
        )
    }

    pub(super) fn parse_equality_expr(&mut self) -> Option<Expr> {
        let mut expr = self.parse_comparison_expr()?;
        loop {
            if self.match_token(TokenKind::EqEq) {
                self.skip_newlines();
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
                self.skip_newlines();
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
                self.skip_newlines();
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

    pub(super) fn parse_comparison_expr(&mut self) -> Option<Expr> {
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

    pub(super) fn parse_term_expr(&mut self) -> Option<Expr> {
        self.parse_left_assoc(
            |parser| parser.parse_factor_expr(),
            &[
                (TokenKind::Plus, BinaryOp::Add),
                (TokenKind::Minus, BinaryOp::Sub),
            ],
        )
    }

    pub(super) fn parse_factor_expr(&mut self) -> Option<Expr> {
        self.parse_left_assoc(
            |parser| parser.parse_unary_expr(),
            &[
                (TokenKind::Star, BinaryOp::Mul),
                (TokenKind::Slash, BinaryOp::Div),
                (TokenKind::Percent, BinaryOp::Mod),
            ],
        )
    }

    pub(super) fn parse_left_assoc<F>(
        &mut self,
        mut parse_operand: F,
        operators: &[(TokenKind, BinaryOp)],
    ) -> Option<Expr>
    where
        F: FnMut(&mut Self) -> Option<Expr>,
    {
        let mut expr = parse_operand(self)?;
        loop {
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
            self.skip_newlines();
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

    pub(super) fn parse_unary_expr(&mut self) -> Option<Expr> {
        if self.match_keyword(Keyword::Try) {
            let start = self.previous_span();
            self.skip_newlines();
            let value = self.parse_unary_expr()?;
            let span = start.cover(value.span());
            return Some(Expr::Try {
                value: Box::new(value),
                span,
            });
        }
        if self.match_token(TokenKind::Bang) {
            let start = self.previous_span();
            self.skip_newlines();
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
            self.skip_newlines();
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

    pub(super) fn parse_postfix_expr(&mut self) -> Option<Expr> {
        let mut expr = self.parse_primary_expr()?;
        loop {
            if self.match_token(TokenKind::LParen) {
                let start = expr.span();
                let args = self.parse_call_args()?;
                let end = self.consume(TokenKind::RParen, "expected ')' after arguments")?;
                expr = Expr::Call {
                    callee: Box::new(expr),
                    args,
                    uses_brace_syntax: false,
                    span: start.cover(end),
                };
                continue;
            }
            if self.match_token(TokenKind::Dot) {
                self.skip_newlines();
                let (name, end) = if self.at(TokenKind::Keyword(Keyword::Expect)) {
                    let token = self.current().clone();
                    self.advance();
                    (token.lexeme, token.span)
                } else {
                    self.expect_identifier("expected member name after '.'")?
                };
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
            if self.match_token(TokenKind::ColonLess) {
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
                let open_span = self.current_span();
                let checkpoint = self.checkpoint();
                let mut lambda_probe = self.checkpoint();
                lambda_probe.index += 1;
                let lambda_head_can_start = self
                    .tokens
                    .get(lambda_probe.index)
                    .is_some_and(|token| token.span.start_pos.line == open_span.start_pos.line);
                self.restore(lambda_probe);
                let prefers_block = lambda_head_can_start && self.try_parse_lambda_expr().is_some();
                self.restore(checkpoint);
                let arg = if !prefers_block
                    && (self.looks_like_brace_record_literal(true)
                        || Self::is_constructor_like_expr(&expr))
                {
                    self.parse_brace_record_literal_expr()?
                } else {
                    let block = self.parse_block()?;
                    self.validate_trailing_lambda_block(&block, open_span);
                    Expr::Block {
                        span: block.span,
                        body: block,
                    }
                };
                expr = Expr::Call {
                    callee: Box::new(expr),
                    args: vec![CallArg {
                        name: None,
                        ty: None,
                        span: arg.span(),
                        value: arg.clone(),
                    }],
                    uses_brace_syntax: true,
                    span: start.cover(arg.span()),
                };
                continue;
            }
            break;
        }
        Some(expr)
    }

    fn validate_trailing_lambda_block(&mut self, block: &Block, open_span: Span) {
        let [
            Stmt::Expr(ExprStmt {
                expr: Expr::Lambda { span, .. },
                ..
            }),
        ] = block.statements.as_slice()
        else {
            return;
        };

        if span.start_pos.line != open_span.start_pos.line {
            self.diagnostics.push(Diagnostic::error(
                "invalid_trailing_lambda",
                "trailing lambda parameters must start on the same line as '{'",
                *span,
            ));
        }
    }

    pub(super) fn parse_call_args(&mut self) -> Option<Vec<CallArg>> {
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
                    ty: None,
                    value,
                    span,
                });
            } else {
                let value = self.parse_expr()?;
                args.push(CallArg {
                    name: None,
                    ty: None,
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

    pub(super) fn parse_primary_expr(&mut self) -> Option<Expr> {
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
            TokenKind::Keyword(Keyword::Class) => {
                let _ = self.consume_keyword(Keyword::Class, "expected 'class'")?;
                let message = if self.at(TokenKind::LBrace) {
                    "anonymous record literals now use '{ ... }'; 'class { ... }' was removed"
                } else {
                    "anonymous record literals use '{ ... }'; 'class(...)' is not supported"
                };
                self.error_at_current("unexpected_token", message);
                None
            }
            TokenKind::Keyword(Keyword::Record) => {
                let _ = self.consume_keyword(Keyword::Record, "expected 'record'")?;
                let message = if self.at(TokenKind::LBrace) {
                    "anonymous record literals now use '{ ... }'; 'record { ... }' was removed"
                } else {
                    "anonymous record literals use '{ ... }'; 'record(...)' is not supported"
                };
                self.error_at_current("unexpected_token", message);
                None
            }
            TokenKind::Keyword(Keyword::Shape) => {
                let _ = self.consume_keyword(Keyword::Shape, "expected 'shape'")?;
                let message = if self.at(TokenKind::LBrace) {
                    "anonymous shape literals use '{ ... }'; 'shape { ... }' is not supported"
                } else if self.at(TokenKind::LParen) {
                    "shape(...) expression syntax was removed; assign a tuple to an explicitly typed shape"
                } else {
                    "anonymous shape literals use '{ ... }'"
                };
                self.error_at_current("unexpected_token", message);
                None
            }
            TokenKind::Keyword(Keyword::Match) => {
                let start = self.consume_keyword(Keyword::Match, "expected 'match'")?;
                self.parse_match_expr_after_keyword(start, false)
            }
            TokenKind::Keyword(Keyword::Partial) => {
                let start = self.consume_keyword(Keyword::Partial, "expected 'partial'")?;
                self.parse_partial_match_expr_after_partial(start)
            }
            TokenKind::Keyword(Keyword::For) => {
                let start = self.consume_keyword(Keyword::For, "expected 'for'")?;
                self.parse_for_yield_expr_after_start(start)
            }
            TokenKind::LBracket => self.parse_list_literal(),
            TokenKind::LParen => self.parse_group_or_tuple_expr(),
            TokenKind::LBrace => {
                if self.looks_like_brace_record_literal(false) {
                    self.parse_brace_record_literal_expr()
                } else {
                    let block = self.parse_block()?;
                    Some(Expr::Block {
                        span: block.span,
                        body: block,
                    })
                }
            }
            _ => {
                self.error_at_current("expected_expression", "expected expression");
                None
            }
        }
    }

    pub(super) fn looks_like_brace_record_literal(&self, allow_empty: bool) -> bool {
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
        if self
            .tokens
            .get(lookahead)
            .is_some_and(|token| token.kind == TokenKind::RBrace)
        {
            return allow_empty;
        }
        if self
            .tokens
            .get(lookahead)
            .is_some_and(|token| token.kind == TokenKind::Identifier)
        {
            if self
                .tokens
                .get(lookahead + 1)
                .is_some_and(|token| token.kind == TokenKind::Colon)
            {
                return true;
            }

            let mut parser = Parser {
                tokens: self.tokens,
                index: lookahead + 1,
                diagnostics: Vec::new(),
                allow_trailing_block_call: self.allow_trailing_block_call,
            };
            if parser.parse_type_ref().is_some() && parser.at(TokenKind::Colon) {
                return true;
            }
        }

        let mut depth = 1usize;
        let mut nested_parens = 0usize;
        let mut nested_brackets = 0usize;
        while let Some(token) = self.tokens.get(lookahead) {
            match token.kind {
                TokenKind::LBrace => depth += 1,
                TokenKind::RBrace => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        break;
                    }
                }
                TokenKind::LParen => nested_parens += 1,
                TokenKind::RParen => nested_parens = nested_parens.saturating_sub(1),
                TokenKind::LBracket => nested_brackets += 1,
                TokenKind::RBracket => nested_brackets = nested_brackets.saturating_sub(1),
                TokenKind::Comma if depth == 1 && nested_parens == 0 && nested_brackets == 0 => {
                    return true;
                }
                _ => {}
            }
            lookahead += 1;
        }
        false
    }

    fn is_constructor_like_expr(expr: &Expr) -> bool {
        match expr {
            Expr::Identifier { name, .. } => name
                .chars()
                .next()
                .is_some_and(|ch| ch.is_ascii_uppercase()),
            Expr::Member { receiver, name, .. } => {
                Self::is_constructor_like_expr(receiver)
                    && name
                        .chars()
                        .next()
                        .is_some_and(|ch| ch.is_ascii_uppercase())
            }
            _ => false,
        }
    }

    pub(super) fn parse_list_literal(&mut self) -> Option<Expr> {
        let start = self.consume(TokenKind::LBracket, "expected '['")?;
        self.skip_newlines();
        let mut items = Vec::new();
        if !self.at(TokenKind::RBracket) {
            items.push(self.parse_expr()?);
            self.skip_newlines();
            while self.match_token(TokenKind::Comma) {
                self.skip_newlines();
                if self.at(TokenKind::RBracket) {
                    break;
                }
                items.push(self.parse_expr()?);
                self.skip_newlines();
            }
        }
        let end = self.consume(TokenKind::RBracket, "expected ']' after list literal")?;
        Some(Expr::ListLiteral {
            items,
            span: start.cover(end),
        })
    }

    pub(super) fn parse_group_or_tuple_expr(&mut self) -> Option<Expr> {
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

    pub(super) fn parse_expr_list(&mut self) -> Option<Vec<Expr>> {
        let mut exprs = vec![self.parse_expr()?];
        while self.match_token(TokenKind::Comma) {
            exprs.push(self.parse_expr()?);
        }
        Some(exprs)
    }
}
