use crate::{Diagnostic, source::Span};

pub(crate) fn hidden_field_constructor(
    span: Span,
    class_name: &str,
    help: Option<String>,
) -> Diagnostic {
    let mut diagnostic = Diagnostic::error(
        "constructor_unavailable",
        "constructor is not available",
        span,
    )
    .with_label("field construction is hidden")
    .with_note(format!(
        "{class_name} declares a private primary constructor"
    ));

    if let Some(help) = help {
        diagnostic = diagnostic.with_help(help);
    }

    diagnostic
}
