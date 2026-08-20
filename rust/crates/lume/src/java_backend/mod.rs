use std::{
    collections::{BTreeMap, HashMap, HashSet},
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

mod emit;

use crate::{
    Diagnostic,
    ast::{
        FieldDecl, ImportDecl, ImportSymbol, Item, MethodDecl, ModuleDecl, Param, Program,
        TypeDecl, TypeKind, TypeMember, TypeParam, TypeRef, Visibility,
    },
    backend::bundle::build_backend_bundle_with_load_options,
    resolver::{LibraryModule, LocatedDiagnostic, ModuleLoadOptions, parse_program_from_path},
    source::{LineColumn, Span},
};

#[derive(Debug, Clone)]
pub struct JavaBackendOptions {
    pub output_dir: PathBuf,
    pub classpath: Vec<PathBuf>,
}

impl JavaBackendOptions {
    pub fn new(output_dir: impl Into<PathBuf>) -> Self {
        Self {
            output_dir: output_dir.into(),
            classpath: Vec::new(),
        }
    }

    pub fn with_classpath_entry(mut self, entry: impl Into<PathBuf>) -> Self {
        self.classpath.push(entry.into());
        self
    }
}

#[derive(Debug, Clone, Default)]
pub struct JavaBackendResult {
    pub diagnostics: Vec<LocatedDiagnostic>,
    pub written_files: Vec<PathBuf>,
}

pub fn generate_java_path(
    path: impl AsRef<Path>,
    options: JavaBackendOptions,
) -> Result<JavaBackendResult, String> {
    let path = path.as_ref();
    let discovered_externals = discover_java_external_symbols(path)?;
    let external_resolution = resolve_external_classes(&discovered_externals, &options)?;
    if !external_resolution.diagnostics.is_empty() {
        return Ok(JavaBackendResult {
            diagnostics: external_resolution.diagnostics,
            written_files: Vec::new(),
        });
    }

    let load_options = ModuleLoadOptions {
        library_modules: external_resolution.library_modules.clone(),
    };
    let bundled = build_backend_bundle_with_load_options(path, &load_options)?;
    if !bundled.diagnostics.is_empty() {
        return Ok(JavaBackendResult {
            diagnostics: bundled.diagnostics,
            written_files: Vec::new(),
        });
    }

    let bundle = bundled
        .bundle
        .expect("backend bundle after successful build");
    let sources = emit::render_declaration_skeletons(&bundle, &external_resolution.classes);
    let unsupported_diagnostics =
        unsupported_java_body_diagnostics(&bundle.root_display_path, &sources);
    if !unsupported_diagnostics.is_empty() {
        return Ok(JavaBackendResult {
            diagnostics: unsupported_diagnostics,
            written_files: Vec::new(),
        });
    }

    let mut written_files = Vec::new();
    for source in sources {
        let path = options.output_dir.join(source.relative_path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|err| format!("create {}: {err}", parent.display()))?;
        }
        fs::write(&path, source.contents)
            .map_err(|err| format!("write {}: {err}", path.display()))?;
        written_files.push(path);
    }

    Ok(JavaBackendResult {
        diagnostics: Vec::new(),
        written_files,
    })
}

fn unsupported_java_body_diagnostics(
    root_display_path: &str,
    sources: &[emit::JavaSource],
) -> Vec<LocatedDiagnostic> {
    sources
        .iter()
        .filter(|source| source.contents.contains(emit::JAVA_UNSUPPORTED_STUB_MARKER))
        .map(|source| LocatedDiagnostic {
            path: root_display_path.to_string(),
            diagnostic: Diagnostic::error(
                "java_backend_unsupported_body",
                format!(
                    "Java backend cannot generate every method body in '{}'",
                    source.relative_path.display()
                ),
                Span::new(0, 0, LineColumn::new(1, 1), LineColumn::new(1, 1)),
            )
            .with_label("Java generation stopped here")
            .with_note("previous versions emitted a runtime UnsupportedOperationException stub; this is now a compile-time error")
            .with_help("simplify the Lume method body or implement the missing Java backend emission path"),
        })
        .collect()
}

#[derive(Debug, Clone, Default)]
struct JavaExternalSymbols {
    symbols: Vec<JavaExternalSymbol>,
    local_type_names: HashMap<String, String>,
}

#[derive(Debug, Clone)]
struct JavaExternalSymbol {
    module_path: String,
    lume_name: String,
    qualified_name: String,
    source_path: String,
    span: crate::source::Span,
}

#[derive(Debug, Clone, Default)]
struct ExternalClassResolution {
    diagnostics: Vec<LocatedDiagnostic>,
    library_modules: HashMap<String, LibraryModule>,
    classes: HashMap<String, JavaExternalClass>,
}

#[derive(Debug, Clone)]
pub(crate) struct JavaExternalClass {
    pub(crate) qualified_name: String,
    pub(crate) kind: TypeKind,
    pub(crate) type_params: Vec<String>,
    with_bounds: Vec<TypeRef>,
    inherit_qualified_names: Vec<String>,
    fields: Vec<JavaExternalField>,
    constructors: Vec<JavaExternalCallable>,
    pub(crate) methods: Vec<JavaExternalCallable>,
}

#[derive(Debug, Clone)]
struct JavaExternalField {
    name: String,
    ty: Option<TypeRef>,
    initializer: Option<crate::ast::Expr>,
}

#[derive(Debug, Clone)]
pub(crate) struct JavaExternalCallable {
    pub(crate) name: String,
    type_params: Vec<String>,
    reified_type_params: Vec<String>,
    pub(crate) params: Vec<JavaExternalParam>,
    pub(crate) return_type: Option<TypeRef>,
}

