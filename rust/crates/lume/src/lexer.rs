use crate::{
    diagnostic::Diagnostic,
    source::{LineColumn, SourceFile, Span},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Keyword {
    Annotation,
    Assert,
    As,
    Break,
    Continue,
    Case,
    Class,
    Defer,
    Def,
    Else,
    Enum,
    Expect,
    False,
    For,
    Hidden,
    If,
    Impl,
    Interface,
    Is,
    Let,
    Match,
    Module,
    Partial,
    Public,
    Return,
    Shape,
    Single,
    True,
    Try,
    Use,
    Var,
    Vararg,
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
    DotArrow,
    Colon,
    At,
    Ellipsis,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Eq,
    Bang,
    Less,
    Greater,
    Arrow,
    FatArrow,
    LeftArrow,
    ColonPlus,
    ColonLess,
    ColonAssign,
    EqEq,
    NotEq,
    LessEq,
    GreaterEq,
    PlusEq,
    MinusEq,
    StarEq,
    SlashEq,
    AndAnd,
    OrOr,
    Eof,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
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

        let mut escaped = false;
        while let Some(ch) = self.peek() {
            self.bump();
            if escaped {
                escaped = false;
                continue;
            }
            match ch {
                '\\' => escaped = true,
                '"' => {
                    self.push_token(TokenKind::String, start, self.mark());
                    return;
                }
                _ => {}
            }
        }

        self.error(
            "unterminated_string",
            "unterminated string literal",
            start,
            self.mark(),
        );
    }

    #[inline]
    fn bump_ascii_digits(&mut self) {
        while matches!(self.peek(), Some('0'..='9')) {
            self.bump();
        }
    }

    fn lex_number(&mut self) {
        let start = self.mark();
        self.bump_ascii_digits();

        let kind = if self.peek() == Some('.') {
            let next = self.peek_n(1);

            if matches!(next, Some('0'..='9'))
                || (next != Some('.') && !matches!(next, Some('A'..='Z' | 'a'..='z' | '_')))
            {
                self.bump();
                self.bump_ascii_digits();
                TokenKind::Float
            } else {
                TokenKind::Integer
            }
        } else {
            TokenKind::Integer
        };
        self.push_token(kind, start, self.mark());
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
            "assert" => TokenKind::Keyword(Keyword::Assert),
            "as" => TokenKind::Keyword(Keyword::As),
            "break" => TokenKind::Keyword(Keyword::Break),
            "continue" => TokenKind::Keyword(Keyword::Continue),
            "case" => TokenKind::Keyword(Keyword::Case),
            "class" => TokenKind::Keyword(Keyword::Class),
            "defer" => TokenKind::Keyword(Keyword::Defer),
            "def" => TokenKind::Keyword(Keyword::Def),
            "else" => TokenKind::Keyword(Keyword::Else),
            "enum" => TokenKind::Keyword(Keyword::Enum),
            "expect" => TokenKind::Keyword(Keyword::Expect),
            "false" => TokenKind::Keyword(Keyword::False),
            "for" => TokenKind::Keyword(Keyword::For),
            "hidden" => TokenKind::Keyword(Keyword::Hidden),
            "if" => TokenKind::Keyword(Keyword::If),
            "impl" => TokenKind::Keyword(Keyword::Impl),
            "use" => TokenKind::Keyword(Keyword::Use),
            "interface" => TokenKind::Keyword(Keyword::Interface),
            "is" => TokenKind::Keyword(Keyword::Is),
            "let" => TokenKind::Keyword(Keyword::Let),
            "match" => TokenKind::Keyword(Keyword::Match),
            "module" => TokenKind::Keyword(Keyword::Module),
            "partial" => TokenKind::Keyword(Keyword::Partial),
            "public" => TokenKind::Keyword(Keyword::Public),
            "return" => TokenKind::Keyword(Keyword::Return),
            "shape" => TokenKind::Keyword(Keyword::Shape),
            "single" => TokenKind::Keyword(Keyword::Single),
            "true" => TokenKind::Keyword(Keyword::True),
            "try" => TokenKind::Keyword(Keyword::Try),
            "var" => TokenKind::Keyword(Keyword::Var),
            "vararg" => TokenKind::Keyword(Keyword::Vararg),
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
            '.' if self.peek() == Some('-') && self.peek_n(1) == Some('>') => {
                self.bump();
                self.bump();
                Some(TokenKind::DotArrow)
            }
            '.' if self.take('.') && self.take('.') => Some(TokenKind::Ellipsis),
            '.' => Some(TokenKind::Dot),
            ':' if self.take('+') => Some(TokenKind::ColonPlus),
            ':' if self.take('-') => return self.unsupported_operator(start, ":-"),
            ':' if self.take(':') => return self.unsupported_operator(start, "::"),
            ':' if self.take('<') => Some(TokenKind::ColonLess),
            ':' if self.take('=') => Some(TokenKind::ColonAssign),
            ':' => Some(TokenKind::Colon),
            '@' => Some(TokenKind::At),
            '+' if self.take('+') => return self.unsupported_operator(start, "++"),
            '+' if self.take('=') => Some(TokenKind::PlusEq),
            '+' => Some(TokenKind::Plus),
            '-' if self.take('-') => return self.unsupported_operator(start, "--"),
            '-' if self.take('>') => Some(TokenKind::Arrow),
            '-' if self.take('=') => Some(TokenKind::MinusEq),
            '-' => Some(TokenKind::Minus),
            '*' if self.take('=') => Some(TokenKind::StarEq),
            '*' => Some(TokenKind::Star),
            '/' if self.take('=') => Some(TokenKind::SlashEq),
            '/' => Some(TokenKind::Slash),
            '%' => Some(TokenKind::Percent),
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
            '|' => return self.unsupported_operator(start, "|"),
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
    fn reports_unterminated_string() {
        let result = lex(&source("value = \"oops\n"));
        assert_eq!(result.diagnostics.len(), 1);
        assert_eq!(result.diagnostics[0].code, "unterminated_string");
    }

    #[test]
    fn rejects_reserved_symbolic_collection_operators() {
        let result = lex(&source("c :- d\ne :: f\ng ++ h\ni -- j\na & b\nc | d\n"));
        let messages = result
            .diagnostics
            .iter()
            .map(|diag| (diag.code, diag.message.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(messages.len(), 6);
        for operator in [":-", "::", "++", "--", "&", "|"] {
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
            "annotation Route { path Str }\nassert true\nuse model/things/{A as Alias}\nif true { 1 } else { 0 }\nitems = for value <- values yield value + 1\nupdated = value :< { amount: 1 }\nmerged = left :+ right\nlifted = value.->name()\nspread [Str] vararg = \"\"\"\nhello\n\"\"\"\nrawText = raw\"$name\\n\"\npi = 1.25\n",
        ));
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
        let kinds: Vec<TokenKind> = result.tokens.iter().map(|token| token.kind).collect();
        assert!(kinds.contains(&TokenKind::Keyword(Keyword::Annotation)));
        assert!(kinds.contains(&TokenKind::Keyword(Keyword::Assert)));
        assert!(kinds.contains(&TokenKind::Keyword(Keyword::As)));
        assert!(kinds.contains(&TokenKind::Keyword(Keyword::Yield)));
        assert!(kinds.contains(&TokenKind::Keyword(Keyword::Vararg)));
        assert!(kinds.contains(&TokenKind::ColonPlus));
        assert!(kinds.contains(&TokenKind::ColonLess));
        assert!(kinds.contains(&TokenKind::DotArrow));
        assert!(kinds.contains(&TokenKind::Float));
        assert!(
            result
                .tokens
                .iter()
                .any(|token| token.kind == TokenKind::String && token.lexeme == "raw\"$name\\n\"")
        );
    }

    #[test]
    fn lexes_lifted_access_operator_as_atomic_token() {
        let exact = lex(&source("user.->name()"));
        assert!(exact.diagnostics.is_empty(), "{:#?}", exact.diagnostics);
        assert!(
            exact
                .tokens
                .iter()
                .any(|token| token.kind == TokenKind::DotArrow && token.lexeme == ".->"),
            "{:#?}",
            exact.tokens
        );

        let spaced = lex(&source("user. ->name()"));
        assert!(spaced.diagnostics.is_empty(), "{:#?}", spaced.diagnostics);
        let kinds: Vec<TokenKind> = spaced.tokens.iter().map(|token| token.kind).collect();
        assert!(
            kinds
                .windows(2)
                .any(|pair| pair[0] == TokenKind::Dot && pair[1] == TokenKind::Arrow)
        );
        assert!(!kinds.contains(&TokenKind::DotArrow));
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
