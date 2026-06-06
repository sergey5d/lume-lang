use super::*;
use crate::{lexer::lex, source::SourceFile};
use std::{
    fs,
    path::{Path, PathBuf},
};

fn parse(src: &str) -> ParseResult {
    let file = SourceFile::new("test.lum", src);
    let lexed = lex(&file);
    assert!(
        lexed.diagnostics.is_empty(),
        "lexer diagnostics: {:#?}",
        lexed.diagnostics
    );
    parse_program(&lexed.tokens)
}

fn parse_expr_only(src: &str) -> Expr {
    let file = SourceFile::new("test.lum", src);
    let lexed = lex(&file);
    assert!(
        lexed.diagnostics.is_empty(),
        "lexer diagnostics: {:#?}",
        lexed.diagnostics
    );
    let mut parser = Parser::new(&lexed.tokens);
    let expr = parser.parse_expr().expect("expression");
    parser.skip_newlines();
    assert!(parser.at(TokenKind::Eof), "unexpected trailing tokens");
    assert!(
        parser.diagnostics.is_empty(),
        "parser diagnostics: {:#?}",
        parser.diagnostics
    );
    expr
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .expect("workspace root")
}

fn collect_lum_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = fs::read_dir(dir).expect("read dir");
    for entry in entries {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|name| name == "failures") {
                continue;
            }
            collect_lum_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "lum") {
            out.push(path);
        }
    }
}

#[test]
fn parses_function_with_range_loop() {
    let result = parse(
        r#"
def run(limit Int) Int {
    var total Int = 0
    for i <- Range(0, limit) {
        total += i
    }
    return total
}
"#,
    );
    assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    let program = result.program.expect("program");
    assert_eq!(program.items.len(), 1);
    match &program.items[0] {
        Item::Function(function) => {
            assert_eq!(function.name, "run");
            match &function.body {
                CallableBody::Block(block) => {
                    assert_eq!(block.statements.len(), 3);
                }
                other => panic!("expected block body, got {other:#?}"),
            }
        }
        other => panic!("expected function, got {other:#?}"),
    }
}

#[test]
fn parses_class_and_impl() {
    let result = parse(
        r#"
class Counter {
    hidden var count Int
}

impl Counter {
    def init(count Int) {
        this.count = count
    }

    def bump(delta Int) Int = this.count + delta
}
"#,
    );
    assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    let program = result.program.expect("program");
    assert_eq!(program.items.len(), 2);
    assert!(matches!(program.items[0], Item::Type(_)));
    assert!(matches!(program.items[1], Item::Impl(_)));
}

