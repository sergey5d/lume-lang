//! Filesystem-free projects shared by browser and native embedding clients.

use std::{
    collections::{BTreeMap, HashMap},
    path::{Component, Path, PathBuf},
};

use crate::{
    Diagnostic, SourceFile, Span, TokenKind,
    backend::{BackendBundle, BackendBundleResult},
    interpreter::{PathRunResult, RunOptions, run_program_with_options},
    ir,
    resolver::{LocatedDiagnostic, ModuleLoadOptions, load_module_graph_with_options},
};

const MAX_FILES: usize = 64;
const MAX_SOURCE_BYTES: usize = 1_048_576;
const MAX_TOKENS: usize = 50_000;
const MAX_NESTING: usize = 128;

#[derive(Debug, Clone)]
pub struct Project {
    pub entry: String,
    pub files: BTreeMap<String, String>,
}

fn origin() -> Span {
    let position = crate::LineColumn::new(1, 1);
    Span::new(0, 0, position, position)
}

fn located(
    path: impl Into<String>,
    code: &'static str,
    message: impl Into<String>,
    span: Span,
) -> LocatedDiagnostic {
    LocatedDiagnostic {
        path: path.into(),
        diagnostic: Diagnostic::error(code, message, span),
    }
}

fn virtual_path(name: &str) -> Result<PathBuf, &'static str> {
    if name.is_empty()
        || name.len() > 1024
        || name.contains("//")
        || name.ends_with('/')
        || name.contains('\\')
        || name.contains(':')
        || name.contains('\0')
    {
        return Err("use a nonempty relative project path with '/' separators");
    }
    let path = Path::new(name);
    if path
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err("project paths must be relative and cannot contain '.' or '..'");
    }
    Ok(Path::new("/").join(path))
}

/// Cheap compiler-input bounds, also applied before the standalone formatter.
pub fn source_limits(file: &SourceFile) -> Vec<Diagnostic> {
    if file.text.len() > MAX_SOURCE_BYTES {
        return vec![Diagnostic::error(
            "project_limit",
            "source file is too large",
            origin(),
        )];
    }
    let lexed = crate::lex(file);
    if let Some(diagnostic) = token_limits(&lexed.tokens) {
        return vec![diagnostic];
    }
    lexed.diagnostics
}

fn token_limits(tokens: &[crate::Token]) -> Option<Diagnostic> {
    if tokens.len() > MAX_TOKENS {
        return Some(Diagnostic::error(
            "project_limit",
            "too many tokens in source file",
            origin(),
        ));
    }
    let mut depth = 0usize;
    for token in tokens {
        match token.kind {
            TokenKind::LParen | TokenKind::LBrace | TokenKind::LBracket => depth += 1,
            TokenKind::RParen | TokenKind::RBrace | TokenKind::RBracket => {
                depth = depth.saturating_sub(1)
            }
            _ => {}
        }
        if depth > MAX_NESTING {
            return Some(Diagnostic::error(
                "project_limit",
                "source nesting limit exceeded",
                token.span,
            ));
        }
    }
    None
}

fn load_project(
    project: &Project,
) -> Result<(crate::resolver::ModuleGraph, PathBuf), Vec<LocatedDiagnostic>> {
    let error = |code, message: String| vec![located(&project.entry, code, message, origin())];
    if project.files.len() > MAX_FILES
        || project.files.values().map(String::len).sum::<usize>() > MAX_SOURCE_BYTES
    {
        return Err(error(
            "project_limit",
            format!("project exceeds {MAX_FILES} files or {MAX_SOURCE_BYTES} source bytes"),
        ));
    }
    let root = virtual_path(&project.entry)
        .map_err(|message| error("invalid_project_path", message.into()))?;
    let mut programs = HashMap::new();
    let mut diagnostics = Vec::new();
    for (name, source) in &project.files {
        let path = virtual_path(name)
            .map_err(|message| error("invalid_project_path", format!("{name}: {message}")))?;
        let file = SourceFile::new(name, source);
        let lexed = crate::lex(&file);
        if let Some(diagnostic) = token_limits(&lexed.tokens) {
            diagnostics.push(LocatedDiagnostic {
                path: name.clone(),
                diagnostic,
            });
            continue;
        }
        if !lexed.diagnostics.is_empty() {
            diagnostics.extend(
                lexed
                    .diagnostics
                    .into_iter()
                    .map(|diagnostic| LocatedDiagnostic {
                        path: name.clone(),
                        diagnostic,
                    }),
            );
            continue;
        }
        let parsed = crate::parse_program(&lexed.tokens);
        diagnostics.extend(
            parsed
                .diagnostics
                .into_iter()
                .map(|diagnostic| LocatedDiagnostic {
                    path: name.clone(),
                    diagnostic,
                }),
        );
        if let Some(program) = parsed.program {
            programs.insert(path, program);
        }
    }
    if !diagnostics.is_empty() {
        return Err(diagnostics);
    }
    if !programs.contains_key(&root) {
        return Err(error("missing_entry", "entry file was not supplied".into()));
    }
    let mut options = ModuleLoadOptions {
        virtual_sources: Some(programs),
        ..ModuleLoadOptions::default()
    };
    for module in options.library_modules.values_mut() {
        module.source_path = None;
    }
    load_module_graph_with_options(&root, &options)
        .map_err(|message| error("module_load_error", message))
}

