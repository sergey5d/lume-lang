use super::*;

impl<'a> Parser<'a> {
    fn at_match_case_body_boundary(&self) -> bool {
        matches!(
            self.next_significant_token().kind,
            TokenKind::Keyword(Keyword::Case) | TokenKind::RBrace | TokenKind::Eof
        )
    }

    pub(super) fn parse_block(&mut self) -> Option<Block> {
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

    pub(super) fn parse_stmt(&mut self) -> Option<Stmt> {
        self.skip_newlines();
        match self.current_kind() {
            TokenKind::Keyword(Keyword::Def) => {
                let function = self.parse_function_decl(Vec::new(), Visibility::Default, false)?;
                Some(Stmt::LocalFunction(function))
            }
            TokenKind::Keyword(Keyword::Match) => self.parse_match_stmt(false).map(Stmt::Match),
            TokenKind::Keyword(Keyword::Partial) => self.parse_match_stmt(true).map(Stmt::Match),
            TokenKind::Keyword(Keyword::Let) => {
                let checkpoint = self.checkpoint();
                if let Some(expr) = self.try_parse_lambda_expr() {
                    let span = expr.span();
                    return Some(Stmt::Expr(ExprStmt { expr, span }));
                }
                self.restore(checkpoint);
                self.parse_let_stmt()
            }
            TokenKind::Keyword(Keyword::Guard) => self.parse_guard_stmt(),
            TokenKind::Keyword(Keyword::Expect) => self.parse_expect_stmt(),
            TokenKind::Keyword(Keyword::Var) => {
                let stmt = self.parse_binding_stmt_after_var()?;
                Some(Stmt::Binding(stmt))
            }
            TokenKind::Keyword(Keyword::Defer) => self.parse_defer_stmt().map(Stmt::Defer),
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
            TokenKind::Keyword(Keyword::Continue) => self.parse_continue_stmt().map(Stmt::Continue),
            _ => {
                if self.at_removed_assert_statement() {
                    self.error_at_current(
                        "removed_assert_statement",
                        "assert statement syntax was removed; use assert(condition) or assert(condition, message)",
                    );
                    return None;
                }
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

    pub(super) fn parse_binding_stmt_after_var(&mut self) -> Option<BindingStmt> {
        let start = self.consume_keyword(Keyword::Var, "expected 'var'")?;
        let bindings = self.parse_binding_list(true)?;
        self.consume(TokenKind::Eq, "expected '=' after bindings")?;
        let values = self.parse_expr_list()?;
        if bindings.len() > 1 && values.len() == 1 {
            self.error_at_current(
                "unexpected_token",
                "destructuring bindings require 'let (...) = value' or 'let { ... } = value'",
            );
            return None;
        }
        let end = values.last().map(Expr::span).unwrap_or(start);
        Some(BindingStmt {
            visibility: Visibility::Default,
            bindings,
            values,
            destructure: None,
            span: start.cover(end),
        })
    }

    pub(super) fn parse_defer_stmt(&mut self) -> Option<DeferStmt> {
        let start = self.consume_keyword(Keyword::Defer, "expected 'defer'")?;
        if self.at(TokenKind::LBrace) {
            let block = self.parse_block()?;
            return Some(DeferStmt {
                action: DeferAction::Block(block.clone()),
                span: start.cover(block.span),
            });
        }
        if self.at(TokenKind::Newline) {
            self.error_at_current(
                "expected_expression",
                "expected call expression or block on same line after \"defer\"",
            );
            return None;
        }
        let expr = self.parse_expr()?;
        if !matches!(expr, Expr::Call { .. }) {
            self.diagnostics.push(Diagnostic::error(
                "invalid_defer_target",
                "defer expects a call expression or block",
                expr.span(),
            ));
            return None;
        }
        let end = expr.span();
        Some(DeferStmt {
            action: DeferAction::Call(expr),
            span: start.cover(end),
        })
    }

    pub(super) fn parse_let_stmt(&mut self) -> Option<Stmt> {
        let start = self.consume_keyword(Keyword::Let, "expected 'let'")?;

        if self.at(TokenKind::LBrace) {
            if self.is_brace_destructuring_binding_start() {
                self.consume(TokenKind::LBrace, "expected '{' after 'let'")?;
                let bindings = self.parse_brace_destructure_binding_list(false)?;
                self.consume(
                    TokenKind::RBrace,
                    "expected '}' after destructuring bindings",
                )?;
                self.consume(TokenKind::Eq, "expected '=' after destructuring bindings")?;
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
                    destructure: Some(DestructureKind::Record),
                    span: start.cover(end),
                }));
            }
            let (clauses, clauses_end) = self.parse_refutable_clause_block("let")?;
            if self.match_keyword(Keyword::Else) {
                self.error_at_current(
                    "guard_required",
                    "recoverable refutable bindings use 'guard { ... } else ...'; keep 'let' for irrefutable destructuring",
                );
                return None;
            }
            return Some(Stmt::PatternBinding(PatternBindingStmt {
                kind: PatternBindingKind::Let,
                clauses,
                pattern: Pattern::Wildcard { span: clauses_end },
                value: Expr::Unit { span: clauses_end },
                span: start.cover(clauses_end),
            }));
        }

        if self.match_token(TokenKind::LParen) {
            let bindings = self.parse_binding_list(false)?;
            self.consume(
                TokenKind::RParen,
                "expected ')' after destructuring bindings",
            )?;
            self.consume(TokenKind::Eq, "expected '=' after destructuring bindings")?;
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
                destructure: Some(DestructureKind::Tuple),
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
                    if bindings.len() == 1 && bindings[0].ty.is_none() {
                        self.error_at_current(
                            "plain_let_binding",
                            "plain 'let name = value' is not supported; use 'name = value' for ordinary bindings, or use 'let' for destructuring/pattern matching",
                        );
                        return None;
                    }
                    let values = self.parse_expr_list()?;
                    if self.match_keyword(Keyword::Else) {
                        if bindings.len() == 1 && bindings[0].ty.is_some() && values.len() == 1 {
                        } else {
                            self.error_at_current(
                                "unexpected_token",
                                "plain 'let name = value' bindings do not support 'else'; use a refutable pattern like 'guard Some(name) = value else { ... }'",
                            );
                            return None;
                        }
                    } else {
                        if bindings.len() > 1 && values.len() == 1 {
                            self.error_at_current(
                                "unexpected_token",
                                "destructuring bindings require 'let (...) = value' or 'let { ... } = value'",
                            );
                            return None;
                        }
                        let end = values.last().map(Expr::span).unwrap_or(start);
                        return Some(Stmt::Binding(BindingStmt {
                            visibility: Visibility::Default,
                            bindings,
                            values,
                            destructure: None,
                            span: start.cover(end),
                        }));
                    }
                }
            }
        }
        self.restore(checkpoint);