#[derive(Debug, Clone)]
pub(crate) struct JavaExternalParam {
    name: String,
    ty: Option<TypeRef>,
    variadic: bool,
    pub(crate) coercion: Option<JavaPrimitiveCoercion>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum JavaPrimitiveCoercion {
    Byte,
    Short,
    Int,
    Float,
}

fn discover_java_external_symbols(path: &Path) -> Result<JavaExternalSymbols, String> {
    let mut discovered = JavaExternalSymbols::default();
    let mut visited = HashSet::new();
    let source_root = path
        .parent()
        .ok_or_else(|| format!("resolve module base for {}", path.display()))?
        .to_path_buf();
    discover_java_external_symbols_from_path(path, &source_root, &mut visited, &mut discovered)?;
    Ok(discovered)
}

fn discover_java_external_symbols_from_path(
    path: &Path,
    source_root: &Path,
    visited: &mut HashSet<PathBuf>,
    discovered: &mut JavaExternalSymbols,
) -> Result<(), String> {
    let abs = fs::canonicalize(path).map_err(|err| format!("resolve {}: {err}", path.display()))?;
    if !visited.insert(abs.clone()) {
        return Ok(());
    }

    let program = parse_program_from_path(&abs)?;
    collect_local_lume_type_names(&program, discovered);
    let base_dir = abs
        .parent()
        .ok_or_else(|| format!("resolve module base for {}", abs.display()))?;
    for import in &program.imports {
        let module_file = format!("{}.lum", import.path);
        let rooted_child_path = source_root.join(&module_file);
        let relative_child_path = base_dir.join(&module_file);
        let child_path = if rooted_child_path.exists() {
            Some(rooted_child_path)
        } else if relative_child_path.exists() {
            Some(relative_child_path)
        } else {
            None
        };
        if let Some(child_path) = child_path {
            discover_java_external_symbols_from_path(
                &child_path,
                source_root,
                visited,
                discovered,
            )?;
        } else {
            collect_java_external_symbols_from_import(import, &abs, discovered);
        }
    }
    Ok(())
}

fn collect_local_lume_type_names(program: &Program, discovered: &mut JavaExternalSymbols) {
    let Some(module) = program.module.as_ref() else {
        return;
    };
    let package = module.name.replace('/', ".");
    for item in &program.items {
        let Item::Type(decl) = item else {
            continue;
        };
        discovered
            .local_type_names
            .entry(format!("{package}.{}", decl.name))
            .or_insert_with(|| decl.name.clone());
    }
}

fn collect_java_external_symbols_from_import(
    import: &ImportDecl,
    source_path: &Path,
    discovered: &mut JavaExternalSymbols,
) {
    if import.object_name.is_some() || import.wildcard || import.symbols.is_empty() {
        return;
    }
    let package = import.path.replace('/', ".");
    for symbol in &import.symbols {
        let qualified_name = format!("{package}.{}", symbol.name);
        let lume_name = symbol.alias.clone().unwrap_or_else(|| symbol.name.clone());
        if discovered
            .symbols
            .iter()
            .any(|existing| existing.qualified_name == qualified_name)
        {
            continue;
        }
        discovered.symbols.push(JavaExternalSymbol {
            module_path: import.path.clone(),
            lume_name,
            qualified_name,
            source_path: source_path.display().to_string(),
            span: symbol.span,
        });
    }
}

fn java_library_modules(
    externals: &JavaExternalSymbols,
    classes_by_qualified: &HashMap<String, JavaExternalClass>,
) -> HashMap<String, LibraryModule> {
    let modules_by_name = externals
        .symbols
        .iter()
        .filter(|symbol| classes_by_qualified.contains_key(&symbol.qualified_name))
        .map(|symbol| (symbol.lume_name.clone(), symbol.module_path.clone()))
        .collect::<HashMap<_, _>>();
    let mut exposed_names = modules_by_name.keys().cloned().collect::<HashSet<_>>();
    exposed_names.extend(externals.local_type_names.values().cloned());
    let mut grouped = HashMap::<String, Vec<&JavaExternalSymbol>>::new();
    for symbol in &externals.symbols {
        if classes_by_qualified.contains_key(&symbol.qualified_name) {
            grouped
                .entry(symbol.module_path.clone())
                .or_default()
                .push(symbol);
        }
    }

    grouped
        .into_iter()
        .map(|(module_path, symbols)| {
            let span = symbols
                .first()
                .map(|symbol| symbol.span)
                .expect("grouped java library module has at least one symbol");
            let mut seen = HashSet::new();
            let mut items = symbols
                .into_iter()
                .filter(|symbol| seen.insert(symbol.lume_name.clone()))
                .filter_map(|symbol| {
                    classes_by_qualified
                        .get(&symbol.qualified_name)
                        .map(|class| {
                            Item::Type(java_library_type_decl(
                                &symbol.lume_name,
                                class,
                                &exposed_names,
                                symbol.span,
                            ))
                        })
                })
                .collect::<Vec<_>>();
            let existing_names = items
                .iter()
                .filter_map(|item| match item {
                    Item::Type(decl) => Some(decl.name.clone()),
                    _ => None,
                })
                .collect::<HashSet<_>>();
            let typecheck_only_types =
                java_library_local_placeholder_names(&module_path, externals, &existing_names);
            items.extend(java_library_local_placeholder_items(
                &typecheck_only_types,
                span,
            ));
            let imports = java_library_imports(&module_path, &items, &modules_by_name, span);
            (
                module_path.clone(),
                LibraryModule {
                    program: Program {
                        module: Some(ModuleDecl {
                            name: module_path,
                            span,
                        }),
                        imports,
                        items,
                        span: Some(span),
                    },
                    typecheck_only_types,
                },
            )
        })
        .collect()
}

fn java_library_local_placeholder_names(
    module_path: &str,
    externals: &JavaExternalSymbols,
    existing_names: &HashSet<String>,
) -> HashSet<String> {
    let package = module_path.replace('/', ".");
    let prefix = format!("{package}.");
    externals
        .local_type_names
        .iter()
        .filter_map(|(qualified, name)| {
            (qualified.starts_with(&prefix) && !existing_names.contains(name))
                .then_some(name.clone())
        })
        .collect()
}

fn java_library_local_placeholder_items(
    typecheck_only_types: &HashSet<String>,
    span: crate::source::Span,
) -> Vec<Item> {
    let mut names = typecheck_only_types.iter().cloned().collect::<Vec<_>>();
    names.sort();
    names
        .into_iter()
        .map(|name| {
            Item::Type(TypeDecl {
                annotations: Vec::new(),
                visibility: Visibility::Default,
                kind: TypeKind::Interface,
                name,
                type_params: Vec::new(),
                type_conditions: Vec::new(),
                with_bounds: Vec::new(),
                members: Vec::new(),
                span,
            })
        })
        .collect()
}

fn java_library_type_decl(
    name: &str,
    external_class: &JavaExternalClass,
    exposed_names: &HashSet<String>,
    span: crate::source::Span,
) -> TypeDecl {
    TypeDecl {
        annotations: Vec::new(),
        visibility: Visibility::Default,
        kind: external_class.kind,
        name: name.to_string(),
        type_params: external_class
            .type_params
            .iter()
            .map(|name| TypeParam {
                name: name.clone(),
                reified: false,
                bounds: Vec::new(),
                span,
            })
            .collect(),
        type_conditions: Vec::new(),
        with_bounds: external_class
            .with_bounds
            .iter()
            .filter(|bound| java_library_type_ref_is_exposed(bound, exposed_names))
            .cloned()
            .collect(),
        members: java_library_members(external_class, exposed_names, span),
        span,
    }
}

fn java_library_type_ref_is_exposed(ty: &TypeRef, exposed_names: &HashSet<String>) -> bool {
    match ty {
        TypeRef::Wildcard { .. } => true,
        TypeRef::Named { name, args, .. } => {
            java_library_name_is_exposed(name, exposed_names)
                && args
                    .iter()
                    .all(|arg| java_library_type_ref_is_exposed(arg, exposed_names))
        }
        TypeRef::Tuple { fields, .. } => fields
            .iter()
            .all(|field| java_library_type_ref_is_exposed(&field.ty, exposed_names)),
        TypeRef::Record { fields, .. } => fields
            .iter()
            .all(|field| java_library_type_ref_is_exposed(&field.ty, exposed_names)),
        TypeRef::Function { params, ret, .. } => {
            params
                .iter()
                .all(|param| java_library_type_ref_is_exposed(param, exposed_names))
                && java_library_type_ref_is_exposed(ret, exposed_names)
        }
    }
}

fn java_library_name_is_exposed(name: &str, exposed_names: &HashSet<String>) -> bool {
    matches!(
        name,
        "Any"
            | "Bool"
            | "Int"
            | "Float"
            | "Rune"
            | "Str"
            | "Unit"
            | "Never"
            | "Array"
            | "Iterator"
            | "Vector"
            | "LinkedList"
            | "Set"
            | "Map"
            | "Option"
            | "Result"
            | "Either"
    ) || exposed_names.contains(name)
}

fn java_library_imports(
    module_path: &str,
    items: &[Item],
    modules_by_name: &HashMap<String, String>,
    span: crate::source::Span,
) -> Vec<ImportDecl> {
    let mut names = HashSet::new();
    for item in items {
        collect_java_library_item_type_refs(item, &mut names);
    }

    let mut grouped = BTreeMap::<String, Vec<String>>::new();
    for name in names {
        let Some(dep_module) = modules_by_name.get(&name) else {
            continue;
        };
        if dep_module == module_path {
            continue;
        }
        grouped
            .entry(dep_module.clone())
            .or_default()
            .push(name.clone());
    }

    grouped
        .into_iter()
        .map(|(path, mut names)| {
            names.sort();
            names.dedup();
            ImportDecl {
                path,
                object_name: None,
                wildcard: false,
                symbols: names
                    .into_iter()
                    .map(|name| ImportSymbol {
                        name,
                        alias: None,
                        span,
                    })
                    .collect(),
                span,
            }
        })
        .collect()
}

fn collect_java_library_item_type_refs(item: &Item, names: &mut HashSet<String>) {
    let Item::Type(decl) = item else {
        if let Item::Extension(block) = item {
            collect_java_library_type_ref(&block.target, names);
            for method in &block.methods {
                collect_java_library_method_type_refs(method, names);
            }
        }
        return;
    };
    for bound in &decl.with_bounds {
        collect_java_library_type_ref(bound, names);
    }
    for param in &decl.type_params {
        for bound in &param.bounds {
            collect_java_library_type_ref(bound, names);
        }
    }
    for member in &decl.members {
        match member {
            TypeMember::Field(field) => {
                if let Some(ty) = &field.ty {
                    collect_java_library_type_ref(ty, names);
                }
            }
            TypeMember::Method(method) => collect_java_library_method_type_refs(method, names),
            TypeMember::Case(case) => {
                for field in &case.fields {
                    if let Some(ty) = &field.ty {
                        collect_java_library_type_ref(ty, names);
                    }
                }
            }
        }
    }
}

fn collect_java_library_method_type_refs(method: &MethodDecl, names: &mut HashSet<String>) {
    for type_param in &method.type_params {
        for bound in &type_param.bounds {
            collect_java_library_type_ref(bound, names);
        }
    }
    for param in &method.params {
        if let Some(ty) = &param.ty {
            collect_java_library_type_ref(ty, names);
        }
    }
    if let Some(ret) = &method.return_type {
        collect_java_library_type_ref(ret, names);
    }
}

fn collect_java_library_type_ref(ty: &TypeRef, names: &mut HashSet<String>) {
    match ty {
        TypeRef::Wildcard { .. } => {}
        TypeRef::Named { name, args, .. } => {
            names.insert(name.clone());
            for arg in args {
                collect_java_library_type_ref(arg, names);
            }
        }
        TypeRef::Tuple { fields, .. } => {
            for field in fields {
                collect_java_library_type_ref(&field.ty, names);
            }
        }
        TypeRef::Record { fields, .. } => {
            for field in fields {
                collect_java_library_type_ref(&field.ty, names);
            }
        }
        TypeRef::Function { params, ret, .. } => {
            for param in params {
                collect_java_library_type_ref(param, names);
            }
            collect_java_library_type_ref(ret, names);
        }
    }
}

fn java_library_members(
    external_class: &JavaExternalClass,
    exposed_names: &HashSet<String>,
    span: crate::source::Span,
) -> Vec<TypeMember> {
    let mut class_exposed_names = exposed_names.clone();
    class_exposed_names.extend(external_class.type_params.iter().cloned());

    if external_class.kind == TypeKind::Annotation {
        return external_class
            .methods
            .iter()
            .map(|method| sanitize_java_callable_for_library(method, &class_exposed_names, span))
            .filter_map(|method| java_library_annotation_field(&method, span))
            .collect();
    }
    let fields = external_class
        .fields
        .iter()
        .map(|field| java_library_field(field, &class_exposed_names, span));

    fields
        .chain(
            external_class
                .constructors
                .iter()
                .map(|constructor| {
                    let constructor =
                        sanitize_java_callable_for_library(constructor, &class_exposed_names, span);
                    java_library_method("new", &constructor, None, span)
                })
                .chain(external_class.methods.iter().map(|method| {
                    let mut method_exposed_names = class_exposed_names.clone();
                    method_exposed_names.extend(method.type_params.iter().cloned());
                    let method =
                        sanitize_java_callable_for_library(method, &method_exposed_names, span);
                    java_library_method(
                        method.name.as_str(),
                        &method,
                        method.return_type.clone(),
                        span,
                    )
                })),
        )
        .collect()
}

fn sanitize_java_callable_for_library(
    callable: &JavaExternalCallable,
    exposed_names: &HashSet<String>,
    span: crate::source::Span,
) -> JavaExternalCallable {
    JavaExternalCallable {
        name: callable.name.clone(),
        type_params: callable.type_params.clone(),
        reified_type_params: callable.reified_type_params.clone(),
        params: callable
            .params
            .iter()
            .map(|param| JavaExternalParam {
                name: param.name.clone(),
                ty: param
                    .ty
                    .as_ref()
                    .map(|ty| sanitize_java_type_ref_for_library(ty, exposed_names, span)),
                variadic: param.variadic,
                coercion: param.coercion,
            })
            .collect(),
        return_type: callable
            .return_type
            .as_ref()
            .map(|ty| sanitize_java_type_ref_for_library(ty, exposed_names, span)),
    }
}

fn sanitize_java_type_ref_for_library(
    ty: &TypeRef,
    exposed_names: &HashSet<String>,
    fallback_span: crate::source::Span,
) -> TypeRef {
    match ty {
        TypeRef::Wildcard { span } => TypeRef::Wildcard { span: *span },
        TypeRef::Named { name, args, span } => {
            if !java_library_name_is_exposed(name, exposed_names) {
                return TypeRef::Named {
                    name: "Any".to_string(),
                    args: Vec::new(),
                    span: *span,
                };
            }
            TypeRef::Named {
                name: name.clone(),
                args: args
                    .iter()
                    .map(|arg| {
                        sanitize_java_type_ref_for_library(arg, exposed_names, fallback_span)
                    })
                    .collect(),
                span: *span,
            }
        }
        TypeRef::Tuple { fields, span } => TypeRef::Tuple {
            fields: fields
                .iter()
                .map(|field| crate::ast::TupleTypeField {
                    ty: sanitize_java_type_ref_for_library(&field.ty, exposed_names, fallback_span),
                    span: field.span,
                })
                .collect(),
            span: *span,
        },
        TypeRef::Record { fields, span } => TypeRef::Record {
            fields: fields
                .iter()
                .map(|field| crate::ast::RecordTypeField {
                    name: field.name.clone(),
                    ty: sanitize_java_type_ref_for_library(&field.ty, exposed_names, fallback_span),
                    span: field.span,
                })
                .collect(),
            span: *span,
        },
        TypeRef::Function { params, ret, span } => TypeRef::Function {
            params: params
                .iter()
                .map(|param| {
                    sanitize_java_type_ref_for_library(param, exposed_names, fallback_span)
                })
                .collect(),
            ret: Box::new(sanitize_java_type_ref_for_library(
                ret,
                exposed_names,
                fallback_span,
            )),
            span: *span,
        },
    }
}

fn java_library_annotation_field(
    callable: &JavaExternalCallable,
    span: crate::source::Span,
) -> Option<TypeMember> {
    if !callable.params.is_empty() {
        return None;
    }
    Some(TypeMember::Field(FieldDecl {
        annotations: Vec::new(),
        visibility: Visibility::Default,
        mutable: false,
        name: callable.name.clone(),
        ty: callable.return_type.clone(),
        initializer: None,
        span,
    }))
}

fn java_library_field(
    field: &JavaExternalField,
    exposed_names: &HashSet<String>,
    span: crate::source::Span,
) -> TypeMember {
    TypeMember::Field(FieldDecl {
        annotations: Vec::new(),
        visibility: Visibility::Default,
        mutable: false,
        name: field.name.clone(),
        ty: field
            .ty
            .as_ref()
            .map(|ty| sanitize_java_type_ref_for_library(ty, exposed_names, span)),
        initializer: field.initializer.clone(),
        span,
    })
}

fn java_library_default_initializer_marker(
    ty: Option<&TypeRef>,
    exposed_names: &HashSet<String>,
    span: crate::source::Span,
) -> crate::ast::Expr {
    let ty = ty.map(|ty| sanitize_java_type_ref_for_library(ty, exposed_names, span));
    match ty.as_ref() {
        Some(TypeRef::Named { name, .. }) if name == "Bool" => {
            crate::ast::Expr::Bool { value: false, span }
        }
        Some(TypeRef::Named { name, .. }) if matches!(name.as_str(), "Int" | "Rune") => {
            crate::ast::Expr::Integer {
                raw: "0".to_string(),
                span,
            }
        }
        Some(TypeRef::Named { name, .. }) if name == "Float" => crate::ast::Expr::Float {
            raw: "0.0".to_string(),
            span,
        },
        Some(TypeRef::Named { name, .. }) if name == "Unit" => crate::ast::Expr::Unit { span },
        _ => crate::ast::Expr::String {
            raw: String::new(),
            span,
        },
    }
}

fn java_library_method(
    name: &str,
    callable: &JavaExternalCallable,
    return_type: Option<TypeRef>,
    span: crate::source::Span,
) -> TypeMember {
    let reified_type_params = callable
        .reified_type_params
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    TypeMember::Method(MethodDecl {
        annotations: Vec::new(),
        visibility: Visibility::Default,
        name: name.to_string(),
        type_params: callable
            .type_params
            .iter()
            .map(|name| TypeParam {
                name: name.clone(),
                reified: reified_type_params.contains(name.as_str()),
                bounds: Vec::new(),
                span,
            })
            .collect(),
        type_conditions: Vec::new(),
        params: callable
            .params
            .iter()
            .map(|param| Param {
                name: param.name.clone(),
                ty: param.ty.clone(),
                initializer: None,
                variadic: param.variadic,
                lazy: false,
                span,
            })
            .collect(),
        return_type,
        body: None,
        span,
    })
}

fn resolve_external_classes(
    externals: &JavaExternalSymbols,
    options: &JavaBackendOptions,
) -> Result<ExternalClassResolution, String> {
    let classpath_entries = effective_java_classpath(options);
    let index = JavaClasspathIndex::from_entries(&classpath_entries)?;
    let classpath = java_classpath(&classpath_entries)?;
    let mut local_type_names = externals.local_type_names.clone();
    local_type_names.extend(
        externals
            .symbols
            .iter()
            .map(|symbol| (symbol.qualified_name.clone(), symbol.lume_name.clone())),
    );
    let mut diagnostics = Vec::new();
    let mut classes_by_qualified = HashMap::new();
    let mut seen = HashSet::new();
    for symbol in &externals.symbols {
        if !seen.insert(symbol.qualified_name.clone()) {
            continue;
        }
        if !index.could_contain(&symbol.qualified_name) {
            diagnostics.push(missing_java_class_diagnostic(symbol));
            continue;
        }
        let Some(descriptor) = inspect_java_class(
            classpath.as_deref(),
            &symbol.qualified_name,
            &local_type_names,
            symbol.span,
        )?
        else {
            diagnostics.push(missing_java_class_diagnostic(symbol));
            continue;
        };
        classes_by_qualified.insert(symbol.qualified_name.clone(), descriptor.class);
    }
    load_inherited_java_classes(
        &mut classes_by_qualified,
        &mut local_type_names,
        classpath.as_deref(),
        &index,
    )?;
    flatten_inherited_java_methods(&mut classes_by_qualified, &local_type_names);
    let classes = externals
        .symbols
        .iter()
        .filter_map(|symbol| {
            classes_by_qualified
                .get(&symbol.qualified_name)
                .cloned()
                .map(|class| (symbol.lume_name.clone(), class))
        })
        .collect::<HashMap<_, _>>();
    let library_modules = java_library_modules(externals, &classes_by_qualified);
    Ok(ExternalClassResolution {
        diagnostics,
        library_modules,
        classes,
    })
}

fn load_inherited_java_classes(
    classes: &mut HashMap<String, JavaExternalClass>,
    local_type_names: &mut HashMap<String, String>,
    classpath: Option<&std::ffi::OsStr>,
    index: &JavaClasspathIndex,
) -> Result<(), String> {
    let mut inspected = classes.keys().cloned().collect::<HashSet<_>>();
    let mut queue = classes
        .values()
        .flat_map(|class| class.inherit_qualified_names.iter().cloned())
        .collect::<Vec<_>>();

    while let Some(qualified_name) = queue.pop() {
        if !inspected.insert(qualified_name.clone()) {
            continue;
        }
        if !index.could_contain(&qualified_name) {
            continue;
        }

        local_type_names
            .entry(qualified_name.clone())
            .or_insert_with(|| java_simple_name(&qualified_name).to_string());

        let Some(descriptor) = inspect_java_class(
            classpath,
            &qualified_name,
            local_type_names,
            synthetic_java_span(),
        )?
        else {
            continue;
        };

        queue.extend(descriptor.class.inherit_qualified_names.iter().cloned());
        classes.insert(qualified_name, descriptor.class);
    }

    Ok(())
}

fn flatten_inherited_java_methods(
    classes: &mut HashMap<String, JavaExternalClass>,
    local_type_names: &HashMap<String, String>,
) {
    let by_local_name = local_type_names
        .iter()
        .map(|(qualified, local)| (local.clone(), qualified.clone()))
        .collect::<HashMap<_, _>>();
    let snapshot = classes.clone();
    for class in classes.values_mut() {
        let mut seen = HashSet::new();
        let inherited = inherited_java_methods(class, &snapshot, &by_local_name, &mut seen);
        class.methods.extend(inherited);
    }
}

fn inherited_java_methods(
    class: &JavaExternalClass,
    classes: &HashMap<String, JavaExternalClass>,
    by_local_name: &HashMap<String, String>,
    seen: &mut HashSet<String>,
) -> Vec<JavaExternalCallable> {
    let mut methods = Vec::new();
    for bound in &class.with_bounds {
        let TypeRef::Named { name, args, .. } = bound else {
            continue;
        };
        let Some(qualified_name) = by_local_name.get(name) else {
            continue;
        };
        if !seen.insert(qualified_name.clone()) {
            continue;
        }
        let Some(bound_class) = classes.get(qualified_name) else {
            continue;
        };
        let subst = bound_class
            .type_params
            .iter()
            .cloned()
            .zip(args.iter().cloned())
            .collect::<HashMap<_, _>>();
        methods.extend(
            bound_class
                .methods
                .iter()
                .map(|method| substitute_java_callable(method, &subst)),
        );
        methods.extend(
            inherited_java_methods(bound_class, classes, by_local_name, seen)
                .into_iter()
                .map(|method| substitute_java_callable(&method, &subst)),
        );
    }
    methods
}

fn substitute_java_callable(
    callable: &JavaExternalCallable,
    subst: &HashMap<String, TypeRef>,
) -> JavaExternalCallable {
    JavaExternalCallable {
        name: callable.name.clone(),
        type_params: callable.type_params.clone(),
        reified_type_params: callable.reified_type_params.clone(),
        params: callable
            .params
            .iter()
            .map(|param| JavaExternalParam {
                name: param.name.clone(),
                ty: param
                    .ty
                    .as_ref()
                    .map(|ty| substitute_java_type_ref(ty, subst)),
                variadic: param.variadic,
                coercion: param.coercion,
            })
            .collect(),
        return_type: callable
            .return_type
            .as_ref()
            .map(|ty| substitute_java_type_ref(ty, subst)),
    }
}

fn substitute_java_type_ref(ty: &TypeRef, subst: &HashMap<String, TypeRef>) -> TypeRef {
    match ty {
        TypeRef::Wildcard { span } => TypeRef::Wildcard { span: *span },
        TypeRef::Named { name, args, span } if args.is_empty() => {
            subst.get(name).cloned().unwrap_or_else(|| TypeRef::Named {
                name: name.clone(),
                args: Vec::new(),
                span: *span,
            })
        }
        TypeRef::Named { name, args, span } => TypeRef::Named {
            name: name.clone(),
            args: args
                .iter()
                .map(|arg| substitute_java_type_ref(arg, subst))
                .collect(),
            span: *span,
        },
        TypeRef::Tuple { fields, span } => TypeRef::Tuple {
            fields: fields
                .iter()
                .map(|field| crate::ast::TupleTypeField {
                    ty: substitute_java_type_ref(&field.ty, subst),
                    span: field.span,
                })
                .collect(),
            span: *span,
        },
        TypeRef::Record { fields, span } => TypeRef::Record {
            fields: fields
                .iter()
                .map(|field| crate::ast::RecordTypeField {
                    name: field.name.clone(),
                    ty: substitute_java_type_ref(&field.ty, subst),
                    span: field.span,
                })
                .collect(),
            span: *span,
        },
        TypeRef::Function { params, ret, span } => TypeRef::Function {
            params: params
                .iter()
                .map(|param| substitute_java_type_ref(param, subst))
                .collect(),
            ret: Box::new(substitute_java_type_ref(ret, subst)),
            span: *span,
        },
    }
}

fn missing_java_class_diagnostic(symbol: &JavaExternalSymbol) -> LocatedDiagnostic {
    LocatedDiagnostic {
        path: symbol.source_path.clone(),
        diagnostic: Diagnostic::error(
            "missing_java_class",
            format!(
                "Java class '{}' is not available on the provided classpath",
                symbol.qualified_name
            ),
            symbol.span,
        )
        .with_label("class imported here")
        .with_help("add the jar or classes directory with --classpath <path>"),
    }
}

#[derive(Debug, Clone, Default)]
struct JavaClasspathIndex {
    classes: HashSet<String>,
    indexed_entries: bool,
}

impl JavaClasspathIndex {
    fn from_entries(entries: &[PathBuf]) -> Result<Self, String> {
        let mut index = Self::default();
        for entry in entries {
            index.indexed_entries = true;
            if entry.is_dir() {
                index_class_dir(entry, entry, &mut index.classes)?;
            } else if entry.extension().is_some_and(|ext| ext == "jar") {
                index_jar(entry, &mut index.classes)?;
            }
        }
        Ok(index)
    }

