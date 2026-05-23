pub mod ast;
pub mod diagnostic;
pub mod interpreter;
pub mod ir;
pub mod lexer;
pub mod lower;
pub mod parser;
pub mod resolver;
pub mod source;
pub mod typecheck;

pub use diagnostic::{Diagnostic, Severity};
pub use lexer::{Keyword, LexResult, Token, TokenKind, lex};
pub use parser::{ParseResult, parse_program};
pub use resolver::{CheckResult, LocatedDiagnostic, ResolveResult, resolve_path, resolve_program};
pub use source::{LineColumn, SourceFile, Span};
pub use typecheck::{CheckResult as TypeCheckResult, PathCheckResult, check_path, check_program};
