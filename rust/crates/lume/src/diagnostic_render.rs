use std::{fs, path::Path};

use crate::{
    Diagnostic, Severity,
    source::{LineColumn, Span},
};

pub fn render_diagnostic(path: &str, source: Option<&str>, diagnostic: &Diagnostic) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "{}: {}\n",
        render_header(diagnostic),
        diagnostic.message
    ));
    out.push_str(&format!(
        "  --> {}:{}:{}\n",
        path, diagnostic.span.start_pos.line, diagnostic.span.start_pos.column
    ));

    if let Some(source_line) =
        source.and_then(|source| line_text(source, diagnostic.span.start_pos))
    {
        render_source_snippet(&mut out, source_line, diagnostic);
    }

    render_notes_and_help(&mut out, diagnostic);
    trim_trailing_newline(out)
}

pub fn render_path_diagnostic(path: &Path, diagnostic: &Diagnostic) -> String {
    let source = fs::read_to_string(path).ok();
    render_diagnostic(&path.display().to_string(), source.as_deref(), diagnostic)
}

pub fn render_path_diagnostics(path: &Path, diagnostics: &[Diagnostic]) -> String {
    let source = fs::read_to_string(path).ok();
    let display = path.display().to_string();
    diagnostics
        .iter()
        .map(|diagnostic| render_diagnostic(&display, source.as_deref(), diagnostic))
        .collect::<Vec<_>>()
        .join("\n")
}

fn render_header(diagnostic: &Diagnostic) -> String {
    let severity = match diagnostic.severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
    };
    match diagnostic_display_code(diagnostic.code) {
        Some(code) => format!("{severity}[{code}]"),
        None => format!("{severity}[{}]", diagnostic.code),
    }
}

fn diagnostic_display_code(code: &str) -> Option<&'static str> {
    match code {
        "constructor_unavailable" => Some("E0312"),
        _ => None,
    }
}

fn render_source_snippet(out: &mut String, source_line: &str, diagnostic: &Diagnostic) {
    let line = diagnostic.span.start_pos.line;
    let gutter_width = line.to_string().len();
    out.push_str(&format!("{:>gutter_width$} |\n", ""));
    out.push_str(&format!("{line:>gutter_width$} | {source_line}\n"));
    out.push_str(&format!(
        "{:>gutter_width$} | {}{}",
        "",
        " ".repeat(diagnostic.span.start_pos.column.saturating_sub(1)),
        "^".repeat(caret_width(diagnostic.span))
    ));
    out.push(' ');
    out.push_str(diagnostic.label.as_deref().unwrap_or(&diagnostic.message));
    out.push('\n');
}

fn render_notes_and_help(out: &mut String, diagnostic: &Diagnostic) {
    if diagnostic.notes.is_empty() && diagnostic.helps.is_empty() {
        return;
    }

    out.push_str("   |\n");
    for note in &diagnostic.notes {
        out.push_str(&format!("   = note: {note}\n"));
    }
    for help in &diagnostic.helps {
        out.push_str(&format!("   = help: {help}\n"));
    }
}

fn line_text(source: &str, position: LineColumn) -> Option<&str> {
    source.lines().nth(position.line.saturating_sub(1))
}

fn caret_width(span: Span) -> usize {
    if span.start_pos.line == span.end_pos.line {
        span.end_pos
            .column
            .saturating_sub(span.start_pos.column)
            .max(1)
    } else {
        1
    }
}

fn trim_trailing_newline(mut value: String) -> String {
    while value.ends_with('\n') {
        value.pop();
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_labeled_notes_and_help() {
        let diagnostic = Diagnostic::error(
            "constructor_unavailable",
            "constructor is not available",
            Span::new(7, 28, LineColumn::new(1, 8), LineColumn::new(1, 29)),
        )
        .with_label("field construction is hidden")
        .with_note("User declares a private primary constructor")
        .with_help("use User.create(name = \"Ada\")");

        let rendered = render_diagnostic(
            "app.lum",
            Some("user = User { name: \"Ada\" }\n"),
            &diagnostic,
        );

        assert_eq!(
            rendered,
            "error[E0312]: constructor is not available\n  --> app.lum:1:8\n  |\n1 | user = User { name: \"Ada\" }\n  |        ^^^^^^^^^^^^^^^^^^^^^ field construction is hidden\n   |\n   = note: User declares a private primary constructor\n   = help: use User.create(name = \"Ada\")"
        );
    }

    #[test]
    fn renders_message_as_default_caret_label() {
        let diagnostic = Diagnostic::error(
            "undefined_name",
            "undefined name 'value'",
            Span::new(6, 11, LineColumn::new(1, 7), LineColumn::new(1, 12)),
        );

        let rendered = render_diagnostic("app.lum", Some("print(value)\n"), &diagnostic);

        assert_eq!(
            rendered,
            "error[undefined_name]: undefined name 'value'\n  --> app.lum:1:7\n  |\n1 | print(value)\n  |       ^^^^^ undefined name 'value'"
        );
    }
}
