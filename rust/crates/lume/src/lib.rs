pub mod ast;
pub mod backend;
pub mod core;
pub mod desugar;
pub mod diagnostic;
pub mod diagnostic_render;
pub mod interpreter;
pub mod ir;
pub mod java_backend;
pub mod lexer;
pub mod lower;
pub mod parser;
pub mod resolver;
pub mod runtime;
pub mod source;
pub mod typecheck;
pub(crate) mod typecheck_diagnostics;

pub use backend::{BackendBundle, BackendBundleResult, BackendDescriptors, build_backend_bundle};
pub use desugar::{desugar_block, desugar_callable_body, desugar_expr, desugar_function_decl};
pub use diagnostic::{Diagnostic, Severity};
pub use diagnostic_render::{render_diagnostic, render_path_diagnostic, render_path_diagnostics};
pub use interpreter::{
    PathRunResult, RunResult, run_path, run_program, run_program_entry, run_program_specs,
    test_path,
};
pub use java_backend::{JavaBackendOptions, JavaBackendResult, generate_java_path};
pub use lexer::{Keyword, LexResult, Token, TokenKind, lex};
pub use lower::{LowerResult, lower_program};
pub use parser::{ParseResult, parse_program};
pub use resolver::{CheckResult, LocatedDiagnostic, ResolveResult, resolve_path, resolve_program};
pub use source::{LineColumn, SourceFile, Span};
pub use typecheck::{CheckResult as TypeCheckResult, PathCheckResult, check_path, check_program};
