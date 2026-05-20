use crate::{ast, diagnostic::Diagnostic, ir};

pub struct LowerResult {
    pub program: Option<ir::Program>,
    pub diagnostics: Vec<Diagnostic>,
}

pub fn lower_program(_program: &ast::Program) -> LowerResult {
    LowerResult {
        program: None,
        diagnostics: vec![Diagnostic::todo(
            "lower",
            "lowering will target a compact IR rather than directly interpreting source-shaped AST nodes",
        )],
    }
}
