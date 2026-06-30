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

fn assert_lexes_unsupported_operator(src: &str, operator: &str) {
    let file = SourceFile::new("test.lum", src);
    let lexed = lex(&file);
    assert!(
        lexed
            .diagnostics
            .iter()
            .any(|diag| diag.code == "unsupported_operator" && diag.message.contains(operator)),
        "expected unsupported operator rejection for {operator}, got {:#?}",
        lexed.diagnostics
    );
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
fn parses_top_level_bindings() {
    let result = parse(
        r#"
seed Int = 1
var counter Int = 0
"#,
    );
    assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    let program = result.program.expect("program");
    assert_eq!(program.items.len(), 2);
    assert!(matches!(
        &program.items[0],
        Item::Statement(Stmt::Binding(_))
    ));
    assert!(matches!(
        &program.items[1],
        Item::Statement(Stmt::Binding(_))
    ));
}

#[test]
fn rejects_top_level_control_flow_statements() {
    let result = parse(
        r#"
if true {
    println("nope")
}
"#,
    );
    assert!(
        result
            .diagnostics
            .iter()
            .any(|diag| diag.code == "unexpected_top_level_statement"),
        "{:#?}",
        result.diagnostics
    );
}

#[test]
fn rejects_top_level_expression_statements() {
    let result = parse(
        r#"
println("nope")
"#,
    );
    assert!(
        result
            .diagnostics
            .iter()
            .any(|diag| diag.code == "unexpected_top_level_statement"),
        "{:#?}",
        result.diagnostics
    );
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
fn parses_for_tuple_destructuring_with_parentheses() {
    let result = parse(
        r#"
def run(rows List[(Int, Int)]) Unit {
    for (value, idx) <- rows {
        OS.println(value, idx)
    }
}
"#,
    );
    assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
}

#[test]
fn parses_for_class_destructuring_with_braces() {
    let result = parse(
        r#"
def run(rows List[Row]) Unit {
    for { value, label } <- rows {
        OS.println(value, label)
    }
}
"#,
    );
    assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    let program = result.program.expect("program");
    match &program.items[0] {
        Item::Function(function) => match &function.body {
            CallableBody::Block(block) => match &block.statements[0] {
                Stmt::For(stmt) => {
                    assert_eq!(stmt.bindings.len(), 1);
                    let binding = &stmt.bindings[0];
                    assert_eq!(binding.destructure, Some(DestructureKind::Record));
                    assert_eq!(binding.bindings.len(), 2);
                    assert_eq!(binding.bindings[0].name, "value");
                    assert_eq!(binding.bindings[0].field_name.as_deref(), Some("value"));
                    assert_eq!(binding.bindings[1].name, "label");
                    assert_eq!(binding.bindings[1].field_name.as_deref(), Some("label"));
                }
                other => panic!("expected for stmt, got {other:#?}"),
            },
            other => panic!("expected block body, got {other:#?}"),
        },
        other => panic!("expected function, got {other:#?}"),
    }
}

#[test]
fn parses_for_named_class_destructuring_with_braces() {
    let result = parse(
        r#"
def run(users List[User]) Unit {
    for { name, location Str as loc, country as skipped } <- users {
        OS.println(name, loc)
    }
}
"#,
    );
    assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    let program = result.program.expect("program");
    match &program.items[0] {
        Item::Function(function) => match &function.body {
            CallableBody::Block(block) => match &block.statements[0] {
                Stmt::For(stmt) => {
                    assert_eq!(stmt.bindings.len(), 1);
                    let binding = &stmt.bindings[0];
                    assert_eq!(binding.destructure, Some(DestructureKind::Record));
                    assert_eq!(binding.bindings.len(), 3);
                    assert_eq!(binding.bindings[0].name, "name");
                    assert_eq!(binding.bindings[0].field_name.as_deref(), Some("name"));
                    assert_eq!(binding.bindings[1].name, "loc");
                    assert_eq!(binding.bindings[1].field_name.as_deref(), Some("location"));
                    assert!(binding.bindings[1].ty.is_some());
                    assert_eq!(binding.bindings[2].name, "skipped");
                    assert_eq!(binding.bindings[2].field_name.as_deref(), Some("country"));
                }
                other => panic!("expected for stmt, got {other:#?}"),
            },
            other => panic!("expected block body, got {other:#?}"),
        },
        other => panic!("expected function, got {other:#?}"),
    }
}

#[test]
fn parses_for_constructor_pattern() {
    let result = parse(
        r#"
def run(values List[Option[Int]]) Unit {
    for Some(value) <- values {
        OS.println(value)
    }
}
"#,
    );
    assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    let program = result.program.expect("program");
    match &program.items[0] {
        Item::Function(function) => match &function.body {
            CallableBody::Block(block) => match &block.statements[0] {
                Stmt::For(stmt) => {
                    assert_eq!(stmt.bindings.len(), 1);
                    let binding = &stmt.bindings[0];
                    assert!(binding.bindings.is_empty());
                    assert_eq!(binding.destructure, None);
                    match binding.pattern.as_ref() {
                        Some(Pattern::Constructor { path, args, .. }) => {
                            assert_eq!(path, &vec!["Some".to_string()]);
                            assert_eq!(args.len(), 1);
                        }
                        other => panic!("expected constructor pattern, got {other:#?}"),
                    }
                }
                other => panic!("expected for stmt, got {other:#?}"),
            },
            other => panic!("expected block body, got {other:#?}"),
        },
        other => panic!("expected function, got {other:#?}"),
    }
}

#[test]
fn rejects_for_tuple_destructuring_without_parentheses() {
    let result = parse(
        r#"
def run(rows List[(Int, Int)]) Unit {
    for value, idx <- rows {
        OS.println(value, idx)
    }
}
"#,
    );
    assert!(
        result.diagnostics.iter().any(|diag| diag
            .message
            .contains("tuple destructuring in 'for' requires parentheses")),
        "{:#?}",
        result.diagnostics
    );
}

#[test]
fn parses_class_and_impl() {
    let result = parse(
        r#"
class Counter {
    hidden var count Int
}

impl Counter {
    new {
        count Int
    } {
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
fn rejects_class_field_after_method() {
    let result = parse(
        r#"
class User {
    def label() Str = "user"
    name Str
}
"#,
    );
    assert!(
        result.diagnostics.iter().any(|diag| {
            diag.code == "invalid_member_order"
                && diag
                    .message
                    .contains("storage fields must appear before methods")
        }),
        "{:#?}",
        result.diagnostics
    );
}

#[test]
fn rejects_enum_case_after_method() {
    let result = parse(
        r#"
enum MaybeInt {
    def isSet() Bool = true
    case Some { value Int }
}
"#,
    );
    assert!(
        result.diagnostics.iter().any(|diag| {
            diag.code == "invalid_member_order"
                && diag
                    .message
                    .contains("enum cases must appear before methods")
        }),
        "{:#?}",
        result.diagnostics
    );
}

#[test]
fn rejects_constructor_after_impl_method() {
    let result = parse(
        r#"
class User {
    name Str
}

impl User {
    def label() Str = this.name

    new {
        name Str
    } {
        this.name = name
    }
}
"#,
    );
    assert!(
        result.diagnostics.iter().any(|diag| {
            diag.code == "invalid_member_order"
                && diag
                    .message
                    .contains("constructors must appear before methods")
        }),
        "{:#?}",
        result.diagnostics
    );
}

#[test]
fn rejects_class_field_after_inline_constructor() {
    let result = parse(
        r#"
class User {
    new {
        name Str
    } {
        this.name = name
    }

    name Str
}
"#,
    );
    assert!(
        result.diagnostics.iter().any(|diag| {
            diag.code == "invalid_member_order"
                && diag
                    .message
                    .contains("storage fields must appear before constructors")
        }),
        "{:#?}",
        result.diagnostics
    );
    assert!(
        result
            .diagnostics
            .iter()
            .any(|diag| diag.code == "unexpected_constructor_decl"),
        "{:#?}",
        result.diagnostics
    );
}

#[test]
fn rejects_inline_constructor_after_class_method() {
    let result = parse(
        r#"
class User {
    def label() Str = "user"

    new {
        name Str
    } {
        this.name = name
    }
}
"#,
    );
    assert!(
        result.diagnostics.iter().any(|diag| {
            diag.code == "invalid_member_order"
                && diag
                    .message
                    .contains("constructors must appear before methods")
        }),
        "{:#?}",
        result.diagnostics
    );
    assert!(
        result
            .diagnostics
            .iter()
            .any(|diag| diag.code == "unexpected_constructor_decl"),
        "{:#?}",
        result.diagnostics
    );
}

#[test]
fn parses_expression_bodied_constructor() {
    let result = parse(
        r#"
class User {
    name Str
}

impl User {
    new {
        name Str
    } = new { name }
}
"#,
    );
    assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    let program = result.program.expect("program");
    let Item::Impl(block) = &program.items[1] else {
        panic!("expected impl block");
    };
    assert!(matches!(
        block.methods[0].body,
        Some(CallableBody::Expr(Expr::Call { .. }))
    ));
}

#[test]
fn parses_variadic_constructor_parameter() {
    let result = parse(
        r#"
class Path {
    segments [Str]
}

impl Path {
    new {
        segments [Str] vararg
    } {
        this.segments = segments
    }
}
"#,
    );
    assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    let program = result.program.expect("program");
    let Item::Impl(block) = &program.items[1] else {
        panic!("expected impl block");
    };
    assert_eq!(block.methods[0].params.len(), 1);
    assert!(block.methods[0].params[0].variadic);
}

#[test]
fn rejects_ellipsis_constructor_parameter() {
    let result = parse(
        r#"
class Path {
    segments [Str]
}

impl Path {
    new {
        segments Str...
    } {
        this.segments = segments
    }
}
"#,
    );
    assert!(!result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
}

#[test]
fn rejects_removed_symbolic_operator_methods_at_lexer() {
    for operator in ["++", "--", ":-", "::"] {
        let source = format!(
            r#"
class Vec {{}}

impl Vec {{
    def {operator}(other Vec) Vec = this
}}
"#
        );
        assert_lexes_unsupported_operator(&source, operator);
    }
}

#[test]
fn rejects_removed_symbolic_infix_operators_at_lexer() {
    for operator in ["++", "--", ":-", "::"] {
        let source = format!(
            r#"
def main() Unit {{
    left = 1
    right = 2
    value = left {operator} right
}}
"#
        );
        assert_lexes_unsupported_operator(&source, operator);
    }
}

#[test]
fn parses_empty_method_body() {
    let result = parse(
        r#"
class Counter {}

impl Counter {
    def reset() {
    }
}
"#,
    );
    assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
}

#[test]
fn parses_empty_function_body() {
    let result = parse(
        r#"
def main() {
}
"#,
    );
    assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
}

#[test]
fn parses_single_and_impl_single() {
    let result = parse(
        r#"
single Counter {
    hidden value = 0
}

impl single Counter {
    def next() Int = this.value + 1
}
"#,
    );
    assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    let program = result.program.expect("program");
    assert_eq!(program.items.len(), 2);
    match &program.items[0] {
        Item::Type(decl) => assert_eq!(decl.kind, TypeKind::Single),
        other => panic!("expected singleton type, got {other:#?}"),
    }
    match &program.items[1] {
        Item::Impl(block) => assert_eq!(block.target_kind, ImplTargetKind::Single),
        other => panic!("expected impl block, got {other:#?}"),
    }
}

#[test]
fn parses_annotation_decl_with_default_field_values() {
    let result = parse(
        r#"
annotation Route {
    path Str
    method Str = "GET"
}

@Route { path: "/health" }
def health() Str = "ok"
"#,
    );
    assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    let program = result.program.expect("program");
    match &program.items[0] {
        Item::Type(decl) => {
            assert_eq!(decl.kind, TypeKind::Annotation);
            assert_eq!(decl.name, "Route");
            assert_eq!(decl.members.len(), 2);
        }
        other => panic!("expected annotation type, got {other:#?}"),
    }
}

#[test]
fn parses_methods_in_single_body() {
    let result = parse(
        r#"
single Counter {
    hidden value = 0

    def next() Int = 1
}
"#,
    );
    assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    let program = result.program.expect("program");
    match &program.items[0] {
        Item::Type(decl) => {
            assert_eq!(decl.kind, TypeKind::Single);
            assert_eq!(decl.members.len(), 2);
        }
        other => panic!("expected singleton type, got {other:#?}"),
    }
}

#[test]
fn rejects_question_field_placeholder() {
    let file = SourceFile::new(
        "test.lum",
        r#"
class Box {
    hidden label Str = ?
}
"#,
    );
    let lexed = lex(&file);
    assert!(
        !lexed.diagnostics.is_empty(),
        "expected lexer rejection for '?', got {:#?}",
        lexed.diagnostics
    );
}

#[test]
fn parses_shape_literal_forms() {
    match parse_expr_only(r#"{ name: "Ana", age: 10 }"#) {
        Expr::RecordLiteral { fields, values, .. } => {
            assert!(values.is_empty());
            assert_eq!(fields.len(), 2);
            assert_eq!(fields[0].name.as_deref(), Some("name"));
            assert_eq!(fields[1].name.as_deref(), Some("age"));
            assert!(fields[0].ty.is_none());
        }
        other => panic!("expected named shape literal, got {other:#?}"),
    }

    match parse_expr_only(r#"{ name Str: "Ana", age Int: 10 }"#) {
        Expr::RecordLiteral { fields, values, .. } => {
            assert!(values.is_empty());
            assert_eq!(fields.len(), 2);
            assert_eq!(fields[0].name.as_deref(), Some("name"));
            assert!(matches!(
                fields[0].ty,
                Some(TypeRef::Named { ref name, .. }) if name == "Str"
            ));
            assert_eq!(fields[1].name.as_deref(), Some("age"));
            assert!(matches!(
                fields[1].ty,
                Some(TypeRef::Named { ref name, .. }) if name == "Int"
            ));
        }
        other => panic!("expected typed shape literal, got {other:#?}"),
    }

    match parse_expr_only("(name, age)") {
        Expr::TupleLiteral { items, .. } => {
            assert_eq!(items.len(), 2);
        }
        other => panic!("expected tuple literal, got {other:#?}"),
    }

    match parse_expr_only(r#"(1, "x")"#) {
        Expr::TupleLiteral { items, .. } => {
            assert_eq!(items.len(), 2);
        }
        other => panic!("expected tuple literal, got {other:#?}"),
    }

    match parse_expr_only(r#""a": 1"#) {
        Expr::Binary {
            op: BinaryOp::Colon,
            ..
        } => {}
        other => panic!("expected pair expression, got {other:#?}"),
    }

    match parse_expr_only(r#"{ entry: ("a": 1) }"#) {
        Expr::RecordLiteral { fields, values, .. } => {
            assert!(values.is_empty());
            assert_eq!(fields.len(), 1);
        }
        other => panic!("expected shape literal, got {other:#?}"),
    }

    match parse_expr_only("Person { name: name, age: age }") {
        Expr::Call {
            args,
            uses_brace_syntax,
            ..
        } => {
            assert_eq!(args.len(), 1);
            assert!(uses_brace_syntax);
            match &args[0].value {
                Expr::RecordLiteral { fields, values, .. } => {
                    assert_eq!(fields.len(), 2);
                    assert!(values.is_empty());
                }
                other => panic!("expected shape literal call arg, got {other:#?}"),
            }
        }
        other => panic!("expected call, got {other:#?}"),
    }

    match parse_expr_only("Settings {}") {
        Expr::Call {
            args,
            uses_brace_syntax,
            ..
        } => {
            assert_eq!(args.len(), 1);
            assert!(uses_brace_syntax);
            match &args[0].value {
                Expr::RecordLiteral { fields, values, .. } => {
                    assert!(fields.is_empty());
                    assert!(values.is_empty());
                }
                other => panic!("expected empty shape literal call arg, got {other:#?}"),
            }
        }
        other => panic!("expected call, got {other:#?}"),
    }

    match parse_expr_only("Box(5)") {
        Expr::Call {
            args,
            uses_brace_syntax,
            ..
        } => {
            assert_eq!(args.len(), 1);
            assert!(!uses_brace_syntax);
        }
        other => panic!("expected call, got {other:#?}"),
    }

    let file = SourceFile::new("test.lum", r#"def run() Unit = class(1, "x")"#);
    let lexed = lex(&file);
    assert!(
        lexed.diagnostics.is_empty(),
        "lexer diagnostics: {:#?}",
        lexed.diagnostics
    );
    let result = parse_program(&lexed.tokens);
    assert!(
        result.diagnostics.iter().any(|diag| diag
            .message
            .contains("anonymous shape literals use '{ ... }'")),
        "expected class(...) rejection, got diagnostics: {:#?}",
        result.diagnostics
    );
}

#[test]
fn rejects_chained_pair_expression_without_parentheses() {
    let result = parse(
        r#"
def main() Unit {
    value = "a": 1: true
}
"#,
    );
    assert!(
        result.diagnostics.iter().any(|diag| {
            diag.code == "invalid_pair_expression" && diag.message.contains("non-associative")
        }),
        "{:#?}",
        result.diagnostics
    );
}

#[test]
fn rejects_unparenthesized_pair_field_initializer() {
    let result = parse(
        r#"
def main() Unit {
    holder = Holder {
        entry: "a": 1
    }
}
"#,
    );
    assert!(
        result.diagnostics.iter().any(|diag| {
            diag.code == "invalid_pair_expression"
                && diag
                    .message
                    .contains("field initializers must be parenthesized")
        }),
        "{:#?}",
        result.diagnostics
    );
}

#[test]
fn rejects_equals_in_named_shape_literal_fields() {
    let result = parse(r#"def run() Unit = { name = "Ana", age = 10 }"#);
    assert!(
        !result.diagnostics.is_empty(),
        "expected parser diagnostics for '=' named construction fields"
    );
}

#[test]
fn parses_colon_shape_update_fields() {
    match parse_expr_only("value :< { amount: 42, label: value.label }") {
        Expr::RecordUpdate { updates, .. } => {
            assert_eq!(updates.len(), 2);
            assert_eq!(updates[0].name.as_deref(), Some("amount"));
            assert_eq!(updates[1].name.as_deref(), Some("label"));
        }
        other => panic!("expected shape update, got {other:#?}"),
    }
}

#[test]
fn parses_shape_merge_operator() {
    match parse_expr_only("left :+ right") {
        Expr::Binary {
            op: BinaryOp::RecordMerge,
            ..
        } => {}
        other => panic!("expected shape merge binary expr, got {other:#?}"),
    }
}

#[test]
fn rejects_equals_in_shape_update_fields() {
    let result = parse(r#"def run(value Amount) Unit = value :< { amount = 42 }"#);
    assert!(
        result.diagnostics.iter().any(|diag| {
            diag.code == "unexpected_token"
                && diag
                    .message
                    .contains("expected ':' after shape update field name")
        }),
        "{:#?}",
        result.diagnostics
    );
}

#[test]
fn parses_shape_literal_arguments_inside_parens() {
    let result = parse(
        r#"
class User {
    name Str
}

def describe(user { name Str }) Str = user.name

def run() Unit {
    _ = describe({
        name: "Ana"
    })
}
"#,
    );
    assert!(
        result.diagnostics.is_empty(),
        "expected parser to accept shape literal call args, got diagnostics: {:#?}",
        result.diagnostics
    );
}

#[test]
fn parses_single_expression_braces_as_block_not_shape() {
    match parse_expr_only("{ value }") {
        Expr::Block { body, .. } => {
            assert_eq!(body.statements.len(), 1);
        }
        other => panic!("expected block, got {other:#?}"),
    }
}

#[test]
fn parses_brace_destructuring_binding() {
    let result = parse(
        r#"
def run(box Box) Int {
    let { value Int, label Str } = box
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
                    assert_eq!(binding.destructure, Some(DestructureKind::Record));
                    assert_eq!(binding.bindings.len(), 2);
                    assert_eq!(binding.bindings[0].name, "value");
                    assert_eq!(binding.bindings[0].field_name.as_deref(), Some("value"));
                    assert_eq!(binding.bindings[1].name, "label");
                    assert_eq!(binding.bindings[1].field_name.as_deref(), Some("label"));
                }
                other => panic!("expected binding, got {other:#?}"),
            },
            other => panic!("expected block body, got {other:#?}"),
        },
        other => panic!("expected function, got {other:#?}"),
    }
}

#[test]
fn parses_named_brace_destructuring_binding() {
    let result = parse(
        r#"
def run(user User) Str {
    let { name, location Str as loc, country as skipped } = user
    return name + loc
}
"#,
    );
    assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    let program = result.program.expect("program");
    match &program.items[0] {
        Item::Function(function) => match &function.body {
            CallableBody::Block(block) => match &block.statements[0] {
                Stmt::Binding(binding) => {
                    assert_eq!(binding.destructure, Some(DestructureKind::Record));
                    assert_eq!(binding.bindings.len(), 3);
                    assert_eq!(binding.bindings[0].name, "name");
                    assert_eq!(binding.bindings[0].field_name.as_deref(), Some("name"));
                    assert_eq!(binding.bindings[1].name, "loc");
                    assert_eq!(binding.bindings[1].field_name.as_deref(), Some("location"));
                    assert!(binding.bindings[1].ty.is_some());
                    assert_eq!(binding.bindings[2].name, "skipped");
                    assert_eq!(binding.bindings[2].field_name.as_deref(), Some("country"));
                }
                other => panic!("expected binding, got {other:#?}"),
            },
            other => panic!("expected block body, got {other:#?}"),
        },
        other => panic!("expected function, got {other:#?}"),
    }
}

#[test]
fn rejects_at_style_brace_destructuring() {
    let result = parse(
        r#"
def run(user User) Unit {
    let { @name } = user
}
"#,
    );
    assert!(
        result.diagnostics.iter().any(|diag| {
            diag.code == "unexpected_token"
                && diag
                    .message
                    .contains("brace destructuring uses 'field', 'field Type', 'field as local', or 'field Type as local'")
        }),
        "{:#?}",
        result.diagnostics
    );
}

#[test]
fn rejects_wildcard_brace_destructuring_entry() {
    let result = parse(
        r#"
def run(user User) Unit {
    let { _, location } = user
}
"#,
    );
    assert!(
        result.diagnostics.iter().any(|diag| {
            diag.code == "unexpected_token" && diag.message.contains("omit fields you do not need")
        }),
        "{:#?}",
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
fn parses_same_line_if_expression_function_body() {
    let result = parse(
        r#"
def pick(flag Bool) Int = if flag { 5 } else { 6 }
"#,
    );
    assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    let program = result.program.expect("program");
    match &program.items[0] {
        Item::Function(function) => match &function.body {
            CallableBody::Expr(Expr::If { .. }) => {}
            other => panic!("expected if expr body, got {other:#?}"),
        },
        other => panic!("expected function, got {other:#?}"),
    }
}

#[test]
fn parses_newline_after_equals_before_callable_expression_body() {
    let result = parse(
        r#"
def pick(flag Bool) Int =
    if flag { 5 } else { 6 }
"#,
    );
    assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    let program = result.program.expect("program");
    match &program.items[0] {
        Item::Function(function) => match &function.body {
            CallableBody::Expr(Expr::If { .. }) => {}
            other => panic!("expected if expr body, got {other:#?}"),
        },
        other => panic!("expected function, got {other:#?}"),
    }
}

#[test]
fn parses_newline_after_equals_before_binding_value() {
    let result = parse(
        r#"
def run() Unit {
    value =
        5
}
"#,
    );
    assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
}

#[test]
fn rejects_equals_before_block_callable_body() {
    let result = parse(
        r#"
def run() Unit = {
    OS.println("old block form")
}
"#,
    );
    assert!(
        result.diagnostics.iter().any(|diag| {
            diag.code == "invalid_callable_body"
                && diag.message.contains("block callable bodies omit '='")
        }),
        "{:#?}",
        result.diagnostics
    );
}

#[test]
fn allows_equals_before_shape_expression_callable_body() {
    let result = parse(
        r#"
def user() { name Str, age Int } = { name: "Ada", age: 10 }
"#,
    );
    assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    let program = result.program.expect("program");
    match &program.items[0] {
        Item::Function(function) => match &function.body {
            CallableBody::Expr(Expr::RecordLiteral { fields, .. }) => {
                assert_eq!(fields.len(), 2);
            }
            other => panic!("expected shape literal expr body, got {other:#?}"),
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
fn parses_if_type_pattern_bindings() {
    let result = parse(
        r#"
class Worker {
}

def run(value Worker) Unit {
    if let item Worker = value {
        OS.println(item)
    }
    if let _ Worker = value {
        OS.println("matched")
    }
}
"#,
    );
    assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    let program = result.program.expect("program");
    let function = match &program.items[1] {
        Item::Function(function) => function,
        other => panic!("expected function, got {other:#?}"),
    };
    match &function.body {
        CallableBody::Block(block) => {
            match &block.statements[0] {
                Stmt::If(stmt) => match stmt.pattern.as_ref() {
                    Some(Pattern::Type { name, .. }) => assert_eq!(name.as_deref(), Some("item")),
                    other => panic!("expected first if-let type pattern, got {other:#?}"),
                },
                other => panic!("expected if statement, got {other:#?}"),
            }
            match &block.statements[1] {
                Stmt::If(stmt) => match stmt.pattern.as_ref() {
                    Some(Pattern::Type { name, .. }) => assert!(name.is_none()),
                    other => panic!("expected second if-let wildcard type pattern, got {other:#?}"),
                },
                other => panic!("expected if statement, got {other:#?}"),
            }
        }
        other => panic!("expected block body, got {other:#?}"),
    }
}

#[test]
fn parses_let_else_type_patterns() {
    let result = parse(
        r#"
class Worker {
}

def run(value Worker) Int {
    let item Worker = value else {
        return 1
    }
    let _ Worker = value else {
        return 2
    }
    return 0
}
"#,
    );
    assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    let program = result.program.expect("program");
    let function = match &program.items[1] {
        Item::Function(function) => function,
        other => panic!("expected function, got {other:#?}"),
    };
    match &function.body {
        CallableBody::Block(block) => {
            match &block.statements[0] {
                Stmt::LetElse(stmt) => match &stmt.pattern {
                    Pattern::Type { name, .. } => assert_eq!(name.as_deref(), Some("item")),
                    other => panic!("expected first let-else type pattern, got {other:#?}"),
                },
                other => panic!("expected let-else statement, got {other:#?}"),
            }
            match &block.statements[1] {
                Stmt::LetElse(stmt) => match &stmt.pattern {
                    Pattern::Type { name, .. } => assert!(name.is_none()),
                    other => {
                        panic!("expected second let-else wildcard type pattern, got {other:#?}")
                    }
                },
                other => panic!("expected let-else statement, got {other:#?}"),
            }
        }
        other => panic!("expected block body, got {other:#?}"),
    }
}

#[test]
fn parses_inline_let_else_return_body() {
    let result = parse(
        r#"
def run(value Option[Int]) Unit {
    let Some(item) = value else return ()
    OS.println(item)
}
"#,
    );
    assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    let program = result.program.expect("program");
    let function = match &program.items[0] {
        Item::Function(function) => function,
        other => panic!("expected function, got {other:#?}"),
    };
    match &function.body {
        CallableBody::Block(block) => match &block.statements[0] {
            Stmt::LetElse(stmt) => {
                assert_eq!(stmt.else_block.statements.len(), 1);
                match &stmt.else_block.statements[0] {
                    Stmt::Return(_) => {}
                    other => panic!("expected inline return body, got {other:#?}"),
                }
            }
            other => panic!("expected let-else statement, got {other:#?}"),
        },
        other => panic!("expected block body, got {other:#?}"),
    }
}

#[test]
fn parses_option_extract_let_else_shorthand() {
    let result = parse(
        r#"
def run(value Option[Int]) Unit {
    let item <- value else return ()
    OS.println(item)
}
"#,
    );
    assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    let program = result.program.expect("program");
    let function = match &program.items[0] {
        Item::Function(function) => function,
        other => panic!("expected function, got {other:#?}"),
    };
    match &function.body {
        CallableBody::Block(block) => match &block.statements[0] {
            Stmt::LetElse(stmt) => match &stmt.pattern {
                Pattern::Extract { inner, .. } => {
                    assert!(
                        matches!(inner.as_ref(), Pattern::Binding { name, .. } if name == "item")
                    );
                }
                other => panic!("expected extract shorthand pattern, got {other:#?}"),
            },
            other => panic!("expected let-else statement, got {other:#?}"),
        },
        other => panic!("expected block body, got {other:#?}"),
    }
}

#[test]
fn parses_plain_let_pattern_binding() {
    let result = parse(
        r#"
def run(value Option[Int]) Int {
    let Some(item) = value
    return item
}
"#,
    );
    assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    let program = result.program.expect("program");
    let function = match &program.items[0] {
        Item::Function(function) => function,
        other => panic!("expected function, got {other:#?}"),
    };
    match &function.body {
        CallableBody::Block(block) => match &block.statements[0] {
            Stmt::PatternBinding(stmt) => match &stmt.pattern {
                Pattern::Constructor { path, args, .. } => {
                    assert_eq!(stmt.kind, PatternBindingKind::Let);
                    assert_eq!(path, &vec!["Some".to_string()]);
                    assert_eq!(args.len(), 1);
                }
                other => panic!("expected pattern binding, got {other:#?}"),
            },
            other => panic!("expected pattern binding statement, got {other:#?}"),
        },
        other => panic!("expected block body, got {other:#?}"),
    }
}

#[test]
fn parses_plain_let_option_extract_shorthand_without_else() {
    let result = parse(
        r#"
def run(value Option[Int]) Int {
    let item <- value
    return item
}
"#,
    );
    assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    let program = result.program.expect("program");
    let function = match &program.items[0] {
        Item::Function(function) => function,
        other => panic!("expected function, got {other:#?}"),
    };
    match &function.body {
        CallableBody::Block(block) => match &block.statements[0] {
            Stmt::PatternBinding(stmt) => match &stmt.pattern {
                Pattern::Extract { inner, .. } => {
                    assert_eq!(stmt.kind, PatternBindingKind::Let);
                    assert!(
                        matches!(inner.as_ref(), Pattern::Binding { name, .. } if name == "item")
                    );
                }
                other => panic!("expected extract shorthand pattern, got {other:#?}"),
            },
            other => panic!("expected pattern binding statement, got {other:#?}"),
        },
        other => panic!("expected block body, got {other:#?}"),
    }
}

#[test]
fn parses_grouped_plain_let_option_extract_shorthand_without_else() {
    let result = parse(
        r#"
def run(value Option[Int]) Int {
    let {
        item <- value
    }
    return item
}
"#,
    );
    assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    let program = result.program.expect("program");
    let function = match &program.items[0] {
        Item::Function(function) => function,
        other => panic!("expected function, got {other:#?}"),
    };
    match &function.body {
        CallableBody::Block(block) => match &block.statements[0] {
            Stmt::PatternBinding(stmt) => {
                assert_eq!(stmt.kind, PatternBindingKind::Let);
                assert_eq!(stmt.clauses.len(), 1);
                assert!(matches!(stmt.clauses[0].pattern, Pattern::Extract { .. }));
            }
            other => panic!("expected pattern binding statement, got {other:#?}"),
        },
        other => panic!("expected block body, got {other:#?}"),
    }
}

#[test]
fn parses_expect_pattern_binding() {
    let result = parse(
        r#"
def run(value Option[Int]) Int {
    expect Some(item) = value
    return item
}
"#,
    );
    assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    let program = result.program.expect("program");
    let function = match &program.items[0] {
        Item::Function(function) => function,
        other => panic!("expected function, got {other:#?}"),
    };
    match &function.body {
        CallableBody::Block(block) => match &block.statements[0] {
            Stmt::PatternBinding(stmt) => {
                assert_eq!(stmt.kind, PatternBindingKind::Expect);
                match &stmt.pattern {
                    Pattern::Constructor { path, args, .. } => {
                        assert_eq!(path, &vec!["Some".to_string()]);
                        assert_eq!(args.len(), 1);
                    }
                    other => panic!("expected pattern binding, got {other:#?}"),
                }
            }
            other => panic!("expected pattern binding statement, got {other:#?}"),
        },
        other => panic!("expected block body, got {other:#?}"),
    }
}

#[test]
fn parses_expect_option_extract_shorthand() {
    let result = parse(
        r#"
def run(value Option[Int]) Int {
    expect item <- value
    return item
}
"#,
    );
    assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    let program = result.program.expect("program");
    let function = match &program.items[0] {
        Item::Function(function) => function,
        other => panic!("expected function, got {other:#?}"),
    };
    match &function.body {
        CallableBody::Block(block) => match &block.statements[0] {
            Stmt::PatternBinding(stmt) => {
                assert_eq!(stmt.kind, PatternBindingKind::Expect);
                match &stmt.pattern {
                    Pattern::Extract { inner, .. } => {
                        assert!(
                            matches!(inner.as_ref(), Pattern::Binding { name, .. } if name == "item")
                        );
                    }
                    other => panic!("expected extract shorthand pattern, got {other:#?}"),
                }
            }
            other => panic!("expected pattern binding statement, got {other:#?}"),
        },
        other => panic!("expected block body, got {other:#?}"),
    }
}

#[test]
fn parses_defer_call_and_block_statements() {
    let result = parse(
        r#"
def run() Unit {
    defer cleanup()
    defer {
        OS.println("later")
    }
}
"#,
    );
    assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    let program = result.program.expect("program");
    let function = match &program.items[0] {
        Item::Function(function) => function,
        other => panic!("expected function, got {other:#?}"),
    };
    match &function.body {
        CallableBody::Block(block) => {
            match &block.statements[0] {
                Stmt::Defer(stmt) => {
                    assert!(matches!(stmt.action, DeferAction::Call(Expr::Call { .. })));
                }
                other => panic!("expected defer statement, got {other:#?}"),
            }
            match &block.statements[1] {
                Stmt::Defer(stmt) => {
                    assert!(matches!(stmt.action, DeferAction::Block(_)));
                }
                other => panic!("expected defer statement, got {other:#?}"),
            }
        }
        other => panic!("expected block body, got {other:#?}"),
    }
}

#[test]
fn rejects_non_call_defer_target() {
    let result = parse(
        r#"
def run() Unit {
    value = 1
    defer value
}
"#,
    );
    assert!(
        result
            .diagnostics
            .iter()
            .any(|diag| diag.code == "invalid_defer_target"
                && diag.message.contains("call expression or block")),
        "{:#?}",
        result.diagnostics
    );
}

#[test]
fn parses_grouped_option_extract_shorthand() {
    let result = parse(
        r#"
def run(left Option[Int], right Option[Int]) Int {
    let {
        first <- left
        second <- right
    } else return 0
    return first + second
}
"#,
    );
    assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    let program = result.program.expect("program");
    let function = match &program.items[0] {
        Item::Function(function) => function,
        other => panic!("expected function, got {other:#?}"),
    };
    match &function.body {
        CallableBody::Block(block) => match &block.statements[0] {
            Stmt::LetElse(stmt) => {
                assert_eq!(stmt.clauses.len(), 2);
                for (clause, expected) in stmt.clauses.iter().zip(["first", "second"]) {
                    match &clause.pattern {
                        Pattern::Extract { inner, .. } => {
                            assert!(
                                matches!(inner.as_ref(), Pattern::Binding { name, .. } if name == expected)
                            );
                        }
                        other => panic!("expected extract shorthand clause, got {other:#?}"),
                    }
                }
            }
            other => panic!("expected let-else statement, got {other:#?}"),
        },
        other => panic!("expected block body, got {other:#?}"),
    }
}

#[test]
fn parses_if_let_option_extract_shorthand() {
    let result = parse(
        r#"
def run(value Option[Int]) Unit {
    if let item <- value {
        OS.println(item)
    }
}
"#,
    );
    assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    let program = result.program.expect("program");
    let function = match &program.items[0] {
        Item::Function(function) => function,
        other => panic!("expected function, got {other:#?}"),
    };
    match &function.body {
        CallableBody::Block(block) => match &block.statements[0] {
            Stmt::If(stmt) => match stmt.pattern.as_ref() {
                Some(Pattern::Extract { inner, .. }) => {
                    assert!(
                        matches!(inner.as_ref(), Pattern::Binding { name, .. } if name == "item")
                    );
                }
                other => panic!("expected extract shorthand if-let pattern, got {other:#?}"),
            },
            other => panic!("expected if statement, got {other:#?}"),
        },
        other => panic!("expected block body, got {other:#?}"),
    }
}

#[test]
fn parses_expect_condition_statement() {
    let result = parse(
        r#"
def run(split [Str]) Unit {
    expect split.size() == 3
}
"#,
    );
    assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    let program = result.program.expect("program");
    let function = match &program.items[0] {
        Item::Function(function) => function,
        other => panic!("expected function, got {other:#?}"),
    };
    match &function.body {
        CallableBody::Block(block) => match &block.statements[0] {
            Stmt::ExpectCondition(stmt) => match &stmt.condition {
                Expr::Binary {
                    op: BinaryOp::Eq, ..
                } => {}
                other => panic!("expected equality condition, got {other:#?}"),
            },
            other => panic!("expected expect condition statement, got {other:#?}"),
        },
        other => panic!("expected block body, got {other:#?}"),
    }
}

#[test]
fn parses_if_expression_without_else_as_unit_block_expr() {
    let result = parse(
        r#"
def run(flag Bool) Unit = match flag {
    case true => if flag { println("x") }
    case false => ()
}
"#,
    );
    assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    let program = result.program.expect("program");
    let function = match &program.items[0] {
        Item::Function(function) => function,
        other => panic!("expected function, got {other:#?}"),
    };
    match &function.body {
        CallableBody::Expr(Expr::Match { cases, .. }) => match &cases[0].body {
            MatchCaseBody::Expr(Expr::Block { body, .. }) => match &body.statements[0] {
                Stmt::If(stmt) => assert!(stmt.else_branch.is_none()),
                other => panic!("expected synthesized if statement, got {other:#?}"),
            },
            other => panic!("expected block-wrapped if expression, got {other:#?}"),
        },
        other => panic!("expected match expression body, got {other:#?}"),
    }
}

#[test]
fn rejects_omitted_match_case_body() {
    let result = parse(
        r#"
def run(flag Bool) Unit = match flag {
    case true =>
    case false => ()
}
"#,
    );
    assert!(
        result
            .diagnostics
            .iter()
            .any(|diag| diag.code == "expected_match_case_body"),
        "{:#?}",
        result.diagnostics
    );
}

#[test]
fn parses_single_statement_match_case_body_without_braces() {
    let result = parse(
        r#"
def run(flag Bool) Unit {
    var total Int = 0
    match flag {
        case true => total += 1
        case false => total += 2
    }
}
"#,
    );
    assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    let program = result.program.expect("program");
    let function = match &program.items[0] {
        Item::Function(function) => function,
        other => panic!("expected function, got {other:#?}"),
    };
    match &function.body {
        CallableBody::Block(block) => match &block.statements[1] {
            Stmt::Match(stmt) => match &stmt.cases[0].body {
                MatchCaseBody::Block(body) => match &body.statements[0] {
                    Stmt::Assignment(_) => {}
                    other => panic!("expected assignment statement, got {other:#?}"),
                },
                other => panic!("expected block-wrapped statement arm, got {other:#?}"),
            },
            other => panic!("expected match statement, got {other:#?}"),
        },
        other => panic!("expected block body, got {other:#?}"),
    }
}

#[test]
fn parses_lambda_expression() {
    let result = parse("def make() Unit = values.map((x, y) -> x + y)\n");
    assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
}

#[test]
fn parses_supported_lambda_parameter_forms() {
    let result = parse(
        r#"
def main() Unit {
    empty = () -> 0
    bare = x -> x
    one = (x) -> x
    pair = (x, y) -> x + y
    typed = (x Int) -> x + 1
    typedPair = (x Int, y Int) -> x + y
}
"#,
    );
    assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
}

#[test]
fn rejects_destructured_lambda_parameter_forms() {
    let tuple = parse(
        r#"
def main() Unit {
    mapper = let (x Int, y Int) -> x + y
}
"#,
    );
    assert!(
        tuple.diagnostics.iter().any(|diag| {
            diag.code == "invalid_lambda_params"
                && diag
                    .message
                    .contains("lambda parameters cannot use 'let' destructuring")
        }),
        "{:#?}",
        tuple.diagnostics
    );

    let shape = parse(
        r#"
def main() Unit {
    mapper = let { name, age } -> name
}
"#,
    );
    assert!(
        shape.diagnostics.iter().any(|diag| {
            diag.code == "invalid_lambda_params"
                && diag
                    .message
                    .contains("lambda parameters cannot use 'let' destructuring")
        }),
        "{:#?}",
        shape.diagnostics
    );
}

#[test]
fn rejects_parenthesized_let_destructuring_lambda_param() {
    let result = parse(
        r#"
def main() Unit {
    mapper = (let (x, y)) -> x + y
}
"#,
    );
    assert!(
        result
            .diagnostics
            .iter()
            .any(|diag| diag.code == "invalid_lambda_params"
                && diag
                    .message
                    .contains("lambda parameters cannot use 'let' destructuring")),
        "{:#?}",
        result.diagnostics
    );
}

#[test]
fn rejects_let_destructuring_inside_multi_parameter_lambdas() {
    let result = parse(
        r#"
def main() Unit {
    mapper = (let (x, y), index) -> x + y + index
}
"#,
    );
    assert!(
        result
            .diagnostics
            .iter()
            .any(|diag| diag.code == "invalid_lambda_params"
                && diag
                    .message
                    .contains("lambda parameters cannot use 'let' destructuring")),
        "{:#?}",
        result.diagnostics
    );
}

#[test]
fn parses_trailing_block_lambda_call_syntax() {
    let result = parse(
        r#"
def make() Unit = values.map { value -> value + 5 }
"#,
    );
    assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    let program = result.program.expect("program");
    let function = match &program.items[0] {
        Item::Function(function) => function,
        other => panic!("expected function, got {other:#?}"),
    };
    match &function.body {
        CallableBody::Expr(Expr::Call {
            args,
            uses_brace_syntax,
            ..
        }) => {
            assert!(uses_brace_syntax);
            assert_eq!(args.len(), 1);
            match &args[0].value {
                Expr::Block { body, .. } => match &body.statements[0] {
                    Stmt::Expr(ExprStmt {
                        expr: Expr::Lambda { params, .. },
                        ..
                    }) => {
                        assert_eq!(params.len(), 1);
                        assert_eq!(params[0].name, "value");
                    }
                    other => panic!("expected lambda expression inside block arg, got {other:#?}"),
                },
                other => panic!("expected trailing block arg, got {other:#?}"),
            }
        }
        other => panic!("expected trailing block call expression, got {other:#?}"),
    }
}

#[test]
fn parses_trailing_block_lambda_with_multiline_body() {
    let result = parse(
        r#"
def make() Unit = values.forEach { value ->
    plusOne = value + 1
    println(plusOne)
}
"#,
    );
    assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    let program = result.program.expect("program");
    let function = match &program.items[0] {
        Item::Function(function) => function,
        other => panic!("expected function, got {other:#?}"),
    };
    match &function.body {
        CallableBody::Expr(Expr::Call {
            args,
            uses_brace_syntax,
            ..
        }) => {
            assert!(uses_brace_syntax);
            assert_eq!(args.len(), 1);
            match &args[0].value {
                Expr::Block { body, .. } => match &body.statements[0] {
                    Stmt::Expr(ExprStmt {
                        expr:
                            Expr::Lambda {
                                params,
                                body: LambdaBody::Block(block),
                                ..
                            },
                        ..
                    }) => {
                        assert_eq!(params.len(), 1);
                        assert_eq!(params[0].name, "value");
                        assert_eq!(block.statements.len(), 2);
                    }
                    other => {
                        panic!("expected block-bodied lambda inside block arg, got {other:#?}")
                    }
                },
                other => panic!("expected trailing block arg, got {other:#?}"),
            }
        }
        other => panic!("expected trailing block call expression, got {other:#?}"),
    }
}

#[test]
fn parses_trailing_block_lambda_with_typed_single_param() {
    let result = parse(
        r#"
def make() Unit = values.map { (value Int) -> value + 5 }
"#,
    );
    assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
}

#[test]
fn rejects_trailing_block_lambda_head_on_next_line() {
    let result = parse(
        r#"
def make() Unit = values.map {
    value -> value + 5
}
"#,
    );
    assert!(
        result.diagnostics.iter().any(|diag| {
            diag.code == "invalid_trailing_lambda" && diag.message.contains("same line as '{'")
        }),
        "{:#?}",
        result.diagnostics
    );
}

#[test]
fn parses_trailing_block_lambda_with_zero_or_multiple_params() {
    let zero = parse(
        r#"
def make() Unit = values.map { () -> 5 }
"#,
    );
    assert!(zero.diagnostics.is_empty(), "{:#?}", zero.diagnostics);

    let multi = parse(
        r#"
def make() Unit = values.map { (left, right) -> left + right }
"#,
    );
    assert!(multi.diagnostics.is_empty(), "{:#?}", multi.diagnostics);
}

#[test]
fn parses_arrow_chain_as_lifted_segments() {
    fn segment_member_name(expr: &Expr) -> &str {
        match expr {
            Expr::Call { callee, .. } => {
                let Expr::Member { name, .. } = callee.as_ref() else {
                    panic!("expected method callee, got {callee:#?}");
                };
                name
            }
            Expr::Member { name, .. } => name,
            other => panic!("expected segment member access, got {other:#?}"),
        }
    }

    let expr = parse_expr_only(r#"source.->profileOpt().->nameOpt().->first"#);
    let Expr::LiftedChain { base, segments, .. } = expr else {
        panic!("expected lifted chain, got {expr:#?}");
    };
    assert!(matches!(base.as_ref(), Expr::Identifier { name, .. } if name == "source"));
    assert_eq!(segments.len(), 3);
    assert_eq!(segments[0].param, "__lume_chain0");
    assert_eq!(segments[1].param, "__lume_chain1");
    assert_eq!(segments[2].param, "__lume_chain2");
    assert_eq!(segment_member_name(&segments[0].body), "profileOpt");
    assert_eq!(segment_member_name(&segments[1].body), "nameOpt");
    assert_eq!(segment_member_name(&segments[2].body), "first");
}

#[test]
fn parses_trailing_block_lambda_when_parameter_list_starts_on_opening_line() {
    let result = parse(
        r#"
def make() Unit = values.map { (left,
    right) -> left + right }
"#,
    );
    assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
}

#[test]
fn parses_list_type_ref_shorthand() {
    let result = parse(
        r#"
class Store[T] {
    values [T]
    matrix [[T]]
}

def wrap(input Map[Str, [Int]]) [[[(Str, Int)]]] {
    cache Map[Str, [Int]] = input
    return []
}
"#,
    );
    assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);

    let program = result.program.expect("program");
    let store = match &program.items[0] {
        Item::Type(ty) => ty,
        other => panic!("expected type declaration, got {other:#?}"),
    };
    match &store.members[0] {
        TypeMember::Field(field) => match field.ty.as_ref().expect("field type") {
            TypeRef::Named { name, args, .. } => {
                assert_eq!(name, "List");
                assert_eq!(args.len(), 1);
                assert!(matches!(&args[0], TypeRef::Named { name, .. } if name == "T"));
            }
            other => panic!("expected List[T], got {other:#?}"),
        },
        other => panic!("expected field, got {other:#?}"),
    }
    match &store.members[1] {
        TypeMember::Field(field) => match field.ty.as_ref().expect("field type") {
            TypeRef::Named { name, args, .. } => {
                assert_eq!(name, "List");
                assert!(
                    matches!(&args[0], TypeRef::Named { name, args, .. } if name == "List" && matches!(&args[0], TypeRef::Named { name, .. } if name == "T"))
                );
            }
            other => panic!("expected nested list shorthand, got {other:#?}"),
        },
        other => panic!("expected field, got {other:#?}"),
    }

    let function = match &program.items[1] {
        Item::Function(function) => function,
        other => panic!("expected function, got {other:#?}"),
    };
    match function.params[0].ty.as_ref().expect("parameter type") {
        TypeRef::Named { name, args, .. } => {
            assert_eq!(name, "Map");
            assert!(matches!(&args[0], TypeRef::Named { name, .. } if name == "Str"));
            assert!(
                matches!(&args[1], TypeRef::Named { name, args, .. } if name == "List" && matches!(&args[0], TypeRef::Named { name, .. } if name == "Int"))
            );
        }
        other => panic!("expected Map[Str, List[Int]], got {other:#?}"),
    }
    match function.return_type.as_ref().expect("return type") {
        TypeRef::Named { name, args, .. } => {
            assert_eq!(name, "List");
            let level2 = &args[0];
            let level3 = match level2 {
                TypeRef::Named { name, args, .. } => {
                    assert_eq!(name, "List");
                    &args[0]
                }
                other => panic!("expected second list layer, got {other:#?}"),
            };
            match level3 {
                TypeRef::Named { name, args, .. } => {
                    assert_eq!(name, "List");
                    match &args[0] {
                        TypeRef::Tuple { fields, .. } => assert_eq!(fields.len(), 2),
                        other => {
                            panic!("expected tuple payload inside nested lists, got {other:#?}")
                        }
                    }
                }
                other => panic!("expected third list layer, got {other:#?}"),
            }
        }
        other => panic!("expected nested list shorthand return type, got {other:#?}"),
    }
}

#[test]
fn parses_parenthesized_function_type_refs() {
    let result = parse(
        r#"
def apply(f (Int) -> Int, both (Int, Str) -> Bool) Unit {}
"#,
    );
    assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    let program = result.program.expect("program");
    let function = match &program.items[0] {
        Item::Function(function) => function,
        other => panic!("expected function, got {other:#?}"),
    };
    match function.params[0].ty.as_ref().expect("first param type") {
        TypeRef::Function { params, ret, .. } => {
            assert_eq!(params.len(), 1);
            assert!(matches!(&params[0], TypeRef::Named { name, .. } if name == "Int"));
            assert!(matches!(ret.as_ref(), TypeRef::Named { name, .. } if name == "Int"));
        }
        other => panic!("expected single-param function type, got {other:#?}"),
    }
    match function.params[1].ty.as_ref().expect("second param type") {
        TypeRef::Function { params, ret, .. } => {
            assert_eq!(params.len(), 2);
            assert!(matches!(&params[0], TypeRef::Named { name, .. } if name == "Int"));
            assert!(matches!(&params[1], TypeRef::Named { name, .. } if name == "Str"));
            assert!(matches!(ret.as_ref(), TypeRef::Named { name, .. } if name == "Bool"));
        }
        other => panic!("expected multi-param function type, got {other:#?}"),
    }
}

#[test]
fn rejects_unparenthesized_function_type_refs() {
    let result = parse(
        r#"
def apply(f Int -> Int) Unit {}
"#,
    );
    assert!(
        result.diagnostics.iter().any(|diag| {
            diag.code == "invalid_function_type"
                && diag
                    .message
                    .contains("function type parameters must be parenthesized")
        }),
        "{:#?}",
        result.diagnostics
    );
}

#[test]
fn rejects_single_param_typed_lambda_without_parens() {
    let result = parse(
        r#"
def main() Unit {
    value = item Int -> item + 1
}
"#,
    );
    assert!(
        result.diagnostics.iter().any(|diag| {
            diag.code == "invalid_lambda_params"
                && diag
                    .message
                    .contains("typed single-parameter lambdas must use parentheses")
        }),
        "{:#?}",
        result.diagnostics
    );
}

#[test]
fn rejects_mixed_typed_and_untyped_lambda_params() {
    let result = parse(
        r#"
def main() Unit {
    value = (left Int, right) -> left + right
}
"#,
    );
    assert!(
        result.diagnostics.iter().any(|diag| {
            diag.code == "invalid_lambda_params"
                && diag
                    .message
                    .contains("lambda parameters must be either all typed or all untyped")
        }),
        "{:#?}",
        result.diagnostics
    );
}

#[test]
fn parses_newline_continuation_after_operator_or_postfix_dot() {
    let result = parse(
        r#"
def main() Unit {
    sum Int = 1 +
        2
    size Int = "haha".
        size()
    leadingSize Int = "haha"
        .size()
    lifted = userOpt
        .->profileOpt()
        .->name()
        .first
    user = { name: "Ada", age: 41 }
    updated = user :<
        { age: 42 }
    named = { name: "Ada" }
    located = { location: "Tampa" }
    merged = named :+
        located
}
"#,
    );
    assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
}

#[test]
fn rejects_newline_continuation_before_non_postfix_operators() {
    let leading_operator = parse(
        r#"
def main() Unit {
    sum Int = 1
        + 2
}
"#,
    );
    assert!(
        leading_operator
            .diagnostics
            .iter()
            .any(|diag| diag.code == "expected_expression"),
        "{:#?}",
        leading_operator.diagnostics
    );

    let leading_shape_update = parse(
        r#"
def main() Unit {
    user = { age: 41 }
    updated = user
        :< { age: 42 }
}
"#,
    );
    assert!(
        leading_shape_update
            .diagnostics
            .iter()
            .any(|diag| diag.code == "expected_expression"),
        "{:#?}",
        leading_shape_update.diagnostics
    );

    let leading_shape_merge = parse(
        r#"
def main() Unit {
    named = { name: "Ada" }
    located = { location: "Tampa" }
    merged = named
        :+ located
}
"#,
    );
    assert!(
        leading_shape_merge
            .diagnostics
            .iter()
            .any(|diag| diag.code == "expected_expression"),
        "{:#?}",
        leading_shape_merge.diagnostics
    );
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
fn parses_multiline_string_interpolation_as_binary_concat() {
    let result = parse(
        r#"
def run(name Str) Str {
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
fn keeps_raw_strings_as_literals() {
    let result = parse(
        r#"
def run() Unit {
    rawSingle = raw"$name\n"
    rawMulti = raw"""
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
            CallableBody::Block(block) => {
                assert!(matches!(
                    &block.statements[0],
                    Stmt::Binding(binding)
                        if matches!(&binding.values[0], Expr::String { raw, .. } if raw.starts_with("raw\""))
                ));
                assert!(matches!(
                    &block.statements[1],
                    Stmt::Binding(binding)
                        if matches!(&binding.values[0], Expr::String { raw, .. } if raw.starts_with("raw\"\"\""))
                ));
            }
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
