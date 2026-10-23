use crate::{
    diagnostic::Diagnostic,
    source::{LineColumn, SourceFile, Span},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Keyword {
    Annotation,
    As,
    Break,
    Continue,
    Case,
    Class,
    Defer,
    Def,
    Default,
    Do,
    Else,
    Ext,
    False,
    Fn,
    For,
    Get,
    Internal,
    Private,
    Public,
    If,
    Interface,
    Is,
    Let,
    Match,
    Mixin,
    Module,
    Object,
    Return,
    Reified,
    Shape,
    True,
    Try,
    Type,
    Use,
    Var,
    Vararg,
    When,
    While,
    With,
    Yield,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    Keyword(Keyword),
    Identifier,
    Integer,
    Float,
    Decimal,
    String,
    Newline,
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Comma,
    Dot,
    Colon,
    At,
    Ellipsis,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Caret,
    Eq,
    Bang,
    Less,
    Greater,
    FatArrow,
    LeftArrow,
    Question,
    QuestionQuestion,
    ColonAssign,
    EqEq,
    NotEq,
    LessEq,
    GreaterEq,
    PlusEq,
    MinusEq,
    StarEq,
    SlashEq,
    PercentEq,
    AndAnd,
    OrOr,
    Pipe,
    Eof,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
    /// Exact source text consumed for this token, including delimiters and
    /// escapes. For example, a string token retains its quotes; EOF is empty.
    pub lexeme: String,
    pub span: Span,
}

#[derive(Debug, Clone, Default)]
pub struct LexResult {
    pub tokens: Vec<Token>,
    pub diagnostics: Vec<Diagnostic>,
}

impl LexResult {
    pub fn has_errors(&self) -> bool {
        !self.diagnostics.is_empty()
    }
}

pub fn lex(file: &SourceFile) -> LexResult {
    Lexer::new(file).lex_all()
}

struct Lexer<'a> {
    file: &'a SourceFile,
    chars: Vec<char>,
    byte_offsets: Vec<usize>,
    index: usize,
    byte_index: usize,
    line: usize,
    column: usize,
    result: LexResult,
}

impl<'a> Lexer<'a> {
    fn new(file: &'a SourceFile) -> Self {
        let mut chars = Vec::new();
        let mut byte_offsets = Vec::new();
        for (offset, ch) in file.text.char_indices() {
            chars.push(ch);
            byte_offsets.push(offset);
        }
        Self {
            file,
            chars,
            byte_offsets,
            index: 0,
            byte_index: 0,
            line: 1,
            column: 1,
            result: LexResult::default(),
        }
    }

    fn lex_all(mut self) -> LexResult {
        // The lexer is a single forward cursor over pre-decoded chars. Each
        // branch either consumes simple trivia directly or hands control to a
        // token-specific scanner that advances the same shared cursor.
        while let Some(ch) = self.peek() {
            match ch {
                ' ' | '\t' | '\r' => {
                    self.bump();
                }
                '\n' => self.lex_newline(),
                '#' => self.skip_comment(),
                '"' => self.lex_string(),
                '0'..='9' => self.lex_number(),
                'A'..='Z' | 'a'..='z' | '_' => self.lex_identifier_or_keyword(),
                _ => self.lex_symbol(),
            }
        }

        let pos = LineColumn::new(self.line, self.column);
        self.result.tokens.push(Token {
            kind: TokenKind::Eof,
            lexeme: String::new(),
            span: Span::new(self.byte_index, self.byte_index, pos, pos),
        });
        self.result
    }

    fn lex_newline(&mut self) {
        let start = self.mark();
        self.bump();
        self.push_token(TokenKind::Newline, start, self.mark());
    }

    fn skip_comment(&mut self) {
        while let Some(ch) = self.peek() {
            if ch == '\n' {
                break;
            }
            self.bump();
        }
    }

    fn lex_string(&mut self) {
        let start = self.mark();
        self.lex_string_from(start);
    }

    fn lex_string_from(&mut self, start: Mark) {
        if self.peek_n(0) == Some('"') && self.peek_n(1) == Some('"') && self.peek_n(2) == Some('"')
        {
            self.lex_multiline_string(start);
        } else {
            self.lex_single_line_string(start);
        }
    }

