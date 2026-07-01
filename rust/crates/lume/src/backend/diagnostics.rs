use crate::{diagnostic::Diagnostic, resolver::LocatedDiagnostic};

pub fn locate_backend_diagnostic(path: &str, diagnostic: Diagnostic) -> LocatedDiagnostic {
    LocatedDiagnostic {
        path: path.to_string(),
        diagnostic,
    }
}
