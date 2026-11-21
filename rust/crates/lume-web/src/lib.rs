//! A string-based protocol keeps JavaScript glue small and preserves numeric
//! precision: program values are rendered text, never coerced to JS numbers.

use std::collections::BTreeMap;

use lume::{
    Diagnostic, LineColumn, PathRunResult, RunOptions, Severity, SourceFile, Span,
    project::{Project, check_browser_project, run_browser_project},
};
use serde_json::{Value, json};
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

const MAX_REQUEST_BYTES: usize = 8 * 1_048_576;

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = globalThis, js_name = lumeOutput)]
    fn emit_output(text: &str);
}

fn output(text: &str) {
    #[cfg(target_arch = "wasm32")]
    emit_output(text);
    #[cfg(not(target_arch = "wasm32"))]
    let _ = text;
}

fn failure(message: impl Into<String>) -> String {
    let position = LineColumn::new(1, 1);
    response(PathRunResult {
        diagnostics: vec![lume::LocatedDiagnostic {
            path: String::new(),
            diagnostic: Diagnostic::error(
                "invalid_request",
                message,
                Span::new(0, 0, position, position),
            ),
        }],
        ..PathRunResult::default()
    })
}

fn request(text: &str) -> Result<Value, String> {
    if text.len() > MAX_REQUEST_BYTES {
        return Err("request is too large".into());
    }
    let request: Value =
        serde_json::from_str(text).map_err(|error| format!("invalid request JSON: {error}"))?;
    if !request.is_object() {
        return Err("request must be an object".into());
    }
    Ok(request)
}

fn project(request: &Value) -> Result<Project, String> {
    let project = request.get("project").unwrap_or(request);
    let entry = project
        .get("entry")
        .and_then(Value::as_str)
        .ok_or("project.entry must be a string")?;
    let files = project
        .get("files")
        .and_then(Value::as_object)
        .ok_or("project.files must map paths to source strings")?;
    if files.len() > 64 {
        return Err("at most 64 project files are allowed".into());
    }
    let files = files
        .iter()
        .map(|(path, source)| {
            if path.len() > 1024 {
                return Err("project path is too long".into());
            }
            Ok((
                path.clone(),
                source
                    .as_str()
                    .ok_or("file contents must be strings")?
                    .to_string(),
            ))
        })
        .collect::<Result<BTreeMap<_, _>, String>>()?;
    Ok(Project {
        entry: entry.into(),
        files,
    })
}

fn diagnostic(value: &lume::LocatedDiagnostic) -> Value {
    let diagnostic = &value.diagnostic;
    json!({
        "path": value.path.trim_start_matches('/'),
        "severity": if diagnostic.severity == Severity::Error { "error" } else { "warning" },
        "code": diagnostic.code,
        "message": diagnostic.message,
        "span": {
            "start": diagnostic.span.start, "end": diagnostic.span.end,
            "line": diagnostic.span.start_pos.line, "column": diagnostic.span.start_pos.column,
            "endLine": diagnostic.span.end_pos.line, "endColumn": diagnostic.span.end_pos.column,
        },
        "label": diagnostic.label, "notes": diagnostic.notes, "helps": diagnostic.helps,
    })
}

fn response(result: PathRunResult) -> String {
    json!({
        "protocol": 1,
        "ok": result.diagnostics.is_empty(),
        "diagnostics": result.diagnostics.iter().map(diagnostic).collect::<Vec<_>>(),
        "warnings": result.warnings.iter().map(diagnostic).collect::<Vec<_>>(),
        "output": result.output,
        "returnValue": result.return_value,
    })
    .to_string()
}

/// Validate and type-check a virtual project. No host filesystem is consulted.
#[cfg_attr(target_arch = "wasm32", wasm_bindgen)]
pub fn check(text: &str) -> String {
    let result = request(text).and_then(|request| project(&request));
    match result {
        Ok(project) => response(check_browser_project(&project)),
        Err(message) => failure(message),
    }
}

fn strings(value: Option<&Value>, name: &str) -> Result<Vec<String>, String> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let values = value
        .as_array()
        .ok_or_else(|| format!("{name} must be an array of strings"))?;
    if values.len() > 1024 {
        return Err(format!("too many {name}"));
    }
    values
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| format!("{name} must contain strings"))
        })
        .collect()
}

