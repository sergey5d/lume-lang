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
    pub label: Option<String>,
    pub notes: Vec<String>,
    pub helps: Vec<String>,
}

impl Diagnostic {
    pub fn error(code: &'static str, message: impl Into<String>, span: Span) -> Self {
        Self {
            severity: Severity::Error,
            code,
            message: message.into(),
            span,
            label: None,
            notes: Vec::new(),
            helps: Vec::new(),
        }
    }

    pub fn todo(stage: &'static str, message: impl Into<String>) -> Self {
        let origin = crate::source::LineColumn::new(1, 1);
        Self {
            severity: Severity::Warning,
            code: "todo",
            message: format!("{stage}: {}", message.into()),
            span: Span::new(0, 0, origin, origin),
            label: None,
            notes: Vec::new(),
            helps: Vec::new(),
        }
    }

    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.notes.push(note.into());
        self
    }

    pub fn with_help(mut self, help: impl Into<String>) -> Self {
        self.helps.push(help.into());
        self
    }
}