#[test]
fn parses_record_literal_forms() {
    match parse_expr_only("record { name, age }") {
        Expr::RecordLiteral { fields, values, .. } => {
            assert!(fields.is_empty());
            assert_eq!(values.len(), 2);
        }
        other => panic!("expected record literal, got {other:#?}"),
    }

    match parse_expr_only(r#"record { 1, "x" }"#) {
        Expr::RecordLiteral { fields, values, .. } => {
            assert!(fields.is_empty());
            assert_eq!(values.len(), 2);
        }
        other => panic!("expected record literal, got {other:#?}"),
    }

    match parse_expr_only("Person { name, age }") {
        Expr::Call { args, .. } => {
            assert_eq!(args.len(), 1);
            match &args[0].value {
                Expr::RecordLiteral { fields, values, .. } => {
                    assert!(fields.is_empty());
                    assert_eq!(values.len(), 2);
                }
                other => panic!("expected record literal call arg, got {other:#?}"),
            }
        }
        other => panic!("expected call, got {other:#?}"),
    }

    let file = SourceFile::new("test.lum", r#"def run() Unit = record(1, "x")"#);
    let lexed = lex(&file);
    assert!(
        lexed.diagnostics.is_empty(),
        "lexer diagnostics: {:#?}",
        lexed.diagnostics
    );
    let result = parse_program(&lexed.tokens);
    assert!(
        result
            .diagnostics
            .iter()
            .any(|diag| diag.message.contains("'record(...)' is not supported")),
        "expected record(...) rejection, got diagnostics: {:#?}",
        result.diagnostics
    );
}

#[test]
fn parses_single_expression_function_body() {
    let result = parse("def zero() Int = 0\n");
    assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    let program = result.program.expect("program");
    match &program.items[0] {
        Item::Function(function) => match &function.body {
            CallableBody::Expr(Expr::Integer { raw, .. }) => assert_eq!(raw, "0"),
            other => panic!("expected integer expr body, got {other:#?}"),
        },
        other => panic!("expected function, got {other:#?}"),
    }
}

#[test]
fn parses_if_expression_and_calls() {
    let result = parse(
        r#"
def run(flag Bool) Int {
    value Int = if flag {
        foo(1)
    } else {
        bar(2)
    }
    return value
}
"#,
    );
    assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    let program = result.program.expect("program");
    match &program.items[0] {
        Item::Function(function) => match &function.body {
            CallableBody::Block(block) => match &block.statements[0] {
                Stmt::Binding(binding) => {
                    assert_eq!(binding.bindings[0].name, "value");
                }
                other => panic!("expected binding, got {other:#?}"),
            },
            other => panic!("expected block body, got {other:#?}"),
        },
        other => panic!("expected function, got {other:#?}"),
    }
}

#[test]
fn parses_lambda_expression() {
    let result = parse("def make() Unit = values.map((x, y) -> x + y)\n");
    assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
}

#[test]
fn parses_single_param_typed_lambda_without_parens() {
    let result = parse(
        r#"
def main() Unit {
    value = item Int -> item + 1
}
"#,
    );
    assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
}

#[test]
fn parses_string_interpolation_as_binary_concat() {
    let result = parse(
        r#"
def run(name Str, count Int) Str {
    return "hello $name ${count + 1} \$done"
}
"#,
    );
    assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    let program = result.program.expect("program");
    match &program.items[0] {
        Item::Function(function) => match &function.body {
            CallableBody::Block(block) => match &block.statements[0] {
                Stmt::Return(ret) => {
                    assert!(matches!(ret.value, Some(Expr::Binary { .. })));
                }
                other => panic!("expected return statement, got {other:#?}"),
            },
            other => panic!("expected block body, got {other:#?}"),
        },
        other => panic!("expected function, got {other:#?}"),
    }
}

#[test]
fn keeps_multiline_string_as_literal() {
    let result = parse(
        r#"
def run() Str {
    return """
hello
$name
\n
"""
}
"#,
    );
    assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    let program = result.program.expect("program");
    match &program.items[0] {
        Item::Function(function) => match &function.body {
            CallableBody::Block(block) => match &block.statements[0] {
                Stmt::Return(ret) => {
                    assert!(matches!(ret.value, Some(Expr::String { .. })));
                }
                other => panic!("expected return statement, got {other:#?}"),
            },
            other => panic!("expected block body, got {other:#?}"),
        },
        other => panic!("expected function, got {other:#?}"),
    }
}

#[test]
fn parses_repo_sources_except_skipped_and_failures() {
    let root = workspace_root();
    let mut files = Vec::new();
    collect_lum_files(&root.join("stdlib"), &mut files);
    collect_lum_files(&root.join("examples"), &mut files);
    files.sort();

    let mut failures = Vec::new();
    for path in files {
        let text = fs::read_to_string(&path).expect("source text");
        if text
            .lines()
            .next()
            .is_some_and(|line| line.trim() == "# SKIP")
        {
            continue;
        }
        let file = SourceFile::new(path.display().to_string(), text);
        let lexed = lex(&file);
        if !lexed.diagnostics.is_empty() {
            failures.push(format!(
                "lex {}: {:#?}",
                path.strip_prefix(&root).unwrap_or(&path).display(),
                lexed.diagnostics
            ));
            continue;
        }
        let parsed = parse_program(&lexed.tokens);
        if !parsed.diagnostics.is_empty() {
            failures.push(format!(
                "parse {}: {:#?}",
                path.strip_prefix(&root).unwrap_or(&path).display(),
                parsed.diagnostics
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "repo parse failures:\n{}",
        failures.join("\n\n")
    );
}
