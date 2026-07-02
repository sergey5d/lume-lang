use super::*;

impl<'a> Parser<'a> {
    pub(super) fn wrap_extract_pattern(&self, pattern: Pattern) -> Pattern {
        let span = pattern.span();
        Pattern::Extract {
            inner: Box::new(pattern),
            span,
        }
    }

    pub(super) fn parse_refutable_pattern_head(
        &mut self,
        owner: &'static str,
    ) -> Option<(Pattern, &'static str)> {
        let pattern = self.parse_pattern()?;
        if self.match_token(TokenKind::Eq) {
            return Some((pattern, "="));
        }
        if self.match_token(TokenKind::LeftArrow) {
            return Some((self.wrap_extract_pattern(pattern), "<-"));
        }

        let message = match owner {
            "if let" => "expected '=' or '<-' after if pattern",
            "let" => "expected '=' or '<-' after let pattern",
            "expect" => "expected '=' or '<-' after expect pattern",
            _ => "expected '=' or '<-' after pattern",
        };
        self.error_at_current("unexpected_token", message);
        None
    }

    pub(super) fn parse_refutable_clause(
        &mut self,
        owner: &'static str,
    ) -> Option<RefutableClause> {
        let (pattern, operator) = self.parse_refutable_pattern_head(owner)?;
        if operator != "=" && self.at(TokenKind::Newline) {
            self.error_at_current(
                "expected_expression",
                format!("expected expression on same line after \"{operator}\""),
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

    pub(super) fn parse_if_condition_refutable_clause(
        &mut self,
        owner: &'static str,
    ) -> Option<RefutableClause> {
        let (pattern, operator) = self.parse_refutable_pattern_head(owner)?;
        if operator != "=" && self.at(TokenKind::Newline) {
            self.error_at_current(
                "expected_expression",
                format!("expected expression on same line after \"{operator}\""),
            );
            return None;
        }
        let value = self.parse_if_condition_expr()?;
        let span = pattern.span().cover(value.span());
        Some(RefutableClause {
            pattern,
            value,
            span,
        })
    }

    pub(super) fn parse_refutable_clause_block(
        &mut self,
        owner: &'static str,
    ) -> Option<(Vec<RefutableClause>, Span)> {
        let open_message = match owner {
            "if let" => "expected '{' after if let",
            "let" => "expected '{' after let",
            "expect" => "expected '{' after expect",
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
                format!(
                    "{owner} clause block must contain at least one 'PATTERN = value' or 'PATTERN <- value' clause"
                ),
            );
            return None;
        }
        let close_message = match owner {
            "if let" => "expected '}' after if let clause block",
            "let" => "expected '}' after let clause block",
            "expect" => "expected '}' after expect clause block",
            _ => "expected '}' after clause block",
        };
        let close = self.consume(TokenKind::RBrace, close_message)?;
        Some((clauses, open.cover(close)))
    }

    pub(super) fn parse_if_condition_clauses(
        &mut self,
        mut clauses: Vec<IfConditionClause>,
    ) -> Option<Vec<IfConditionClause>> {
        while self.match_token(TokenKind::AndAnd) {
            if self.at(TokenKind::Newline) {
                self.error_at_current(
                    "expected_expression",
                    "expected expression on same line after \"&&\"",
                );
                return None;
            }
            if self.match_keyword(Keyword::Let) {
                if self.at(TokenKind::LBrace) {
                    let (grouped, _) = self.parse_refutable_clause_block("if let")?;
                    clauses.extend(grouped.into_iter().map(IfConditionClause::Let));
                    continue;
                }
                clauses.push(IfConditionClause::Let(
                    self.parse_if_condition_refutable_clause("if let")?,
                ));
                continue;
            }
            clauses.push(IfConditionClause::Expr(self.parse_if_condition_expr()?));
        }
        Some(clauses)
    }

    pub(super) fn parse_if_condition_expr(&mut self) -> Option<Expr> {
        let end = self.scan_if_condition_expr_end(self.index);
        if end == self.index {
            self.error_at_current("expected_expression", "expected expression");
            return None;
        }

        let mut owned = self.tokens[self.index..end].to_vec();
        let eof_span = owned
            .last()
            .map(|token| token.span)
            .unwrap_or_else(|| self.current_span());
        owned.push(Token {
            kind: TokenKind::Eof,
            lexeme: String::new(),
            span: eof_span,
        });

        let mut parser = Parser {
            tokens: &owned,
            index: 0,
            diagnostics: Vec::new(),
            allow_trailing_block_call: false,
        };
        let expr = parser.parse_expr()?;
        if !parser.at(TokenKind::Eof) {
            parser.error_at_current(
                "unexpected_token",
                format!(
                    "expected end of expression, got {}",
                    parser.current_token_string()
                ),
            );
        }
        self.diagnostics.extend(parser.diagnostics);
        self.index = end;
        Some(expr)
    }

    pub(super) fn parse_pattern(&mut self) -> Option<Pattern> {
        self.parse_pattern_at_depth(0)
    }

    pub(super) fn parse_pattern_at_depth(&mut self, depth: usize) -> Option<Pattern> {
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
                if path.len() == 1 && !looks_like_constructor_pattern(&name) {
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
}

fn looks_like_constructor_pattern(name: &str) -> bool {
    name.chars()
        .next()
        .is_some_and(|ch| ch.is_ascii_uppercase())
}
