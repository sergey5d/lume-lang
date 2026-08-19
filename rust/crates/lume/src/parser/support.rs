use super::*;

impl<'a> Parser<'a> {
    pub(super) fn synchronize_item(&mut self) {
        while !self.at(TokenKind::Eof) {
            if self.at(TokenKind::Newline) {
                self.advance();
                return;
            }
            match self.current_kind() {
                TokenKind::Keyword(Keyword::Annotation)
                | TokenKind::Keyword(Keyword::Def)
                | TokenKind::Keyword(Keyword::Class)
                | TokenKind::Keyword(Keyword::Shape)
                | TokenKind::Keyword(Keyword::Object)
                | TokenKind::Keyword(Keyword::Interface)
                | TokenKind::Keyword(Keyword::Enum)
                | TokenKind::Keyword(Keyword::Ext) => return,
                _ => self.advance(),
            }
        }
    }

    pub(super) fn synchronize_member(&mut self) {
        while !self.at(TokenKind::Eof) && !self.at(TokenKind::RBrace) {
            if self.at(TokenKind::Newline) {
                self.advance();
                return;
            }
            self.advance();
        }
    }

    pub(super) fn synchronize_stmt(&mut self) {
        while !self.at(TokenKind::Eof) && !self.at(TokenKind::RBrace) {
            if self.at(TokenKind::Newline) {
                self.advance();
                return;
            }
            self.advance();
        }
    }

    pub(super) fn checkpoint(&self) -> Checkpoint {
        Checkpoint {
            index: self.index,
            diagnostics_len: self.diagnostics.len(),
            allow_trailing_block_call: self.allow_trailing_block_call,
            allow_shape_update: self.allow_shape_update,
        }
    }

    pub(super) fn restore(&mut self, checkpoint: Checkpoint) {
        self.index = checkpoint.index;
        self.diagnostics.truncate(checkpoint.diagnostics_len);
        self.allow_trailing_block_call = checkpoint.allow_trailing_block_call;
        self.allow_shape_update = checkpoint.allow_shape_update;
    }

    pub(super) fn skip_newlines(&mut self) {
        while self.match_token(TokenKind::Newline) {}
    }

    pub(super) fn consume(&mut self, kind: TokenKind, message: &'static str) -> Option<Span> {
        if self.match_token(kind) {
            Some(self.previous_span())
        } else {
            self.error_at_current("unexpected_token", message);
            None
        }
    }

    pub(super) fn consume_keyword(
        &mut self,
        keyword: Keyword,
        message: &'static str,
    ) -> Option<Span> {
        if self.match_keyword(keyword) {
            Some(self.previous_span())
        } else {
            self.error_at_current("unexpected_token", message);
            None
        }
    }

    pub(super) fn error_missing_match_value(&mut self, partial: bool) {
        let message = if partial {
            "partial match requires a value before '{'; use 'partial match value { ... }'"
        } else {
            "match requires a value before '{'; use 'match value { ... }'"
        };
        self.error_at_current("missing_match_value", message);
    }

    pub(super) fn expect_identifier(&mut self, message: &'static str) -> Option<(String, Span)> {
        if self.at(TokenKind::Identifier) {
            let token = self.current().clone();
            self.advance();
            Some((token.lexeme, token.span))
        } else {
            self.error_at_current("expected_identifier", message);
            None
        }
    }

    pub(super) fn expect_binding_name(&mut self, message: &'static str) -> Option<(String, Span)> {
        self.expect_identifier(message)
    }

    pub(super) fn parse_callable_name(&mut self, message: &'static str) -> Option<(String, Span)> {
        if self.at(TokenKind::Identifier) {
            return self.expect_identifier(message);
        }
        if self.at_keyword(Keyword::Annotation) || self.at_keyword(Keyword::Case) {
            let token = self.current().clone();
            self.advance();
            return Some((token.lexeme, token.span));
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
            _ => {
                self.error_at_current("expected_identifier", message);
                return None;
            }
        };
        self.advance();
        Some((name.to_string(), token.span))
    }

    pub(super) fn starts_callable_decl(&self) -> bool {
        self.starts_callable_decl_at(self.index)
    }

