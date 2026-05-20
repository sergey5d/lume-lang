use crate::{
    diagnostic::Diagnostic,
    source::{LineColumn, SourceFile, Span},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Keyword {
    Break,
    Case,
    Class,
    Def,
    Else,
    Enum,
    False,
    For,
    Hidden,
    If,
    Impl,
    Import,
    Interface,
    Match,
    Object,
    Package,
    Partial,
    Public,
    Record,
    Return,
    True,
    Unwrap,
    Var,
    While,
    With,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    Keyword(Keyword),
    Identifier,
    Integer,
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
    Question,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Eq,
    Bang,
    Less,
    Greater,
    Ampersand,
    Pipe,
    Arrow,
    FatArrow,
    LeftArrow,
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

    fn lex_number(&mut self) {
        let start = self.mark();
        while matches!(self.peek(), Some('0'..='9')) {
            self.bump();
        }
        self.push_token(TokenKind::Integer, start, self.mark());
    }

    fn lex_identifier_or_keyword(&mut self) {
        let start = self.mark();
        while matches!(self.peek(), Some('A'..='Z' | 'a'..='z' | '0'..='9' | '_')) {
            self.bump();
        }
        let lexeme = self.slice(start.byte, self.byte_index);
        let kind = match lexeme.as_str() {
            "break" => TokenKind::Keyword(Keyword::Break),
            "case" => TokenKind::Keyword(Keyword::Case),
            "class" => TokenKind::Keyword(Keyword::Class),
            "def" => TokenKind::Keyword(Keyword::Def),
            "else" => TokenKind::Keyword(Keyword::Else),
            "enum" => TokenKind::Keyword(Keyword::Enum),
            "false" => TokenKind::Keyword(Keyword::False),
            "for" => TokenKind::Keyword(Keyword::For),
            "hidden" => TokenKind::Keyword(Keyword::Hidden),
            "if" => TokenKind::Keyword(Keyword::If),
            "impl" => TokenKind::Keyword(Keyword::Impl),
            "import" => TokenKind::Keyword(Keyword::Import),
            "interface" => TokenKind::Keyword(Keyword::Interface),
            "match" => TokenKind::Keyword(Keyword::Match),
            "object" => TokenKind::Keyword(Keyword::Object),
            "package" => TokenKind::Keyword(Keyword::Package),
            "partial" => TokenKind::Keyword(Keyword::Partial),
            "public" => TokenKind::Keyword(Keyword::Public),
            "record" => TokenKind::Keyword(Keyword::Record),
            "return" => TokenKind::Keyword(Keyword::Return),
            "true" => TokenKind::Keyword(Keyword::True),
            "unwrap" => TokenKind::Keyword(Keyword::Unwrap),
            "var" => TokenKind::Keyword(Keyword::Var),
            "while" => TokenKind::Keyword(Keyword::While),
            "with" => TokenKind::Keyword(Keyword::With),
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
            '.' => Some(TokenKind::Dot),
            ':' if self.take('=') => Some(TokenKind::ColonAssign),
            ':' => Some(TokenKind::Colon),
            '@' => Some(TokenKind::At),
            '?' => Some(TokenKind::Question),
            '+' if self.take('=') => Some(TokenKind::PlusEq),
            '+' => Some(TokenKind::Plus),
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
            '&' => Some(TokenKind::Ampersand),
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

    fn bump(&mut self) -> Option<char> {
        let ch = self.peek()?;
        self.index += 1;
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
}
