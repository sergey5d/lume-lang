use crate::{
    ast::*,
    diagnostic::Diagnostic,
    lexer::{Keyword, Token, TokenKind},
    source::Span,
};

mod expr;
mod items;
mod pattern;
mod stmt;
mod strings;
mod support;
mod types;

#[cfg(test)]
mod tests;

pub struct ParseResult {
    pub program: Option<Program>,
    pub diagnostics: Vec<Diagnostic>,
}

pub fn parse_program(tokens: &[Token]) -> ParseResult {
    Parser::new(tokens).parse_program()
}

struct Parser<'a> {
    tokens: &'a [Token],
    index: usize,
    diagnostics: Vec<Diagnostic>,
    allow_trailing_block_call: bool,
}

#[derive(Clone, Copy)]
struct Checkpoint {
    index: usize,
    diagnostics_len: usize,
    allow_trailing_block_call: bool,
}

impl<'a> Parser<'a> {
    fn new(tokens: &'a [Token]) -> Self {
        Self {
            tokens,
            index: 0,
            diagnostics: Vec::new(),
            allow_trailing_block_call: true,
        }
    }

    fn parse_program(mut self) -> ParseResult {
        self.skip_newlines();
        let start_span = self.current_span();

        let module = if self.match_keyword(Keyword::Module) {
            self.parse_module_decl()
        } else {
            None
        };

        self.skip_newlines();
        let mut imports = Vec::new();
        while self.match_keyword(Keyword::Use) {
            if let Some(import) = self.parse_import_decl() {
                imports.push(import);
            }
            self.skip_newlines();
        }

        let mut items = Vec::new();
        while !self.at(TokenKind::Eof) {
            self.skip_newlines();
            if self.at(TokenKind::Eof) {
                break;
            }

            let before = self.index;
            if let Some(item) = self.parse_item() {
                items.push(item);
            } else {
                self.synchronize_item();
                if self.index == before && !self.at(TokenKind::Eof) {
                    self.advance();
                }
            }
            self.skip_newlines();
        }

        let end_span = if self.index > 0 {
            self.tokens[self.index.saturating_sub(1)].span
        } else {
            start_span
        };

        let program = Program {
            module,
            imports,
            items,
            span: Some(start_span.cover(end_span)),
        };

        ParseResult {
            program: Some(program),
            diagnostics: self.diagnostics,
        }
    }
}

impl CallableBody {
    fn span(&self) -> Span {
        match self {
            CallableBody::Block(block) => block.span,
            CallableBody::Expr(expr) => expr.span(),
        }
    }
}

impl LambdaBody {
    fn span(&self) -> Span {
        match self {
            LambdaBody::Expr(expr) => expr.span(),
            LambdaBody::Block(block) => block.span,
        }
    }
}