    fn lex_multiline_string(&mut self, start: Mark) {
        self.bump();
        self.bump();
        self.bump();
        while let Some(ch) = self.peek() {
            if ch == '"' && self.peek_n(1) == Some('"') && self.peek_n(2) == Some('"') {
                self.bump();
                self.bump();
                self.bump();
                self.push_token(TokenKind::String, start, self.mark());
                return;
            }
            self.bump();
        }

        self.error(
            "unterminated_string",
            "unterminated multiline string literal",
            start,
            self.mark(),
        );
    }

    fn lex_single_line_string(&mut self, start: Mark) {
        self.bump();

        // if next char is escaped (after \\)
        let mut escaped = false;
        let mut crossed_newline = false;
        while let Some(ch) = self.peek() {
            if ch == '\n' {
                if !crossed_newline {
                    self.error(
                        "newline_in_string",
                        "ordinary string literals cannot span physical lines; use triple quotes for multiline text",
                        start,
                        self.mark(),
                    );
                    crossed_newline = true;
                }
                self.bump();
                escaped = false;
                continue;
            }
            self.bump();
            if escaped {
                escaped = false;
                continue;
            }
            match ch {
                '\\' => escaped = true,
                '"' => {
                    if !crossed_newline {
                        self.push_token(TokenKind::String, start, self.mark());
                    }
                    return;
                }
                _ => {}
            }
        }

        if !crossed_newline {
            self.error(
                "unterminated_string",
                "unterminated string literal",
                start,
                self.mark(),
            );
        }
    }

    fn bump_decimal_digits(&mut self) -> (bool, bool) {
        let mut saw_digit = false;
        let mut previous_was_digit = false;
        let mut separators_valid = true;

        while let Some(ch) = self.peek() {
            match ch {
                '0'..='9' => {
                    self.bump();
                    saw_digit = true;
                    previous_was_digit = true;
                }
                '_' => {
                    if !previous_was_digit || !matches!(self.peek_n(1), Some('0'..='9')) {
                        separators_valid = false;
                    }
                    self.bump();
                    previous_was_digit = false;
                }
                _ => break,
            }
        }

        (saw_digit, separators_valid)
    }

    fn lex_number(&mut self) {
        let start = self.mark();
        let (_, mut separators_valid) = self.bump_decimal_digits();
        let mut kind = TokenKind::Integer;

        if self.peek() == Some('.') {
            let next = self.peek_n(1);

            if matches!(next, Some('0'..='9'))
                || (next != Some('.') && !matches!(next, Some('A'..='Z' | 'a'..='z' | '_')))
            {
                self.bump();
                let (_, fraction_separators_valid) = self.bump_decimal_digits();
                separators_valid &= fraction_separators_valid;
                kind = TokenKind::Float;
            }
            // Otherwise leave the dot for member access, as in 111.toStr().
        }

        let mut exponent_has_digits = true;
        if matches!(self.peek(), Some('e' | 'E')) {
            kind = TokenKind::Float;
            self.bump();
            if matches!(self.peek(), Some('+' | '-')) {
                self.bump();
            }
            let (saw_digit, exponent_separators_valid) = self.bump_decimal_digits();
            exponent_has_digits = saw_digit;
            separators_valid &= exponent_separators_valid;
        }

        if self.peek() == Some('d') {
            self.bump();
            kind = TokenKind::Decimal;
        }
        let end = self.mark();
        self.push_token(kind, start, self.mark());
        if !separators_valid {
            self.error(
                "invalid_numeric_separator",
                "numeric separators must appear between decimal digits",
                start,
                end,
            );
        } else if !exponent_has_digits {
            self.error(
                "invalid_numeric_literal",
                "scientific-notation exponents must contain decimal digits",
                start,
                end,
            );
        }
    }