    fn could_contain(&self, qualified_name: &str) -> bool {
        !self.indexed_entries
            || qualified_name.starts_with("java.")
            || self.classes.contains(qualified_name)
    }
}

#[derive(Debug, Clone)]
struct JavaClassDescriptor {
    class: JavaExternalClass,
}

fn inspect_java_class(
    classpath: Option<&std::ffi::OsStr>,
    qualified_name: &str,
    local_type_names: &HashMap<String, String>,
    span: crate::source::Span,
) -> Result<Option<JavaClassDescriptor>, String> {
    let mut command = Command::new("javap");
    if let Some(classpath) = classpath {
        command.arg("-classpath").arg(classpath);
    }
    let output = command
        .arg("-private")
        .arg("-constants")
        .arg(qualified_name)
        .output()
        .map_err(|err| format!("run javap to inspect Java classpath: {err}"))?;
    if !output.status.success() {
        return Ok(None);
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    if !javap_declares_public_type(&stdout, qualified_name) {
        return Ok(None);
    }
    Ok(Some(JavaClassDescriptor {
        class: parse_javap_class(&stdout, qualified_name, local_type_names, span),
    }))
}

fn javap_declares_public_type(output: &str, qualified_name: &str) -> bool {
    output.lines().any(|line| {
        let line = line.trim();
        line.starts_with("public ")
            && (line.contains(" class ") || line.contains(" interface ") || line.contains(" enum "))
            && line.contains(qualified_name)
    })
}

fn java_classpath(entries: &[PathBuf]) -> Result<Option<std::ffi::OsString>, String> {
    if entries.is_empty() {
        return Ok(None);
    }
    env::join_paths(entries)
        .map(Some)
        .map_err(|err| format!("build Java classpath: {err}"))
}

fn effective_java_classpath(options: &JavaBackendOptions) -> Vec<PathBuf> {
    let mut entries = options.classpath.clone();
    if let Some(core_jar) = discover_lume_core_jar() {
        push_unique_classpath_entry(&mut entries, core_jar);
    }
    entries
}

fn push_unique_classpath_entry(entries: &mut Vec<PathBuf>, entry: PathBuf) {
    let normalized = entry.canonicalize().unwrap_or(entry);
    let already_present = entries.iter().any(|existing| {
        existing
            .canonicalize()
            .map(|path| path == normalized)
            .unwrap_or_else(|_| existing == &normalized)
    });
    if !already_present {
        entries.push(normalized);
    }
}

fn discover_lume_core_jar() -> Option<PathBuf> {
    if let Some(path) = env::var_os("LUME_CORE_JAR").map(PathBuf::from) {
        if path.is_file() {
            return Some(path);
        }
    }

    let mut candidates = Vec::new();
    if let Ok(current_dir) = env::current_dir() {
        candidates.extend(lume_core_candidates_from_ancestors(&current_dir));
    }
    if let Ok(exe) = env::current_exe() {
        if let Some(parent) = exe.parent() {
            candidates.push(parent.join("lume-core.jar"));
            if let Some(bin_parent) = parent.parent() {
                candidates.push(bin_parent.join("lib/lume-core.jar"));
            }
            candidates.extend(lume_core_candidates_from_ancestors(parent));
        }
    }
    candidates.push(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../lume/core/build/libs/lume-core.jar"),
    );

    candidates.into_iter().find(|path| path.is_file())
}

fn lume_core_candidates_from_ancestors(path: &Path) -> Vec<PathBuf> {
    path.ancestors()
        .map(|ancestor| ancestor.join("lume/core/build/libs/lume-core.jar"))
        .collect()
}

fn parse_javap_type_params(output: &str, qualified_name: &str) -> Vec<String> {
    let prefix = format!("{qualified_name}<");
    let Some(line) = output
        .lines()
        .find(|line| line.contains(" class ") || line.contains(" interface "))
    else {
        return Vec::new();
    };
    let Some(start) = line.find(&prefix).map(|index| index + prefix.len()) else {
        return Vec::new();
    };
    let Some(end) = line[start..].find('>').map(|index| start + index) else {
        return Vec::new();
    };
    line[start..end]
        .split(',')
        .filter_map(|param| {
            param
                .split_whitespace()
                .next()
                .filter(|name| !name.is_empty())
                .map(str::to_string)
        })
        .collect()
}

enum ParsedJavaCallable {
    Constructor(JavaExternalCallable),
    Method(JavaExternalCallable),
}

#[derive(Clone)]
struct JavaTypeContext<'a> {
    local_type_names: &'a HashMap<String, String>,
    type_params: HashSet<String>,
    current_package: &'a str,
    allow_cross_package_refs: bool,
    span: crate::source::Span,
}

fn parse_javap_class(
    output: &str,
    qualified_name: &str,
    local_type_names: &HashMap<String, String>,
    span: crate::source::Span,
) -> JavaExternalClass {
    let type_params = parse_javap_type_params(output, qualified_name);
    let lume_generated = javap_lume_generated(output);
    let header = output
        .lines()
        .find(|line| line.contains(" class ") || line.contains(" interface "))
        .unwrap_or_default();
    let kind = if lume_generated {
        parse_javap_lume_kind(output).unwrap_or_else(|| {
            if output.contains("extends java.lang.annotation.Annotation") {
                crate::ast::TypeKind::Annotation
            } else if header.contains(" interface ") {
                crate::ast::TypeKind::Interface
            } else {
                crate::ast::TypeKind::Class
            }
        })
    } else if output.contains("extends java.lang.annotation.Annotation") {
        TypeKind::Annotation
    } else if header.contains(" interface ") {
        TypeKind::Interface
    } else {
        TypeKind::Class
    };
    let current_package = java_package_name(qualified_name);
    let ctx = JavaTypeContext {
        local_type_names,
        type_params: type_params.iter().cloned().collect(),
        current_package,
        allow_cross_package_refs: false,
        span,
    };
    let bounds_ctx = JavaTypeContext {
        allow_cross_package_refs: true,
        ..ctx.clone()
    };
    let with_bounds = parse_javap_bounds(header, kind, &bounds_ctx);
    let default_fields = parse_javap_lume_default_fields(output);
    let default_values = parse_javap_lume_default_field_values(output, span);
    let fields = if lume_generated && kind == TypeKind::Record {
        parse_javap_record_fields(output, &ctx, &default_fields, &default_values, span)
    } else {
        Vec::new()
    };
    let mut constructors = Vec::new();
    let mut methods = Vec::new();

    for line in output.lines() {
        let ctx = JavaTypeContext {
            local_type_names,
            type_params: type_params.iter().cloned().collect(),
            current_package,
            allow_cross_package_refs: false,
            span,
        };
        match parse_javap_callable_line(line, qualified_name, ctx, javap_lume_generated(output)) {
            Some(ParsedJavaCallable::Constructor(constructor)) => constructors.push(constructor),
            Some(ParsedJavaCallable::Method(method)) => methods.push(method),
            None => {}
        }
    }
    if lume_generated && kind == TypeKind::Record {
        constructors.clear();
    }
    if lume_generated {
        methods.retain(|method| method.name != "runtimeType");
    }
    if kind == TypeKind::Record && !fields.is_empty() {
        let field_names = fields
            .iter()
            .map(|field| field.name.as_str())
            .collect::<HashSet<_>>();
        methods.retain(|method| {
            !(method.params.is_empty() && field_names.contains(method.name.as_str()))
        });
    }

    JavaExternalClass {
        qualified_name: qualified_name.to_string(),
        kind,
        type_params,
        with_bounds,
        inherit_qualified_names: parse_javap_bound_qualified_names(header, kind),
        fields,
        constructors,
        methods,
    }
}

fn parse_javap_lume_kind(output: &str) -> Option<TypeKind> {
    match parse_javap_lume_string_constant(output, "LUME_KIND")?.as_str() {
        "annotation" => Some(TypeKind::Annotation),
        "class" => Some(TypeKind::Class),
        "shape" => Some(TypeKind::Record),
        "object" => Some(TypeKind::Object),
        "interface" => Some(TypeKind::Interface),
        "enum" => Some(TypeKind::Enum),
        _ => None,
    }
}

fn parse_javap_lume_default_fields(output: &str) -> HashSet<String> {
    parse_javap_lume_string_constant(output, "LUME_DEFAULT_FIELDS")
        .unwrap_or_default()
        .split(',')
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .collect()
}

fn parse_javap_lume_default_field_values(
    output: &str,
    span: crate::source::Span,
) -> HashMap<String, crate::ast::Expr> {
    parse_javap_lume_string_constant(output, "LUME_DEFAULT_FIELD_VALUES")
        .unwrap_or_default()
        .lines()
        .filter_map(|line| {
            let mut parts = line.splitn(3, '\t');
            let name = metadata_unescape(parts.next()?);
            let tag = parts.next()?;
            let value = metadata_unescape(parts.next().unwrap_or_default());
            parse_lume_default_metadata_expr(tag, &value, span).map(|expr| (name, expr))
        })
        .collect()
}

fn parse_lume_default_metadata_expr(
    tag: &str,
    value: &str,
    span: crate::source::Span,
) -> Option<crate::ast::Expr> {
    match tag {
        "unit" => Some(crate::ast::Expr::Unit { span }),
        "bool" => Some(crate::ast::Expr::Bool {
            value: value == "true",
            span,
        }),
        "int" => Some(crate::ast::Expr::Integer {
            raw: value.to_string(),
            span,
        }),
        "float" => Some(crate::ast::Expr::Float {
            raw: value.to_string(),
            span,
        }),
        "str" => Some(crate::ast::Expr::String {
            raw: value.to_string(),
            span,
        }),
        _ => None,
    }
}

fn parse_javap_lume_string_constant(output: &str, name: &str) -> Option<String> {
    let marker = format!("public static final java.lang.String {name} = ");
    output.lines().find_map(|line| {
        let value = line.trim().strip_prefix(&marker)?.strip_suffix(';')?.trim();
        parse_java_string_constant(value)
    })
}

fn parse_java_string_constant(value: &str) -> Option<String> {
    let body = value.strip_prefix('"')?.strip_suffix('"')?;
    let mut out = String::new();
    let mut chars = body.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        let escaped = chars.next()?;
        match escaped {
            '\\' => out.push('\\'),
            '"' => out.push('"'),
            'n' => out.push('\n'),
            'r' => out.push('\r'),
            't' => out.push('\t'),
            other => out.push(other),
        }
    }
    Some(out)
}

fn metadata_unescape(value: &str) -> String {
    let mut out = String::new();
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('\\') => out.push('\\'),
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

fn parse_javap_record_fields(
    output: &str,
    ctx: &JavaTypeContext<'_>,
    default_fields: &HashSet<String>,
    default_values: &HashMap<String, crate::ast::Expr>,
    span: crate::source::Span,
) -> Vec<JavaExternalField> {
    output
        .lines()
        .filter_map(|line| {
            parse_javap_record_field_line(line, ctx, default_fields, default_values, span)
        })
        .collect()
}

fn parse_javap_record_field_line(
    line: &str,
    ctx: &JavaTypeContext<'_>,
    default_fields: &HashSet<String>,
    default_values: &HashMap<String, crate::ast::Expr>,
    span: crate::source::Span,
) -> Option<JavaExternalField> {
    let line = line.trim().strip_suffix(';')?.trim();
    let rest = line.strip_prefix("private final ")?;
    if rest.contains('(') {
        return None;
    }
    let (raw_ty, raw_name) = split_java_return_and_name(rest)?;
    let name = java_method_name_to_lume(raw_name);
    let ty = java_type_to_lume_type_ref(raw_ty, ctx);
    let initializer = default_values.get(&name).cloned().or_else(|| {
        default_fields
            .contains(&name)
            .then(|| java_library_default_initializer_marker(ty.as_ref(), &HashSet::new(), span))
    });
    Some(JavaExternalField {
        name,
        ty,
        initializer,
    })
}

fn parse_javap_bound_qualified_names(header: &str, kind: crate::ast::TypeKind) -> Vec<String> {
    let header = header.trim().strip_suffix('{').unwrap_or(header).trim();
    let bounds = match kind {
        crate::ast::TypeKind::Class => header
            .split_once(" implements ")
            .map(|(_, rest)| rest)
            .unwrap_or_default(),
        crate::ast::TypeKind::Interface => header
            .split_once(" extends ")
            .map(|(_, rest)| rest)
            .unwrap_or_default(),
        crate::ast::TypeKind::Annotation => "",
        _ => "",
    };

    split_java_signature_list(bounds)
        .into_iter()
        .filter_map(|bound| {
            let (base, _) = split_java_generic_type(bound);
            base.contains('.').then(|| base.to_string())
        })
        .collect()
}

fn parse_javap_bounds(
    header: &str,
    kind: crate::ast::TypeKind,
    ctx: &JavaTypeContext<'_>,
) -> Vec<TypeRef> {
    let header = header.trim().strip_suffix('{').unwrap_or(header).trim();
    let bounds = match kind {
        crate::ast::TypeKind::Class => header
            .split_once(" implements ")
            .map(|(_, rest)| rest)
            .unwrap_or_default(),
        crate::ast::TypeKind::Interface => header
            .split_once(" extends ")
            .map(|(_, rest)| rest)
            .unwrap_or_default(),
        crate::ast::TypeKind::Annotation => "",
        _ => "",
    };
    split_java_signature_list(bounds)
        .into_iter()
        .filter_map(|bound| java_type_to_lume_type_ref(bound, ctx))
        .filter(|bound| {
            !matches!(bound, TypeRef::Named { name, args, .. } if is_lume_builtin_java_bound(name, args))
        })
        .collect()
}

fn is_lume_builtin_java_bound(name: &str, args: &[TypeRef]) -> bool {
    args.is_empty()
        && matches!(
            name,
            "Any" | "Bool" | "Int" | "Float" | "Rune" | "Str" | "Unit"
        )
        || matches!(name, "Vector" | "LinkedList" | "Set" | "Map" | "Option")
}

fn parse_javap_callable_line(
    line: &str,
    qualified_name: &str,
    mut ctx: JavaTypeContext<'_>,
    lume_generated: bool,
) -> Option<ParsedJavaCallable> {
    let line = line.trim().strip_suffix(';')?.trim();
    let line = line.strip_prefix("public ")?;
    let open = line.find('(')?;
    let close = line.rfind(')')?;
    if close < open {
        return None;
    }

    let mut before = strip_java_modifiers(line[..open].trim());
    let (method_type_params, rest) = strip_leading_java_generic_decl(before);
    before = strip_java_modifiers(rest);
    ctx.type_params.extend(method_type_params.iter().cloned());

    let raw_param_types = split_java_signature_list(&line[open + 1..close]);
    let mut params = parse_javap_params(&raw_param_types, &ctx);
    let reified_type_params = strip_lume_reified_evidence_params(
        &mut params,
        &raw_param_types,
        &method_type_params,
        lume_generated,
    );
    if java_constructor_name_matches(before, qualified_name) {
        return Some(ParsedJavaCallable::Constructor(JavaExternalCallable {
            name: "new".to_string(),
            type_params: method_type_params,
            reified_type_params,
            params,
            return_type: None,
        }));
    }

    let (return_ty, name) = split_java_return_and_name(before)?;
    let return_type = java_type_to_lume_type_ref(return_ty, &ctx).or_else(|| {
        (return_ty == "void").then(|| TypeRef::Named {
            name: "Unit".to_string(),
            args: Vec::new(),
            span: ctx.span,
        })
    });
    Some(ParsedJavaCallable::Method(JavaExternalCallable {
        name: java_method_name_to_lume(name),
        type_params: method_type_params,
        reified_type_params,
        params,
        return_type,
    }))
}

fn javap_lume_generated(output: &str) -> bool {
    output
        .lines()
        .any(|line| line.trim() == "public static final lume.core.LumeType TYPE;")
}

fn strip_lume_reified_evidence_params(
    params: &mut Vec<JavaExternalParam>,
    raw_param_types: &[&str],
    type_params: &[String],
    lume_generated: bool,
) -> Vec<String> {
    if !lume_generated || type_params.is_empty() || raw_param_types.is_empty() {
        return Vec::new();
    }

    let evidence_count = raw_param_types
        .iter()
        .rev()
        .take_while(|param| java_type_erases_to_lume_type(param))
        .count();
    if evidence_count == 0 || evidence_count > type_params.len() || evidence_count > params.len() {
        return Vec::new();
    }

    params.truncate(params.len() - evidence_count);
    type_params[type_params.len() - evidence_count..].to_vec()
}

fn java_type_erases_to_lume_type(raw: &str) -> bool {
    let (base, _) = split_java_generic_type(raw);
    matches!(base.trim(), "lume.core.LumeType" | "LumeType")
}

fn java_method_name_to_lume(name: &str) -> String {
    match name {
        "toString" => "toStr".to_string(),
        _ => name
            .strip_suffix('_')
            .filter(|base| is_java_reserved(base))
            .unwrap_or(name)
            .to_string(),
    }
}

fn is_java_reserved(name: &str) -> bool {
    matches!(
        name,
        "abstract"
            | "assert"
            | "boolean"
            | "break"
            | "byte"
            | "case"
            | "catch"
            | "char"
            | "class"
            | "const"
            | "continue"
            | "default"
            | "do"
            | "double"
            | "else"
            | "enum"
            | "extends"
            | "final"
            | "finally"
            | "float"
            | "for"
            | "goto"
            | "if"
            | "implements"
            | "import"
            | "instanceof"
            | "int"
            | "interface"
            | "long"
            | "native"
            | "new"
            | "package"
            | "private"
            | "protected"
            | "public"
            | "return"
            | "short"
            | "static"
            | "strictfp"
            | "super"
            | "switch"
            | "synchronized"
            | "this"
            | "throw"
            | "throws"
            | "transient"
            | "try"
            | "void"
            | "volatile"
            | "while"
    )
}

fn parse_javap_params(params: &[&str], ctx: &JavaTypeContext<'_>) -> Vec<JavaExternalParam> {
    if params.is_empty() {
        return Vec::new();
    }
    params
        .iter()
        .enumerate()
        .map(|(index, raw)| {
            let raw = raw.trim();
            let variadic = raw.ends_with("...");
            let raw_ty = raw.strip_suffix("...").map(str::trim).unwrap_or(raw);
            let ty = java_type_to_lume_type_ref(raw_ty, ctx).map(|ty| {
                if variadic {
                    TypeRef::Named {
                        name: "Vector".to_string(),
                        args: vec![ty],
                        span: ctx.span,
                    }
                } else {
                    ty
                }
            });
            JavaExternalParam {
                name: format!("arg{index}"),
                ty,
                variadic,
                coercion: java_primitive_coercion(raw_ty),
            }
        })
        .collect()
}

fn java_primitive_coercion(raw_ty: &str) -> Option<JavaPrimitiveCoercion> {
    match raw_ty {
        "byte" | "java.lang.Byte" | "Byte" => Some(JavaPrimitiveCoercion::Byte),
        "short" | "java.lang.Short" | "Short" => Some(JavaPrimitiveCoercion::Short),
        "int" | "java.lang.Integer" | "Integer" => Some(JavaPrimitiveCoercion::Int),
        "float" | "java.lang.Float" | "Float" => Some(JavaPrimitiveCoercion::Float),
        _ => None,
    }
}

fn strip_java_modifiers(mut value: &str) -> &str {
    loop {
        let trimmed = value.trim_start();
        let Some((head, rest)) = split_first_word(trimmed) else {
            return trimmed;
        };
        if matches!(
            head,
            "abstract"
                | "default"
                | "final"
                | "native"
                | "static"
                | "strictfp"
                | "synchronized"
                | "transient"
        ) {
            value = rest;
        } else {
            return trimmed;
        }
    }
}

fn split_first_word(value: &str) -> Option<(&str, &str)> {
    let value = value.trim_start();
    if value.is_empty() {
        return None;
    }
    let end = value
        .char_indices()
        .find_map(|(index, ch)| ch.is_whitespace().then_some(index))
        .unwrap_or(value.len());
    Some((&value[..end], value[end..].trim_start()))
}

fn strip_leading_java_generic_decl(value: &str) -> (Vec<String>, &str) {
    let value = value.trim_start();
    if !value.starts_with('<') {
        return (Vec::new(), value);
    }
    let Some(end) = find_matching_angle(value, 0) else {
        return (Vec::new(), value);
    };
    let params = split_java_signature_list(&value[1..end])
        .into_iter()
        .filter_map(|param| {
            param
                .trim()
                .split_whitespace()
                .next()
                .filter(|name| !name.is_empty())
                .map(str::to_string)
        })
        .collect();
    (params, value[end + 1..].trim_start())
}

fn java_constructor_name_matches(value: &str, qualified_name: &str) -> bool {
    let simple_name = qualified_name.rsplit('.').next().unwrap_or(qualified_name);
    let erased = erase_java_generic_suffix(value.trim());
    erased == qualified_name || erased == simple_name
}

fn split_java_return_and_name(value: &str) -> Option<(&str, &str)> {
    let mut depth = 0usize;
    let mut split = None;
    for (index, ch) in value.char_indices() {
        match ch {
            '<' => depth += 1,
            '>' => depth = depth.saturating_sub(1),
            ch if ch.is_whitespace() && depth == 0 => split = Some(index),
            _ => {}
        }
    }
    let split = split?;
    let return_ty = value[..split].trim();
    let name = value[split..].trim();
    (!return_ty.is_empty() && !name.is_empty()).then_some((return_ty, name))
}

fn java_type_to_lume_type_ref(src: &str, ctx: &JavaTypeContext<'_>) -> Option<TypeRef> {
    let mut src = src.trim();
    while let Some(rest) = src.strip_prefix("final ") {
        src = rest.trim_start();
    }
    if let Some(rest) = src.strip_prefix("? extends ") {
        return java_type_to_lume_type_ref(rest, ctx);
    }
    if let Some(rest) = src.strip_prefix("? super ") {
        return java_type_to_lume_type_ref(rest, ctx);
    }
    if src == "?" {
        return Some(any_type_ref(ctx));
    }

    let mut array_depth = 0usize;
    while let Some(rest) = src.strip_suffix("[]") {
        array_depth += 1;
        src = rest.trim_end();
    }

    let mut ty = java_non_array_type_to_lume_type_ref(src, ctx)?;
    for _ in 0..array_depth {
        ty = TypeRef::Named {
            name: "Array".to_string(),
            args: vec![ty],
            span: ctx.span,
        };
    }
    Some(ty)
}

fn java_non_array_type_to_lume_type_ref(src: &str, ctx: &JavaTypeContext<'_>) -> Option<TypeRef> {
    let (base, arg_sources) = split_java_generic_type(src);
    let args = arg_sources
        .into_iter()
        .map(|arg| java_type_to_lume_type_ref(arg, ctx))
        .collect::<Option<Vec<_>>>()?;

    if let Some(function) = java_function_type_to_lume_type_ref(base, &args, ctx) {
        return Some(function);
    }
    if let Some(tuple) = java_tuple_type_to_lume_type_ref(base, &args, ctx) {
        return Some(tuple);
    }

    if let Some(name) = java_builtin_lume_type_name(base, args.len()) {
        return Some(TypeRef::Named {
            name: name.to_string(),
            args,
            span: ctx.span,
        });
    }

    if ctx.type_params.contains(base) && args.is_empty() {
        return Some(TypeRef::Named {
            name: base.to_string(),
            args: Vec::new(),
            span: ctx.span,
        });
    }

    if let Some(local_name) = java_local_type_name(base, ctx) {
        return Some(TypeRef::Named {
            name: local_name,
            args,
            span: ctx.span,
        });
    }

    Some(any_type_ref(ctx))
}

fn java_function_type_to_lume_type_ref(
    base: &str,
    args: &[TypeRef],
    ctx: &JavaTypeContext<'_>,
) -> Option<TypeRef> {
    match (base, args) {
        ("java.util.function.Supplier" | "Supplier", [ret]) => Some(TypeRef::Function {
            params: Vec::new(),
            ret: Box::new(ret.clone()),
            span: ctx.span,
        }),
        ("java.util.function.Function" | "Function", [param, ret]) => Some(TypeRef::Function {
            params: vec![param.clone()],
            ret: Box::new(ret.clone()),
            span: ctx.span,
        }),
        ("java.util.function.BiFunction" | "BiFunction", [left, right, ret]) => {
            Some(TypeRef::Function {
                params: vec![left.clone(), right.clone()],
                ret: Box::new(ret.clone()),
                span: ctx.span,
            })
        }
        _ => {
            let arity = lume_function_type_arity(base)?;
            if args.len() != arity + 1 {
                return None;
            }
            Some(TypeRef::Function {
                params: args[..arity].to_vec(),
                ret: Box::new(args[arity].clone()),
                span: ctx.span,
            })
        }
    }
}

fn lume_function_type_arity(base: &str) -> Option<usize> {
    let short = base.strip_prefix("lume.core.").unwrap_or(base);
    let suffix = short.strip_prefix("Function")?;
    let arity = suffix.parse::<usize>().ok()?;
    (3..=12).contains(&arity).then_some(arity)
}

fn java_tuple_type_to_lume_type_ref(
    base: &str,
    args: &[TypeRef],
    ctx: &JavaTypeContext<'_>,
) -> Option<TypeRef> {
    let arity = match base {
        "lume.core.Tuple2" | "Tuple2" => 2,
        "lume.core.Tuple3" | "Tuple3" => 3,
        "lume.core.Tuple4" | "Tuple4" => 4,
        "lume.core.Tuple5" | "Tuple5" => 5,
        "lume.core.Tuple6" | "Tuple6" => 6,
        "lume.core.Tuple7" | "Tuple7" => 7,
        "lume.core.Tuple8" | "Tuple8" => 8,
        _ => return None,
    };
    if args.len() != arity {
        return None;
    }
    Some(TypeRef::Tuple {
        fields: args
            .iter()
            .cloned()
            .map(|ty| crate::ast::TupleTypeField { ty, span: ctx.span })
            .collect(),
        span: ctx.span,
    })
}

fn java_builtin_lume_type_name(base: &str, arg_count: usize) -> Option<&'static str> {
    match base {
        "void" => Some("Unit"),
        "lume.core.LumeUnit" | "LumeUnit" if arg_count == 0 => Some("Unit"),
        "java.lang.Object" | "Object" if arg_count == 0 => Some("Any"),
        "boolean" | "java.lang.Boolean" | "Boolean" => Some("Bool"),
        "byte" | "short" | "int" | "java.lang.Byte" | "java.lang.Short" | "java.lang.Integer"
        | "Byte" | "Short" | "Integer" | "long" | "java.lang.Long" | "Long" => Some("Int"),
        "float" | "java.lang.Float" | "Float" | "double" | "java.lang.Double" | "Double" => {
            Some("Float")
        }
        "char" | "java.lang.Character" | "Character" => Some("Rune"),
        "java.lang.String" | "String" => Some("Str"),
        "java.util.List"
        | "java.util.Collection"
        | "java.lang.Iterable"
        | "lume.core.LumeVector"
        | "Vector"
        | "Collection"
        | "Iterable"
            if arg_count == 1 =>
        {
            Some("Vector")
        }
        "lume.core.LumeLinkedList" | "LinkedList" if arg_count == 1 => Some("LinkedList"),
        "lume.core.LumeIterator" | "Iterator" if arg_count == 1 => Some("Iterator"),
        "java.util.Set" | "lume.core.LumeSet" | "Set" if arg_count == 1 => Some("Set"),
        "java.util.Map" | "lume.core.LumeMap" | "Map" if arg_count == 2 => Some("Map"),
        "java.util.Optional" | "lume.core.Option" | "Option" if arg_count == 1 => Some("Option"),
        "lume.core.Result" | "Result" if arg_count == 2 => Some("Result"),
        "lume.core.Either" | "Either" if arg_count == 2 => Some("Either"),
        "lume.core.LumeArray" | "Array" if arg_count == 1 => Some("Array"),
        _ => None,
    }
}

