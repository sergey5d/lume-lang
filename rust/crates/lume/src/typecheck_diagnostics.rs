use crate::{Diagnostic, source::Span};

pub(crate) fn parenthesized_pattern(path: &[String], span: Span) -> Diagnostic {
    let name = path.join(".");
    let field = match path.last().map(String::as_str) {
        Some("Some" | "Ok" | "Left" | "Right") => "value",
        Some("Err") => "error",
        _ => "field",
    };
    Diagnostic::error(
        "parenthesized_pattern",
        format!("named-value unpacking uses braces; '{name}(...)' is not supported"),
        span,
    )
    .with_help(format!(
        "write `{name} {{ {field} as binding }}` to unpack a field; match singleton objects by their bare name"
    ))
}

pub(crate) fn interface_field(name: &str, span: Span) -> Diagnostic {
    Diagnostic::error(
        "interface_field",
        format!("interfaces cannot declare stored field '{name}'; use a getter signature instead"),
        span,
    )
    .with_help(format!(
        "declare `get {name} Type` in the interface; implementations may supply a public field or a getter"
    ))
}

pub(crate) fn interface_member_body(span: Span) -> Diagnostic {
    Diagnostic::error(
        "interface_member_body",
        "interface members declare signatures, not implementations",
        span,
    )
    .with_help(
        "move this implementation into a mixin and adopt it with 'use MixinName' in the class",
    )
}

pub(crate) fn interface_nested_declaration(span: Span) -> Diagnostic {
    Diagnostic::error(
        "interface_nested_declaration",
        "interface bodies contain only 'get' and 'fn' signatures, not nested declarations",
        span,
    )
    .with_help("move the declaration to module scope or into a class, shape, object, or annotation")
}

pub(crate) fn private_field_constructor(
    span: Span,
    class_name: &str,
    help: Option<String>,
) -> Diagnostic {
    let mut diagnostic = Diagnostic::error(
        "constructor_unavailable",
        "constructor is not available",
        span,
    )
    .with_label("field construction is private")
    .with_note(format!(
        "{class_name} declares a private primary constructor"
    ));

    if let Some(help) = help {
        diagnostic = diagnostic.with_help(help);
    }

    diagnostic
}