    fn lex_identifier_or_keyword(&mut self) {
        let start = self.mark();
        while matches!(self.peek(), Some('A'..='Z' | 'a'..='z' | '0'..='9' | '_')) {
            self.bump();
        }
        let lexeme = self.slice(start.byte, self.byte_index);
        if lexeme == "raw" && self.peek() == Some('"') {
            self.lex_string_from(start);
            return;
        }
        let kind = match lexeme.as_str() {
            "annotation" => TokenKind::Keyword(Keyword::Annotation),
            "as" => TokenKind::Keyword(Keyword::As),
            "break" => TokenKind::Keyword(Keyword::Break),
            "continue" => TokenKind::Keyword(Keyword::Continue),
            "case" => TokenKind::Keyword(Keyword::Case),
            "class" => TokenKind::Keyword(Keyword::Class),
            "defer" => TokenKind::Keyword(Keyword::Defer),
            "def" => TokenKind::Keyword(Keyword::Def),
            "default" => TokenKind::Keyword(Keyword::Default),
            "do" => TokenKind::Keyword(Keyword::Do),
            "else" => TokenKind::Keyword(Keyword::Else),
            "ext" => TokenKind::Keyword(Keyword::Ext),
            "false" => TokenKind::Keyword(Keyword::False),
            "fn" => TokenKind::Keyword(Keyword::Fn),
            "for" => TokenKind::Keyword(Keyword::For),
            "get" => TokenKind::Keyword(Keyword::Get),
            "internal" => TokenKind::Keyword(Keyword::Internal),
            "private" => TokenKind::Keyword(Keyword::Private),
            "public" => TokenKind::Keyword(Keyword::Public),
            "if" => TokenKind::Keyword(Keyword::If),
            "use" => TokenKind::Keyword(Keyword::Use),
            "interface" => TokenKind::Keyword(Keyword::Interface),
            "is" => TokenKind::Keyword(Keyword::Is),
            "let" => TokenKind::Keyword(Keyword::Let),
            "match" => TokenKind::Keyword(Keyword::Match),
            "mixin" => TokenKind::Keyword(Keyword::Mixin),
            "module" => TokenKind::Keyword(Keyword::Module),
            "return" => TokenKind::Keyword(Keyword::Return),
            "reified" => TokenKind::Keyword(Keyword::Reified),
            "shape" => TokenKind::Keyword(Keyword::Shape),
            "object" => TokenKind::Keyword(Keyword::Object),
            "true" => TokenKind::Keyword(Keyword::True),
            "try" => TokenKind::Keyword(Keyword::Try),
            "type" => TokenKind::Keyword(Keyword::Type),
            "var" => TokenKind::Keyword(Keyword::Var),
            "vararg" => TokenKind::Keyword(Keyword::Vararg),
            "when" => TokenKind::Keyword(Keyword::When),
            "while" => TokenKind::Keyword(Keyword::While),
            "with" => TokenKind::Keyword(Keyword::With),
            "yield" => TokenKind::Keyword(Keyword::Yield),
            _ => TokenKind::Identifier,
        };
        self.result.tokens.push(Token {
            kind,
            lexeme,
            span: Span::new(start.byte, self.byte_index, start.pos, self.position()),
        });
    }