fn any_type_ref(ctx: &JavaTypeContext<'_>) -> TypeRef {
    TypeRef::Named {
        name: "Any".to_string(),
        args: Vec::new(),
        span: ctx.span,
    }
}

fn java_local_type_name(base: &str, ctx: &JavaTypeContext<'_>) -> Option<String> {
    if base.contains('.') {
        let package = java_package_name(base);
        if ctx.allow_cross_package_refs || package == ctx.current_package {
            return ctx.local_type_names.get(base).cloned().or_else(|| {
                ctx.allow_cross_package_refs
                    .then(|| java_simple_name(base).to_string())
            });
        }
        return None;
    }
    let qualified = if ctx.current_package.is_empty() {
        base.to_string()
    } else {
        format!("{}.{}", ctx.current_package, base)
    };
    ctx.local_type_names.get(&qualified).cloned()
}

fn java_simple_name(qualified_name: &str) -> &str {
    qualified_name.rsplit('.').next().unwrap_or(qualified_name)
}

fn synthetic_java_span() -> crate::source::Span {
    crate::source::Span::new(
        0,
        0,
        crate::source::LineColumn::new(1, 1),
        crate::source::LineColumn::new(1, 1),
    )
}

fn split_java_generic_type(src: &str) -> (&str, Vec<&str>) {
    let src = src.trim();
    let Some(start) = src.find('<') else {
        return (src, Vec::new());
    };
    let Some(end) = find_matching_angle(src, start) else {
        return (src, Vec::new());
    };
    let base = src[..start].trim();
    let args = split_java_signature_list(&src[start + 1..end]);
    (base, args)
}

fn split_java_signature_list(src: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;
    for (index, ch) in src.char_indices() {
        match ch {
            '<' => depth += 1,
            '>' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                out.push(src[start..index].trim());
                start = index + ch.len_utf8();
            }
            _ => {}
        }
    }
    let tail = src[start..].trim();
    if !tail.is_empty() {
        out.push(tail);
    }
    out
}

fn find_matching_angle(src: &str, open_index: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (index, ch) in src
        .char_indices()
        .skip_while(|(index, _)| *index < open_index)
    {
        match ch {
            '<' => depth += 1,
            '>' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => {}
        }
    }
    None
}

fn erase_java_generic_suffix(value: &str) -> &str {
    let Some(start) = value.find('<') else {
        return value;
    };
    value[..start].trim_end()
}

fn java_package_name(qualified_name: &str) -> &str {
    qualified_name
        .rsplit_once('.')
        .map(|(package, _)| package)
        .unwrap_or("")
}

