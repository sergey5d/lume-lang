use crate::source::Span;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub severity: Severity,
    pub code: &'static str,
    pub message: String,
    pub span: Span,
}

impl Diagnostic {
    pub fn error(code: &'static str, message: impl Into<String>, span: Span) -> Self {
        Self {
            severity: Severity::Error,
            code,
            message: message.into(),
            span,
        }
    }

    pub fn todo(stage: &'static str, message: impl Into<String>) -> Self {
        let origin = crate::source::LineColumn::new(1, 1);
        Self {
            severity: Severity::Warning,
            code: "todo",
            message: format!("{stage}: {}", message.into()),
            span: Span::new(0, 0, origin, origin),
        }
    }
}