    fn lex_symbol(&mut self) {
        let start = self.mark();
        let Some(ch) = self.bump() else {
            return;
        };
        let kind = match ch {
            '(' => Some(TokenKind::LParen),
            ')' => Some(TokenKind::RParen),
            '{' => Some(TokenKind::LBrace),
            '}' => Some(TokenKind::RBrace),
            '[' => Some(TokenKind::LBracket),
            ']' => Some(TokenKind::RBracket),
            ',' => Some(TokenKind::Comma),
            '.' if self.peek() == Some('.') && self.peek_n(1) == Some('.') => {
                self.bump();
                self.bump();
                Some(TokenKind::Ellipsis)
            }
            '.' => Some(TokenKind::Dot),
            ':' if self.take('+') => return self.unsupported_operator(start, ":+"),
            ':' if self.take('-') => return self.unsupported_operator(start, ":-"),
            ':' if self.take(':') => return self.unsupported_operator(start, "::"),
            ':' if self.take('<') => {
                self.error(
                    "removed_shape_update_operator",
                    "shape update operator ':<' was removed; use '{ ...value, field: replacement }'",
                    start,
                    self.mark(),
                );
                return;
            }
            ':' if self.take('=') => Some(TokenKind::ColonAssign),
            ':' => Some(TokenKind::Colon),
            '@' => Some(TokenKind::At),
            '+' if self.take('+') => return self.unsupported_operator(start, "++"),
            '+' if self.take('=') => Some(TokenKind::PlusEq),
            '+' => Some(TokenKind::Plus),
            '-' if self.take('-') => return self.unsupported_operator(start, "--"),
            '-' if self.take('=') => Some(TokenKind::MinusEq),
            '-' => Some(TokenKind::Minus),
            '*' if self.take('=') => Some(TokenKind::StarEq),
            '*' => Some(TokenKind::Star),
            '/' if self.take('=') => Some(TokenKind::SlashEq),
            '/' => Some(TokenKind::Slash),
            '%' if self.take('=') => Some(TokenKind::PercentEq),
            '%' => Some(TokenKind::Percent),
            '^' => Some(TokenKind::Caret),
            '?' if self.take('?') => Some(TokenKind::QuestionQuestion),
            '?' => Some(TokenKind::Question),
            '=' if self.take('>') => Some(TokenKind::FatArrow),
            '=' if self.take('=') => Some(TokenKind::EqEq),
            '=' => Some(TokenKind::Eq),
            '!' if self.take('=') => Some(TokenKind::NotEq),
            '!' => Some(TokenKind::Bang),
            '<' if self.take('-') => Some(TokenKind::LeftArrow),
            '<' if self.take('=') => Some(TokenKind::LessEq),
            '<' => Some(TokenKind::Less),
            '>' if self.take('=') => Some(TokenKind::GreaterEq),
            '>' => Some(TokenKind::Greater),
            '&' if self.take('&') => Some(TokenKind::AndAnd),
            '&' => return self.unsupported_operator(start, "&"),
            '|' if self.take('|') => Some(TokenKind::OrOr),
            '|' => Some(TokenKind::Pipe),
            _ => None,
        };

        if let Some(kind) = kind {
            self.push_token(kind, start, self.mark());
            return;
        }

        self.error(
            "unexpected_character",
            format!("unexpected character '{}'", ch),
            start,
            self.mark(),
        );
    }

    fn unsupported_operator(&mut self, start: Mark, operator: &'static str) {
        self.error(
            "unsupported_operator",
            format!("operator '{operator}' is reserved and not currently supported"),
            start,
            self.mark(),
        );
    }

    fn push_token(&mut self, kind: TokenKind, start: Mark, end: Mark) {
        let lexeme = self.slice(start.byte, end.byte);
        self.result.tokens.push(Token {
            kind,
            lexeme,
            span: Span::new(start.byte, end.byte, start.pos, end.pos),
        });
    }

    fn error(&mut self, code: &'static str, message: impl Into<String>, start: Mark, end: Mark) {
        self.result.diagnostics.push(Diagnostic::error(
            code,
            message,
            Span::new(start.byte, end.byte, start.pos, end.pos),
        ));
    }

    fn take(&mut self, expected: char) -> bool {
        if self.peek() == Some(expected) {
            self.bump();
            return true;
        }
        false
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.index).copied()
    }

    fn peek_n(&self, offset: usize) -> Option<char> {
        self.chars.get(self.index + offset).copied()
    }

    fn bump(&mut self) -> Option<char> {
        // bump consumes exactly one decoded char and keeps all cursor views in
        // sync:
        // - `index` walks the `chars` vector
        // - `byte_index` stays aligned to the original UTF-8 source
        // - `line` / `column` track human-facing span positions
        let ch = self.peek()?;
        self.index += 1;
        // After stepping past the current char, the next byte position is
        // either the next char's starting offset or the end of the file.
        self.byte_index = if let Some(next) = self.byte_offsets.get(self.index) {
            *next
        } else {
            self.file.text.len()
        };
        if ch == '\n' {
            self.line += 1;
            self.column = 1;
        } else {
            self.column += 1;
        }
        Some(ch)
    }

    fn slice(&self, start: usize, end: usize) -> String {
        self.file.text[start..end].to_string()
    }

    fn mark(&self) -> Mark {
        // mark snapshots the current cursor so token/error spans can be built
        // later without recomputing byte or line/column positions.
        Mark {
            byte: self.byte_index,
            pos: self.position(),
        }
    }

    fn position(&self) -> LineColumn {
        LineColumn::new(self.line, self.column)
    }
}

