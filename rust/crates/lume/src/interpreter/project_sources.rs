use super::*;

/// Keep source ownership separate from IR spans, which are local to each file.
/// Generated closures inherit their enclosing callable's source ownership.
pub(crate) fn function_source_paths(
    graph: &ModuleGraph,
    root: &PathBuf,
    ir: &ir::Program,
) -> HashMap<ir::FunctionId, String> {
    let mut order = Vec::new();
    collect_runtime_module_order(graph, root, &mut HashSet::new(), &mut order);
    let names = runtime_function_names(graph, &order, root);
    let mut origins = HashMap::new();
    let mut type_origins = HashMap::new();
    for path in order {
        let module = &graph.modules[&path];
        let program = prepare_runtime_module(module, graph, &path == root, &names);
        for item in &program.items {
            match item {
                ast::Item::Function(function) => {
                    origins.insert(
                        (
                            None,
                            function.name.clone(),
                            function.span.start,
                            function.span.end,
                        ),
                        module.display_path.clone(),
                    );
                }
                ast::Item::Type(ty) => {
                    type_origins.insert(ty.name.clone(), module.display_path.clone());
                    for member in &ty.members {
                        if let ast::TypeMember::Method(method) = member {
                            origins.insert(
                                (
                                    Some(ty.name.clone()),
                                    method.name.clone(),
                                    method.span.start,
                                    method.span.end,
                                ),
                                module.display_path.clone(),
                            );
                        }
                    }
                }
                ast::Item::Extension(extension) => {
                    if let ast::TypeRef::Named { name, .. } = &extension.target {
                        for method in &extension.methods {
                            origins.insert(
                                (
                                    Some(name.clone()),
                                    method.name.clone(),
                                    method.span.start,
                                    method.span.end,
                                ),
                                module.display_path.clone(),
                            );
                        }
                    }
                }
                _ => {}
            }
        }
    }
    let mut paths = HashMap::new();
    for function in &ir.functions {
        let owner = match function.kind {
            ir::FunctionKind::Method { owner } => Some(ir.types[owner.0].name.clone()),
            _ => None,
        };
        if let Some(span) = function.span
            && let Some(path) = origins.get(&(owner, function.name.clone(), span.start, span.end))
        {
            paths.insert(function.id, path.clone());
        }
    }
    for ty in &ir.types {
        if let Some(path) = type_origins.get(&ty.name) {
            for id in [ty.field_init, ty.mixin_init].into_iter().flatten() {
                paths.insert(id, path.clone());
            }
        }
    }
    loop {
        let before = paths.len();
        for function in &ir.functions {
            if let ir::FunctionKind::Local { parent } = function.kind
                && let Some(path) = paths.get(&parent).cloned()
            {
                paths.entry(function.id).or_insert(path);
            }
            let Some(path) = paths.get(&function.id).cloned() else {
                continue;
            };
            for statement in function.blocks.iter().flat_map(|block| &block.statements) {
                let value = match &statement.kind {
                    ir::StatementKind::Assign { value, .. }
                    | ir::StatementKind::Defer { value }
                    | ir::StatementKind::Eval { value } => value,
                };
                match value {
                    ir::RValue::Closure { function, .. } => {
                        paths.entry(*function).or_insert_with(|| path.clone());
                    }
                    ir::RValue::AnonymousObject { methods, .. } => {
                        for method in methods {
                            paths.entry(method.function).or_insert_with(|| path.clone());
                        }
                    }
                    _ => {}
                }
            }
        }
        if paths.len() == before {
            break;
        }
    }
    paths
}
