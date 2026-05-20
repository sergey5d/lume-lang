use crate::{ast, diagnostic::Diagnostic, lexer::Token};

pub struct ParseResult {
    pub program: Option<ast::Program>,
    pub diagnostics: Vec<Diagnostic>,
}

pub fn parse_program(_tokens: &[Token]) -> ParseResult {
    ParseResult {
        program: None,
        diagnostics: vec![Diagnostic::todo(
            "parser",
            "parser construction has not started yet; lexer is the first implemented frontend stage",
        )],
    }
}