fn run_request(text: &str) -> Result<PathRunResult, String> {
    let request = request(text)?;
    let project = project(&request)?;
    let mut options = RunOptions::browser();
    let supplied = request.get("options").unwrap_or(&Value::Null);
    options.args = strings(supplied.get("args"), "args")?;
    if let Some(environment) = supplied.get("environment") {
        let values = environment
            .as_object()
            .ok_or("environment must map names to strings")?;
        if values.len() > 256 {
            return Err("too many environment entries".into());
        }
        options.environment = Some(
            values
                .iter()
                .map(|(name, value)| {
                    Ok((
                        name.clone(),
                        value
                            .as_str()
                            .ok_or("environment values must be strings")?
                            .to_string(),
                    ))
                })
                .collect::<Result<_, String>>()?,
        );
    }
    let entry = supplied
        .get("entry")
        .map(|value| value.as_str().ok_or("options.entry must be a string"))
        .transpose()?;
    if let Some(limits) = supplied.get("limits") {
        if !limits.is_object() {
            return Err("limits must be an object".into());
        }
        let configured = options.limits.as_mut().unwrap();
        for (name, target) in [
            ("steps", &mut configured.steps),
            ("cleanupSteps", &mut configured.cleanup_steps),
        ] {
            if let Some(value) = limits.get(name) {
                let value = value
                    .as_u64()
                    .ok_or_else(|| format!("{name} must be a nonnegative integer"))?;
                *target = (*target).min(value);
            }
        }
        for (name, target) in [
            ("callDepth", &mut configured.call_depth),
            ("collectionItems", &mut configured.collection_items),
            ("stringBytes", &mut configured.string_bytes),
            ("outputBytes", &mut configured.output_bytes),
        ] {
            if let Some(value) = limits.get(name) {
                let value = value
                    .as_u64()
                    .and_then(|value| usize::try_from(value).ok())
                    .ok_or_else(|| format!("{name} must be a nonnegative integer"))?;
                *target = (*target).min(value);
            }
        }
    }
    options.output = Some(output);
    Ok(run_browser_project(&project, entry, options))
}

/// Run synchronously inside the caller's disposable Web Worker.
#[cfg_attr(target_arch = "wasm32", wasm_bindgen)]
pub fn run(text: &str) -> String {
    match run_request(text) {
        Ok(result) => response(result),
        Err(message) => failure(message),
    }
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen)]
pub fn format(text: &str) -> String {
    let request = match request(text) {
        Ok(request) => request,
        Err(message) => return failure(message),
    };
    let Some(source) = request.get("source").and_then(Value::as_str) else {
        return failure("source must be a string");
    };
    let path = request
        .get("path")
        .and_then(Value::as_str)
        .unwrap_or("main.lum");
    let file = SourceFile::new(path, source);
    let preflight = lume::project::source_limits(&file);
    let result = if preflight.is_empty() {
        lume::format_source(&file)
    } else {
        lume::FormatResult {
            text: source.into(),
            diagnostics: preflight,
        }
    };
    let diagnostics = result
        .diagnostics
        .into_iter()
        .map(|item| {
            diagnostic(&lume::LocatedDiagnostic {
                path: path.into(),
                diagnostic: item,
            })
        })
        .collect::<Vec<_>>();
    json!({ "protocol": 1, "ok": diagnostics.is_empty(), "text": result.text, "diagnostics": diagnostics, "warnings": [], "output": "", "returnValue": null }).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_requests_and_preserves_integer_precision() {
        assert!(
            !serde_json::from_str::<Value>(&check("[]")).unwrap()["ok"]
                .as_bool()
                .unwrap()
        );
        let request = json!({"project": {"entry": "main.lum", "files": {"main.lum": "fn main() Int = 9007199254740993"}}});
        let result: Value = serde_json::from_str(&run(&request.to_string())).unwrap();
        assert_eq!(result["returnValue"], "9007199254740993");
        assert_eq!(result["ok"], true, "{result}");
    }

    #[test]
    fn formats_and_reports_located_errors() {
        let result: Value = serde_json::from_str(&format(
            r#"{"source":"fn main(){println( 1 )}","path":"main.lum"}"#,
        ))
        .unwrap();
        assert_eq!(result["ok"], true, "{result}");
        assert!(result["text"].as_str().unwrap().contains("println(1)"));
        let request =
            json!({"entry": "main.lum", "files": {"main.lum": "fn main() Int = missing"}});
        let result: Value = serde_json::from_str(&check(&request.to_string())).unwrap();
        assert_eq!(result["diagnostics"][0]["path"], "main.lum");
    }
}
