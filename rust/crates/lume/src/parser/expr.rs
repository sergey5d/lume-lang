use super::support::spans_touch;
use super::*;

struct ChainSegment {
    param: String,
    body: Expr,
    span: Span,
}

impl<'a> Parser<'a> {
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
        self.parse_extract_or_expr()
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
        let Some((params, start)) = self.parse_lambda_head() else {
            self.restore(checkpoint);
            return None;
        };
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
        self.skip_newlines();
        let Some(first) = self.parse_stmt() else {
            self.synchronize_stmt();
            self.error_at_current(
                "expected_expression",
                "expected expression or lambda body after '=>'",
            );
            return None;
        };
        self.diagnose_extra_lambda_body_unit(first.span());
        Some(Self::lambda_body_from_stmt(first))
    }

    fn lambda_body_from_stmt(stmt: Stmt) -> LambdaBody {
        if let Stmt::Expr(ExprStmt { expr, .. }) = stmt {
            return LambdaBody::Expr(Box::new(expr));
        }
        let span = stmt.span();
        LambdaBody::Block(Block {
            statements: vec![stmt],
            span,
        })
    }

    fn diagnose_extra_lambda_body_unit(&mut self, body_span: Span) {
        let checkpoint = self.checkpoint();
        if self.at(TokenKind::Newline) {
            self.skip_newlines();
        }
        let current_span = self.current_span();
        let diagnostic_span = if !self.at(TokenKind::RParen)
            && !self.at(TokenKind::RBrace)
            && !self.at(TokenKind::Comma)
            && !self.at(TokenKind::Eof)
            && current_span.start_pos.line > body_span.end_pos.line
            && current_span.start_pos.column >= body_span.start_pos.column
        {
            Some(body_span.cover(current_span))
        } else {
            None
        };
        self.restore(checkpoint);
        if let Some(span) = diagnostic_span {
            self.diagnostics.push(Diagnostic::error(
                "lambda_body_requires_braces",
                "lambda body accepts one statement or expression; use '{ ... }' for multiple statements",
                span,
            ));
        }
    }

    pub(super) fn parse_if_expr(&mut self, start: Span) -> Option<Expr> {
        let condition = self.parse_expr_without_trailing_block_call()?;
        let then_block = self.parse_if_body_block()?;
        if !self.match_keyword(Keyword::Else) {
            self.error_at_current(
                "if_expression_requires_else",
                "if expression requires 'else'; use statement form 'if condition { ... }' when no value is needed",
            );
            return None;
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
        if self.at(TokenKind::LBrace) {
            self.error_missing_match_value(partial);
            return None;
        }
        let value = self.parse_expr_without_trailing_block_call()?;
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
        let bindings = if self.at(TokenKind::LBrace) {
            self.consume(TokenKind::LBrace, "expected '{' after 'for'")?;
            self.parse_for_binding_block()?
        } else {
            let binding = self.parse_plain_for_generator_binding()?;
            self.consume_for_generator_arrow()?;
            let iterable = self.parse_expr_without_trailing_block_call()?;
            let target_span = binding.span;
            vec![ForBinding {
                span: target_span.cover(iterable.span()),
                bindings: vec![binding],
                destructure: None,
                pattern: None,
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
            if self.match_keyword(Keyword::Let) {
                bindings.push(self.parse_for_let_clause()?);
            } else {
                let binding = self.parse_binding(false)?;
                if self.match_token(TokenKind::LeftArrow) {
                    if binding.ty.is_some() {
                        self.error_at_current(
                            "invalid_for_generator",
                            "for generator must bind a plain identifier or '_' before '<-'; use 'let (...) <-' or 'let { ... } <-' for irrefutable destructuring",
                        );
                        return None;
                    }
                    let iterable = self.parse_expr_without_trailing_block_call()?;
                    let target_span = binding.span;
                    let span = target_span.cover(iterable.span());
                    bindings.push(ForBinding {
                        bindings: vec![binding],
                        destructure: None,
                        pattern: None,
                        iterable: Some(iterable),
                        values: Vec::new(),
                        span,
                    });
                } else if self.match_token(TokenKind::Eq) {
                    let value = self.parse_for_clause_value(
                        "for yield binding clauses cannot use 'else'; move refutable logic into the body",
                    )?;
                    let end = value.span();
                    bindings.push(ForBinding {
                        span: binding.span.cover(end),
                        bindings: vec![binding],
                        destructure: None,
                        pattern: None,
                        iterable: None,
                        values: vec![value],
                    });
                } else {
                    self.error_at_current(
                        "invalid_for_clause",
                        "for yield clauses only support 'name <- iterable', 'let (x, y) <- iterable', 'let { ... } <- iterable', 'name = expr', and 'let pattern = expr'",
                    );
                    return None;
                }
            }
            self.skip_newlines();
        }
        self.consume(TokenKind::RBrace, "expected '}' after for bindings")?;
        Some(bindings)
    }

    fn parse_for_let_clause(&mut self) -> Option<ForBinding> {
        if self.at(TokenKind::LBrace) && self.is_brace_destructuring_binding_start() {
            let start = self.consume(TokenKind::LBrace, "expected '{' after 'let'")?;
            let bindings = self.parse_brace_destructure_binding_list(false)?;
            self.consume(
                TokenKind::RBrace,
                "expected '}' after destructuring bindings",
            )?;
            return self.parse_for_destructure_clause_tail(
                bindings,
                DestructureKind::Record,
                start,
            );
        }

        if self.match_token(TokenKind::LParen) {
            let start = self.previous_span();
            let bindings = self.parse_binding_list(false)?;
            self.consume(
                TokenKind::RParen,
                "expected ')' after destructuring bindings",
            )?;
            return self.parse_for_destructure_clause_tail(bindings, DestructureKind::Tuple, start);
        }

        let pattern = self.parse_pattern()?;
        if matches!(&pattern, Pattern::Binding { name, .. } if name != "_")
            && self.at(TokenKind::Eq)
        {
            self.error_at_current(
                "plain_let_binding",
                "plain 'let name = value' is not supported in for-yield clauses; use 'name = value' for ordinary bindings, or use 'let' for destructuring/pattern matching",
            );
            return None;
        }
        if self.match_token(TokenKind::LeftArrow) {
            let iterable = self.parse_expr_without_trailing_block_call()?;
            let end = iterable.span();
            return Some(ForBinding {
                span: pattern.span().cover(end),
                bindings: Vec::new(),
                destructure: None,
                pattern: Some(pattern),
                iterable: Some(iterable),
                values: Vec::new(),
            });
        }
        self.consume_for_let_equals()?;
        let value = self.parse_for_let_value()?;
        let end = value.span();
        let span = pattern.span().cover(end);
        Some(ForBinding {
            bindings: Vec::new(),
            destructure: None,
            pattern: Some(pattern),
            iterable: None,
            values: vec![value],
            span,
        })
    }

    fn parse_for_destructure_clause_tail(
        &mut self,
        bindings: Vec<Binding>,
        destructure: DestructureKind,
        start: Span,
    ) -> Option<ForBinding> {
        if self.match_token(TokenKind::LeftArrow) {
            let iterable = self.parse_expr_without_trailing_block_call()?;
            let end = iterable.span();
            return Some(ForBinding {
                bindings,
                destructure: Some(destructure),
                pattern: None,
                iterable: Some(iterable),
                values: Vec::new(),
                span: start.cover(end),
            });
        }

        self.consume_for_let_equals()?;
        let value = self.parse_for_let_value()?;
        let end = value.span();
        Some(ForBinding {
            bindings,
            destructure: Some(destructure),
            pattern: None,
            iterable: None,
            values: vec![value],
            span: start.cover(end),
        })
    }

    fn consume_for_let_equals(&mut self) -> Option<Span> {
        if self.match_token(TokenKind::Eq) {
            Some(self.previous_span())
        } else {
            self.error_at_current(
                "invalid_for_clause",
                "for yield clauses only support 'name <- iterable', 'let (x, y) <- iterable', 'let { ... } <- iterable', 'name = expr', and 'let pattern = expr'",
            );
            None
        }
    }

    fn parse_for_let_value(&mut self) -> Option<Expr> {
        self.parse_for_clause_value(
            "for yield let clauses cannot use 'else'; move refutable logic into the body",
        )
    }

    fn parse_for_clause_value(&mut self, else_message: &'static str) -> Option<Expr> {
        let value = self.parse_expr_without_trailing_block_call()?;
        if self.match_keyword(Keyword::Else) {
            self.error_at_current("invalid_for_clause", else_message);
            return None;
        }
        Some(value)
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
        enum RecordEntry {
            Field {
                name: String,
                ty: Option<TypeRef>,
                value: Expr,
                span: Span,
            },
            Spread {
                value: Expr,
                span: Span,
            },
            Keyed {
                key: Expr,
                value: Expr,
                span: Span,
            },
            Positional {
                span: Span,
            },
        }

        let mut entries = Vec::new();
        self.skip_newlines();
        while !self.at(TokenKind::RBrace) && !self.at(TokenKind::Eof) {
            let entry = if self.match_token(TokenKind::Ellipsis) {
                let start = self.previous_span();
                let value = self.parse_expr()?;
                RecordEntry::Spread {
                    span: start.cover(value.span()),
                    value,
                }
            } else if self.match_token(TokenKind::LBracket) {
                let start = self.previous_span();
                let key = self.parse_expr()?;
                self.consume(
                    TokenKind::RBracket,
                    "expected ']' after computed keyed entry",
                )?;
                self.consume(TokenKind::Colon, "expected ':' after computed keyed entry")?;
                let value = self.parse_expr()?;
                RecordEntry::Keyed {
                    span: start.cover(value.span()),
                    key,
                    value,
                }
            } else if self.at(TokenKind::Identifier) {
                let checkpoint = self.checkpoint();
                let (name, name_span) = self.expect_identifier("expected shape field name")?;
                if self.match_token(TokenKind::Colon) {
                    let value = self.parse_expr()?;
                    RecordEntry::Field {
                        name,
                        ty: None,
                        span: name_span.cover(value.span()),
                        value,
                    }
                } else if self.can_start_type_ref() {
                    let ty = self.parse_type_ref();
                    if let Some(ty) = ty {
                        if self.match_token(TokenKind::Colon) {
                            let value = self.parse_expr()?;
                            RecordEntry::Field {
                                name,
                                ty: Some(ty),
                                span: name_span.cover(value.span()),
                                value,
                            }
                        } else {
                            self.restore(checkpoint);
                            let value = self.parse_expr()?;
                            RecordEntry::Positional { span: value.span() }
                        }
                    } else {
                        self.restore(checkpoint);
                        let value = self.parse_expr()?;
                        RecordEntry::Positional { span: value.span() }
                    }
                } else {
                    self.restore(checkpoint);
                    let value = self.parse_expr()?;
                    RecordEntry::Positional { span: value.span() }
                }
            } else {
                let key_or_value = self.parse_or_expr()?;
                if self.match_token(TokenKind::Colon) {
                    if matches!(key_or_value, Expr::Group { .. } | Expr::TupleLiteral { .. }) {
                        self.diagnostics.push(Diagnostic::error(
                            "computed_key_requires_brackets",
                            "computed keyed entries use '[expr]: value', not '(expr): value'",
                            key_or_value.span(),
                        ));
                        return None;
                    }
                    let value = self.parse_expr()?;
                    RecordEntry::Keyed {
                        span: key_or_value.span().cover(value.span()),
                        key: key_or_value,
                        value,
                    }
                } else {
                    RecordEntry::Positional {
                        span: key_or_value.span(),
                    }
                }
            };
            entries.push(entry);
            let separated_by_comma = self.match_token(TokenKind::Comma);
            let separated_by_newline = if !separated_by_comma {
                self.match_token(TokenKind::Newline)
            } else {
                false
            };
            self.skip_newlines();
            if separated_by_newline {
                self.match_token(TokenKind::Comma);
                self.skip_newlines();
            }
            if self.at(TokenKind::RBrace) || self.at(TokenKind::Eof) {
                break;
            }
            if !separated_by_comma && !separated_by_newline {
                self.error_at_current(
                    "unexpected_token",
                    "expected ',' or newline between brace entries",
                );
                return None;
            }
        }
        let end = self.consume(TokenKind::RBrace, "expected '}' after anonymous shape")?;
        let has_named = entries.iter().any(|entry| {
            matches!(
                entry,
                RecordEntry::Field { .. } | RecordEntry::Spread { .. }
            )
        });
        let has_keyed = entries
            .iter()
            .any(|entry| matches!(entry, RecordEntry::Keyed { .. }));
        if has_named && has_keyed {
            self.diagnostics.push(Diagnostic::error(
                "mixed_brace_entries",
                "cannot mix construction fields and keyed entries in the same brace payload",
                start.cover(end),
            ));
            return None;
        }
        let mut fields = Vec::new();
        let mut values = Vec::new();
        if has_named {
            for entry in entries {
                match entry {
                    RecordEntry::Field {
                        name,
                        ty,
                        value,
                        span,
                    } => {
                        fields.push(CallArg {
                            name: Some(name),
                            ty,
                            value,
                            span,
                        });
                    }
                    RecordEntry::Spread { value, span } => {
                        fields.push(CallArg {
                            name: None,
                            ty: None,
                            value,
                            span,
                        });
                    }
                    RecordEntry::Keyed { .. } => unreachable!("keyed entries were rejected above"),
                    RecordEntry::Positional { span, .. } => {
                        self.diagnostics.push(Diagnostic::error(
                            "unexpected_token",
                            "cannot mix construction fields and positional shape fields",
                            span,
                        ));
                        return None;
                    }
                }
            }
        } else if has_keyed {
            for entry in entries {
                match entry {
                    RecordEntry::Keyed { key, value, span } => {
                        values.push(Expr::TupleLiteral {
                            items: vec![key, value],
                            span,
                        });
                    }
                    RecordEntry::Positional { span, .. } => {
                        self.diagnostics.push(Diagnostic::error(
                            "unexpected_token",
                            "cannot mix keyed entries and positional brace values",
                            span,
                        ));
                        return None;
                    }
                    RecordEntry::Field { .. } | RecordEntry::Spread { .. } => {
                        unreachable!("named entries were rejected above")
                    }
                }
            }
        } else {
            if !entries.is_empty() {
                self.diagnostics.push(Diagnostic::error(
                    "positional_brace_construction",
                    "braces are for construction fields; use 'Type(...)' for positional constructors",
                    start.cover(end),
                ));
                return None;
            }
        }
        Some(Expr::RecordLiteral {
            fields,
            values,
            span: start.cover(end),
        })
    }

    fn keyed_record_literal_call(&self, callee: Expr, record: Expr, start: Span) -> Option<Expr> {
        let Expr::RecordLiteral {
            fields,
            values,
            span: record_span,
        } = record
        else {
            return None;
        };
        if !fields.is_empty() || values.is_empty() {
            return None;
        }
        let member_span = callee.span().cover(record_span);
        Some(Expr::Call {
            callee: Box::new(Expr::Member {
                receiver: Box::new(callee),
                name: "keyed".to_string(),
                span: member_span,
            }),
            args: vec![CallArg {
                name: None,
                ty: None,
                value: Expr::ListLiteral {
                    items: values,
                    span: record_span,
                },
                span: record_span,
            }],
            uses_brace_syntax: false,
            span: start.cover(record_span),
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
            allow_shape_update_operator: self.allow_shape_update_operator,
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
            if !self.at_keyword(Keyword::Def) && !self.starts_callable_decl() {
                self.error_at_current("unexpected_token", "expected anonymous interface member");
                return None;
            }
            methods.push(self.parse_method_decl(annotations, visibility, false, true)?);
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
                self.diagnostics.push(Diagnostic::error(
                    "removed_pair_expression",
                    "':' pair expressions are no longer supported; use '(left, right)' for tuple pairs or keyed construction inside 'Type { key: value }'",
                    self.previous_span(),
                ));
                BinaryOp::Colon
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

    pub(super) fn parse_extract_or_expr(&mut self) -> Option<Expr> {
        let value = self.parse_colon_expr()?;
        if !self.match_token(TokenKind::QuestionQuestion) {
            return Some(value);
        }

        self.skip_newlines();
        let fallback = self.parse_extract_or_expr()?;
        let span = value.span().cover(fallback.span());
        Some(Expr::ExtractOr {
            value: Box::new(value),
            fallback: Box::new(fallback),
            span,
        })
    }

    pub(super) fn parse_or_expr(&mut self) -> Option<Expr> {
        self.parse_left_assoc(
            |parser| parser.parse_and_expr(),
            &[(TokenKind::OrOr, BinaryOp::Or)],
        )
    }

    pub(super) fn parse_and_expr(&mut self) -> Option<Expr> {
        self.parse_left_assoc(
            |parser| parser.parse_equality_expr(),
            &[(TokenKind::AndAnd, BinaryOp::And)],
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
        if self.match_keyword(Keyword::Return) {
            return self.parse_return_expr(self.previous_span());
        }
        if self.match_keyword(Keyword::Break) {
            let span = self.previous_span();
            return Some(Expr::Break { span });
        }
        if self.match_keyword(Keyword::Continue) {
            let span = self.previous_span();
            return Some(Expr::Continue { span });
        }
        if self.match_keyword(Keyword::Try) {
            let start = self.previous_span();
            self.skip_newlines();
            let value = self.parse_unary_expr()?;
            if self.at(TokenKind::Identifier) && self.current().lexeme == "catch" {
                self.error_at_current(
                    "removed_try_catch",
                    "try catch syntax was removed; transform the source before try, for example 'try source.mapError { err => mappedFailure }'",
                );
            }
            let span = start.cover(value.span());
            return Some(Expr::Try {
                value: Box::new(value),
                span,
            });
        }
        if self.at_removed_lift_operator() {
            self.error_at_current(
                "lift_removed",
                "lift operator was removed; use explicit try/let extraction and construct the value directly",
            );
            return None;
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

    fn parse_return_expr(&mut self, start: Span) -> Option<Expr> {
        if self.return_expr_value_is_omitted() {
            return Some(Expr::Return {
                value: None,
                span: start,
            });
        }
        let value = self.parse_expr()?;
        let span = start.cover(value.span());
        Some(Expr::Return {
            value: Some(Box::new(value)),
            span,
        })
    }

    fn return_expr_value_is_omitted(&self) -> bool {
        matches!(
            self.current_kind(),
            TokenKind::Newline
                | TokenKind::RBrace
                | TokenKind::RParen
                | TokenKind::RBracket
                | TokenKind::Comma
                | TokenKind::Eof
        )
    }

    pub(super) fn parse_postfix_expr(&mut self) -> Option<Expr> {
        let mut expr = self.parse_primary_expr()?;
        let mut chain_segment_count = 0;
        loop {
            if self.at(TokenKind::Newline)
                && (self.at_next(TokenKind::Dot) || self.at_next(TokenKind::DotArrow))
            {
                self.advance();
                continue;
            }
            if self.match_token(TokenKind::DotArrow) {
                let operator_span = self.previous_span();
                let (name, end) = self.parse_member_name("expected member name after '.->'")?;
                let param = format!("__lume_chain{}", chain_segment_count);
                chain_segment_count += 1;
                let receiver = Expr::Identifier {
                    name: param.clone(),
                    span: end,
                };
                let start = receiver.span();
                let body = Expr::Member {
                    receiver: Box::new(receiver),
                    name,
                    span: start.cover(end),
                };
                let body = self.parse_lifted_hop_postfixes(body)?;
                let span = operator_span.cover(body.span());
                expr = Self::append_lifted_hop(expr, ChainSegment { param, body, span });
                continue;
            }
            if self.match_token(TokenKind::LParen) {
                let args = self.parse_call_args()?;
                let end = self.consume(TokenKind::RParen, "expected ')' after arguments")?;
                let start = expr.span();
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
                if self.at(TokenKind::Arrow) {
                    self.error_at_current(
                        "spaced_lifted_access_operator",
                        "lifted access operator must be written as '.->' without whitespace",
                    );
                    return None;
                }
                let (name, end) = self.parse_member_name("expected member name after '.'")?;
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
                if matches!(expr, Expr::Identifier { ref name, .. } if name == "typeOf") {
                    let ty = self.parse_type_ref()?;
                    let end =
                        self.consume(TokenKind::RBracket, "expected ']' after typeOf type")?;
                    expr = Expr::TypeOf {
                        ty,
                        span: start.cover(end),
                    };
                    continue;
                }
                let index = self.parse_expr()?;
                let end = self.consume(TokenKind::RBracket, "expected ']' after index")?;
                expr = Expr::Index {
                    receiver: Box::new(expr),
                    index: Box::new(index),
                    span: start.cover(end),
                };
                continue;
            }
            if self.allow_shape_update_operator && self.match_token(TokenKind::ColonLess) {
                let start = expr.span();
                self.skip_newlines();
                let previous = self.allow_shape_update_operator;
                self.allow_shape_update_operator = false;
                let patch = self.parse_expr();
                self.allow_shape_update_operator = previous;
                let patch = patch?;
                let end = patch.span();
                expr = Expr::RecordUpdate {
                    receiver: Box::new(expr),
                    patch: Box::new(patch),
                    span: start.cover(end),
                };
                continue;
            }
            if self.at_keyword(Keyword::Match) {
                self.error_at_current(
                    "postfix_match_not_supported",
                    "postfix match is not supported; use 'match value { ... }'",
                );
                return None;
            }
            if self.allow_trailing_block_call && self.at(TokenKind::LBrace) {
                let start = expr.span();
                let open_span = self.current_span();
                let arg = if self.looks_like_brace_record_literal(true)
                    || Self::is_constructor_like_expr(&expr)
                {
                    self.parse_brace_record_literal_expr()?
                } else if let Some(arg) = self.parse_trailing_lambda_block_arg(open_span) {
                    arg
                } else {
                    let block = self.parse_block()?;
                    if !self.validate_trailing_lambda_block(&block, open_span) {
                        self.diagnostics.push(Diagnostic::error(
                            "invalid_trailing_lambda",
                        "trailing lambda syntax requires an explicit parameter arrow; write '{ () => ... }' for zero-argument callbacks",
                            open_span,
                        ));
                    }
                    Expr::Block {
                        span: block.span,
                        body: block,
                    }
                };
                let arg_span = arg.span();
                if let Some(keyed_call) =
                    self.keyed_record_literal_call(expr.clone(), arg.clone(), start)
                {
                    expr = keyed_call;
                } else {
                    expr = Expr::Call {
                        callee: Box::new(expr),
                        args: vec![CallArg {
                            name: None,
                            ty: None,
                            span: arg_span,
                            value: arg,
                        }],
                        uses_brace_syntax: true,
                        span: start.cover(arg_span),
                    };
                }
                continue;
            }
            break;
        }
        Some(expr)
    }

    fn parse_trailing_lambda_block_arg(&mut self, open_span: Span) -> Option<Expr> {
        let checkpoint = self.checkpoint();
        let start = self.consume(TokenKind::LBrace, "expected '{'")?;
        if self.current_span().start_pos.line != open_span.start_pos.line {
            self.restore(checkpoint);
            return None;
        }
        let Some((params, lambda_start)) = self.parse_lambda_head() else {
            self.restore(checkpoint);
            return None;
        };

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
        let body_span = statements
            .first()
            .map(Stmt::span)
            .unwrap_or(end)
            .cover(statements.last().map(Stmt::span).unwrap_or(end));
        let lambda_span = lambda_start.cover(body_span);
        let lambda = Expr::Lambda {
            params,
            body: LambdaBody::Block(Block {
                statements,
                span: body_span,
            }),
            span: lambda_span,
        };
        let block_span = start.cover(end);
        Some(Expr::Block {
            body: Block {
                statements: vec![Stmt::Expr(ExprStmt {
                    expr: lambda,
                    span: lambda_span,
                })],
                span: block_span,
            },
            span: block_span,
        })
    }

    fn parse_lambda_head(&mut self) -> Option<(Vec<LambdaParam>, Span)> {
        let checkpoint = self.checkpoint();
        if self.match_keyword(Keyword::Let) {
            let start = self.previous_span();
            let Some(param) = self.parse_lambda_destructure_param(start, 0) else {
                self.restore(checkpoint);
                return None;
            };
            if !self.match_lambda_arrow() {
                self.restore(checkpoint);
                return None;
            }
            self.diagnostics.push(Diagnostic::error(
                "invalid_lambda_params",
                "lambda parameters cannot use 'let' destructuring; name the parameter and destructure inside the lambda body",
                param.span,
            ));
            return Some((vec![param], start));
        }

        if self.at(TokenKind::Identifier) {
            let (name, start) = self.expect_identifier("expected lambda parameter")?;
            if self.match_lambda_arrow() {
                return Some((
                    vec![LambdaParam {
                        name,
                        ty: None,
                        destructure: None,
                        span: start,
                    }],
                    start,
                ));
            }

            let ty_checkpoint = self.checkpoint();
            if self.can_start_type_ref() {
                if let Some(ty) = self.parse_primary_type_ref() {
                    if self.match_lambda_arrow() {
                        let ty_span = ty.span();
                        self.diagnostics.push(Diagnostic::error(
                            "invalid_lambda_params",
                            "typed single-parameter lambdas must use parentheses; write '(x T) => ...'",
                            start.cover(ty_span),
                        ));
                        return Some((
                            vec![LambdaParam {
                                name,
                                ty: Some(ty),
                                destructure: None,
                                span: start.cover(ty_span),
                            }],
                            start,
                        ));
                    }
                }
                self.restore(ty_checkpoint);
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
        if !self.match_lambda_arrow() {
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
        Some((params, start))
    }

    fn match_lambda_arrow(&mut self) -> bool {
        if self.match_token(TokenKind::FatArrow) {
            return true;
        }
        if self.match_token(TokenKind::Arrow) {
            self.diagnostics.push(Diagnostic::error(
                "old_lambda_arrow",
                "lambda syntax uses '=>'; replace '->' with '=>'",
                self.previous_span(),
            ));
            return true;
        }
        false
    }

    fn parse_lifted_hop_postfixes(&mut self, mut body: Expr) -> Option<Expr> {
        loop {
            if self.match_token(TokenKind::LParen) {
                let args = self.parse_call_args()?;
                let end = self.consume(TokenKind::RParen, "expected ')' after arguments")?;
                let start = body.span();
                body = Expr::Call {
                    callee: Box::new(body),
                    args,
                    uses_brace_syntax: false,
                    span: start.cover(end),
                };
                continue;
            }
            if self.match_token(TokenKind::LBracket) {
                let start = body.span();
                let index = self.parse_expr()?;
                let end = self.consume(TokenKind::RBracket, "expected ']' after index")?;
                body = Expr::Index {
                    receiver: Box::new(body),
                    index: Box::new(index),
                    span: start.cover(end),
                };
                continue;
            }
            break;
        }
        Some(body)
    }

    fn at_removed_lift_operator(&self) -> bool {
        if self.current_kind() != TokenKind::Identifier || self.current().lexeme != "lift" {
            return false;
        }
        let Some(next) = self.tokens.get(self.index + 1) else {
            return false;
        };
        if next.kind == TokenKind::LBrace {
            return true;
        }
        next.kind == TokenKind::LParen && !spans_touch(self.current_span(), next.span)
    }

    fn parse_member_name(&mut self, message: &'static str) -> Option<(String, Span)> {
        if self.at(TokenKind::Keyword(Keyword::Annotation))
            || self.at(TokenKind::Keyword(Keyword::Case))
        {
            let token = self.current().clone();
            self.advance();
            Some((token.lexeme, token.span))
        } else {
            self.expect_identifier(message)
        }
    }

    fn append_lifted_hop(expr: Expr, segment: ChainSegment) -> Expr {
        match expr {
            Expr::LiftedChain {
                base,
                mut segments,
                span: chain_span,
            } => {
                let span = chain_span.cover(segment.span);
                segments.push(LiftedChainSegment {
                    param: segment.param,
                    body: segment.body,
                    span: segment.span,
                });
                Expr::LiftedChain {
                    base,
                    segments,
                    span,
                }
            }
            other => {
                let span = other.span().cover(segment.span);
                Expr::LiftedChain {
                    base: Box::new(other),
                    segments: vec![LiftedChainSegment {
                        param: segment.param,
                        body: segment.body,
                        span: segment.span,
                    }],
                    span,
                }
            }
        }
    }

    fn validate_trailing_lambda_block(&mut self, block: &Block, open_span: Span) -> bool {
        let [
            Stmt::Expr(ExprStmt {
                expr: Expr::Lambda { span, .. },
                ..
            }),
        ] = block.statements.as_slice()
        else {
            return false;
        };

        if span.start_pos.line != open_span.start_pos.line {
            self.diagnostics.push(Diagnostic::error(
                "invalid_trailing_lambda",
                "trailing lambda parameters must start on the same line as '{'",
                *span,
            ));
        }
        true
    }

    pub(super) fn parse_call_args(&mut self) -> Option<Vec<CallArg>> {
        let mut args = Vec::new();
        self.skip_newlines();
        if self.at(TokenKind::RParen) {
            return Some(args);
        }
        loop {
            let start = self.current_span();
            if self.match_token(TokenKind::Ellipsis) {
                let value = self.parse_expr()?;
                let span = start.cover(value.span());
                args.push(CallArg {
                    name: None,
                    ty: None,
                    span,
                    value: Expr::Spread {
                        value: Box::new(value),
                        span,
                    },
                });
            } else if self.at(TokenKind::Identifier) && self.at_next(TokenKind::Eq) {
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
                    "anonymous shape literals now use '{ ... }'; 'class { ... }' was removed"
                } else {
                    "anonymous shape literals use '{ ... }'; 'class(...)' is not supported"
                };
                self.error_at_current("unexpected_token", message);
                None
            }
            TokenKind::Keyword(Keyword::Shape) => {
                let start = self.consume_keyword(Keyword::Shape, "expected 'shape'")?;
                if self.match_token(TokenKind::LParen) {
                    let args = self.parse_call_args()?;
                    let end = self.consume(TokenKind::RParen, "expected ')' after shape values")?;
                    let mut items = Vec::new();
                    for arg in args {
                        if arg.name.is_some() || arg.ty.is_some() {
                            self.diagnostics.push(Diagnostic::error(
                                "invalid_shape_positional_argument",
                                "shape(...) accepts positional values only; use '{ field: value }' for named anonymous shape construction",
                                arg.span,
                            ));
                        }
                        items.push(arg.value);
                    }
                    return Some(Expr::ShapeLiteral {
                        items,
                        span: start.cover(end),
                    });
                }
                let message = if self.at(TokenKind::LBrace) {
                    "anonymous shape literals use '{ ... }'; 'shape { ... }' is not supported"
                } else {
                    "shape positional construction uses 'shape(...)'"
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
            .is_some_and(|token| token.kind == TokenKind::Ellipsis)
        {
            return true;
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
                allow_shape_update_operator: self.allow_shape_update_operator,
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
                TokenKind::Colon if depth == 1 && nested_parens == 0 && nested_brackets == 0 => {
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
            Expr::Identifier { name, .. } if name == "new" || name == "this" => true,
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
            items.push(self.parse_list_literal_item()?);
            self.skip_newlines();
            while self.match_token(TokenKind::Comma) {
                self.skip_newlines();
                if self.at(TokenKind::RBracket) {
                    break;
                }
                items.push(self.parse_list_literal_item()?);
                self.skip_newlines();
            }
        }
        let end = self.consume(TokenKind::RBracket, "expected ']' after list literal")?;
        Some(Expr::ListLiteral {
            items,
            span: start.cover(end),
        })
    }

    fn parse_list_literal_item(&mut self) -> Option<Expr> {
        if self.match_token(TokenKind::Ellipsis) {
            let start = self.previous_span();
            let value = self.parse_expr()?;
            let span = start.cover(value.span());
            return Some(Expr::Spread {
                value: Box::new(value),
                span,
            });
        }
        self.parse_expr()
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
