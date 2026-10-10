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
                | TokenKind::Keyword(Keyword::Type)
                | TokenKind::Keyword(Keyword::Class)
                | TokenKind::Keyword(Keyword::Shape)
                | TokenKind::Keyword(Keyword::Object)
                | TokenKind::Keyword(Keyword::Interface)
                | TokenKind::Keyword(Keyword::Ext) => return,
                _ => self.advance(),
            }
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
        }
    }

    pub(super) fn restore(&mut self, checkpoint: Checkpoint) {
        self.index = checkpoint.index;
        self.diagnostics.truncate(checkpoint.diagnostics_len);
        self.allow_trailing_block_call = checkpoint.allow_trailing_block_call;
    }

    pub(super) fn skip_newlines(&mut self) {
        while self.match_token(TokenKind::Newline) {}
    }

    pub(super) fn report_trailing_comma(&mut self, comma: Span, context: &'static str) {
        self.diagnostics.push(Diagnostic::error(
            "trailing_comma",
            format!(
                "trailing commas are allowed only in bracket collection and brace construction literals; remove the trailing comma from {context}"
            ),
            comma,
        ));
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

    pub(super) fn error_missing_match_value(&mut self) {
        self.error_at_current(
            "missing_match_value",
            "match requires a value before '{'; use 'match value { ... }'",
        );
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

    pub(super) fn expect_data_name(&mut self, message: &'static str) -> Option<(String, Span)> {
        if self.at(TokenKind::Identifier) || self.at_keyword(Keyword::Type) {
            let token = self.current().clone();
            self.advance();
            Some((token.lexeme, token.span))
        } else {
            self.error_at_current("expected_identifier", message);
            None
        }
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
            TokenKind::Caret => "^",
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
            type_path: self.type_path.clone(),
            nested_items: Vec::new(),
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
            type_path: self.type_path.clone(),
            nested_items: Vec::new(),
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
            type_path: self.type_path.clone(),
            nested_items: Vec::new(),
        };
        if parser.parse_pattern().is_none() {
            return false;
        }
        matches!(parser.current_kind(), TokenKind::Eq | TokenKind::LeftArrow)
    }

    pub(super) fn is_headless_record_pattern_assignment_start(&self) -> bool {
        if !self.at(TokenKind::LBrace) {
            return false;
        }
        let mut index = self.index;
        let mut depth = 0usize;
        loop {
            let Some(token) = self.tokens.get(index) else {
                return false;
            };
            match token.kind {
                TokenKind::LBrace => depth += 1,
                TokenKind::RBrace => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        index += 1;
                        break;
                    }
                }
                TokenKind::Eof => return false,
                _ => {}
            }
            index += 1;
        }
        while self
            .tokens
            .get(index)
            .is_some_and(|token| token.kind == TokenKind::Newline)
        {
            index += 1;
        }
        if self
            .tokens
            .get(index)
            .is_some_and(|token| token.kind == TokenKind::Keyword(Keyword::As))
        {
            index += 1;
            if !self
                .tokens
                .get(index)
                .is_some_and(|token| token.kind == TokenKind::Identifier)
            {
                return false;
            }
            index += 1;
        }
        matches!(
            self.tokens.get(index).map(|token| token.kind),
            Some(TokenKind::Eq | TokenKind::LeftArrow)
        )
    }

    pub(super) fn parenthesized_for_generator_header(&self) -> bool {
        if !self.at(TokenKind::LParen) {
            return false;
        }

        let mut depth = 0usize;
        let mut contains_generator_arrow = false;
        for token in &self.tokens[self.index..] {
            match token.kind {
                TokenKind::LParen => depth += 1,
                TokenKind::RParen => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        return contains_generator_arrow;
                    }
                }
                TokenKind::LeftArrow if depth > 0 => contains_generator_arrow = true,
                TokenKind::Eof => return false,
                _ => {}
            }
        }
        false
    }

    pub(super) fn discard_parenthesized_for_generator_header(&mut self) {
        let mut depth = 0usize;
        while !self.at(TokenKind::Eof) {
            match self.current_kind() {
                TokenKind::LParen => depth += 1,
                TokenKind::RParen => {
                    depth = depth.saturating_sub(1);
                    self.advance();
                    if depth == 0 {
                        return;
                    }
                    continue;
                }
                _ => {}
            }
            self.advance();
        }
    }

    pub(super) fn scan_if_condition_expr_end(&self, start: usize) -> usize {
        self.scan_if_condition_segment_end(start, false)
    }

    pub(super) fn scan_if_condition_let_value_end(&self, start: usize) -> usize {
        self.scan_if_condition_segment_end(start, true)
    }

    pub(super) fn find_top_level_or(&self, start: usize, end: usize) -> Option<Span> {
        let mut paren_depth = 0isize;
        let mut brace_depth = 0isize;
        let mut bracket_depth = 0isize;
        for token in &self.tokens[start..end] {
            let at_top_level = paren_depth == 0 && brace_depth == 0 && bracket_depth == 0;
            match token.kind {
                TokenKind::OrOr if at_top_level => return Some(token.span),
                TokenKind::LParen => paren_depth += 1,
                TokenKind::RParen => paren_depth = paren_depth.saturating_sub(1),
                TokenKind::LBrace => brace_depth += 1,
                TokenKind::RBrace => brace_depth = brace_depth.saturating_sub(1),
                TokenKind::LBracket => bracket_depth += 1,
                TokenKind::RBracket => bracket_depth = bracket_depth.saturating_sub(1),
                _ => {}
            }
        }
        None
    }

    fn scan_if_condition_segment_end(&self, start: usize, stop_at_any_and: bool) -> usize {
        let mut i = start;
        let mut paren_depth = 0isize;
        let mut brace_depth = 0isize;
        let mut bracket_depth = 0isize;
        while let Some(token) = self.tokens.get(i) {
            let at_top_level = paren_depth == 0 && brace_depth == 0 && bracket_depth == 0;
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
                TokenKind::AndAnd
                    if at_top_level
                        && (stop_at_any_and
                            || self.tokens[i + 1..]
                                .iter()
                                .find(|next| next.kind != TokenKind::Newline)
                                .is_some_and(|next| {
                                    next.kind == TokenKind::Keyword(Keyword::Let)
                                })) =>
                {
                    break;
                }
                TokenKind::Newline
                    if at_top_level && !self.condition_segment_continues_after(i) =>
                {
                    break;
                }
                _ => {}
            }
            i += 1;
        }
        i
    }

    fn condition_segment_continues_after(&self, newline_index: usize) -> bool {
        let Some(previous_index) = self.tokens[..newline_index]
            .iter()
            .rposition(|token| token.kind != TokenKind::Newline)
        else {
            return false;
        };
        let previous = &self.tokens[previous_index];
        if matches!(
            previous.kind,
            TokenKind::AndAnd
                | TokenKind::OrOr
                | TokenKind::EqEq
                | TokenKind::NotEq
                | TokenKind::StrictEq
                | TokenKind::StrictNotEq
                | TokenKind::Less
                | TokenKind::LessEq
                | TokenKind::Greater
                | TokenKind::GreaterEq
                | TokenKind::Plus
                | TokenKind::Minus
                | TokenKind::Star
                | TokenKind::Slash
                | TokenKind::Percent
                | TokenKind::QuestionQuestion
                | TokenKind::Keyword(Keyword::Is)
        ) {
            return true;
        }

        previous.kind == TokenKind::Identifier
            && previous.lexeme == "not"
            && self.tokens[..previous_index]
                .iter()
                .rfind(|token| token.kind != TokenKind::Newline)
                .is_some_and(|token| token.kind == TokenKind::Keyword(Keyword::Is))
    }

    pub(super) fn scan_match_guard_expr_end(&self, start: usize) -> usize {
        let mut i = start;
        let mut paren_depth = 0isize;
        let mut brace_depth = 0isize;
        let mut bracket_depth = 0isize;
        while let Some(token) = self.tokens.get(i) {
            let at_top_level = paren_depth == 0 && brace_depth == 0 && bracket_depth == 0;
            match token.kind {
                TokenKind::FatArrow if at_top_level => break,
                TokenKind::Keyword(Keyword::Case) | TokenKind::Eof if at_top_level => break,
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
            TokenKind::Keyword(Keyword::Ext) => "EXT",
            TokenKind::Keyword(Keyword::Internal) => "INTERNAL",
            TokenKind::Keyword(Keyword::Private) => "PRIVATE",
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
