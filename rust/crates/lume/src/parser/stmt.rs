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
                self.finish_braced_statement();
            } else {
                self.synchronize_stmt();
                self.skip_newlines();
            }
        }
        let end = self.consume(TokenKind::RBrace, "expected '}' after block")?;
        Some(Block {
            statements,
            span: start.cover(end),
        })
    }

    pub(super) fn finish_braced_statement(&mut self) {
        if self.at(TokenKind::Newline) {
            self.skip_newlines();
            return;
        }
        if self.at(TokenKind::RBrace) || self.at(TokenKind::Eof) {
            return;
        }
        self.error_at_current(
            "missing_statement_separator",
            "expected newline or '}' after statement",
        );
        self.synchronize_stmt();
        self.skip_newlines();
    }

    pub(super) fn parse_stmt(&mut self) -> Option<Stmt> {
        self.skip_newlines();
        if self.starts_named_type_declaration_in_callable() {
            self.error_at_current(
                "nested_declaration_in_callable",
                "named type aliases, unions, classes, shapes, interfaces, objects, and annotations are allowed only at module or declaration scope, not inside a callable body",
            );
            return None;
        }
        match self.current_kind() {
            TokenKind::Keyword(Keyword::Def) => {
                let function = self.parse_function_decl(Vec::new(), Visibility::Default)?;
                Some(Stmt::LocalFunction(function))
            }
            TokenKind::Keyword(Keyword::Match) => self.parse_match_stmt().map(Stmt::Match),
            TokenKind::Keyword(Keyword::Let) => {
                let checkpoint = self.checkpoint();
                if let Some(expr) = self.try_parse_lambda_expr() {
                    let span = expr.span();
                    return Some(Stmt::Expr(ExprStmt { expr, span }));
                }
                self.restore(checkpoint);
                self.parse_let_stmt()
            }
            TokenKind::Identifier
                if self.current().lexeme == "guard"
                    && matches!(
                        self.next_significant_token().kind,
                        TokenKind::Identifier | TokenKind::LBrace
                    ) =>
            {
                self.error_at_current(
                    "guard_removed",
                    "guard binding syntax was removed; use 'let PATTERN = value else ...' for recoverable refutable bindings",
                );
                None
            }
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
                if self.starts_local_callable_decl() {
                    let function = self.parse_function_decl(Vec::new(), Visibility::Default)?;
                    return Some(Stmt::LocalFunction(function));
                }
                if self.at_removed_assert_statement() {
                    self.error_at_current(
                        "removed_assert_statement",
                        "assert statement syntax was removed; use assert(condition) or assert(condition, message)",
                    );
                    return None;
                }
                if self.at_removed_expect_statement() {
                    self.error_at_current(
                        "expect_removed",
                        "expect binding syntax was removed; use 'let PATTERN = value else panic(...)' for assertive refutable bindings",
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

    fn starts_named_type_declaration_in_callable(&self) -> bool {
        let mut index = self.index;
        if matches!(
            self.tokens.get(index).map(|token| token.kind),
            Some(TokenKind::Keyword(Keyword::Private | Keyword::Internal))
        ) {
            index += 1;
        }
        match self.tokens.get(index).map(|token| token.kind) {
            Some(TokenKind::Keyword(
                Keyword::Annotation
                | Keyword::Class
                | Keyword::Shape
                | Keyword::Interface
                | Keyword::Type,
            )) => true,
            Some(TokenKind::Keyword(Keyword::Object)) => self
                .tokens
                .get(index + 1)
                .is_some_and(|token| token.kind == TokenKind::Identifier),
            _ => false,
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
            if self.is_headless_record_pattern_assignment_start() {
                return self.parse_single_let_pattern_stmt(start);
            }
            let (clauses, clauses_end) = self.parse_refutable_clause_block("let")?;
            let else_checkpoint = self.checkpoint();
            self.skip_newlines();
            if self.match_keyword(Keyword::Else) {
                let else_block = self.parse_let_else_body()?;
                let end = else_block.span;
                return Some(Stmt::LetElse(LetElseStmt {
                    clauses,
                    pattern: Pattern::Wildcard { span: clauses_end },
                    value: Expr::Unit { span: clauses_end },
                    else_block,
                    span: start.cover(end),
                }));
            }
            self.restore(else_checkpoint);
            if let Some(clause) = clauses
                .iter()
                .find(|clause| Self::pattern_contains_extract(&clause.pattern))
            {
                self.diagnostics.push(Diagnostic::error(
                    "missing_let_extract_fallback",
                    "let '<-' extraction requires an 'else' fallback; write 'let name <- value else ...'",
                    clause.pattern.span(),
                ));
                return None;
            }
            return Some(Stmt::PatternBinding(PatternBindingStmt {
                clauses,
                pattern: Pattern::Wildcard { span: clauses_end },
                value: Expr::Unit { span: clauses_end },
                span: start.cover(clauses_end),
            }));
        }

        self.parse_single_let_pattern_stmt(start)
    }

    fn parse_single_let_pattern_stmt(&mut self, start: Span) -> Option<Stmt> {
        let (pattern, operator) = self.parse_refutable_pattern_head("let")?;
        if operator == "=" && matches!(&pattern, Pattern::Binding { name, .. } if name != "_") {
            self.error_at_current(
                "plain_let_binding",
                "plain 'let name = value' is not supported; use 'name = value' for ordinary bindings, or use 'let' for destructuring/pattern matching",
            );
            return None;
        }
        let value = self.parse_expr()?;
        let else_checkpoint = self.checkpoint();
        self.skip_newlines();
        if self.match_keyword(Keyword::Else) {
            let else_block = self.parse_let_else_body()?;
            let end = else_block.span;
            return Some(Stmt::LetElse(LetElseStmt {
                clauses: Vec::new(),
                pattern,
                value,
                else_block,
                span: start.cover(end),
            }));
        }
        self.restore(else_checkpoint);
        if Self::pattern_contains_extract(&pattern) {
            self.diagnostics.push(Diagnostic::error(
                "missing_let_extract_fallback",
                "let '<-' extraction requires an 'else' fallback; write 'let name <- value else ...'",
                pattern.span(),
            ));
            return None;
        }
        let end = value.span();
        Some(Stmt::PatternBinding(PatternBindingStmt {
            clauses: Vec::new(),
            pattern,
            value,
            span: start.cover(end),
        }))
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
                        | TokenKind::Caret
                        | TokenKind::Bang
                        | TokenKind::Minus
                        | TokenKind::LBracket
                        | TokenKind::LBrace
                        | TokenKind::Keyword(Keyword::True)
                        | TokenKind::Keyword(Keyword::False)
                )
            )
    }

    fn at_removed_expect_statement(&self) -> bool {
        self.current_kind() == TokenKind::Identifier
            && self.current().lexeme == "expect"
            && matches!(
                self.tokens.get(self.index + 1).map(|token| token.kind),
                Some(
                    TokenKind::Identifier
                        | TokenKind::Integer
                        | TokenKind::Float
                        | TokenKind::String
                        | TokenKind::Caret
                        | TokenKind::Bang
                        | TokenKind::Minus
                        | TokenKind::LParen
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
            self.expect_data_name("expected field name in brace destructuring")?;
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
            let comma = self.previous_span();
            self.skip_newlines();
            if self.at(TokenKind::RBrace) {
                self.report_trailing_comma(comma, "brace destructuring pattern");
                break;
            }
            bindings.push(self.parse_brace_destructure_binding(mutable)?);
        }
        Some(bindings)
    }

    pub(super) fn parse_binding_list(&mut self, mutable: bool) -> Option<Vec<Binding>> {
        let mut bindings = vec![self.parse_binding(mutable)?];
        while self.match_token(TokenKind::Comma) {
            let comma = self.previous_span();
            self.skip_newlines();
            if self.at(TokenKind::Eq) {
                self.report_trailing_comma(comma, "binding list");
                break;
            }
            bindings.push(self.parse_binding(mutable)?);
        }
        Some(bindings)
    }

    pub(super) fn parse_tuple_binding_list(&mut self, mutable: bool) -> Option<Vec<Binding>> {
        let mut bindings = vec![self.parse_binding(mutable)?];
        while self.match_token(TokenKind::Comma) {
            let comma = self.previous_span();
            self.skip_newlines();
            if self.at(TokenKind::RParen) {
                self.report_trailing_comma(comma, "tuple destructuring pattern");
                break;
            }
            bindings.push(self.parse_binding(mutable)?);
        }
        Some(bindings)
    }

    pub(super) fn parse_plain_for_generator_binding(&mut self) -> Option<Binding> {
        const MESSAGE: &str = "for generator must bind a plain identifier or '_' before '<-'; use 'for let (...) <-' or 'for let { ... } <-' for irrefutable destructuring";
        if !self.at(TokenKind::Identifier) {
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
        const MESSAGE: &str = "for generator must bind a plain identifier or '_' before '<-'; use 'for let (...) <-' or 'for let { ... } <-' for irrefutable destructuring";
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

    pub(super) fn parse_for_let_generator_head(&mut self) -> Option<ForBinding> {
        let pattern = self.parse_pattern()?;
        self.consume_for_generator_arrow()?;
        let iterable = self.parse_expr_without_trailing_block_call()?;
        let end = iterable.span();
        Some(ForBinding {
            span: pattern.span().cover(end),
            bindings: Vec::new(),
            destructure: None,
            pattern: Some(pattern),
            iterable: Some(iterable),
            values: Vec::new(),
        })
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
        let mut parsed_clauses = self.parse_condition_clauses("if")?;
        let (condition, condition_clauses) = if parsed_clauses.len() == 1 {
            match parsed_clauses.pop().expect("one condition clause") {
                IfConditionClause::Expr(condition) => (Some(condition), Vec::new()),
                clause => (None, vec![clause]),
            }
        } else {
            (None, parsed_clauses)
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
            pattern: None,
            pattern_value: None,
            pattern_clauses: Vec::new(),
            bindings: Vec::new(),
            binding_value: None,
            then_block,
            else_branch,
            span: start.cover(end),
        })
    }

    pub(super) fn parse_while_stmt(&mut self) -> Option<WhileStmt> {
        let start = self.consume_keyword(Keyword::While, "expected 'while'")?;
        let condition_clauses = self.parse_condition_clauses("while")?;
        let body = self.parse_block()?;
        Some(WhileStmt {
            condition_clauses,
            body: body.clone(),
            span: start.cover(body.span),
        })
    }

    pub(super) fn parse_for_stmt(&mut self) -> Option<ForStmt> {
        let start = self.consume_keyword(Keyword::For, "expected 'for'")?;
        if self.parenthesized_for_generator_header() {
            self.error_at_current(
                "parenthesized_for_generator",
                "for generator headers cannot be parenthesized; write 'for item <- items { ... }' without parentheses",
            );
            self.discard_parenthesized_for_generator_header();
            if self.at(TokenKind::LBrace) {
                self.parse_block()?;
            }
            return None;
        }
        let binding = if self.match_keyword(Keyword::Let) {
            self.parse_for_let_generator_head()?
        } else {
            let binding = self.parse_plain_for_generator_binding()?;
            let target_span = binding.span;
            self.consume_for_generator_arrow()?;
            let iterable = self.parse_expr_without_trailing_block_call()?;
            ForBinding {
                span: target_span.cover(iterable.span()),
                bindings: vec![binding],
                destructure: None,
                pattern: None,
                iterable: Some(iterable),
                values: Vec::new(),
            }
        };
        if !self.at(TokenKind::LBrace) {
            self.error_at_current(
                "unexpected_token",
                "for requires a '{ ... }' block body; one-line for forms are not supported",
            );
            return None;
        }
        let body = self.parse_block()?;
        Some(ForStmt {
            bindings: vec![binding],
            body: body.clone(),
            span: start.cover(body.span),
        })
    }

    pub(super) fn parse_match_stmt(&mut self) -> Option<MatchStmt> {
        let start = self.consume_keyword(Keyword::Match, "expected 'match'")?;
        if self.at(TokenKind::LBrace) {
            self.error_missing_match_value();
            return None;
        }
        let value = self.parse_expr_without_trailing_block_call()?;
        let (cases, end) = self.parse_match_cases()?;
        Some(MatchStmt {
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
            let mut patterns = vec![self.parse_match_pattern()?];
            while self.match_token(TokenKind::Pipe) {
                patterns.push(self.parse_match_pattern()?);
            }
            let guard = if self.match_keyword(Keyword::If) {
                Some(self.parse_match_guard_expr()?)
            } else {
                None
            };
            self.consume(TokenKind::FatArrow, "expected '=>' after match pattern")?;
            let body = self.parse_match_case_body()?;
            let end = match &body {
                MatchCaseBody::Block(block) => block.span,
                MatchCaseBody::Expr(expr) => expr.span(),
            };
            let alternative_count = patterns.len();
            for (index, pattern) in patterns.into_iter().enumerate() {
                cases.push(MatchCase {
                    span: pattern.span().cover(end),
                    pattern,
                    guard: guard.clone(),
                    body: body.clone(),
                    remaining_alternatives: alternative_count - index - 1,
                });
            }
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
                "expected expression after '=>'; use '()' for a Unit-valued match case",
            );
            return None;
        }

        if self.at(TokenKind::LBrace) {
            return self
                .parse_expression_brace_body_with_continuation()
                .map(|body| match body {
                    ExpressionBraceBody::Expression(expr) => MatchCaseBody::Expr(expr),
                    ExpressionBraceBody::Block(block) => MatchCaseBody::Block(block),
                });
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