        let (pattern, operator) = self.parse_refutable_pattern_head("let")?;
        if operator != "=" && self.at(TokenKind::Newline) {
            self.error_at_current(
                "expected_expression",
                format!("expected expression on same line after \"{operator}\""),
            );
            return None;
        }
        let value = self.parse_expr()?;
        if self.match_keyword(Keyword::Else) {
            self.error_at_current(
                "guard_required",
                "recoverable refutable bindings use 'guard Pattern = value else ...'; keep 'let' for irrefutable destructuring",
            );
            return None;
        }
        let end = value.span();
        Some(Stmt::PatternBinding(PatternBindingStmt {
            kind: PatternBindingKind::Let,
            clauses: Vec::new(),
            pattern,
            value,
            span: start.cover(end),
        }))
    }

    pub(super) fn parse_guard_stmt(&mut self) -> Option<Stmt> {
        let start = self.consume_keyword(Keyword::Guard, "expected 'guard'")?;

        if self.at(TokenKind::LBrace) {
            let (clauses, clauses_end) = self.parse_refutable_clause_block("guard")?;
            if !self.match_keyword(Keyword::Else) {
                self.error_at_current("expected_else", "guard bindings require an 'else' fallback");
                return None;
            }
            let else_block = self.parse_block_or_inline_stmt_body("guard else")?;
            let end = else_block.span;
            return Some(Stmt::LetElse(LetElseStmt {
                clauses,
                pattern: Pattern::Wildcard { span: clauses_end },
                value: Expr::Unit { span: clauses_end },
                else_block,
                span: start.cover(end),
            }));
        }

        let (pattern, operator) = self.parse_refutable_pattern_head("guard")?;
        if operator != "=" && self.at(TokenKind::Newline) {
            self.error_at_current(
                "expected_expression",
                format!("expected expression on same line after \"{operator}\""),
            );
            return None;
        }
        let value = self.parse_expr()?;
        if !self.match_keyword(Keyword::Else) {
            self.error_at_current("expected_else", "guard bindings require an 'else' fallback");
            return None;
        }
        let else_block = self.parse_block_or_inline_stmt_body("guard else")?;
        let end = else_block.span;
        Some(Stmt::LetElse(LetElseStmt {
            clauses: Vec::new(),
            pattern,
            value,
            else_block,
            span: start.cover(end),
        }))
    }

    pub(super) fn parse_expect_stmt(&mut self) -> Option<Stmt> {
        let start = self.consume_keyword(Keyword::Expect, "expected 'expect'")?;

        if self.at(TokenKind::LBrace) {
            let (clauses, clauses_end) = self.parse_refutable_clause_block("expect")?;
            if self.match_keyword(Keyword::Else) {
                self.error_at_current(
                    "unexpected_token",
                    "expect does not support 'else'; use 'guard ... else ...' for recoverable pattern matching",
                );
                return None;
            }
            return Some(Stmt::PatternBinding(PatternBindingStmt {
                kind: PatternBindingKind::Expect,
                clauses,
                pattern: Pattern::Wildcard { span: clauses_end },
                value: Expr::Unit { span: clauses_end },
                span: start.cover(clauses_end),
            }));
        }

        let checkpoint = self.checkpoint();
        if let Some(pattern) = self.parse_pattern() {
            let (pattern, operator) = if self.match_token(TokenKind::Eq) {
                (pattern, "=")
            } else if self.match_token(TokenKind::LeftArrow) {
                (self.wrap_extract_pattern(pattern), "<-")
            } else {
                self.error_at_current(
                    "expected_pattern_binding",
                    "expect only supports pattern/assertive binding; use assert(condition) for boolean assertions",
                );
                return None;
            };
            if operator != "=" && self.at(TokenKind::Newline) {
                self.error_at_current(
                    "expected_expression",
                    format!("expected expression on same line after \"{operator}\""),
                );
                return None;
            }
            let value = self.parse_expr()?;
            if self.match_keyword(Keyword::Else) {
                self.error_at_current(
                    "unexpected_token",
                    "expect does not support 'else'; use 'guard ... else ...' for recoverable pattern matching",
                );
                return None;
            }
            let end = value.span();
            return Some(Stmt::PatternBinding(PatternBindingStmt {
                kind: PatternBindingKind::Expect,
                clauses: Vec::new(),
                pattern,
                value,
                span: start.cover(end),
            }));
        }
        self.restore(checkpoint);
        self.error_at_current(
            "expected_pattern_binding",
            "expect only supports pattern/assertive binding; use assert(condition) for boolean assertions",
        );
        None
    }

    pub(super) fn try_parse_binding_stmt(&mut self) -> Option<BindingStmt> {
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
        let Some(values) = self.parse_expr_list() else {
            return None;
        };
        if bindings.len() > 1 && values.len() == 1 {
            self.error_at_current(
                "unexpected_token",
                "destructuring bindings require 'let (...) = value' or 'let { ... } = value'",
            );
            return None;
        }
        let start = bindings[0].span;
        let end = values.last().map(Expr::span).unwrap_or(start);
        Some(BindingStmt {
            visibility: Visibility::Default,
            bindings,
            values,
            destructure: None,
            span: start.cover(end),
        })
    }

    fn at_removed_assert_statement(&self) -> bool {
        self.current_kind() == TokenKind::Identifier
            && self.current().lexeme == "assert"
            && matches!(
                self.tokens.get(self.index + 1).map(|token| token.kind),
                Some(
                    TokenKind::Identifier
                        | TokenKind::Integer
                        | TokenKind::Float
                        | TokenKind::String
                        | TokenKind::Bang
                        | TokenKind::Minus
                        | TokenKind::LBracket
                        | TokenKind::LBrace
                        | TokenKind::Keyword(Keyword::True)
                        | TokenKind::Keyword(Keyword::False)
                )
            )
    }

    pub(super) fn parse_binding(&mut self, mutable: bool) -> Option<Binding> {
        let (name, start) = self.expect_binding_name("expected binding name")?;
        let ty = if self.binding_type_starts_on_same_line(start) && self.can_start_type_ref() {
            Some(self.parse_type_ref()?)
        } else {
            None
        };
        let span = ty.as_ref().map(TypeRef::span).unwrap_or(start);
        Some(Binding {
            name,
            field_name: None,
            ty,
            mutable,
            span: start.cover(span),
        })
    }

    pub(super) fn parse_brace_destructure_binding(&mut self, mutable: bool) -> Option<Binding> {
        if self.at(TokenKind::At) {
            self.error_at_current(
                "unexpected_token",
                "brace destructuring uses 'field', 'field Type', 'field as local', or 'field Type as local'; '@field' is unsupported",
            );
            return None;
        }

        let (field_name, field_span) =
            self.expect_identifier("expected field name in brace destructuring")?;
        if field_name == "_" {
            self.error_at_current(
                "unexpected_token",
                "brace destructuring matches by field name; omit fields you do not need",
            );
            return None;
        }

        let ty = if self.binding_type_starts_on_same_line(field_span) && self.can_start_type_ref() {
            Some(self.parse_type_ref()?)
        } else {
            None
        };
        let typed_span = ty.as_ref().map(TypeRef::span).unwrap_or(field_span);

        let (name, end) = if self.match_keyword(Keyword::As) {
            let (alias, alias_span) =
                self.expect_binding_name("expected local binding name after 'as'")?;
            if alias == "_" {
                self.error_at_current(
                    "unexpected_token",
                    "brace destructuring matches by field name; omit fields you do not need",
                );
                return None;
            }
            (alias, alias_span)
        } else {
            (field_name.clone(), typed_span)
        };
        Some(Binding {
            name,
            field_name: Some(field_name),
            ty,
            mutable,
            span: field_span.cover(end),
        })
    }

    pub(super) fn parse_brace_destructure_binding_list(
        &mut self,
        mutable: bool,
    ) -> Option<Vec<Binding>> {
        let mut bindings = vec![self.parse_brace_destructure_binding(mutable)?];
        while self.match_token(TokenKind::Comma) {
            bindings.push(self.parse_brace_destructure_binding(mutable)?);
        }
        Some(bindings)
    }

    pub(super) fn parse_binding_list(&mut self, mutable: bool) -> Option<Vec<Binding>> {
        let mut bindings = vec![self.parse_binding(mutable)?];
        while self.match_token(TokenKind::Comma) {
            bindings.push(self.parse_binding(mutable)?);
        }
        Some(bindings)
    }

    pub(super) fn parse_plain_for_generator_binding(&mut self) -> Option<Binding> {
        const MESSAGE: &str = "for generator must bind a plain identifier before '<-'; destructure inside the body with 'let'";
        if !self.at(TokenKind::Identifier) || self.is_placeholder_identifier() {
            self.error_at_current("invalid_for_generator", MESSAGE);
            return None;
        }
        let (name, span) = self.expect_binding_name(MESSAGE)?;
        Some(Binding {
            name,
            field_name: None,
            ty: None,
            mutable: false,
            span,
        })
    }

    pub(super) fn consume_for_generator_arrow(&mut self) -> Option<Span> {
        const MESSAGE: &str = "for generator must bind a plain identifier before '<-'; type annotations and destructuring belong in a 'let' inside the body";
        if self.match_token(TokenKind::LeftArrow) {
            Some(self.previous_span())
        } else {
            self.error_at_current("invalid_for_generator", MESSAGE);
            None
        }
    }

    pub(super) fn is_binding_start(&self) -> bool {
        self.at(TokenKind::Identifier)
    }

    pub(super) fn is_brace_destructuring_binding_start(&self) -> bool {
        if !self.at(TokenKind::LBrace) {
            return false;
        }
        let mut parser = Parser {
            tokens: self.tokens,
            index: self.index,
            diagnostics: Vec::new(),
            allow_trailing_block_call: self.allow_trailing_block_call,
            allow_shape_update_operator: self.allow_shape_update_operator,
        };
        if !parser.match_token(TokenKind::LBrace) {
            return false;
        }
        if parser.at(TokenKind::At) || parser.is_placeholder_identifier() {
            return true;
        }
        if !parser.at(TokenKind::Identifier) {
            return false;
        }
        if parser.parse_brace_destructure_binding_list(false).is_none() {
            return false;
        }
        parser.match_token(TokenKind::RBrace) && parser.at(TokenKind::Eq)
    }

    pub(super) fn try_parse_assignment_stmt(&mut self) -> Option<AssignmentStmt> {
        let checkpoint = self.checkpoint();
        let Some(targets) = self.parse_expr_list() else {
            self.restore(checkpoint);
            return None;
        };
        let operator = if self.match_token(TokenKind::Eq) {
            AssignOp::Assign
        } else if self.match_token(TokenKind::ColonAssign) {
            AssignOp::Reassign
        } else if self.match_token(TokenKind::PlusEq) {
            AssignOp::AddAssign
        } else if self.match_token(TokenKind::MinusEq) {
            AssignOp::SubAssign
        } else if self.match_token(TokenKind::StarEq) {
            AssignOp::MulAssign
        } else if self.match_token(TokenKind::SlashEq) {
            AssignOp::DivAssign
        } else if self.match_token(TokenKind::PercentEq) {
            AssignOp::ModAssign
        } else {
            self.restore(checkpoint);
            return None;
        };
        if !matches!(operator, AssignOp::Assign) && self.at(TokenKind::Newline) {
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

    pub(super) fn parse_if_stmt(&mut self) -> Option<IfStmt> {
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
                let clause = self.parse_if_condition_refutable_clause("if let")?;
                if self.at(TokenKind::AndAnd) {
                    let clauses =
                        self.parse_if_condition_clauses(vec![IfConditionClause::Let(clause)])?;
                    (None, clauses, None, None, Vec::new(), Vec::new(), None)
                } else {
                    (
                        None,
                        Vec::new(),
                        Some(clause.pattern),
                        Some(clause.value),
                        Vec::new(),
                        Vec::new(),
                        None,
                    )
                }
            }
        } else if self.pattern_followed_by_refutable_operator(self.index) {
            self.error_at_current(
                "unexpected_token",
                "pattern matches in 'if' require 'let'; use 'if let Pattern = value { ... }' or 'if let Pattern <- value { ... }'",
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
        let then_block = self.parse_if_body_block()?;
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

    pub(super) fn parse_while_stmt(&mut self) -> Option<WhileStmt> {
        let start = self.consume_keyword(Keyword::While, "expected 'while'")?;
        let condition = self.parse_expr_without_trailing_block_call()?;
        let body = self.parse_block()?;
        Some(WhileStmt {
            condition,
            body: body.clone(),
            span: start.cover(body.span),
        })
    }

    pub(super) fn parse_for_stmt(&mut self) -> Option<ForStmt> {
        let start = self.consume_keyword(Keyword::For, "expected 'for'")?;
        let binding = self.parse_plain_for_generator_binding()?;
        self.consume_for_generator_arrow()?;
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
        let target_span = binding.span;
        Some(ForStmt {
            bindings: vec![ForBinding {
                span: target_span.cover(iterable.span()),
                bindings: vec![binding],
                destructure: None,
                pattern: None,
                iterable: Some(iterable),
                values: Vec::new(),
            }],
            body: body.clone(),
            span: start.cover(body.span),
        })
    }

    pub(super) fn parse_match_stmt(&mut self, partial: bool) -> Option<MatchStmt> {
        let start = if partial {
            let start = self.consume_keyword(Keyword::Partial, "expected 'partial'")?;
            self.consume_keyword(Keyword::Match, "expected 'match' after 'partial'")?;
            start
        } else {
            self.consume_keyword(Keyword::Match, "expected 'match'")?
        };
        if self.at(TokenKind::LBrace) {
            self.error_missing_match_value(partial);
            return None;
        }
        let value = self.parse_expr_without_trailing_block_call()?;
        let (cases, end) = self.parse_match_cases()?;
        Some(MatchStmt {
            partial,
            value,
            cases,
            span: start.cover(end),
        })
    }

    pub(super) fn parse_match_cases(&mut self) -> Option<(Vec<MatchCase>, Span)> {
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
            let body = self.parse_match_case_body()?;
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

    fn parse_match_case_body(&mut self) -> Option<MatchCaseBody> {
        self.skip_newlines();
        if self.at_match_case_body_boundary() {
            self.error_at_current(
                "expected_match_case_body",
                "expected match case body; use '()' for Unit or '{}' for an empty block",
            );
            return None;
        }

        if self.at(TokenKind::LBrace) {
            return self.parse_block().map(MatchCaseBody::Block);
        }

        let checkpoint = self.checkpoint();
        if let Some(expr) = self.parse_expr() {
            if self.at_match_case_body_boundary() {
                return Some(MatchCaseBody::Expr(expr));
            }
        }
        self.restore(checkpoint);

        let stmt = self.parse_stmt()?;
        match stmt {
            Stmt::Expr(expr_stmt) => Some(MatchCaseBody::Expr(expr_stmt.expr)),
            other => {
                let span = other.span();
                Some(MatchCaseBody::Block(Block {
                    statements: vec![other],
                    span,
                }))
            }
        }
    }

    pub(super) fn parse_return_stmt(&mut self) -> Option<ReturnStmt> {
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

    pub(super) fn parse_break_stmt(&mut self) -> Option<BreakStmt> {
        let span = self.consume_keyword(Keyword::Break, "expected 'break'")?;
        Some(BreakStmt { span })
    }

    pub(super) fn parse_continue_stmt(&mut self) -> Option<ContinueStmt> {
        let span = self.consume_keyword(Keyword::Continue, "expected 'continue'")?;
        Some(ContinueStmt { span })
    }
}
