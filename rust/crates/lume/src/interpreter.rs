use crate::{diagnostic::Diagnostic, ir};

pub struct RunResult {
    pub diagnostics: Vec<Diagnostic>,
}

pub fn run_program(_program: &ir::Program) -> RunResult {
    RunResult {
        diagnostics: vec![Diagnostic::todo(
            "interpreter",
            "the Rust interpreter should execute lowered IR, not the source AST",
        )],
    }
}