fn index_jar(path: &Path, classes: &mut HashSet<String>) -> Result<(), String> {
    let output = Command::new("jar")
        .arg("tf")
        .arg(path)
        .output()
        .map_err(|err| format!("run jar to inspect {}: {err}", path.display()))?;
    if !output.status.success() {
        return Err(format!(
            "inspect jar {}\n{}",
            path.display(),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        if let Some(class_name) = class_name_from_relative_path(line) {
            classes.insert(class_name);
        }
    }
    Ok(())
}

fn index_class_dir(root: &Path, dir: &Path, classes: &mut HashSet<String>) -> Result<(), String> {
    for entry in fs::read_dir(dir).map_err(|err| format!("read {}: {err}", dir.display()))? {
        let entry = entry.map_err(|err| format!("read {} entry: {err}", dir.display()))?;
        let path = entry.path();
        if path.is_dir() {
            index_class_dir(root, &path, classes)?;
        } else if path.extension().is_some_and(|ext| ext == "class") {
            let relative = path
                .strip_prefix(root)
                .map_err(|err| format!("index class {}: {err}", path.display()))?;
            let relative = relative.to_string_lossy().replace('\\', "/");
            if let Some(class_name) = class_name_from_relative_path(&relative) {
                classes.insert(class_name);
            }
        }
    }
    Ok(())
}

fn class_name_from_relative_path(path: &str) -> Option<String> {
    let path = path.strip_suffix(".class")?;
    if path == "module-info" || path.ends_with("/module-info") {
        return None;
    }
    Some(path.replace('/', "."))
}

#[cfg(test)]
mod tests {
    use std::{
        env, fs,
        path::{Path, PathBuf},
        process::Command,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;
    use crate::{
        run_path,
        source::{LineColumn, Span},
    };

    #[test]
    fn maps_lume_core_java_boundary_types_back_to_lume_types() {
        let span = Span::new(0, 0, LineColumn::new(1, 1), LineColumn::new(1, 1));
        let local_type_names = HashMap::from([
            ("lume.db.DbError".to_string(), "DbError".to_string()),
            ("lume.db.Row".to_string(), "Row".to_string()),
        ]);
        let ctx = JavaTypeContext {
            type_params: HashSet::from(["T".to_string()]),
            local_type_names: &local_type_names,
            current_package: "lume.db",
            allow_cross_package_refs: false,
            span,
        };

        let rows = java_type_to_lume_type_ref(
            "lume.core.Result<lume.core.LumeVector<lume.db.Row>, lume.db.DbError>",
            &ctx,
        )
        .expect("rows result type");
        assert_eq!(
            rows,
            TypeRef::Named {
                name: "Result".to_string(),
                args: vec![
                    TypeRef::Named {
                        name: "Vector".to_string(),
                        args: vec![TypeRef::Named {
                            name: "Row".to_string(),
                            args: Vec::new(),
                            span,
                        }],
                        span,
                    },
                    TypeRef::Named {
                        name: "DbError".to_string(),
                        args: Vec::new(),
                        span,
                    },
                ],
                span,
            }
        );

        let mapper = java_type_to_lume_type_ref(
            "java.util.function.Function<lume.db.Row, lume.core.Result<T, lume.db.DbError>>",
            &ctx,
        )
        .expect("mapper type");
        assert!(matches!(
            mapper,
            TypeRef::Function {
                params,
                ret,
                ..
            } if params.len() == 1 && matches!(ret.as_ref(), TypeRef::Named { name, .. } if name == "Result")
        ));

        let tuple =
            java_type_to_lume_type_ref("lume.core.Tuple2<java.lang.Long, java.lang.String>", &ctx)
                .expect("tuple type");
        assert!(matches!(
            tuple,
            TypeRef::Tuple { fields, .. }
                if fields.len() == 2
                    && matches!(fields[0].ty, TypeRef::Named { ref name, .. } if name == "Int")
                    && matches!(fields[1].ty, TypeRef::Named { ref name, .. } if name == "Str")
        ));
    }

    #[test]
    fn imports_generated_lume_reified_methods_without_visible_evidence_params() {
        let span = Span::new(0, 0, LineColumn::new(1, 1), LineColumn::new(1, 1));
        let local_type_names = HashMap::from([
            ("lume.db.DbError".to_string(), "DbError".to_string()),
            ("lume.db.Row".to_string(), "Row".to_string()),
        ]);
        let ctx = JavaTypeContext {
            type_params: HashSet::new(),
            local_type_names: &local_type_names,
            current_package: "lume.db",
            allow_cross_package_refs: false,
            span,
        };

        let parsed = parse_javap_callable_line(
            "public abstract <T extends java.lang.Object> lume.core.Result<lume.core.LumeVector<T>, lume.db.DbError> decodeAll(lume.core.LumeType);",
            "lume.db.Query",
            ctx,
            true,
        );
        let Some(ParsedJavaCallable::Method(method)) = parsed else {
            panic!("expected generated Lume method");
        };

        assert_eq!(method.name, "decodeAll");
        assert_eq!(method.type_params, vec!["T"]);
        assert_eq!(method.reified_type_params, vec!["T"]);
        assert!(method.params.is_empty());
        assert!(matches!(
            method.return_type,
            Some(TypeRef::Named { ref name, .. }) if name == "Result"
        ));
    }

    #[test]
    fn keeps_lume_type_params_visible_for_plain_java_methods() {
        let span = Span::new(0, 0, LineColumn::new(1, 1), LineColumn::new(1, 1));
        let local_type_names = HashMap::new();
        let ctx = JavaTypeContext {
            type_params: HashSet::new(),
            local_type_names: &local_type_names,
            current_package: "third.party",
            allow_cross_package_refs: false,
            span,
        };

        let parsed = parse_javap_callable_line(
            "public abstract <T extends java.lang.Object> T inspect(lume.core.LumeType);",
            "third.party.Inspector",
            ctx,
            false,
        );
        let Some(ParsedJavaCallable::Method(method)) = parsed else {
            panic!("expected plain Java method");
        };

        assert_eq!(method.name, "inspect");
        assert!(method.reified_type_params.is_empty());
        assert_eq!(method.params.len(), 1);
    }

    #[test]
    fn generates_declaration_skeletons_for_checked_program() {
        let temp = temp_path("lume-java-generate");
        let source = temp.join("main.lum");
        let out = temp.join("out");
        fs::create_dir_all(&temp).expect("create temp dir");
        fs::write(
            &source,
            r#"
module demo/app

shape Point {
    x Int
    y Int
}

class User {
    name Str
    age Int
}

class RuntimeBox {
    items [Int]
    names Set[Str]
    index Map[Str, [Int]]
    maybe Option[Str]
    result Result[Int, Str]
    either Either[Str, Int]
    pair (Int, Str)
}

object Routes {
    health Str = "/health"

    def healthPath() Str = this.health
}

interface Named {
    def name() Str
}

enum Maybe[T] {
    case None
    case Some {
        value T
    }
}

annotation Route {
    path Str
}

def main() Unit {
    println("hello")
}
"#,
        )
        .expect("write source");

        let result = generate_java_path(&source, JavaBackendOptions::new(&out)).expect("generate");

        assert!(result.diagnostics.is_empty());
        assert!(
            result
                .written_files
                .iter()
                .any(|path| path.ends_with("demo/app/AppModule.java"))
        );
        assert!(
            result
                .written_files
                .iter()
                .any(|path| path.ends_with("demo/app/AppMain.java"))
        );
        assert!(
            result
                .written_files
                .iter()
                .any(|path| path.ends_with("demo/app/Point.java"))
        );
        assert!(
            result
                .written_files
                .iter()
                .any(|path| path.ends_with("demo/app/User.java"))
        );
        assert!(
            result
                .written_files
                .iter()
                .any(|path| path.ends_with("demo/app/RuntimeBox.java"))
        );
        assert!(
            result
                .written_files
                .iter()
                .any(|path| path.ends_with("demo/app/Routes.java"))
        );
        assert!(
            result
                .written_files
                .iter()
                .any(|path| path.ends_with("demo/app/Named.java"))
        );
        assert!(
            result
                .written_files
                .iter()
                .any(|path| path.ends_with("demo/app/Maybe.java"))
        );
        assert!(
            result
                .written_files
                .iter()
                .any(|path| path.ends_with("demo/app/Route.java"))
        );

        let module = fs::read_to_string(out.join("demo/app/AppModule.java")).expect("read module");
        assert!(module.contains("package demo.app;"));
        assert!(module.contains("final class AppModule"));
        assert!(module.contains("static void main()"));

        let runner = fs::read_to_string(out.join("demo/app/AppMain.java")).expect("read runner");
        assert!(runner.contains("public static void main(String[] args)"));
        assert!(runner.contains("AppModule.main();"));

        let shape = fs::read_to_string(out.join("demo/app/Point.java")).expect("read shape");
        assert!(shape.contains("record Point(Long x, Long y)"));
        assert!(shape.contains("public lume.core.LumeType runtimeType()"));
        assert!(!shape.contains("default lume.core.LumeType runtimeType()"));

        let class = fs::read_to_string(out.join("demo/app/User.java")).expect("read class");
        assert!(class.contains("class User"));
        assert!(class.contains("String name;"));
        assert!(class.contains("Long age;"));

        let runtime_box =
            fs::read_to_string(out.join("demo/app/RuntimeBox.java")).expect("read runtime box");
        assert!(runtime_box.contains("lume.core.LumeVector<Long> items;"));
        assert!(runtime_box.contains("lume.core.LumeSet<String> names;"));
        assert!(
            runtime_box.contains("lume.core.LumeMap<String, lume.core.LumeVector<Long>> index;")
        );
        assert!(runtime_box.contains("lume.core.Option<String> maybe;"));
        assert!(runtime_box.contains("lume.core.Result<Long, String> result;"));
        assert!(runtime_box.contains("lume.core.Either<String, Long> either;"));
        assert!(runtime_box.contains("lume.core.Tuple2<Long, String> pair;"));

        let object = fs::read_to_string(out.join("demo/app/Routes.java")).expect("read object");
        assert!(object.contains("final class Routes"));
        assert!(object.contains("static final Routes INSTANCE"));
        assert!(object.contains("String healthPath()"));

        let interface =
            fs::read_to_string(out.join("demo/app/Named.java")).expect("read interface");
        assert!(interface.contains("interface Named"));
        assert!(interface.contains("String name();"));

        let _ = fs::remove_dir_all(temp);
    }

    #[test]
    fn discovers_nested_local_imports_from_source_root() {
        let temp = temp_path("lume-java-nested-imports");
        let source = temp.join("main.lum");
        let out = temp.join("out");
        let feature_dir = temp.join("feature");
        fs::create_dir_all(&feature_dir).expect("create feature dir");
        fs::write(
            &source,
            r#"
module demo/app

use common/{Shared}
use feature/repo/{FeatureRepo}

def main() Unit {
    repo FeatureRepo = FeatureRepo(Shared("ok"))
    println(repo.shared.name)
}
"#,
        )
        .expect("write main source");
        fs::write(
            temp.join("common.lum"),
            r#"
module common

class Shared {
    name Str
}
"#,
        )
        .expect("write common source");
        fs::write(
            feature_dir.join("repo.lum"),
            r#"
module feature/repo

use common/{Shared}

class FeatureRepo {
    shared Shared
}
"#,
        )
        .expect("write repo source");

        let result = generate_java_path(&source, JavaBackendOptions::new(&out)).expect("generate");

        assert!(result.diagnostics.is_empty());
        assert!(
            result
                .written_files
                .iter()
                .any(|path| path.ends_with("FeatureRepo.java"))
        );

        let _ = fs::remove_dir_all(temp);
    }

    #[test]
    fn preserves_lazy_option_to_result_in_generated_java() {
        if !command_available("javac") || !command_available("java") {
            eprintln!(
                "skipping lazy Option.toResult Java test because a JDK tool is not available"
            );
            return;
        }

        let temp = temp_path("lume-java-lazy-option-to-result");
        let source = temp.join("lazy_option_to_result.lum");
        let out = temp.join("out");
        let classes = temp.join("classes");
        fs::create_dir_all(&temp).expect("create temp dir");
        fs::write(
            &source,
            r#"
module demo/lazyoption

def fail() Str {
    println("eager")
    "bad"
}

def main() Unit {
    maybe Option[Int] = Some(5)
    result Result[Int, Str] = maybe.toResult(fail())
    parsed Result[Int, Str] = Int.parse("7").toResult(fail())

    match result {
        case Ok { value } => println("ok")
        case Err { error } => println(error)
    }

    match parsed {
        case Ok { value } => println("parsed")
        case Err { error } => println(error)
    }
}
"#,
        )
        .expect("write source");

        let result = generate_java_path(&source, JavaBackendOptions::new(&out)).expect("generate");
        assert!(result.diagnostics.is_empty());

        let module = fs::read_to_string(out.join("demo/lazyoption/LazyoptionModule.java"))
            .expect("read module");
        assert!(module.contains(".toResult("));
        assert!(module.contains("java.util.function.Supplier"));

        let mut sources = core_runtime_sources();
        collect_java_sources(&out, &mut sources).expect("collect generated java");
        fs::create_dir_all(&classes).expect("create classes dir");
        run_checked(
            Command::new("javac").arg("-d").arg(&classes).args(&sources),
            "javac",
        );

        let output = run_checked(
            Command::new("java")
                .arg("-cp")
                .arg(&classes)
                .arg("demo.lazyoption.LazyoptionMain"),
            "java",
        );
        assert_eq!(
            String::from_utf8(output.stdout).expect("java stdout utf8"),
            "ok\nparsed\n"
        );

        let _ = fs::remove_dir_all(temp);
    }

    #[test]
    fn emits_predef_parse_calls_for_java_backend() {
        if !command_available("javac") || !command_available("java") {
            eprintln!("skipping predef parse Java test because a JDK tool is not available");
            return;
        }

        let temp = temp_path("lume-java-predef-parse");
        let source = temp.join("predef_parse.lum");
        let out = temp.join("out");
        let classes = temp.join("classes");
        fs::create_dir_all(&temp).expect("create temp dir");
        fs::write(
            &source,
            r#"
module demo/predefparse

def main() Unit {
    parsedInt Option[Int] = Int.parse("41")
    parsedFloat Option[Float] = Float.parse("1.5")

    println((parsedInt !!) + 1)
    println((parsedFloat !!) + 0.5)
    println(Int.parse("oops").isEmpty())
}
"#,
        )
        .expect("write source");

        let result = generate_java_path(&source, JavaBackendOptions::new(&out)).expect("generate");
        assert!(result.diagnostics.is_empty());

        let module = fs::read_to_string(out.join("demo/predefparse/PredefparseModule.java"))
            .expect("read module");
        assert!(module.contains("lume.core.LumeRuntime.parseInt("));
        assert!(module.contains("lume.core.LumeRuntime.parseFloat("));

        let mut sources = core_runtime_sources();
        collect_java_sources(&out, &mut sources).expect("collect generated java");
        fs::create_dir_all(&classes).expect("create classes dir");
        run_checked(
            Command::new("javac").arg("-d").arg(&classes).args(&sources),
            "javac",
        );

        let output = run_checked(
            Command::new("java")
                .arg("-cp")
                .arg(&classes)
                .arg("demo.predefparse.PredefparseMain"),
            "java",
        );
        assert_eq!(
            String::from_utf8(output.stdout).expect("java stdout utf8"),
            "42\n2.0\ntrue\n"
        );

        let _ = fs::remove_dir_all(temp);
    }

    #[test]
    fn emits_map_literal_for_java_backend() {
        let temp = temp_path("lume-java-map-literal");
        let source = temp.join("map_literal.lum");
        let out = temp.join("out");
        fs::create_dir_all(&temp).expect("create temp dir");
        fs::write(
            &source,
            r#"
module demo/mapliteral

def main() Unit {
    entries [Str : Int] = ["one": 1, "two": 2]
    copy [Str : Int] = [...entries]
    merged [Str : Int] = [...copy, "three": 3]
    entryList [(Str, Int)] = [...merged.entries()]
    empty [Str : Int] = []
    println(entries.size(), copy.size(), merged.size(), entryList.size(), empty.size())
}
"#,
        )
        .expect("write source");

        let result = generate_java_path(&source, JavaBackendOptions::new(&out)).expect("generate");
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);

        let module = fs::read_to_string(out.join("demo/mapliteral/MapliteralModule.java"))
            .expect("read module");
        assert!(module.contains("lume.core.LumeMap.fromParts("));
        assert!(module.contains("new lume.core.Tuple2<>("));
        assert!(module.contains(".entries()"));
        assert!(module.contains("lume.core.LumeMap.empty()"));

        let _ = fs::remove_dir_all(temp);
    }

    #[test]
    fn generated_java_compiles_tuple_destructuring() {
        if !command_available("javac") || !command_available("java") {
            eprintln!("skipping tuple destructuring Java test because a JDK tool is not available");
            return;
        }

        let temp = temp_path("lume-java-tuple-destructuring");
        let source = temp.join("tuple_destructuring.lum");
        let out = temp.join("out");
        let classes = temp.join("classes");
        fs::create_dir_all(&temp).expect("create temp dir");
        fs::write(
            &source,
            r#"
module demo/tupledestructuring

def pick(flag Bool) (Int, Int) =
    if flag {
        (1, 2)
    } else {
        (3, 4)
    }

def main() Unit {
    let (left Int, right Int) = pick(true)
    println(left + right)
}
"#,
        )
        .expect("write source");

        let result = generate_java_path(&source, JavaBackendOptions::new(&out)).expect("generate");
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);

        let module =
            fs::read_to_string(out.join("demo/tupledestructuring/TupledestructuringModule.java"))
                .expect("read module");
        assert!(module.contains(".first()"));
        assert!(module.contains(".second()"));

        let mut sources = core_runtime_sources();
        collect_java_sources(&out, &mut sources).expect("collect generated java");
        fs::create_dir_all(&classes).expect("create classes dir");
        run_checked(
            Command::new("javac").arg("-d").arg(&classes).args(&sources),
            "javac",
        );

        let output = run_checked(
            Command::new("java")
                .arg("-cp")
                .arg(&classes)
                .arg("demo.tupledestructuring.TupledestructuringMain"),
            "java",
        );
        assert_eq!(
            String::from_utf8(output.stdout).expect("java stdout utf8"),
            "3\n"
        );

        let _ = fs::remove_dir_all(temp);
    }

    #[test]
    fn generated_java_compiles_zip_with_index_loop_destructuring() {
        if !command_available("javac") || !command_available("java") {
            eprintln!("skipping zipWithIndex Java test because a JDK tool is not available");
            return;
        }

        let temp = temp_path("lume-java-zip-with-index");
        let source = temp.join("zip_with_index.lum");
        let out = temp.join("out");
        let classes = temp.join("classes");
        fs::create_dir_all(&temp).expect("create temp dir");
        fs::write(
            &source,
            r#"
module demo/zipwithindex

def main() Unit {
    values [Str] = ["first", "second"]
    for let (value Str, index Int) <- values.zipWithIndex() {
        println(index, value)
    }
}
"#,
        )
        .expect("write source");

        let result = generate_java_path(&source, JavaBackendOptions::new(&out)).expect("generate");
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);

        let module = fs::read_to_string(out.join("demo/zipwithindex/ZipwithindexModule.java"))
            .expect("read module");
        assert!(module.contains(".zipWithIndex()"));
        assert!(module.contains(".first()"));
        assert!(module.contains(".second()"));

        let mut sources = core_runtime_sources();
        collect_java_sources(&out, &mut sources).expect("collect generated java");
        fs::create_dir_all(&classes).expect("create classes dir");
        run_checked(
            Command::new("javac").arg("-d").arg(&classes).args(&sources),
            "javac",
        );

        let output = run_checked(
            Command::new("java")
                .arg("-cp")
                .arg(&classes)
                .arg("demo.zipwithindex.ZipwithindexMain"),
            "java",
        );
        assert_eq!(
            String::from_utf8(output.stdout).expect("java stdout utf8"),
            "0 first\n1 second\n"
        );

        let _ = fs::remove_dir_all(temp);
    }

    #[test]
    fn generated_java_compiles_negative_numeric_comparison() {
        if !command_available("javac") || !command_available("java") {
            eprintln!("skipping negative comparison Java test because a JDK tool is unavailable");
            return;
        }

        let temp = temp_path("lume-java-negative-comparison");
        let source = temp.join("negative_comparison.lum");
        let out = temp.join("out");
        let classes = temp.join("classes");
        fs::create_dir_all(&temp).expect("create temp dir");
        fs::write(
            &source,
            r#"
module demo/negativecomparison

def main() Unit {
    value Float = -0.005
    println(value < 0.0)
}
"#,
        )
        .expect("write source");

        let result = generate_java_path(&source, JavaBackendOptions::new(&out)).expect("generate");
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);

        let mut sources = core_runtime_sources();
        collect_java_sources(&out, &mut sources).expect("collect generated java");
        fs::create_dir_all(&classes).expect("create classes dir");
        run_checked(
            Command::new("javac").arg("-d").arg(&classes).args(&sources),
            "javac",
        );

        let output = run_checked(
            Command::new("java")
                .arg("-cp")
                .arg(&classes)
                .arg("demo.negativecomparison.NegativecomparisonMain"),
            "java",
        );
        assert_eq!(
            String::from_utf8(output.stdout).expect("java stdout utf8"),
            "true\n"
        );

        let _ = fs::remove_dir_all(temp);
    }

    #[test]
    fn emits_structured_java_for_simple_enum_match_methods() {
        let temp = temp_path("lume-java-enum-match-methods");
        let source = temp.join("maybe.lum");
        let out = temp.join("out");
        fs::create_dir_all(&temp).expect("create temp dir");
        fs::write(
            &source,
            r#"
module demo/maybe

enum Maybe[T] {
    case None
    case Some {
        value T
    }

    def isDefined() Bool = match this {
        case Some { value: _ } => true
        case None => false
    }

    def unsafeValue() T = match this {
        case Some { value } => value
        case None => panic("expected Maybe.Some")
    }
}

"#,
        )
        .expect("write source");

        let result = generate_java_path(&source, JavaBackendOptions::new(&out)).expect("generate");

        assert!(result.diagnostics.is_empty());
        let maybe = fs::read_to_string(out.join("demo/maybe/Maybe.java")).expect("read maybe");
        assert!(!maybe.contains("__block"));
        assert!(!maybe.contains("while (true)"));
        assert!(!maybe.contains("variantField"));
        assert!(maybe.contains("if (this instanceof Some<?>)"));
        assert!(maybe.contains("if (this instanceof None<?>)"));
        assert!(maybe.contains("if (this instanceof Some<?> __case0)"));
        assert!(maybe.contains("return ((T) __case0.value());"));

        let _ = fs::remove_dir_all(temp);
    }

    #[test]
    fn emits_typed_enum_payload_pattern_bindings() {
        let temp = temp_path("lume-java-typed-enum-pattern-bindings");
        let source = temp.join("main.lum");
        let out = temp.join("out");
        fs::create_dir_all(&temp).expect("create temp dir");
        fs::write(
            &source,
            r#"
module demo/extract

def length(maybe Option[Str]) Int {
    let Some { value as text } = maybe else return 0
    text.size()
}

def keepUnit(result Result[Unit, Str]) Result[Unit, Str] {
    let Ok { value } = result else return result
    Ok(value)
}
"#,
        )
        .expect("write source");

        let result = generate_java_path(&source, JavaBackendOptions::new(&out)).expect("generate");

        assert!(result.diagnostics.is_empty());
        let module =
            fs::read_to_string(out.join("demo/extract/ExtractModule.java")).expect("read module");
        assert!(module.contains("String text_"));
        assert!(!module.contains("Object text_"));
        assert!(module.contains("lume.core.LumeUnit value_"));
        assert!(module.contains("new lume.core.Result.Ok<>(value_"));

        let _ = fs::remove_dir_all(temp);
    }

    #[test]
    fn emits_structured_java_for_core_option_result_and_either() {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let repo_root = manifest_dir
            .parent()
            .and_then(|crates_dir| crates_dir.parent())
            .and_then(|rust_dir| rust_dir.parent())
            .expect("repo root");

        for (source_name, expected_lines) in [
            (
                "Option",
                vec![
                    "if (this instanceof Some<?>)",
                    "if (this instanceof None<?>)",
                    "if (this instanceof Some<?> __case0)",
                    "default <X> lume.core.Option<X> map(java.util.function.Function<T, X> f_1)",
                    "default <X> lume.core.Option<X> flatMap(java.util.function.Function<T, lume.core.Option<X>> f_1)",
                    "default lume.core.LumeIterator<T> iterator()",
                    "return new None<>();",
                ],
            ),
            (
                "Result",
                vec![
                    "if (this instanceof Ok<?, ?>)",
                    "if (this instanceof Err<?, ?>)",
                    "if (this instanceof Ok<?, ?> __case0)",
                    "default <X> lume.core.Result<X, E> map(java.util.function.Function<T, X> f_1)",
                    "default <X> lume.core.Result<X, E> flatMap(java.util.function.Function<T, lume.core.Result<X, E>> f_1)",
                    "return ((T) __case0.value());",
                ],
            ),
            (
                "Either",
                vec![
                    "if (this instanceof Left<?, ?>)",
                    "if (this instanceof Right<?, ?>)",
                    "if (this instanceof Right<?, ?> __case1)",
                    "default <X> lume.core.Either<L, X> map(java.util.function.Function<R, X> f_1)",
                    "default <X> lume.core.Either<L, X> flatMap(java.util.function.Function<R, lume.core.Either<L, X>> f_1)",
                    "default L merge()",
                    "return ((R) __case1.value());",
                    "return ((L) ((Object) ((R) __case1.value())));",
                ],
            ),
        ] {
            let temp = temp_path(&format!("lume-java-core-{}", source_name.to_lowercase()));
            let source = repo_root.join(format!(
                "lume/core/src/main/lume/lume/core/{source_name}.lum"
            ));
            let out = temp.join("out");
            fs::create_dir_all(&temp).expect("create temp dir");

            let result =
                generate_java_path(&source, JavaBackendOptions::new(&out)).expect("generate");

            assert!(result.diagnostics.is_empty());
            let generated = fs::read_to_string(out.join(format!("lume/core/{source_name}.java")))
                .expect("read generated core enum");
            assert!(!generated.contains("__block"));
            assert!(!generated.contains("while (true)"));
            assert!(!generated.contains("variantField"));
            for expected in expected_lines {
                assert!(
                    generated.contains(expected),
                    "generated {source_name}.java did not contain {expected:?}\n{generated}"
                );
            }

            let _ = fs::remove_dir_all(temp);
        }
    }

    #[test]
    fn does_not_write_java_when_lume_has_diagnostics() {
        let temp = temp_path("lume-java-invalid");
        let source = temp.join("broken.lum");
        let out = temp.join("out");
        fs::create_dir_all(&temp).expect("create temp dir");
        fs::write(&source, "def main() { missing() }").expect("write source");

        let result = generate_java_path(&source, JavaBackendOptions::new(&out)).expect("generate");

        assert!(!result.diagnostics.is_empty());
        assert!(result.written_files.is_empty());
        assert!(!out.exists());

        let _ = fs::remove_dir_all(temp);
    }

    #[test]
    fn resolves_java_type_imports_for_generation() {
        let temp = temp_path("lume-java-imports");
        let source = temp.join("external.lum");
        let out = temp.join("out");
        fs::create_dir_all(&temp).expect("create temp dir");
        fs::write(
            &source,
            r#"
module demo/external

use java/time/Instant
use java/time/{Duration as JDuration}

class Event {
    at Instant
    duration JDuration
}

def main() Unit {
}
"#,
        )
        .expect("write source");

        let result = generate_java_path(&source, JavaBackendOptions::new(&out)).expect("generate");

        assert!(result.diagnostics.is_empty());
        let event = fs::read_to_string(out.join("demo/external/Event.java")).expect("read event");
        assert!(event.contains("java.time.Instant at;"));
        assert!(event.contains("java.time.Duration duration;"));
        assert!(
            event.contains("Event(java.time.Instant at_arg0, java.time.Duration duration_arg1)")
        );
        assert!(!out.join("demo/external/Instant.java").exists());
        assert!(!out.join("demo/external/JDuration.java").exists());

        let _ = fs::remove_dir_all(temp);
    }

    #[test]
    fn resolves_java_overload_from_nested_call_return_type() {
        let temp = temp_path("lume-java-overload-nested-call");
        let source = temp.join("string_builder.lum");
        let out = temp.join("out");
        fs::create_dir_all(&temp).expect("create temp dir");
        fs::write(
            &source,
            r#"
module demo/builder

use java/lang/StringBuilder

def text() Str = "value"

def main() Unit {
    builder StringBuilder = StringBuilder()
    builder.append(text())
    println(builder.toStr())
}
"#,
        )
        .expect("write source");

        let result = generate_java_path(&source, JavaBackendOptions::new(&out)).expect("generate");

        assert!(result.diagnostics.is_empty());
        let module =
            fs::read_to_string(out.join("demo/builder/BuilderModule.java")).expect("read module");
        assert!(module.contains("String tmp"));
        assert!(module.contains(" = text();"));
        assert!(module.contains(".append(tmp"));

        let _ = fs::remove_dir_all(temp);
    }

    #[test]
    fn validates_and_compiles_third_party_jar_imports() {
        if !command_available("javac")
            || !command_available("java")
            || !command_available("jar")
            || !command_available("javap")
        {
            eprintln!("skipping Java jar import test because a JDK tool is not available");
            return;
        }

        let temp = temp_path("lume-java-jar-import");
        let source = temp.join("jar_import.lum");
        let out = temp.join("out");
        let classes = temp.join("classes");
        fs::create_dir_all(&temp).expect("create temp dir");
        let jar = create_widget_jar(&temp);
        fs::write(
            &source,
            r#"
module demo/jaruse

use java/util/ArrayList
use third/party/{Widget, GenericBox}

class Holder {
    widget Widget
    generic GenericBox[Str]
    list ArrayList[Str]
}

def main() Unit {
}
"#,
        )
        .expect("write source");

        let result = generate_java_path(
            &source,
            JavaBackendOptions::new(&out).with_classpath_entry(&jar),
        )
        .expect("generate java");

        assert!(result.diagnostics.is_empty());
        let holder = fs::read_to_string(out.join("demo/jaruse/Holder.java")).expect("read holder");
        assert!(holder.contains("third.party.Widget widget;"));
        assert!(holder.contains("third.party.GenericBox<String> generic;"));
        assert!(holder.contains("java.util.ArrayList<String> list;"));

        let mut sources = core_runtime_sources();
        collect_java_sources(&out, &mut sources).expect("collect generated java");
        fs::create_dir_all(&classes).expect("create classes dir");
        run_checked(
            Command::new("javac")
                .arg("-cp")
                .arg(&jar)
                .arg("-d")
                .arg(&classes)
                .args(&sources),
            "javac",
        );

        let runtime_classpath =
            env::join_paths([classes.as_path(), jar.as_path()]).expect("join runtime classpath");
        let output = run_checked(
            Command::new("java")
                .arg("-cp")
                .arg(runtime_classpath)
                .arg("demo.jaruse.JaruseMain"),
            "java",
        );
        assert_eq!(
            String::from_utf8(output.stdout).expect("java stdout utf8"),
            ""
        );

        let _ = fs::remove_dir_all(temp);
    }

    #[test]
    fn validates_java_constructor_and_method_signatures_from_jar() {
        if !command_available("javac")
            || !command_available("java")
            || !command_available("jar")
            || !command_available("javap")
        {
            eprintln!("skipping Java signature test because a JDK tool is not available");
            return;
        }

        let temp = temp_path("lume-java-signatures");
        let source = temp.join("java_signatures.lum");
        let out = temp.join("out");
        let classes = temp.join("classes");
        fs::create_dir_all(&temp).expect("create temp dir");
        let jar = create_widget_jar(&temp);
        fs::write(
            &source,
            r#"
module demo/javasigs

use third/party/{Widget, GenericBox}

def main() Unit {
    widget Widget = Widget("Ada", 7)
    label Str = widget.label()
    count Int = widget.count()
    made Widget = Widget.create("Bob")
    boxed GenericBox[Str] = GenericBox("hello")
    boxedValue Str = boxed.value()
    println(label)
}
"#,
        )
        .expect("write source");

        let result = generate_java_path(
            &source,
            JavaBackendOptions::new(&out).with_classpath_entry(&jar),
        )
        .expect("generate java");

        assert!(result.diagnostics.is_empty());
        let module =
            fs::read_to_string(out.join("demo/javasigs/JavasigsModule.java")).expect("read module");
        assert!(module.contains("new third.party.Widget(\"Ada\", 7L)"));
        assert!(module.contains(".label()"));
        assert!(module.contains(".count()"));
        assert!(module.contains("third.party.Widget.create(\"Bob\")"));
        assert!(module.contains("new third.party.GenericBox<>(\"hello\")"));
        assert!(!out.join("demo/javasigs/Widget.java").exists());
        assert!(!out.join("demo/javasigs/GenericBox.java").exists());

        let mut sources = core_runtime_sources();
        collect_java_sources(&out, &mut sources).expect("collect generated java");
        fs::create_dir_all(&classes).expect("create classes dir");
        run_checked(
            Command::new("javac")
                .arg("-cp")
                .arg(&jar)
                .arg("-d")
                .arg(&classes)
                .args(&sources),
            "javac",
        );

        let runtime_classpath =
            env::join_paths([classes.as_path(), jar.as_path()]).expect("join runtime classpath");
        let output = run_checked(
            Command::new("java")
                .arg("-cp")
                .arg(runtime_classpath)
                .arg("demo.javasigs.JavasigsMain"),
            "java",
        );
        assert_eq!(
            String::from_utf8(output.stdout).expect("java stdout utf8"),
            "Ada\n"
        );

        let _ = fs::remove_dir_all(temp);
    }

    #[test]
    fn packs_external_lume_vararg_calls_from_jar() {
        if !command_available("javac") || !command_available("jar") || !command_available("javap") {
            eprintln!("skipping external Lume vararg test because a JDK tool is not available");
            return;
        }

        let temp = temp_path("lume-java-external-vararg");
        let lib_source = temp.join("lib.lum");
        let app_source = temp.join("app.lum");
        let lib_out = temp.join("lib-out");
        let lib_classes = temp.join("lib-classes");
        let app_out = temp.join("app-out");
        let app_classes = temp.join("app-classes");
        let jar = temp.join("lume-lib.jar");
        fs::create_dir_all(&temp).expect("create temp dir");

        fs::write(
            &lib_source,
            r#"
module demo/lib

interface Binder {
    def queryRow(sql Str, values [Any] vararg) Int
}
"#,
        )
        .expect("write lib source");

        let generated_lib = generate_java_path(&lib_source, JavaBackendOptions::new(&lib_out))
            .expect("generate lib");
        assert!(generated_lib.diagnostics.is_empty());
        let mut lib_sources = core_runtime_sources();
        collect_java_sources(&lib_out, &mut lib_sources).expect("collect lib java");
        fs::create_dir_all(&lib_classes).expect("create lib classes dir");
        run_checked(
            Command::new("javac")
                .arg("-d")
                .arg(&lib_classes)
                .args(&lib_sources),
            "javac",
        );
        run_checked(
            Command::new("jar")
                .arg("cf")
                .arg(&jar)
                .arg("-C")
                .arg(&lib_classes)
                .arg("."),
            "jar",
        );

        fs::write(
            &app_source,
            r#"
module demo/app

use demo/lib/{Binder}

class Client {
    binder Binder

    def run(sub Str) Int {
        this.binder.queryRow("select ?", sub)
    }
}

"#,
        )
        .expect("write app source");

        let generated_app = generate_java_path(
            &app_source,
            JavaBackendOptions::new(&app_out).with_classpath_entry(&jar),
        )
        .expect("generate app");
        assert!(generated_app.diagnostics.is_empty());
        let client = fs::read_to_string(app_out.join("demo/app/Client.java")).expect("read client");
        assert!(client.contains("queryRow(\"select ?\", lume.core.LumeVector.of(sub_"));
        assert!(!client.contains("((lume.core.LumeVector<Object>) ((Object) sub_"));

        let mut app_sources = core_runtime_sources();
        collect_java_sources(&app_out, &mut app_sources).expect("collect app java");
        fs::create_dir_all(&app_classes).expect("create app classes dir");
        run_checked(
            Command::new("javac")
                .arg("-cp")
                .arg(&jar)
                .arg("-d")
                .arg(&app_classes)
                .args(&app_sources),
            "javac",
        );

        let _ = fs::remove_dir_all(temp);
    }

    #[test]
    fn validates_java_inherited_interface_methods_from_jar() {
        if !command_available("javac")
            || !command_available("java")
            || !command_available("jar")
            || !command_available("javap")
        {
            eprintln!("skipping Java inherited method test because a JDK tool is not available");
            return;
        }

        let temp = temp_path("lume-java-inherited-methods");
        let source = temp.join("java_inherited_methods.lum");
        let out = temp.join("out");
        let classes = temp.join("classes");
        fs::create_dir_all(&temp).expect("create temp dir");
        let jar = create_router_jar(&temp);
        fs::write(
            &source,
            r#"
module demo/javainherit

use third/party/Router

def main() Unit {
    router Router = Router()
    again Router = router.ping()
}
"#,
        )
        .expect("write source");

        let result = generate_java_path(
            &source,
            JavaBackendOptions::new(&out).with_classpath_entry(&jar),
        )
        .expect("generate java");

        assert!(result.diagnostics.is_empty());
        let module = fs::read_to_string(out.join("demo/javainherit/JavainheritModule.java"))
            .expect("read module");
        assert!(module.contains(".ping()"));

        let mut sources = core_runtime_sources();
        collect_java_sources(&out, &mut sources).expect("collect generated java");
        fs::create_dir_all(&classes).expect("create classes dir");
        run_checked(
            Command::new("javac")
                .arg("-cp")
                .arg(&jar)
                .arg("-d")
                .arg(&classes)
                .args(&sources),
            "javac",
        );

        let runtime_classpath =
            env::join_paths([classes.as_path(), jar.as_path()]).expect("join runtime classpath");
        let output = run_checked(
            Command::new("java")
                .arg("-cp")
                .arg(runtime_classpath)
                .arg("demo.javainherit.JavainheritMain"),
            "java",
        );
        assert_eq!(
            String::from_utf8(output.stdout).expect("java stdout utf8"),
            ""
        );

        let _ = fs::remove_dir_all(temp);
    }

    #[test]
    fn rejects_java_constructor_signature_mismatch_from_jar() {
        if !command_available("javac") || !command_available("jar") || !command_available("javap") {
            eprintln!("skipping Java signature mismatch test because a JDK tool is not available");
            return;
        }

        let temp = temp_path("lume-java-signature-mismatch");
        let source = temp.join("java_signature_mismatch.lum");
        let out = temp.join("out");
        fs::create_dir_all(&temp).expect("create temp dir");
        let jar = create_widget_jar(&temp);
        fs::write(
            &source,
            r#"
module demo/javamismatch

use third/party/Widget

def main() Unit {
    widget Widget = Widget(5, "bad")
}
"#,
        )
        .expect("write source");

        let result = generate_java_path(
            &source,
            JavaBackendOptions::new(&out).with_classpath_entry(&jar),
        )
        .expect("generate java");

        assert!(result.written_files.is_empty());
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diag| diag.diagnostic.code == "no_matching_overload"
                    || diag.diagnostic.code == "invalid_argument_type"),
            "expected constructor mismatch diagnostic, got {:#?}",
            result.diagnostics
        );

        let _ = fs::remove_dir_all(temp);
    }

    #[test]
    fn rejects_missing_java_method_from_jar() {
        if !command_available("javac") || !command_available("jar") || !command_available("javap") {
            eprintln!("skipping Java missing method test because a JDK tool is not available");
            return;
        }

        let temp = temp_path("lume-java-missing-method");
        let source = temp.join("java_missing_method.lum");
        let out = temp.join("out");
        fs::create_dir_all(&temp).expect("create temp dir");
        let jar = create_widget_jar(&temp);
        fs::write(
            &source,
            r#"
module demo/javamissingmethod

use third/party/Widget

def main() Unit {
    widget Widget = Widget("Ada", 7)
    widget.nope()
}
"#,
        )
        .expect("write source");

        let result = generate_java_path(
            &source,
            JavaBackendOptions::new(&out).with_classpath_entry(&jar),
        )
        .expect("generate java");

        assert!(result.written_files.is_empty());
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diag| diag.diagnostic.code == "unknown_member"),
            "expected missing Java method diagnostic, got {:#?}",
            result.diagnostics
        );

        let _ = fs::remove_dir_all(temp);
    }

    #[test]
    fn reports_missing_java_class_from_classpath() {
        if !command_available("javap") || !command_available("javac") || !command_available("jar") {
            eprintln!("skipping missing Java class test because a JDK tool is not available");
            return;
        }

        let temp = temp_path("lume-java-missing-class");
        let source = temp.join("missing_import.lum");
        let out = temp.join("out");
        fs::create_dir_all(&temp).expect("create temp dir");
        let jar = create_widget_jar(&temp);
        fs::write(
            &source,
            r#"
module demo/missing

use third/party/Missing

class Holder {
    missing Missing
}

def main() Unit {
}
"#,
        )
        .expect("write source");

        let result = generate_java_path(
            &source,
            JavaBackendOptions::new(&out).with_classpath_entry(&jar),
        )
        .expect("generate java");

        assert!(result.written_files.is_empty());
        assert_eq!(result.diagnostics.len(), 1);
        assert_eq!(result.diagnostics[0].diagnostic.code, "missing_java_class");
        assert!(
            result.diagnostics[0]
                .diagnostic
                .message
                .contains("third.party.Missing")
        );

        let _ = fs::remove_dir_all(temp);
    }

    #[test]
    fn generated_java_runs_runtime_type_narrowing() {
        if !command_available("javac") || !command_available("java") {
            eprintln!("skipping Java type-narrowing test because javac/java is not available");
            return;
        }

        let temp = temp_path("lume-java-type-narrowing");
        let source = temp.join("type_narrowing.lum");
        let out = temp.join("out");
        let classes = temp.join("classes");
        fs::create_dir_all(&temp).expect("create temp dir");
        fs::write(
            &source,
            r#"
module demo/typenarrowing

class Worker {
    name Str

    def label() Str = this.name
}

def workerLabel(value Any) Str {
    if !(value is Worker) {
        return "other"
    }
    value.label()
}

def directWorkerLabel(value Any) Str {
    if value is Worker {
        return value.label()
    }
    "other"
}

def main() Unit {
    println(workerLabel(Worker("Ada")))
    println(workerLabel("text"))
    println(directWorkerLabel(Worker("Bob")))
}
"#,
        )
        .expect("write source");

        let result = generate_java_path(&source, JavaBackendOptions::new(&out)).expect("generate");
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);

        let module = fs::read_to_string(out.join("demo/typenarrowing/TypenarrowingModule.java"))
            .expect("read module");
        assert!(module.contains("instanceof Worker"));

        let mut sources = core_runtime_sources();
        collect_java_sources(&out, &mut sources).expect("collect generated java");
        fs::create_dir_all(&classes).expect("create classes dir");
        run_checked(
            Command::new("javac").arg("-d").arg(&classes).args(&sources),
            "javac",
        );

        let output = run_checked(
            Command::new("java")
                .arg("-cp")
                .arg(&classes)
                .arg("demo.typenarrowing.TypenarrowingMain"),
            "java",
        );
        assert_eq!(
            String::from_utf8(output.stdout).expect("java stdout utf8"),
            "Ada\nother\nBob\n"
        );

        let _ = fs::remove_dir_all(temp);
    }

    #[test]
    fn generated_java_runs_named_record_patterns() {
        if !command_available("javac") || !command_available("java") {
            eprintln!("skipping Java record-pattern test because javac/java is not available");
            return;
        }

        let temp = temp_path("lume-java-record-patterns");
        let source = temp.join("record_patterns.lum");
        let out = temp.join("out");
        let classes = temp.join("classes");
        fs::create_dir_all(&temp).expect("create temp dir");
        fs::write(
            &source,
            r#"
module demo/recordpatterns

shape Location {
    city Str
}

class User {
    name Str
    location Location
    age Int
}

def describe(value Any) Str = match value {
    case User {
        location as home
        age: 18
        name
    } => name + " " + home.city
    case _ => "other"
}

def main() Unit {
    println(describe(User { name: "Ada", location: Location { city: "Tampa" }, age: 18 }))
    println(describe(User { name: "Bob", location: Location { city: "Miami" }, age: 19 }))
    println(describe("not a user"))
}
"#,
        )
        .expect("write source");

        let result = generate_java_path(&source, JavaBackendOptions::new(&out)).expect("generate");
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);

        let module = fs::read_to_string(out.join("demo/recordpatterns/RecordpatternsModule.java"))
            .expect("read module");
        assert!(module.contains("LumeRuntime.patternField"));

        let mut sources = core_runtime_sources();
        collect_java_sources(&out, &mut sources).expect("collect generated java");
        fs::create_dir_all(&classes).expect("create classes dir");
        run_checked(
            Command::new("javac").arg("-d").arg(&classes).args(&sources),
            "javac",
        );

        let output = run_checked(
            Command::new("java")
                .arg("-cp")
                .arg(&classes)
                .arg("demo.recordpatterns.RecordpatternsMain"),
            "java",
        );
        assert_eq!(
            String::from_utf8(output.stdout).expect("java stdout utf8"),
            "Ada Tampa\nother\nother\n"
        );

        let _ = fs::remove_dir_all(temp);
    }

    #[test]
    fn generated_java_runs_reference_identity_operators() {
        if !command_available("javac") || !command_available("java") {
            eprintln!("skipping Java identity test because javac/java is not available");
            return;
        }

        let temp = temp_path("lume-java-reference-identity");
        let source = temp.join("identity.lum");
        let out = temp.join("out");
        let classes = temp.join("classes");
        fs::create_dir_all(&temp).expect("create temp dir");
        fs::write(
            &source,
            r#"
module demo/identity

class Box {
    value Int
}

def main() Unit {
    first = Box(1)
    alias = first
    separate = Box(1)

    println(first === alias)
    println(first === separate)
    println(first !== separate)
}
"#,
        )
        .expect("write source");

        let result = generate_java_path(&source, JavaBackendOptions::new(&out)).expect("generate");
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);

        let module =
            fs::read_to_string(out.join("demo/identity/IdentityModule.java")).expect("read module");
        assert!(module.contains(" == "));
        assert!(module.contains(" != "));

        let mut sources = core_runtime_sources();
        collect_java_sources(&out, &mut sources).expect("collect generated java");
        fs::create_dir_all(&classes).expect("create classes dir");
        run_checked(
            Command::new("javac").arg("-d").arg(&classes).args(&sources),
            "javac",
        );

        let output = run_checked(
            Command::new("java")
                .arg("-cp")
                .arg(&classes)
                .arg("demo.identity.IdentityMain"),
            "java",
        );
        assert_eq!(
            String::from_utf8(output.stdout).expect("java stdout utf8"),
            "true\nfalse\ntrue\n"
        );

        let _ = fs::remove_dir_all(temp);
    }

    #[test]
    fn generated_java_shapes_define_structural_equals_and_hash_code() {
        if !command_available("javac") || !command_available("java") {
            eprintln!("skipping Java shape equality test because javac/java is not available");
            return;
        }

        let temp = temp_path("lume-java-shape-value-methods");
        let source = temp.join("shape_value_methods.lum");
        let out = temp.join("out");
        let classes = temp.join("classes");
        fs::create_dir_all(&temp).expect("create temp dir");
        fs::write(
            &source,
            r#"
module demo/shapevalue

shape Point {
    x Int
    label Str
}

shape ReorderedPoint {
    label Str
    x Int
}

class StableReference with Hashed[StableReference] {
    value Int

    def equals(other StableReference) Bool = this.value == other.value
    def hash() Int = this.value
}

class Account with Eq[Account] {
    id Int

    def equals(other Account) Bool = this.id == other.id
}

interface Identified with Eq[Identified] {
    def code() Int
}

class Entry with Identified {
    value Int

    def code() Int = this.value
    def equals(other Identified) Bool = this.value == other.code()
}

class AlternateEntry with Identified {
    value Int

    def code() Int = this.value
    def equals(other Identified) Bool = this.value == other.code()
}

shape Box[T] {
    value T
}

shape Marker {
}

def main() Unit {
    first = Point(1, "one")
    same = Point(1, "one")
    different = Point(2, "two")

    println(first == same)
    println(first == different)

    points [Point: Str] = [first: "found"]
    println(points[same] !!)

    reordered = ReorderedPoint("one", 1)
    println(first == reordered)
    println(reordered == first)
    println(first.equals(reordered))

    println(Marker {} == Marker {})

    println(Account(1) == Account(1))
    println(Account(1) != Account(2))
    left Identified = Entry(1)
    right Identified = AlternateEntry(1)
    println(left == right)

    dynamicFirst Any = Any(first)
    dynamicSame Any = Any(same)
    dynamicAgain Any = Any(dynamicFirst)
    dynamicOtherShape Any = reordered
    println(dynamicFirst.sameValue(dynamicSame))
    println(dynamicFirst.sameValue(dynamicAgain))
    println(dynamicFirst.sameValue(dynamicOtherShape))

    account = Account(3)
    widenedAccount Any = Any(account)
    if let recovered Account = widenedAccount {
        println(recovered === account)
    }
}
"#,
        )
        .expect("write source");

        let result = generate_java_path(&source, JavaBackendOptions::new(&out)).expect("generate");
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);

        let point = fs::read_to_string(out.join("demo/shapevalue/Point.java"))
            .expect("read generated Point");
        assert!(point.contains("public boolean equals(Object other)"));
        assert!(point.contains("other instanceof Point that"));
        assert!(point.contains("java.util.Objects.equals(this.x, that.x)"));
        assert!(point.contains("java.util.Objects.equals(this.label, that.label)"));
        assert!(point.contains("public int hashCode()"));
        assert!(point.contains("java.util.Objects.hash(this.label, this.x)"));
        assert!(point.contains("implements lume.core.Hashed<Point>, lume.core.LumeTyped"));

        let generic =
            fs::read_to_string(out.join("demo/shapevalue/Box.java")).expect("read generated Box");
        assert!(generic.contains("other instanceof Box<?> that"));
        assert!(generic.contains("implements lume.core.Eq<Box<T>>, lume.core.LumeTyped"));
        assert!(!generic.contains("lume.core.Hashed"));

        let reordered = fs::read_to_string(out.join("demo/shapevalue/ReorderedPoint.java"))
            .expect("read generated ReorderedPoint");
        assert!(reordered.contains("java.util.Objects.hash(this.label, this.x)"));

        let stable = fs::read_to_string(out.join("demo/shapevalue/StableReference.java"))
            .expect("read generated StableReference");
        assert!(stable.contains("implements lume.core.Hashed<StableReference>"));
        assert!(stable.contains("public int hashCode()"));
        assert!(stable.contains("return Long.hashCode(this.hash())"));

        let account = fs::read_to_string(out.join("demo/shapevalue/Account.java"))
            .expect("read generated Account");
        assert!(account.contains("implements lume.core.Eq<Account>, lume.core.LumeTyped"));
        assert!(account.contains("public Boolean equals(Account "));
        assert!(account.contains("public boolean equals(Object other)"));

        let entry = fs::read_to_string(out.join("demo/shapevalue/Entry.java"))
            .expect("read generated Entry");
        assert!(entry.contains("public boolean equals(Object other)"));

        let module = fs::read_to_string(out.join("demo/shapevalue/ShapevalueModule.java"))
            .expect("read generated module");
        assert!(module.contains("new Point("));
        assert!(module.contains("new ReorderedPoint("));
        assert!(!module.contains("Any("));

        let marker = fs::read_to_string(out.join("demo/shapevalue/Marker.java"))
            .expect("read generated Marker");
        assert!(marker.contains("return java.util.Objects.hash();"));

        let mut sources = core_runtime_sources();
        collect_java_sources(&out, &mut sources).expect("collect generated java");
        fs::create_dir_all(&classes).expect("create classes dir");
        run_checked(
            Command::new("javac").arg("-d").arg(&classes).args(&sources),
            "javac",
        );

        let output = run_checked(
            Command::new("java")
                .arg("-cp")
                .arg(&classes)
                .arg("demo.shapevalue.ShapevalueMain"),
            "java",
        );
        assert_eq!(
            String::from_utf8(output.stdout).expect("java stdout utf8"),
            "true\nfalse\nfound\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\nfalse\ntrue\n"
        );

        let _ = fs::remove_dir_all(temp);
    }

    #[test]
    fn generated_java_executable_runs_array_of_rune() {
        if !command_available("javac") || !command_available("java") {
            eprintln!("skipping Java executable test because javac/java is not available");
            return;
        }

        let temp = temp_path("lume-java-array-rune");
        let source = temp.join("array_rune.lum");
        let out = temp.join("out");
        let classes = temp.join("classes");
        fs::create_dir_all(&temp).expect("create temp dir");
        fs::write(
            &source,
            r#"
module demo/runarray

def main() Int {
    runes Array[Rune] = Array.ofRune(2)
    0
}
"#,
        )
        .expect("write source");

        let interpreted = run_path(&source, None).expect("run interpreter");
        assert!(interpreted.diagnostics.is_empty());
        let expected = interpreter_stdout(interpreted);

        let generated =
            generate_java_path(&source, JavaBackendOptions::new(&out)).expect("generate java");
        assert!(generated.diagnostics.is_empty());

        let module =
            fs::read_to_string(out.join("demo/runarray/RunarrayModule.java")).expect("read module");
        assert!(!module.contains("UnsupportedOperationException"));
        assert!(module.contains("lume.core.LumeArray.ofRune(2L)"));

        let mut sources = core_runtime_sources();
        collect_java_sources(&out, &mut sources).expect("collect generated java");
        fs::create_dir_all(&classes).expect("create classes dir");
        run_checked(
            Command::new("javac").arg("-d").arg(&classes).args(&sources),
            "javac",
        );

        let output = run_checked(
            Command::new("java")
                .arg("-cp")
                .arg(&classes)
                .arg("demo.runarray.RunarrayMain"),
            "java",
        );
        let actual = String::from_utf8(output.stdout).expect("java stdout utf8");
        assert_eq!(actual, expected);

        let _ = fs::remove_dir_all(temp);
    }

    #[test]
    fn emits_mvp_function_bodies_for_supported_ir() {
        let temp = temp_path("lume-java-bodies");
        let source = temp.join("body.lum");
        let out = temp.join("out");
        fs::create_dir_all(&temp).expect("create temp dir");
        fs::write(
            &source,
            r#"
module demo/body

def add(left Int, right Int) Int {
    result Int = left + right
    result
}

def choose(flag Bool) Int {
    if flag {
        10
    } else {
        20
    }
}

def main() Unit {
    value Int = add(2, 3)
    println(value)
}
"#,
        )
        .expect("write source");

        let result = generate_java_path(&source, JavaBackendOptions::new(&out)).expect("generate");

        assert!(result.diagnostics.is_empty());
        let module =
            fs::read_to_string(out.join("demo/body/BodyModule.java")).expect("read module");
        assert!(!module.contains("UnsupportedOperationException"));
        assert!(module.contains("static Long add(Long left_0, Long right_1)"));
        assert!(module.contains("tmp3_3 = (left_0 + right_1);"));
        assert!(module.contains("result_2 = tmp3_3;"));
        assert!(module.contains("return result_2;"));
        assert!(module.contains("if (flag_0)"));
        assert!(module.contains("return tmp1_1;"));
        assert!(module.contains("tmp1_1 = add(2L, 3L);"));
        assert!(module.contains("value_0 = tmp1_1;"));
        assert!(module.contains("lume.core.LumeRuntime.println(value_0)"));

        let _ = fs::remove_dir_all(temp);
    }

    #[test]
    fn lowers_tail_if_let_value_inside_lambda_body() {
        let temp = temp_path("lume-java-tail-if-let-lambda");
        let source = temp.join("tail_if_let_lambda.lum");
        let out = temp.join("out");
        fs::create_dir_all(&temp).expect("create temp dir");
        fs::write(
            &source,
            r#"
module demo/tail_if_let_lambda

def applyMaybe(work fn(Option[Int]) Result[Int, Str]) Result[Int, Str] {
    work(Some(5))
}

def main() Unit {
    result Result[Int, Str] = applyMaybe((existing Option[Int]) => {
        if let None = existing {
            Ok(0)
        } else {
            Ok(1)
        }
    })
    println("ok")
}
"#,
        )
        .expect("write source");

        let result = generate_java_path(&source, JavaBackendOptions::new(&out)).expect("generate");

        assert!(result.diagnostics.is_empty());
        let module =
            fs::read_to_string(out.join("demo/tail_if_let_lambda/Tail_if_let_lambdaModule.java"))
                .expect("read module");
        assert!(!module.contains(
            "return ((lume.core.Result<Object, String>) ((Object) lume.core.LumeUnit.INSTANCE));"
        ));
        assert!(module.contains("new lume.core.Result.Ok<>(0L)"));
        assert!(module.contains("new lume.core.Result.Ok<>(1L)"));

        let _ = fs::remove_dir_all(temp);
    }

    #[test]
    fn emits_named_shape_payloads_for_result_cases() {
        let temp = temp_path("lume-java-result-shape-payload");
        let source = temp.join("result_shape.lum");
        let out = temp.join("out");
        fs::create_dir_all(&temp).expect("create temp dir");
        fs::write(
            &source,
            r#"
module demo/result_shape

shape HttpResponse {
    status Int = 200
    body Str
    contentType Str = "application/json"
}

shape HttpError {
    status Int
    body Str
    contentType Str = "application/json"
}

def ok() Result[HttpResponse, HttpError] =
    Ok({ body: "ok" })

def err() Result[HttpResponse, HttpError] =
    Err({ status: 400, body: "bad" })
"#,
        )
        .expect("write source");

        let result = generate_java_path(&source, JavaBackendOptions::new(&out)).expect("generate");

        assert!(result.diagnostics.is_empty());
        let module = fs::read_to_string(out.join("demo/result_shape/Result_shapeModule.java"))
            .expect("read module");
        assert!(!module.contains("UnsupportedOperationException"));
        assert!(module.contains("new HttpResponse(200L"));
        assert!(module.contains("new HttpError(400L"));
        assert!(module.contains("\"application/json\""));

        let _ = fs::remove_dir_all(temp);
    }

    #[test]
    fn emits_named_object_calls_from_reified_methods() {
        let temp = temp_path("lume-java-object-reified-call");
        let source = temp.join("object_reified.lum");
        let out = temp.join("out");
        fs::create_dir_all(&temp).expect("create temp dir");
        fs::write(
            &source,
            r#"
module demo/object_reified

object Cache {

    def label(targetType Type[_]) Str =
        targetType.name().getOr("?")
}


class Reader {

    def read[reified T]() Str {
        Cache.label(typeOf[T])
    }
}

"#,
        )
        .expect("write source");

        let result = generate_java_path(&source, JavaBackendOptions::new(&out)).expect("generate");

        assert!(result.diagnostics.is_empty());
        let reader =
            fs::read_to_string(out.join("demo/object_reified/Reader.java")).expect("read reader");
        assert!(!reader.contains("UnsupportedOperationException"));
        assert!(reader.contains("Cache.INSTANCE.label"));
        assert!(reader.contains("__type_T_1"));

        let _ = fs::remove_dir_all(temp);
    }

    #[test]
    fn emits_object_field_initializers() {
        let temp = temp_path("lume-java-object-field-init");
        let source = temp.join("object_field_init.lum");
        let out = temp.join("out");
        fs::create_dir_all(&temp).expect("create temp dir");
        fs::write(
            &source,
            r#"
module demo/object_field_init

object Cache {
    hidden var values Map[Str, Str] = Map()

    def remember(key Str, value Str) Unit {
        updated Map[Str, Str] = this.values.put(key, value)
        this.values := updated
    }
}

"#,
        )
        .expect("write source");

        let result = generate_java_path(&source, JavaBackendOptions::new(&out)).expect("generate");

        assert!(result.diagnostics.is_empty());
        let cache =
            fs::read_to_string(out.join("demo/object_field_init/Cache.java")).expect("read cache");
        assert!(!cache.contains("UnsupportedOperationException"));
        assert!(cache.contains("this.values ="));
        assert!(cache.contains("lume.core.LumeMap"));

        let _ = fs::remove_dir_all(temp);
    }

    #[test]
    fn emits_core_enum_constructors_and_indirect_calls_in_block_methods() {
        if !command_available("javac") {
            eprintln!("skipping Java callback test because javac is not available");
            return;
        }

        let temp = temp_path("lume-java-callback-blocks");
        let source = temp.join("callbacks.lum");
        let out = temp.join("out");
        let classes = temp.join("classes");
        fs::create_dir_all(&temp).expect("create temp dir");
        fs::write(
            &source,
            r#"
module demo/callbacks

class Runner {

    def call(value Int, mapper fn(Int) Result[Int, Str]) Result[Int, Str] {
        mapped Result[Int, Str] = mapper(value)
        match mapped {
            case Ok { value as item } => Ok(item)
            case Err { error } => Err(error)
        }
    }

    def maybe(flag Bool) Result[Option[Int], Str] {
        if flag {
            Ok(Some(5))
        } else {
            Ok(None)
        }
    }
}

"#,
        )
        .expect("write source");

        let result = generate_java_path(&source, JavaBackendOptions::new(&out)).expect("generate");

        assert!(result.diagnostics.is_empty());
        let runner =
            fs::read_to_string(out.join("demo/callbacks/Runner.java")).expect("read runner");
        assert!(!runner.contains("UnsupportedOperationException"));
        assert!(runner.contains("mapper_2.apply"));
        assert!(runner.contains("new lume.core.Result.Ok<>"));
        assert!(runner.contains("new lume.core.Result.Err<>"));
        assert!(runner.contains("new lume.core.Option.Some<>"));
        assert!(runner.contains("new lume.core.Option.None<>"));

        let mut sources = core_runtime_sources();
        collect_java_sources(&out, &mut sources).expect("collect generated java");
        fs::create_dir_all(&classes).expect("create classes dir");
        run_checked(
            Command::new("javac").arg("-d").arg(&classes).args(&sources),
            "javac",
        );

        let _ = fs::remove_dir_all(temp);
    }

    #[test]
    fn emits_lume_positional_constructor_calls() {
        let temp = temp_path("lume-java-lume-constructors");
        let source = temp.join("constructors.lum");
        let out = temp.join("out");
        fs::create_dir_all(&temp).expect("create temp dir");
        fs::write(
            &source,
            r#"
module demo/constructors

class Greeter {
    def hello() Str = "hi"
}

def main() Unit {
    greeter Greeter = Greeter()
    println(greeter.hello())
}
"#,
        )
        .expect("write source");

        let result = generate_java_path(&source, JavaBackendOptions::new(&out)).expect("generate");

        assert!(result.diagnostics.is_empty());
        let module = fs::read_to_string(out.join("demo/constructors/ConstructorsModule.java"))
            .expect("read module");
        assert!(!module.contains("UnsupportedOperationException"));
        assert!(module.contains("new Greeter()"));
        assert!(module.contains("greeter_0 = tmp1_1;"));

        let _ = fs::remove_dir_all(temp);
    }

    #[test]
    fn generated_java_exposes_lume_type_descriptors() {
        if !command_available("javac") || !command_available("java") {
            eprintln!("skipping Java metadata test because javac/java is not available");
            return;
        }

        let temp = temp_path("lume-java-metadata");
        let source = temp.join("metadata.lum");
        let out = temp.join("out");
        let classes = temp.join("classes");
        fs::create_dir_all(&temp).expect("create temp dir");
        fs::write(
            &source,
            r#"
module demo/metadata

annotation Route {
    path Str
}

@Route { path: "/users" }
class User {
    name Str
    age Int
}

enum Status {
    case Pending
    case Done {
        label Str
    }
}

def main() Unit {
    user User = User("Ada", 42)

    declared Type[User] = typeOf[User]
    actual Type[User] = user.runtimeType

    println(declared.name() !!)
    println(actual.qualifiedName() !!)
    println(declared.kind())

    classType ClassType[User] = declared.asClass() !!
    fields [Field] = classType.fields()
    println(fields.size())

    nameField Field = fields.at(0) !!
    ageField Field = fields.at(1) !!

    println(nameField.name())
    println(nameField.fieldType().name() !!)
    println(ageField.name())
    println(ageField.fieldType().name() !!)

    enumType EnumType[Status] = typeOf[Status].asEnum() !!
    println(enumType.name() !!)
    println(enumType.kind())
    println((enumType.case("Pending") !!).name())
}
"#,
        )
        .expect("write source");

        let generated =
            generate_java_path(&source, JavaBackendOptions::new(&out)).expect("generate java");
        assert!(generated.diagnostics.is_empty());

        let user = fs::read_to_string(out.join("demo/metadata/User.java")).expect("read user");
        assert!(user.contains("static final lume.core.LumeType TYPE"));
        assert!(user.contains("lume.core.LumeField.of(\"name\""));
        assert!(user.contains("lume.core.LumeAnnotationField.of(\"path\", \"/users\")"));

        let module =
            fs::read_to_string(out.join("demo/metadata/MetadataModule.java")).expect("read module");
        assert!(!module.contains("UnsupportedOperationException"));
        assert!(module.contains("tmp3_3 = User.TYPE;"));
        assert!(module.matches("User.TYPE").count() >= 2);

        let mut sources = core_runtime_sources();
        collect_java_sources(&out, &mut sources).expect("collect generated java");
        fs::create_dir_all(&classes).expect("create classes dir");
        run_checked(
            Command::new("javac").arg("-d").arg(&classes).args(&sources),
            "javac",
        );

        let output = run_checked(
            Command::new("java")
                .arg("-cp")
                .arg(&classes)
                .arg("demo.metadata.MetadataMain"),
            "java",
        );
        let actual = String::from_utf8(output.stdout).expect("java stdout utf8");
        assert_eq!(
            actual,
            "User\ndemo.metadata.User\nClass\n2\nname\nStr\nage\nInt\nStatus\nEnum\nPending\n"
        );

        let _ = fs::remove_dir_all(temp);
    }

    #[test]
    fn generated_java_compiles_generic_list_appends() {
        if !command_available("javac") {
            eprintln!("skipping Java generic list append test because javac is not available");
            return;
        }

        let temp = temp_path("lume-java-generic-list-append");
        let source = temp.join("generic_list_append.lum");
        let out = temp.join("out");
        let classes = temp.join("classes");
        fs::create_dir_all(&temp).expect("create temp dir");
        fs::write(
            &source,
            r#"
module demo/genericappend

def mapItems[T](items [T], mapper fn(T) T) [T] {
    out [T] = []

    for item <- items {
        out.add(mapper(item))
    }

    out
}

def main() Unit {
}
"#,
        )
        .expect("write source");

        let generated =
            generate_java_path(&source, JavaBackendOptions::new(&out)).expect("generate java");
        assert!(generated.diagnostics.is_empty());

        let module = fs::read_to_string(out.join("demo/genericappend/GenericappendModule.java"))
            .expect("read module");
        assert!(module.contains("out_"));
        assert!(module.contains(".add(((T)"));

        let mut sources = core_runtime_sources();
        collect_java_sources(&out, &mut sources).expect("collect generated java");
        fs::create_dir_all(&classes).expect("create classes dir");
        run_checked(
            Command::new("javac").arg("-d").arg(&classes).args(&sources),
            "javac",
        );

        let _ = fs::remove_dir_all(temp);
    }

    #[test]
    fn generated_java_runs_indexed_collection_methods() {
        if !command_available("javac") || !command_available("java") {
            eprintln!("skipping Java LinkedList test because javac/java is not available");
            return;
        }

        let temp = temp_path("lume-java-linked-list-unsafe-extract");
        let source = temp.join("linked_list.lum");
        let out = temp.join("out");
        let classes = temp.join("classes");
        fs::create_dir_all(&temp).expect("create temp dir");
        fs::write(
            &source,
            r#"
module demo/linkedlist

shape User {
    name Str
    cost Int
}

def main() Unit {
    users LinkedList[User] = LinkedList {}
    users.add(User { name: "Ada", cost: 3 })
    inserted Unit = users.insertAt(0, User { name: "Bob", cost: 2 }) !!
    println(users.at(0)!!.name)
    println(users.setAt(0, User { name: "Cara", cost: 4 })!!.name)
    println(users.removeAt(1)!!.name)
    println(users.fold(0, (cost, user) => cost + user.cost))

    values [Int] = [1, 2]
    println(values.setAt(0, 3) !!)
    vectorInserted Unit = values.insertAt(1, 4) !!
    println(values.removeAt(2) !!)
    println(values[0])
    match values.removeAt(9) {
        case Err { error } => {
            println(error.index)
            println(error.size)
        }
        case Ok { value: _ } => ()
    }

    array Array[Int] = Array.fill(2, 5)
    println(array.at(1) !!)
    println(array.setAt(1, 7) !!)
    println(array[1])

    result Result[Int, Str] = Ok(7)
    println(result !!)
}
"#,
        )
        .expect("write source");

        let generated =
            generate_java_path(&source, JavaBackendOptions::new(&out)).expect("generate java");
        assert!(
            generated.diagnostics.is_empty(),
            "{:#?}",
            generated.diagnostics
        );

        let module = fs::read_to_string(out.join("demo/linkedlist/LinkedlistModule.java"))
            .expect("read module");
        assert!(module.contains("lume.core.LumeLinkedList"));
        assert!(module.contains("lume.core.LumeRuntime.extractSuccessValue"));

        let mut sources = core_runtime_sources();
        collect_java_sources(&out, &mut sources).expect("collect generated java");
        fs::create_dir_all(&classes).expect("create classes dir");
        run_checked(
            Command::new("javac").arg("-d").arg(&classes).args(&sources),
            "javac",
        );

        let output = run_checked(
            Command::new("java")
                .arg("-cp")
                .arg(&classes)
                .arg("demo.linkedlist.LinkedlistMain"),
            "java",
        );
        assert_eq!(
            String::from_utf8(output.stdout).expect("java stdout utf8"),
            "Bob\nBob\nAda\n4\n1\n2\n3\n9\n2\n5\n5\n7\n7\n"
        );

        let _ = fs::remove_dir_all(temp);
    }

    #[test]
    fn generated_java_supports_inline_constructors_and_map_index_assignment() {
        if !command_available("javac") || !command_available("java") {
            eprintln!("skipping Java map assignment test because a JDK tool is unavailable");
            return;
        }

        let temp = temp_path("lume-java-map-index-assignment");
        let source = temp.join("map_index_assignment.lum");
        let out = temp.join("out");
        let classes = temp.join("classes");
        fs::create_dir_all(&temp).expect("create temp dir");
        fs::write(
            &source,
            r#"
module demo/mapassignment

def emptyMap() [Str : Int] = []
def mapSize(values [Str : Int]) Int = values.size()

class Cache {
    hidden var values [Str : Int] = []

    new() {}

    def currentValue() Int = 7

    def empty() [Str : Int] = []

    def store(key Str) Unit {
        values[key] := currentValue()
    }

    def reset() Unit {
        this.values := empty()
    }

    def lookup(key Str) Int = values[key] ?? -1
}

def main() Unit {
    cache = Cache()
    cache.store("answer")
    println(cache.lookup("answer"))
    cache.reset()
    println(cache.lookup("answer"))
    println(emptyMap().size())
    println(mapSize([]))
}
"#,
        )
        .expect("write source");

        let generated =
            generate_java_path(&source, JavaBackendOptions::new(&out)).expect("generate java");
        assert!(
            generated.diagnostics.is_empty(),
            "{:#?}",
            generated.diagnostics
        );

        let mut sources = core_runtime_sources();
        collect_java_sources(&out, &mut sources).expect("collect generated java");
        fs::create_dir_all(&classes).expect("create classes dir");
        run_checked(
            Command::new("javac").arg("-d").arg(&classes).args(&sources),
            "javac",
        );

        let output = run_checked(
            Command::new("java")
                .arg("-cp")
                .arg(&classes)
                .arg("demo.mapassignment.MapassignmentMain"),
            "java",
        );
        assert_eq!(
            String::from_utf8(output.stdout).expect("java stdout utf8"),
            "7\n-1\n0\n0\n"
        );

        let _ = fs::remove_dir_all(temp);
    }

    #[test]
    fn generated_java_supports_high_arity_lambdas() {
        if !command_available("javac") || !command_available("java") {
            eprintln!("skipping Java high-arity lambda test because a JDK tool is not available");
            return;
        }

        let temp = temp_path("lume-java-high-arity-lambda");
        let source = temp.join("high_arity_lambda.lum");
        let out = temp.join("out");
        let classes = temp.join("classes");
        fs::create_dir_all(&temp).expect("create temp dir");
        fs::write(
            &source,
            r#"
module demo/higharity

def apply7(f fn(Int, Int, Int, Int, Int, Int, Int) Int) Int {
    f(1, 2, 3, 4, 5, 6, 7)
}

def main() Unit {
    total Int = apply7((a, b, c, d, e, g, h) => a + b + c + d + e + g + h)
    println(total)
}
"#,
        )
        .expect("write source");

        let generated =
            generate_java_path(&source, JavaBackendOptions::new(&out)).expect("generate java");
        assert!(generated.diagnostics.is_empty());

        let module = fs::read_to_string(out.join("demo/higharity/HigharityModule.java"))
            .expect("read module");
        assert!(module.contains("lume.core.Function7<"));
        assert!(module.contains(".apply(1L, 2L, 3L, 4L, 5L, 6L, 7L)"));
        assert!(module.contains("public Long apply("));

        let mut sources = core_runtime_sources();
        collect_java_sources(&out, &mut sources).expect("collect generated java");
        fs::create_dir_all(&classes).expect("create classes dir");
        run_checked(
            Command::new("javac").arg("-d").arg(&classes).args(&sources),
            "javac",
        );

        let output = run_checked(
            Command::new("java")
                .arg("-cp")
                .arg(&classes)
                .arg("demo.higharity.HigharityMain"),
            "java",
        );
        assert_eq!(
            String::from_utf8(output.stdout).expect("java stdout utf8"),
            "28\n"
        );

        let _ = fs::remove_dir_all(temp);
    }

    #[test]
    fn generated_java_matches_interpreter_for_supported_program() {
        if !command_available("javac") || !command_available("java") {
            eprintln!("skipping Java parity test because javac/java is not available");
            return;
        }

        let temp = temp_path("lume-java-parity");
        let source = temp.join("parity.lum");
        let out = temp.join("out");
        let classes = temp.join("classes");
        fs::create_dir_all(&temp).expect("create temp dir");
        fs::write(
            &source,
            r#"
module demo/parity

def add(left Int, right Int) Int {
    result Int = left + right
    result
}

def main() Int {
    value Int = add(2, 3)
    println(value)

    if value > 4 {
        println("bigger")
    } else {
        println("smaller")
    }

    var next Option[Int] = Some(2)
    while let item <- next && item == 2 {
        println(item)
        next := None
    }

    next := Some(3)
    while let Some { value as item } = next {
        println(item)
        next := None
    }

    0
}
"#,
        )
        .expect("write source");

        let interpreted = run_path(&source, None).expect("run interpreter");
        assert!(interpreted.diagnostics.is_empty());
        let expected = interpreter_stdout(interpreted);

        let generated =
            generate_java_path(&source, JavaBackendOptions::new(&out)).expect("generate java");
        assert!(generated.diagnostics.is_empty());

        let mut sources = core_runtime_sources();
        collect_java_sources(&out, &mut sources).expect("collect generated java");
        fs::create_dir_all(&classes).expect("create classes dir");
        run_checked(
            Command::new("javac").arg("-d").arg(&classes).args(&sources),
            "javac",
        );

        let output = run_checked(
            Command::new("java")
                .arg("-cp")
                .arg(&classes)
                .arg("demo.parity.ParityMain"),
            "java",
        );
        let actual = String::from_utf8(output.stdout).expect("java stdout utf8");
        assert_eq!(actual, expected);

        let _ = fs::remove_dir_all(temp);
    }

    fn temp_path(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time after epoch")
            .as_nanos();
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        manifest_dir
            .parent()
            .and_then(|crates_dir| crates_dir.parent())
            .expect("rust workspace root")
            .join("target")
            .join(format!("{prefix}-{nanos}"))
    }

    fn interpreter_stdout(result: crate::PathRunResult) -> String {
        let mut output = result.output;
        if let Some(value) = result.return_value {
            output.push_str(&value);
            output.push('\n');
        }
        output
    }

    fn command_available(name: &str) -> bool {
        Command::new(name).arg("--version").output().is_ok()
            || Command::new(name).arg("-version").output().is_ok()
    }

    fn create_widget_jar(temp: &Path) -> PathBuf {
        let src_dir = temp.join("java-src/third/party");
        let classes = temp.join("java-classes");
        let jar = temp.join("widget.jar");
        fs::create_dir_all(&src_dir).expect("create java src dir");
        fs::create_dir_all(&classes).expect("create java classes dir");
        let source = src_dir.join("Widget.java");
        fs::write(
            &source,
            r#"
package third.party;

public final class Widget {
    private final String label;
    private final long count;

    public Widget(String label, long count) {
        this.label = label;
        this.count = count;
    }

    public static Widget create(String label) {
        return new Widget(label, 0L);
    }

    public String label() {
        return label;
    }

    public long count() {
        return count;
    }
}
"#,
        )
        .expect("write widget java source");
        let generic_source = src_dir.join("GenericBox.java");
        fs::write(
            &generic_source,
            r#"
package third.party;

public final class GenericBox<T> {
    private final T value;

    public GenericBox(T value) {
        this.value = value;
    }

    public T value() {
        return value;
    }
}
"#,
        )
        .expect("write generic box java source");
        run_checked(
            Command::new("javac")
                .arg("-d")
                .arg(&classes)
                .arg(&source)
                .arg(&generic_source),
            "javac",
        );
        run_checked(
            Command::new("jar")
                .arg("cf")
                .arg(&jar)
                .arg("-C")
                .arg(&classes)
                .arg("."),
            "jar",
        );
        jar
    }

    fn create_router_jar(temp: &Path) -> PathBuf {
        let src_dir = temp.join("java-src/third/party");
        let classes = temp.join("java-classes");
        let jar = temp.join("router.jar");
        fs::create_dir_all(&src_dir).expect("create java src dir");
        fs::create_dir_all(&classes).expect("create java classes dir");

        let api_source = src_dir.join("RoutingApi.java");
        fs::write(
            &api_source,
            r#"
package third.party;

public interface RoutingApi<API extends RoutingApi<API>> {
    @SuppressWarnings("unchecked")
    default API ping() {
        return (API) this;
    }
}
"#,
        )
        .expect("write routing api java source");

        let router_source = src_dir.join("Router.java");
        fs::write(
            &router_source,
            r#"
package third.party;

public final class Router implements RoutingApi<Router> {
    public Router() {
    }
}
"#,
        )
        .expect("write router java source");

        run_checked(
            Command::new("javac")
                .arg("-d")
                .arg(&classes)
                .arg(&api_source)
                .arg(&router_source),
            "javac",
        );
        run_checked(
            Command::new("jar")
                .arg("cf")
                .arg(&jar)
                .arg("-C")
                .arg(&classes)
                .arg("."),
            "jar",
        );
        jar
    }

    fn core_runtime_sources() -> Vec<PathBuf> {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let repo_root = manifest_dir
            .parent()
            .and_then(|crates_dir| crates_dir.parent())
            .and_then(|rust_dir| rust_dir.parent())
            .expect("repo root");
        let runtime_dir = repo_root.join("lume/core/src/main/java/lume/core");
        let mut sources = Vec::new();
        collect_java_source_files(&runtime_dir, &mut sources).expect("collect core java");

        for source_name in ["Option", "Result", "Either"] {
            let source = repo_root.join(format!(
                "lume/core/src/main/lume/lume/core/{source_name}.lum"
            ));
            let generated_core =
                temp_path(&format!("lume-java-core-{}", source_name.to_lowercase()));
            let result = generate_java_path(&source, JavaBackendOptions::new(&generated_core))
                .unwrap_or_else(|err| panic!("generate core {source_name} java: {err}"));
            assert!(
                result.diagnostics.is_empty(),
                "core {source_name} java generation produced diagnostics: {:?}",
                result.diagnostics
            );
            let expected_file_name = format!("{source_name}.java");
            sources.extend(result.written_files.into_iter().filter(|path| {
                path.file_name()
                    .is_some_and(|name| name == expected_file_name.as_str())
            }));
        }
        sources
    }

    fn collect_java_source_files(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "java") {
                out.push(path);
            }
        }
        Ok(())
    }

    fn collect_java_sources(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                collect_java_sources(&path, out)?;
            } else if path.extension().is_some_and(|ext| ext == "java") {
                out.push(path);
            }
        }
        Ok(())
    }

    fn run_checked(command: &mut Command, name: &str) -> std::process::Output {
        let output = command
            .output()
            .unwrap_or_else(|err| panic!("run {name}: {err}"));
        if !output.status.success() {
            panic!(
                "{name} failed\nstdout:\n{}\nstderr:\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        output
    }
}