    pub(super) fn starts_callable_decl_at(&self, start: usize) -> bool {
        let mut parser = Parser {
            tokens: self.tokens,
            index: start,
            diagnostics: Vec::new(),
            allow_trailing_block_call: self.allow_trailing_block_call,
            allow_shape_update: self.allow_shape_update,
        };

        let Some((_, name_span)) = parser.parse_callable_name("expected callable name") else {
            return false;
        };

        let mut head_end = name_span;
        if parser.at(TokenKind::LBracket) {
            if !spans_touch(head_end, parser.current_span()) {
                return false;
            }
            if parser.parse_type_params().is_none() {
                return false;
            }
            head_end = parser.previous_span();
        }

        if !parser.at(TokenKind::LParen) || !spans_touch(head_end, parser.current_span()) {
            return false;
        }

        if parser.parse_param_list().is_none() {
            return false;
        }

        true
    }

    pub(super) fn starts_local_callable_decl(&self) -> bool {
        self.starts_local_callable_decl_at(self.index)
    }

    pub(super) fn starts_local_callable_decl_at(&self, start: usize) -> bool {
        let mut parser = Parser {
            tokens: self.tokens,
            index: start,
            diagnostics: Vec::new(),
            allow_trailing_block_call: self.allow_trailing_block_call,
            allow_shape_update: self.allow_shape_update,
        };

        let Some((_, name_span)) = parser.parse_callable_name("expected callable name") else {
            return false;
        };

        let mut head_end = name_span;
        if parser.at(TokenKind::LBracket) {
            if !spans_touch(head_end, parser.current_span()) {
                return false;
            }
            if parser.parse_type_params().is_none() {
                return false;
            }
            head_end = parser.previous_span();
        }

        if !parser.at(TokenKind::LParen) || !spans_touch(head_end, parser.current_span()) {
            return false;
        }
        if parser.parse_param_list().is_none() || !parser.diagnostics.is_empty() {
            return false;
        }

        let close = parser.previous_span();
        if parser.current_span().start_pos.line != close.end_pos.line {
            return false;
        }
        if parser.at(TokenKind::Eq) {
            return true;
        }
        if parser.at(TokenKind::LBrace) {
            return !parser.looks_like_trailing_lambda_block_start();
        }

        if !parser.can_start_type_ref() || parser.parse_type_ref().is_none() {
            return false;
        }
        parser.skip_newlines();
        matches!(parser.current_kind(), TokenKind::Eq | TokenKind::LBrace)
    }

    pub(super) fn match_keyword(&mut self, keyword: Keyword) -> bool {
        if self.at_keyword(keyword) {
            self.advance();
            true
        } else {
            false
        }
    }

    pub(super) fn at_keyword(&self, keyword: Keyword) -> bool {
        matches!(self.current_kind(), TokenKind::Keyword(k) if k == keyword)
    }

    pub(super) fn match_token(&mut self, kind: TokenKind) -> bool {
        if self.at(kind) {
            self.advance();
            true
        } else {
            false
        }
    }

    pub(super) fn at(&self, kind: TokenKind) -> bool {
        self.current_kind() == kind
    }

    pub(super) fn at_next(&self, kind: TokenKind) -> bool {
        self.tokens
            .get(self.index + 1)
            .map(|token| token.kind == kind)
            .unwrap_or(false)
    }

    pub(super) fn binding_type_starts_on_same_line(&self, name_span: Span) -> bool {
        self.current_span().start_pos.line == name_span.end_pos.line
    }

    pub(super) fn pattern_followed_by_refutable_operator(&self, start: usize) -> bool {
        let mut parser = Parser {
            tokens: self.tokens,
            index: start,
            diagnostics: Vec::new(),
            allow_trailing_block_call: self.allow_trailing_block_call,
            allow_shape_update: self.allow_shape_update,
        };
        if parser.parse_pattern().is_none() {
            return false;
        }
        matches!(parser.current_kind(), TokenKind::Eq | TokenKind::LeftArrow)
    }