/// Check and lower a virtual project using exactly the normal module semantics.
pub fn build_project(project: &Project) -> BackendBundleResult {
    let (graph, root) = match load_project(project) {
        Ok(loaded) => loaded,
        Err(diagnostics) => {
            return BackendBundleResult {
                diagnostics,
                ..BackendBundleResult::default()
            };
        }
    };
    match crate::backend::bundle::build_backend_bundle_from_graph(graph, root) {
        Ok(result) => result,
        Err(message) => BackendBundleResult {
            diagnostics: vec![located(&project.entry, "project_error", message, origin())],
            ..BackendBundleResult::default()
        },
    }
}

fn operand_type<'a>(
    bundle: &'a BackendBundle,
    function: &'a ir::Function,
    operand: &ir::Operand,
) -> Option<&'a ir::Type> {
    match operand {
        ir::Operand::Copy(place) | ir::Operand::Move(place) => match place.as_ref() {
            ir::Place::Local(id) => function.locals.get(id.0).map(|local| &local.ty),
            ir::Place::Global(id) => bundle.ir.globals.get(id.0).map(|global| &global.ty),
            _ => None,
        },
        _ => None,
    }
}

fn host_owner<'a>(
    bundle: &'a BackendBundle,
    function: &'a ir::Function,
    value: &'a ir::RValue,
) -> Option<&'a str> {
    let ty = match value {
        ir::RValue::Call {
            callee: ir::Callee::Named { path },
            ..
        }
        | ir::RValue::NamedValue { path } => return path.first().map(String::as_str),
        ir::RValue::Call {
            callee: ir::Callee::Method { receiver, .. },
            ..
        } => operand_type(bundle, function, receiver),
        ir::RValue::Field { base, .. } => operand_type(bundle, function, base),
        _ => None,
    }?;
    if let ir::Type::Named { name, .. } = ty {
        Some(name)
    } else {
        None
    }
}

fn browser_diagnostics(bundle: &BackendBundle) -> Vec<LocatedDiagnostic> {
    let runtime = crate::runtime::RuntimeProgram::from_ir(&bundle.ir);
    let mut diagnostics = Vec::new();
    for function in &bundle.ir.functions {
        for block in &function.blocks {
            for statement in &block.statements {
                let value = match &statement.kind {
                    ir::StatementKind::Assign { value, .. }
                    | ir::StatementKind::Eval { value }
                    | ir::StatementKind::Defer { value } => value,
                };
                if let Some(owner) = host_owner(bundle, function, value)
                    && runtime.active_name_is_native(owner)
                    && crate::stdlib::descriptors().iter().any(|descriptor| {
                        descriptor.name == owner && !descriptor.host_capabilities.is_empty()
                    })
                {
                    diagnostics.push(located(bundle.function_source_paths.get(&function.id).unwrap_or(&bundle.root_display_path), "unsupported_target", format!("'{owner}' requires native file access and is unavailable in the browser"), statement.span.unwrap_or_else(origin)));
                }
            }
        }
    }
    diagnostics.sort_by_key(|located| located.diagnostic.span.start);
    diagnostics.dedup_by(|left, right| {
        left.path == right.path && left.diagnostic.span == right.diagnostic.span
    });
    diagnostics
}

pub fn check_browser_project(project: &Project) -> PathRunResult {
    let mut checked = build_project(project);
    if let Some(bundle) = &checked.bundle {
        checked.diagnostics.extend(browser_diagnostics(bundle));
    }
    PathRunResult {
        diagnostics: checked.diagnostics,
        warnings: checked.warnings,
        ..PathRunResult::default()
    }
}

pub fn run_browser_project(
    project: &Project,
    entry: Option<&str>,
    mut options: RunOptions,
) -> PathRunResult {
    // Browser embedding cannot opt back into process environment or native I/O.
    options.allow_files = false;
    options.environment.get_or_insert_with(BTreeMap::new);
    options
        .limits
        .get_or_insert_with(crate::interpreter::ExecutionLimits::default);
    let mut checked = build_project(project);
    let Some(bundle) = checked.bundle else {
        return PathRunResult {
            diagnostics: checked.diagnostics,
            warnings: checked.warnings,
            ..PathRunResult::default()
        };
    };
    checked.diagnostics.extend(browser_diagnostics(&bundle));
    if !checked.diagnostics.is_empty() {
        return PathRunResult {
            diagnostics: checked.diagnostics,
            warnings: checked.warnings,
            ..PathRunResult::default()
        };
    }
    let run = run_program_with_options(&bundle.ir, entry, options);
    let error_path = run
        .diagnostic_function
        .and_then(|function| bundle.function_source_paths.get(&function))
        .unwrap_or(&bundle.root_display_path);
    PathRunResult {
        diagnostics: run
            .diagnostics
            .into_iter()
            .map(|diagnostic| LocatedDiagnostic {
                path: error_path.clone(),
                diagnostic,
            })
            .collect(),
        warnings: checked.warnings,
        output: run.output,
        return_value: run.return_value,
    }
}

#[cfg(test)]
mod tests;