#[derive(Debug, Clone, Copy)]
struct Mark {
    byte: usize,
    pos: LineColumn,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(text: &str) -> SourceFile {
        SourceFile::new("test.lum", text)
    }

    #[test]
    fn lexes_exact_decimal_literals_without_consuming_member_access() {
        let result = lex(&source("19.99d 3d 1_000.00d 1e-9d 3d.toStr()"));
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
        let decimals = result
            .tokens
            .iter()
            .filter(|token| token.kind == TokenKind::Decimal)
            .map(|token| token.lexeme.as_str())
            .collect::<Vec<_>>();
        assert_eq!(decimals, ["19.99d", "3d", "1_000.00d", "1e-9d", "3d"]);
        assert!(
            result
                .tokens
                .iter()
                .any(|token| token.kind == TokenKind::Dot)
        );
    }

    #[test]
    fn lexes_range_loop_tokens() {
        let result = lex(&source("for i <- Range(0, 10) {\n}\n"));
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);

        let kinds: Vec<TokenKind> = result.tokens.iter().map(|token| token.kind).collect();
        assert_eq!(
            kinds,
            vec![
                TokenKind::Keyword(Keyword::For),
                TokenKind::Identifier,
                TokenKind::LeftArrow,
                TokenKind::Identifier,
                TokenKind::LParen,
                TokenKind::Integer,
                TokenKind::Comma,
                TokenKind::Integer,
                TokenKind::RParen,
                TokenKind::LBrace,
                TokenKind::Newline,
                TokenKind::RBrace,
                TokenKind::Newline,
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn skips_hash_comments() {
        let result = lex(&source("# PRELUDE_SKIP\ndef run() Int = 0\n"));
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
        assert_eq!(result.tokens[0].kind, TokenKind::Newline);
        assert_eq!(result.tokens[1].kind, TokenKind::Keyword(Keyword::Def));
    }

    #[test]
    fn treats_removed_keywords_as_identifiers() {
        let result = lex(&source(
            "enum Int = 1\npartial Int = 2\nhidden Str = \"value\"\noverride Bool = true\n",
        ));
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
        assert_eq!(result.tokens[0].kind, TokenKind::Identifier);
        assert_eq!(result.tokens[0].lexeme, "enum");
        assert!(
            result
                .tokens
                .iter()
                .any(|token| token.kind == TokenKind::Identifier && token.lexeme == "partial")
        );
        assert!(
            result
                .tokens
                .iter()
                .any(|token| token.kind == TokenKind::Identifier && token.lexeme == "hidden")
        );
        assert!(
            result
                .tokens
                .iter()
                .any(|token| token.kind == TokenKind::Identifier && token.lexeme == "override")
        );
    }

    #[test]
    fn reports_unterminated_string() {
        let result = lex(&source("value = \"oops"));
        assert_eq!(result.diagnostics.len(), 1);
        assert_eq!(result.diagnostics[0].code, "unterminated_string");
    }

    #[test]
    fn rejects_physical_newlines_in_ordinary_strings() {
        for text in [
            "value = \"first\nsecond\"\n",
            "value = raw\"first\nsecond\"\n",
            "value = \"first\\\nsecond\"\n",
        ] {
            let result = lex(&source(text));
            assert_eq!(result.diagnostics.len(), 1, "{:#?}", result.diagnostics);
            assert_eq!(result.diagnostics[0].code, "newline_in_string");
            assert!(result.diagnostics[0].message.contains("use triple quotes"));
        }
    }

    #[test]
    fn keeps_escaped_newline_sequences_and_triple_quoted_strings_valid() {
        for text in [
            "value = \"first\\nsecond\"\n",
            "value = \"\"\"first\nsecond\"\"\"\n",
            "value = raw\"\"\"first\nsecond\"\"\"\n",
        ] {
            let result = lex(&source(text));
            assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
            assert_eq!(
                result
                    .tokens
                    .iter()
                    .filter(|token| token.kind == TokenKind::String)
                    .count(),
                1
            );
        }
    }

    #[test]
    fn rejects_reserved_symbolic_collection_operators() {
        let result = lex(&source("a :+ b\nc :- d\ne :: f\ng ++ h\ni -- j\na & b\n"));
        let messages = result
            .diagnostics
            .iter()
            .map(|diag| (diag.code, diag.message.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(messages.len(), 6);
        for operator in [":+", ":-", "::", "++", "--", "&"] {
            assert!(
                messages.iter().any(|(code, message)| {
                    *code == "unsupported_operator" && message.contains(operator)
                }),
                "{messages:#?}"
            );
        }
    }

    #[test]
    fn lexes_extended_language_tokens() {
        let result = lex(&source(
            "annotation Route { path Str }\next User { def label() Str = this.name get name Str = this.name }\nprivate token Str\ninternal cache Str\npublic def api() Unit = ()\nassert(true)\nuse model/things/{A as Alias}\nif true { 1 } else { 0 }\ntype Value = Int | Str\ndef metadata[reified A]() Type[A] = typeOf[A]\nmapper fn(Int) Str = value => value.toStr()\noptional Int? = None\nitems = for value <- values yield value + 1\nfor { value <- values } do { println(value) }\nvalue = try source.mapError { err => mapped(err) }\nfallback = maybe ?? 0\nupdated = { ...value, amount: 1 }\ndefaulted = default {}\nmerged = { ...left, ...right, amount: right.amount }\ncount %= 2\ndef spread(value [Str] vararg) Unit = ()\nmatch size { case Small | Large => () }\ntext = \"\"\"\nhello\n\"\"\"\nrawText = raw\"$name\\n\"\npi = 1.25\n",
        ));
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
        let kinds: Vec<TokenKind> = result.tokens.iter().map(|token| token.kind).collect();
        assert!(kinds.contains(&TokenKind::Keyword(Keyword::Annotation)));
        assert!(kinds.contains(&TokenKind::Keyword(Keyword::As)));
        assert!(kinds.contains(&TokenKind::Keyword(Keyword::Default)));
        assert!(kinds.contains(&TokenKind::Keyword(Keyword::Do)));
        assert!(kinds.contains(&TokenKind::Keyword(Keyword::Ext)));
        assert!(kinds.contains(&TokenKind::Keyword(Keyword::Fn)));
        assert!(kinds.contains(&TokenKind::Keyword(Keyword::Get)));
        assert!(kinds.contains(&TokenKind::Keyword(Keyword::Internal)));
        assert!(kinds.contains(&TokenKind::Keyword(Keyword::Private)));
        assert!(kinds.contains(&TokenKind::Keyword(Keyword::Public)));
        assert!(kinds.contains(&TokenKind::Keyword(Keyword::Reified)));
        assert!(kinds.contains(&TokenKind::Keyword(Keyword::Type)));
        assert!(kinds.contains(&TokenKind::Keyword(Keyword::Yield)));
        assert!(kinds.contains(&TokenKind::Keyword(Keyword::Vararg)));
        assert!(kinds.contains(&TokenKind::Question));
        assert!(kinds.contains(&TokenKind::QuestionQuestion));
        assert!(kinds.contains(&TokenKind::PercentEq));
        assert!(kinds.contains(&TokenKind::Pipe));
        assert!(kinds.contains(&TokenKind::Float));
        assert!(
            result
                .tokens
                .iter()
                .any(|token| token.kind == TokenKind::String && token.lexeme == "raw\"$name\\n\"")
        );
    }

    #[test]
    fn lexes_digit_separators_and_scientific_notation() {
        let result = lex(&source(
            "1_000_000 12.345_678 1e3 1.0e3 1e-3 2.5E+6 1.2e1_0",
        ));
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);

        let numbers = result
            .tokens
            .iter()
            .filter(|token| matches!(token.kind, TokenKind::Integer | TokenKind::Float))
            .map(|token| (token.kind, token.lexeme.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(
            numbers,
            vec![
                (TokenKind::Integer, "1_000_000"),
                (TokenKind::Float, "12.345_678"),
                (TokenKind::Float, "1e3"),
                (TokenKind::Float, "1.0e3"),
                (TokenKind::Float, "1e-3"),
                (TokenKind::Float, "2.5E+6"),
                (TokenKind::Float, "1.2e1_0"),
            ]
        );
    }

    #[test]
    fn rejects_misplaced_numeric_separators_and_incomplete_exponents() {
        for source_text in ["1__000", "1_", "1_.0", "1e_3", "1e-3_"] {
            let result = lex(&source(source_text));
            assert_eq!(result.diagnostics.len(), 1, "{source_text}: {result:#?}");
            assert_eq!(
                result.diagnostics[0].code, "invalid_numeric_separator",
                "{source_text}"
            );
        }

        for source_text in ["1e", "1e+", "1e-"] {
            let result = lex(&source(source_text));
            assert_eq!(result.diagnostics.len(), 1, "{source_text}: {result:#?}");
            assert_eq!(
                result.diagnostics[0].code, "invalid_numeric_literal",
                "{source_text}"
            );
        }
    }

    #[test]
    fn keeps_two_dots_distinct_without_affecting_ellipsis() {
        let result = lex(&source("user..name\n...items\n"));
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);

        let punctuation = result
            .tokens
            .iter()
            .filter(|token| matches!(token.kind, TokenKind::Dot | TokenKind::Ellipsis))
            .map(|token| (token.kind, token.lexeme.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(
            punctuation,
            vec![
                (TokenKind::Dot, "."),
                (TokenKind::Dot, "."),
                (TokenKind::Ellipsis, "...")
            ]
        );
    }

    #[test]
    fn rejects_removed_shape_update_operator() {
        let result = lex(&source("updated = value :< { amount: 1 }"));
        assert!(result.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "removed_shape_update_operator"
                && diagnostic
                    .message
                    .contains("use '{ ...value, field: replacement }'")
        }));
    }

    #[test]
    fn lexes_repeated_postfix_extraction_as_individual_bangs() {
        let result = lex(&source("value = wrapped !!\n"));
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
        assert_eq!(
            result
                .tokens
                .iter()
                .filter(|token| token.kind == TokenKind::Bang)
                .count(),
            2,
            "{:#?}",
            result.tokens
        );
    }

    #[test]
    fn lexes_option_wrap_operator() {
        let result = lex(&source("value Option[Int] = ^5\n"));
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
        assert!(
            result
                .tokens
                .iter()
                .any(|token| token.kind == TokenKind::Caret)
        );
    }

    #[test]
    fn keeps_postfix_extraction_distinct_from_equality() {
        let result = lex(&source(
            "unwrapped = maybe! == expected\ndifferent = maybe != expected\n",
        ));
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
        let kinds = result
            .tokens
            .iter()
            .map(|token| token.kind)
            .collect::<Vec<_>>();
        assert!(
            kinds
                .windows(2)
                .any(|pair| { pair == [TokenKind::Bang, TokenKind::EqEq] })
        );
        assert!(kinds.contains(&TokenKind::NotEq));
    }

    #[test]
    fn removed_strict_equality_spellings_are_not_atomic_tokens() {
        let result = lex(&source(
            "same = left === right\ndifferent = left !== other\n",
        ));
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
        let kinds = result
            .tokens
            .iter()
            .map(|token| token.kind)
            .collect::<Vec<_>>();
        assert!(
            kinds
                .windows(2)
                .any(|pair| pair == [TokenKind::EqEq, TokenKind::Eq])
        );
        assert!(
            kinds
                .windows(2)
                .any(|pair| pair == [TokenKind::NotEq, TokenKind::Eq])
        );
    }

    #[test]
    fn lexes_raw_strings_as_single_string_tokens() {
        let result = lex(&source(
            "single = raw\"$name\\n\"\nmulti = raw\"\"\"$name\n\\n\"\"\"\n",
        ));
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
        let strings: Vec<&str> = result
            .tokens
            .iter()
            .filter(|token| token.kind == TokenKind::String)
            .map(|token| token.lexeme.as_str())
            .collect();
        assert_eq!(
            strings,
            vec!["raw\"$name\\n\"", "raw\"\"\"$name\n\\n\"\"\""]
        );
    }
}