    pub(super) fn scan_if_condition_expr_end(&self, start: usize) -> usize {
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
                TokenKind::LBrace => {
                    if paren_depth == 0 && brace_depth == 0 && bracket_depth == 0 {
                        break;
                    }
                    brace_depth += 1;
                }
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
                TokenKind::AndAnd | TokenKind::Newline
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

    pub(super) fn is_placeholder_identifier(&self) -> bool {
        self.at(TokenKind::Identifier) && self.current().lexeme == "_"
    }

    pub(super) fn is_for_yield_start(&self) -> bool {
        if !self.at_keyword(Keyword::For) {
            return false;
        }
        if self
            .tokens
            .get(self.index + 1)
            .is_some_and(|token| token.kind == TokenKind::LBrace)
        {
            let mut i = self.index + 1;
            let mut brace_depth = 0isize;
            while let Some(token) = self.tokens.get(i) {
                match token.kind {
                    TokenKind::LBrace => brace_depth += 1,
                    TokenKind::RBrace => {
                        brace_depth -= 1;
                        if brace_depth == 0 {
                            i += 1;
                            break;
                        }
                    }
                    TokenKind::Eof => return false,
                    _ => {}
                }
                i += 1;
            }
            while self
                .tokens
                .get(i)
                .is_some_and(|token| token.kind == TokenKind::Newline)
            {
                i += 1;
            }
            return self
                .tokens
                .get(i)
                .is_some_and(|token| matches!(token.kind, TokenKind::Keyword(Keyword::Yield)));
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

    pub(super) fn current(&self) -> &Token {
        &self.tokens[self.index.min(self.tokens.len().saturating_sub(1))]
    }

    pub(super) fn current_kind(&self) -> TokenKind {
        self.current().kind
    }

    pub(super) fn current_span(&self) -> Span {
        self.current().span
    }

    pub(super) fn previous_span(&self) -> Span {
        self.tokens
            .get(self.index.saturating_sub(1))
            .map(|token| token.span)
            .unwrap_or_else(|| self.current_span())
    }

    pub(super) fn last_non_newline_span(&self, fallback: Span) -> Span {
        for token in self.tokens[..self.index].iter().rev() {
            if token.kind != TokenKind::Newline {
                return token.span;
            }
        }
        fallback
    }

    pub(super) fn next_significant_token(&self) -> &Token {
        let mut index = self.index;
        while let Some(token) = self.tokens.get(index) {
            if token.kind != TokenKind::Newline {
                return token;
            }
            index += 1;
        }
        self.current()
    }

    pub(super) fn next_significant_token_string(&self) -> String {
        self.format_token_like(self.next_significant_token())
    }

    pub(super) fn current_token_string(&self) -> String {
        self.format_token_like(self.current())
    }

    pub(super) fn format_token_like(&self, token: &Token) -> String {
        format!(
            "{}(\"{}\" @ {}:{})",
            self.token_kind_label(token.kind),
            token.lexeme,
            token.span.start_pos.line,
            token.span.start_pos.column
        )
    }

    pub(super) fn token_kind_label(&self, kind: TokenKind) -> &'static str {
        match kind {
            TokenKind::Identifier => "IDENT",
            TokenKind::Integer => "INT",
            TokenKind::Float => "FLOAT",
            TokenKind::String => "STRING",
            TokenKind::Keyword(Keyword::Case) => "CASE",
            TokenKind::Keyword(Keyword::If) => "IF",
            TokenKind::Keyword(Keyword::Else) => "ELSE",
            TokenKind::Keyword(Keyword::Match) => "MATCH",
            TokenKind::Keyword(Keyword::Partial) => "PARTIAL",
            TokenKind::Keyword(Keyword::Reified) => "REIFIED",
            TokenKind::Keyword(Keyword::Fn) => "FN",
            TokenKind::Keyword(Keyword::For) => "FOR",
            TokenKind::Keyword(Keyword::Yield) => "YIELD",
            TokenKind::Keyword(Keyword::Continue) => "CONTINUE",
            TokenKind::Keyword(Keyword::Annotation) => "ANNOTATION",
            TokenKind::Keyword(Keyword::Def) => "DEF",
            TokenKind::Keyword(Keyword::Class) => "CLASS",
            TokenKind::Keyword(Keyword::Shape) => "SHAPE",
            TokenKind::Keyword(Keyword::Object) => "OBJECT",
            TokenKind::Keyword(Keyword::Interface) => "INTERFACE",
            TokenKind::Keyword(Keyword::Enum) => "ENUM",
            TokenKind::Keyword(Keyword::Ext) => "EXT",
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
            TokenKind::Question => "?",
            TokenKind::QuestionQuestion => "??",
            TokenKind::PercentEq => "%=",
            TokenKind::Newline => "NEWLINE",
            TokenKind::Eof => "EOF",
            _ => "TOKEN",
        }
    }

    pub(super) fn advance(&mut self) {
        if !self.at(TokenKind::Eof) {
            self.index += 1;
        }
    }

    pub(super) fn error_at_current(&mut self, code: &'static str, message: impl Into<String>) {
        let span = self.current_span();
        self.diagnostics
            .push(Diagnostic::error(code, message, span));
    }
}

pub(super) fn spans_touch(left: Span, right: Span) -> bool {
    left.end == right.start
}
