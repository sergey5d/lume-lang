use super::*;

impl<'a> Parser<'a> {
    pub(super) fn synchronize_item(&mut self) {
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
                | TokenKind::Keyword(Keyword::Single)
                | TokenKind::Keyword(Keyword::Interface)
                | TokenKind::Keyword(Keyword::Enum)
                | TokenKind::Keyword(Keyword::Impl) => return,
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

    pub(super) fn pattern_followed_by_eq(&self, start: usize) -> bool {
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
                TokenKind::AndAnd | TokenKind::Keyword(Keyword::Then) | TokenKind::Newline
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
            let parser = Parser {
                tokens: self.tokens,
                index: self.index + 1,
                diagnostics: Vec::new(),
                allow_trailing_block_call: self.allow_trailing_block_call,
            };
            return !parser.is_for_brace_destructuring_binding_start();
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
            TokenKind::Keyword(Keyword::Then) => "THEN",
            TokenKind::Keyword(Keyword::Else) => "ELSE",
            TokenKind::Keyword(Keyword::Match) => "MATCH",
            TokenKind::Keyword(Keyword::Partial) => "PARTIAL",
            TokenKind::Keyword(Keyword::For) => "FOR",
            TokenKind::Keyword(Keyword::Yield) => "YIELD",
            TokenKind::Keyword(Keyword::Continue) => "CONTINUE",
            TokenKind::Keyword(Keyword::Def) => "DEF",
            TokenKind::Keyword(Keyword::Class) => "CLASS",
            TokenKind::Keyword(Keyword::Record) => "RECORD",
            TokenKind::Keyword(Keyword::Object) => "OBJECT",
            TokenKind::Keyword(Keyword::Single) => "SINGLE",
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
