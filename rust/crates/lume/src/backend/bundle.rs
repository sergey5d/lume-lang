use std::path::{Path, PathBuf};

use crate::{
    ast,
    backend::{descriptors::BackendDescriptors, externals::ExternalDescriptors},
    interpreter::merged_runtime_program,
    ir,
    lower::lower_program,
    resolver::{LocatedDiagnostic, ModuleLoadOptions, load_module_graph_with_options},
    typecheck::check_path_with_load_options,
};

/// Checked, merged, and lowered program state shared by non-interpreter backends.
#[derive(Debug, Clone)]
pub struct BackendBundle {
    pub root_path: PathBuf,
    pub root_display_path: String,
    pub ast: ast::Program,
    pub ir: ir::Program,
    pub descriptors: BackendDescriptors,
    pub externals: ExternalDescriptors,
}

#[derive(Debug, Clone, Default)]
pub struct BackendBundleResult {
    pub diagnostics: Vec<LocatedDiagnostic>,
    pub bundle: Option<BackendBundle>,
}

pub fn build_backend_bundle(path: impl AsRef<Path>) -> Result<BackendBundleResult, String> {
    build_backend_bundle_with_load_options(path, &ModuleLoadOptions::default())
}

pub(crate) fn build_backend_bundle_with_load_options(
    path: impl AsRef<Path>,
    load_options: &ModuleLoadOptions,
) -> Result<BackendBundleResult, String> {
    let path = path.as_ref();
    let checked = check_path_with_load_options(path, load_options)?;
    if !checked.diagnostics.is_empty() {
        return Ok(BackendBundleResult {
            diagnostics: checked.diagnostics,
            bundle: None,
        });
    }

    let (graph, root_path) = load_module_graph_with_options(path, load_options)?;
    let root_module = graph
        .modules
        .get(&root_path)
        .ok_or_else(|| format!("loaded root module missing {}", root_path.display()))?;
    let root_display_path = root_module.display_path.clone();
    let externals = ExternalDescriptors::from_module_graph(&graph);
    let ast = merged_runtime_program(&graph, &root_path)?;

    let lowered = lower_program(&ast);
    if !lowered.diagnostics.is_empty() {
        return Ok(BackendBundleResult {
            diagnostics: lowered
                .diagnostics
                .into_iter()
                .map(|diagnostic| LocatedDiagnostic {
                    path: root_display_path.clone(),
                    diagnostic,
                })
                .collect(),
            bundle: None,
        });
    }

    let ir = lowered
        .program
        .expect("ir program after successful lowering");
    let descriptors = BackendDescriptors::from_ir_and_externals(&ir, &externals);

    Ok(BackendBundleResult {
        diagnostics: Vec::new(),
        bundle: Some(BackendBundle {
            root_path,
            root_display_path,
            ast,
            ir,
            descriptors,
            externals,
        }),
    })
}
