use crate::{Diagnostic, source::Span};

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
