use crate::{ast, diagnostic::Diagnostic};

pub struct CheckResult {
    pub diagnostics: Vec<Diagnostic>,
}

pub fn check_program(_program: &ast::Program) -> CheckResult {
    CheckResult {
        diagnostics: vec![Diagnostic::todo(
            "typecheck",
            "type checking will sit on top of the Rust parser once AST construction is in place",
        )],
    }
}
