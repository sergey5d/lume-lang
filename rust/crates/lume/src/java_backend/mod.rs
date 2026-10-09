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
        TypeRef::Union { members, .. } => members
            .iter()
            .all(|member| java_library_type_ref_is_exposed(member, exposed_names)),
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
        TypeRef::Union { members, .. } => {
            for member in members {
                collect_java_library_type_ref(member, names);
            }
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
        TypeRef::Union { members, span } => TypeRef::Union {
            members: members
                .iter()
                .map(|member| {
                    sanitize_java_type_ref_for_library(member, exposed_names, fallback_span)
                })
                .collect(),
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
        getter: false,
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
        equals_body: false,
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
        TypeRef::Union { members, span } => TypeRef::Union {
            members: members
                .iter()
                .map(|member| substitute_java_type_ref(member, subst))
                .collect(),
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
            "Any" | "Bool" | "Int" | "Float" | "Rune" | "Str" | "Unit" | "LumeTyped"
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
        run_path, run_path_with_args,
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
    fn hides_java_runtime_marker_from_imported_lume_interfaces() {
        let span = Span::new(0, 0, LineColumn::new(1, 1), LineColumn::new(1, 1));
        let local_type_names = HashMap::new();
        let ctx = JavaTypeContext {
            type_params: HashSet::new(),
            local_type_names: &local_type_names,
            current_package: "lume.http",
            allow_cross_package_refs: true,
            span,
        };

        let bounds = parse_javap_bounds(
            "public interface lume.http.Context extends lume.core.LumeTyped {",
            TypeKind::Interface,
            &ctx,
        );

        assert!(bounds.is_empty());
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

type Maybe[T] =
    object None {}
    | class Some { value T }

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

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
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
        assert!(runner.contains("LumeRuntime.setArgs(args);"));
        assert!(runner.contains("AppModule.main();"));

        let shape = fs::read_to_string(out.join("demo/app/Point.java")).expect("read shape");
        assert!(shape.contains("record Point(Long x, Long y)"));
        assert!(shape.contains("import lume.core.LumeType;"));
        assert!(shape.contains("public LumeType runtimeType()"));
        assert!(!shape.contains("default LumeType runtimeType()"));

        let class = fs::read_to_string(out.join("demo/app/User.java")).expect("read class");
        assert!(class.contains("class User"));
        assert!(class.contains("String name;"));
        assert!(class.contains("Long age;"));

        let runtime_box =
            fs::read_to_string(out.join("demo/app/RuntimeBox.java")).expect("read runtime box");
        assert!(runtime_box.contains("import lume.core.LumeVector;"));
        assert!(runtime_box.contains("LumeVector<Long> items;"));
        assert!(runtime_box.contains("LumeSet<String> names;"));
        assert!(runtime_box.contains("LumeMap<String, LumeVector<Long>> index;"));
        assert!(runtime_box.contains("Option<String> maybe;"));
        assert!(runtime_box.contains("Result<Long, String> result;"));
        assert!(runtime_box.contains("Either<String, Long> either;"));
        assert!(runtime_box.contains("Tuple2<Long, String> pair;"));

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
    fn generated_java_forwards_program_arguments_to_os_args() {
        if !command_available("javac") || !command_available("java") {
            eprintln!("skipping program-argument Java test because a JDK tool is not available");
            return;
        }

        let temp = temp_path("lume-java-program-args");
        let source = temp.join("program_args.lum");
        let out = temp.join("out");
        let classes = temp.join("classes");
        fs::create_dir_all(&temp).expect("create temp dir");
        fs::write(
            &source,
            r#"
module demo/args

def main() Unit {
    received = OS.args
    received.add("local-only")
    fresh = OS.args
    println(fresh.size, fresh[0], fresh[1])
}
"#,
        )
        .expect("write source");

        let arguments = vec!["alpha".to_string(), "two words".to_string()];
        let interpreted =
            run_path_with_args(&source, None, &arguments).expect("run interpreter with arguments");
        assert!(interpreted.diagnostics.is_empty());
        let expected = interpreter_stdout(interpreted);
        assert_eq!(expected, "2 alpha two words\n");

        let generated =
            generate_java_path(&source, JavaBackendOptions::new(&out)).expect("generate java");
        assert!(
            generated.diagnostics.is_empty(),
            "{:#?}",
            generated.diagnostics
        );

        let module = fs::read_to_string(out.join("demo/args/ArgsModule.java"))
            .expect("read generated module");
        assert!(module.contains("LumeRuntime.args()"));

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
                .arg("demo.args.ArgsMain")
                .arg("alpha")
                .arg("two words"),
            "java",
        );
        assert_eq!(
            String::from_utf8(output.stdout).expect("java stdout utf8"),
            expected
        );

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
        assert!(module.contains("() -> fail()"));

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

    println((parsedInt !) + 1)
    println((parsedFloat !) + 0.5)
    println(Int.parse("oops").isEmpty)
}
"#,
        )
        .expect("write source");

        let result = generate_java_path(&source, JavaBackendOptions::new(&out)).expect("generate");
        assert!(result.diagnostics.is_empty());

        let module = fs::read_to_string(out.join("demo/predefparse/PredefparseModule.java"))
            .expect("read module");
        assert!(module.contains("import lume.core.LumeRuntime;"));
        assert!(module.contains("LumeRuntime.parseInt("));
        assert!(module.contains("LumeRuntime.parseFloat("));
        assert!(!module.contains("__block"), "{module}");

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
    keys [Str] = merged.keys()
    empty [Str : Int] = []
    println(entries.size, copy.size, merged.size, entryList.size, keys.size, empty.size)
}
"#,
        )
        .expect("write source");

        let result = generate_java_path(&source, JavaBackendOptions::new(&out)).expect("generate");
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);

        let module = fs::read_to_string(out.join("demo/mapliteral/MapliteralModule.java"))
            .expect("read module");
        assert!(module.contains("import lume.core.LumeMap;"));
        assert!(module.contains("LumeMap.fromParts("));
        assert!(module.contains("new Tuple2<>("));
        assert!(module.contains(".entries()"));
        assert!(module.contains(".keys()"));
        assert!(module.contains("LumeMap.empty()"));

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

    pair (Int, Str) = (5, "right")
    number Int = pair[0]
    text Str = pair[1]
    println(number, text)
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
        assert!(module.contains("Long number = pair.first();"));
        assert!(module.contains("String text = pair.second();"));
        assert!(!module.contains("__block"), "{module}");

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
            "3\n5 right\n"
        );

        let _ = fs::remove_dir_all(temp);
    }

    #[test]
    fn readable_java_keeps_record_destructuring_structured() {
        if !command_available("javac") || !command_available("java") {
            eprintln!("skipping record destructuring Java test because a JDK tool is unavailable");
            return;
        }

        let temp = temp_path("lume-java-record-destructuring");
        let source = temp.join("record_destructuring.lum");
        let out = temp.join("out");
        let classes = temp.join("classes");
        fs::create_dir_all(&temp).expect("create temp dir");
        fs::write(
            &source,
            r#"
module demo/recorddestructuring

shape User {
    name Str
    age Int
}

def main() Unit {
    user User = User { name: "Ada", age: 3 }
    let User { name as userName, age } as wholeUser = user
    var total Int = age + wholeUser.age
    for let User { age as nextAge } as nextUser <- [user] {
        total += nextAge + nextUser.age
    }
    unknown Any = user
    if let User { age as ifAge } as ifUser = unknown {
        total += ifAge + ifUser.age
    }
    current User? = Some(user)
    while let Some(nextUser) as some = current {
        total += nextUser.age + some.value.age
        break
    }
    println(userName, wholeUser.name, total)
}
"#,
        )
        .expect("write source");

        let result = generate_java_path(&source, JavaBackendOptions::new(&out)).expect("generate");
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);

        let module =
            fs::read_to_string(out.join("demo/recorddestructuring/RecorddestructuringModule.java"))
                .expect("read module");
        assert!(module.contains(".name()"), "{module}");
        assert!(module.contains(".age()"), "{module}");
        assert!(!module.contains("__block"), "{module}");

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
                .arg("demo.recorddestructuring.RecorddestructuringMain"),
            "java",
        );
        assert_eq!(
            String::from_utf8(output.stdout).expect("java stdout utf8"),
            "Ada Ada 24\n"
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
        assert!(!module.contains("__block"), "{module}");

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
    fn generated_java_runs_option_wrap_operator() {
        if !command_available("javac") || !command_available("java") {
            eprintln!("skipping option wrap Java test because a JDK tool is unavailable");
            return;
        }

        let temp = temp_path("lume-java-option-wrap-operator");
        let source = temp.join("option_wrap.lum");
        let out = temp.join("out");
        let classes = temp.join("classes");
        fs::create_dir_all(&temp).expect("create temp dir");
        fs::write(
            &source,
            r#"
module demo/option_wrap

shape Point {
    x Int
    y Int
}

def optionValue() Option[Int] = ^5
def resultValue() Result[Str, Str] = Ok("ready")
def eitherValue() Either[Str, Int] = Right(9)
def nestedValue() Option[Option[Int]] = ^^11
def pointValue() Point? = ^new(2, 3)
def choose(flag Bool) Option[Int] = if flag { ^12 } else { ^0 }
def consume(value Option[Int]) Int = value ?? 0
def read(value Option[Int]) Int {
    let {
        ^item = value
    } else return 0
    item
}

def main() Unit {
    inferred = ^6
    println(optionValue()!)
    println(inferred!)
    println(resultValue()!)
    println(eitherValue()!)
    println(nestedValue()!!)
    println(pointValue()!.x)
    println(choose(true)!)
    println(consume(^13))
    println(read(^14))
}
"#,
        )
        .expect("write source");

        let result = generate_java_path(&source, JavaBackendOptions::new(&out)).expect("generate");
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);

        let module = fs::read_to_string(out.join("demo/option_wrap/Option_wrapModule.java"))
            .expect("read module");
        assert!(module.contains("new Option.Some"), "{module}");
        assert!(module.contains("new Result.Ok"), "{module}");
        assert!(module.contains("new Either.Right"), "{module}");

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
                .arg("demo.option_wrap.Option_wrapMain"),
            "java",
        );
        assert_eq!(
            String::from_utf8(output.stdout).expect("java stdout utf8"),
            "5\n6\nready\n9\n11\n2\n12\n13\n14\n"
        );

        let _ = fs::remove_dir_all(temp);
    }

    #[test]
    fn generated_java_infers_equals_block_return_type() {
        if !command_available("javac") || !command_available("java") {
            eprintln!("skipping inferred return Java test because a JDK tool is unavailable");
            return;
        }

        let temp = temp_path("lume-java-inferred-return");
        let source = temp.join("inferred_return.lum");
        let out = temp.join("out");
        let classes = temp.join("classes");
        fs::create_dir_all(&temp).expect("create temp dir");
        fs::write(
            &source,
            r#"
module demo/inferredreturn

def flag() = {
    false
}

def main() Unit {
    if flag() {
        println("wrong")
    } else {
        println("ok")
    }
}
"#,
        )
        .expect("write source");

        let result = generate_java_path(&source, JavaBackendOptions::new(&out)).expect("generate");
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);

        let module = fs::read_to_string(out.join("demo/inferredreturn/InferredreturnModule.java"))
            .expect("read module");
        assert!(module.contains("static Boolean flag()"), "{module}");

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
                .arg("demo.inferredreturn.InferredreturnMain"),
            "java",
        );
        assert_eq!(
            String::from_utf8(output.stdout).expect("java stdout utf8"),
            "ok\n"
        );

        let _ = fs::remove_dir_all(temp);
    }

    #[test]
    fn emits_structured_java_for_simple_union_match_methods() {
        let temp = temp_path("lume-java-union-match-methods");
        let source = temp.join("maybe.lum");
        let out = temp.join("out");
        fs::create_dir_all(&temp).expect("create temp dir");
        fs::write(
            &source,
            r#"
module demo/maybe

type Maybe[T] =
    object None {}
    | class Some { value T }

ext Maybe[T] {
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
        assert!(maybe.contains("switch (__match1)"));
        assert!(maybe.contains("case Some<?> __case"));
        assert!(maybe.contains("case None<?> __case"));
        assert!(maybe.contains("return ((T) __case"));

        let _ = fs::remove_dir_all(temp);
    }

    #[test]
    fn generates_readable_java() {
        let temp = temp_path("lume-java-readable");
        let source = temp.join("readable.lum");
        let out = temp.join("generated");
        fs::create_dir_all(&temp).expect("create temp dir");
        fs::write(
            &source,
            r#"
module demo/readable

def add(left Int, right Int) Int {
    total Int = left + right
    total
}

def twice(value Int) Int = add(value, value)

def choose(flag Bool) Int = if flag {
    10
} else {
    20
}

def sumEven(limit Int) Int {
    var total Int = 0
    var index Int = 0
    while index < limit {
        if index % 2 == 0 {
            total += index
        } else {
            total += 1
        }
        index += 1
    }
    total
}

def incrementer() fn(Int) Int = (value Int) => value + 1

def classify(value Int) Int {
    var result Int = 0
    match value {
        case 1 => { result := 10 }
        case _ => { result := 20 }
    }
    result
}

def main() Int = twice(3) + choose(true) + sumEven(4) + classify(1)
"#,
        )
        .expect("write source");

        let readable = generate_java_path(&source, JavaBackendOptions::new(&out))
            .expect("generate readable Java");
        assert!(
            readable.diagnostics.is_empty(),
            "{:?}",
            readable.diagnostics
        );

        let relative = Path::new("demo/readable/ReadableModule.java");
        let readable_java = fs::read_to_string(out.join(relative)).expect("read readable Java");

        assert!(readable_java.contains("Long total = (left + right);"));
        assert!(readable_java.contains("return total;"));
        assert!(readable_java.contains("return add(value, value);"));
        assert!(readable_java.contains("if (flag) {"));
        assert!(readable_java.contains("return 10L;"));
        assert!(readable_java.contains("return 20L;"));
        assert!(readable_java.contains("while ((index < limit)) {"));
        assert!(readable_java.contains("total += index;"));
        assert!(readable_java.contains("return (value) -> (value + 1L);"));
        assert!(readable_java.contains("Objects.equals(__match"));
        assert!(readable_java.contains("result = 10L;"));
        assert!(!readable_java.contains("__block"));
        assert!(!readable_java.contains("while (true)"));

        if command_available("javac") {
            let classes = out.join("classes");
            let mut sources = core_runtime_sources();
            collect_java_sources(&out, &mut sources).expect("collect generated Java");
            fs::create_dir_all(&classes).expect("create classes dir");
            run_checked(
                Command::new("javac").arg("-d").arg(&classes).args(&sources),
                "javac",
            );
            let output = run_checked(
                Command::new("java")
                    .arg("-cp")
                    .arg(&classes)
                    .arg("demo.readable.ReadableMain"),
                "java",
            );
            assert_eq!(
                String::from_utf8(output.stdout).expect("Java stdout utf8"),
                "30\n"
            );
        }

        let _ = fs::remove_dir_all(temp);
    }

    #[test]
    fn readable_java_emits_getter_reads_as_zero_argument_calls() {
        let temp = temp_path("lume-java-readable-getters");
        let source = temp.join("getters.lum");
        let out = temp.join("generated");
        let classes = temp.join("classes");
        fs::create_dir_all(&temp).expect("create temp dir");
        fs::write(
            &source,
            r#"
module demo/getters

class Item {
    name Str

    def label Str = this.name
    def repeated Str = label + this.label
}

class Factory {
    def creator fn(Int) Int = (value Int) => value + 1
}

def read(item Item) Str = item.repeated

def main() Unit {
    println(read(Item("Ada")))
    println(Factory().creator(5))
}
"#,
        )
        .expect("write source");

        let result = generate_java_path(&source, JavaBackendOptions::new(&out))
            .expect("generate readable Java");
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);

        let item = fs::read_to_string(out.join("demo/getters/Item.java")).expect("read Item");
        let module =
            fs::read_to_string(out.join("demo/getters/GettersModule.java")).expect("read module");
        assert!(item.contains("return (this.label() + this.label());"));
        assert!(module.contains("return item.repeated();"));

        if command_available("javac") && command_available("java") {
            let mut sources = core_runtime_sources();
            collect_java_sources(&out, &mut sources).expect("collect generated Java");
            fs::create_dir_all(&classes).expect("create classes dir");
            run_checked(
                Command::new("javac").arg("-d").arg(&classes).args(&sources),
                "javac",
            );
            let output = run_checked(
                Command::new("java")
                    .arg("-cp")
                    .arg(&classes)
                    .arg("demo.getters.GettersMain"),
                "java",
            );
            assert_eq!(
                String::from_utf8(output.stdout).expect("Java stdout utf8"),
                "AdaAda\n6\n"
            );
        }

        let _ = fs::remove_dir_all(temp);
    }

    #[test]
    fn readable_java_keeps_simple_for_loops_structured() {
        let temp = temp_path("lume-java-readable-for-loop");
        let source = temp.join("for_loop.lum");
        let out = temp.join("generated");
        fs::create_dir_all(&temp).expect("create temp dir");
        fs::write(
            &source,
            r#"
module demo/for_loop

def sum(items [Int]) Int {
    var total Int = 0
    for item <- items {
        total += item
    }
    total
}
"#,
        )
        .expect("write source");

        let result = generate_java_path(&source, JavaBackendOptions::new(&out))
            .expect("generate readable Java");
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);

        let module = fs::read_to_string(out.join("demo/for_loop/For_loopModule.java"))
            .expect("read module Java");
        assert!(module.contains("LumeIterator<?> __iterator1"), "{module}");
        assert!(module.contains("Long item ="), "{module}");
        assert!(module.contains("while (__iterator"), "{module}");
        assert!(!module.contains("__block"), "{module}");

        let _ = fs::remove_dir_all(temp);
    }

    #[test]
    fn readable_java_keeps_explicit_constructors_structured() {
        let temp = temp_path("lume-java-readable-constructor");
        let source = temp.join("constructor.lum");
        let out = temp.join("out");
        fs::create_dir_all(&temp).expect("create temp dir");
        fs::write(
            &source,
            r#"
module demo/constructor

class Account {
    name Str
    count Int = 1

    new(name Str) {
        this.name = name
    }
}
"#,
        )
        .expect("write source");

        let result = generate_java_path(&source, JavaBackendOptions::new(&out))
            .expect("generate readable Java");
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);

        let account =
            fs::read_to_string(out.join("demo/constructor/Account.java")).expect("read class");
        assert!(account.contains("this.__lume_field_init();"), "{account}");
        assert!(account.contains("this.name = name;"), "{account}");
        assert!(!account.contains("__block"), "{account}");

        let _ = fs::remove_dir_all(temp);
    }

    #[test]
    fn readable_java_implicit_constructor_uses_only_public_fields() {
        let temp = temp_path("lume-java-stable-implicit-constructor");
        let source = temp.join("constructor.lum");
        let out = temp.join("out");
        fs::create_dir_all(&temp).expect("create temp dir");
        fs::write(
            &source,
            r#"
module demo/stableconstructor

class Account {
    owner Str
    internal region Str = "US"
    balance Int
    private cache Int = 7
}

def account() Account = Account("Ada", 10)
"#,
        )
        .expect("write source");

        let result = generate_java_path(&source, JavaBackendOptions::new(&out))
            .expect("generate readable Java");
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);

        let account = fs::read_to_string(out.join("demo/stableconstructor/Account.java"))
            .expect("read class");
        assert!(
            account.contains("public Account(String owner, Long balance)"),
            "{account}"
        );
        assert!(account.contains("this.__lume_field_init();"), "{account}");
        assert!(account.contains("this.owner = owner;"), "{account}");
        assert!(account.contains("this.balance = balance;"), "{account}");

        let _ = fs::remove_dir_all(temp);
    }

    #[test]
    fn readable_java_emits_variadic_reified_and_primitive_coercion_calls() {
        let temp = temp_path("lume-java-readable-call-boundaries");
        let source = temp.join("calls.lum");
        let out = temp.join("out");
        fs::create_dir_all(&temp).expect("create temp dir");
        fs::write(
            &source,
            r#"
module demo/calls

use java/lang/StringBuilder

def collect(values [Int] vararg) [Int] = values

def forwarded(values [Int]) Int = collect(...values).size

def direct() Int = collect(1, 2, 3).size

def typeName[reified T]() Str = typeOf[T].name.getOr("?")

def concreteTypeName() Str = typeName[Int]()

def inserted() Str {
    builder StringBuilder = StringBuilder("bc")
    builder.insert(0, "a")
    builder.toStr()
}
"#,
        )
        .expect("write source");

        let result = generate_java_path(&source, JavaBackendOptions::new(&out))
            .expect("generate readable Java");
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);

        let module =
            fs::read_to_string(out.join("demo/calls/CallsModule.java")).expect("read module");
        assert!(module.contains("collect(values)"), "{module}");
        assert!(
            module.contains("collect(LumeVector.of(1L, 2L, 3L))"),
            "{module}"
        );
        assert!(
            module.contains("typeName(LumeType.primitive(\"Int\"))"),
            "{module}"
        );
        assert!(
            module.contains(".insert(((Number) (0L)).intValue(), \"a\")"),
            "{module}"
        );
        assert!(!module.contains("__block"), "{module}");

        let _ = fs::remove_dir_all(temp);
    }

    #[test]
    fn readable_java_keeps_early_control_transfer_structured() {
        let temp = temp_path("lume-java-readable-early-control-transfer");
        let source = temp.join("early.lum");
        let out = temp.join("out");
        fs::create_dir_all(&temp).expect("create temp dir");
        fs::write(
            &source,
            r#"
module demo/early

def valueOrZero(value Option[Int]) Int {
    if let Some(item) = value {
        return item
    }
    0
}

def firstPositive(values [Int]) Int {
    for value <- values {
        if value <= 0 {
            continue
        }
        return value
    }
    0
}

def cached(values Map[Str, Int], key Str) Int {
    if let Some(value) = values.get(key) {
        return value
    }
    0
}
"#,
        )
        .expect("write source");

        let result = generate_java_path(&source, JavaBackendOptions::new(&out))
            .expect("generate readable Java");
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);

        let module =
            fs::read_to_string(out.join("demo/early/EarlyModule.java")).expect("read module");
        assert!(module.contains("return ((Long) __case"), "{module}");
        assert!(module.contains("continue;"), "{module}");
        assert!(module.contains("return value;"), "{module}");
        assert!(module.contains("values.get(key)"), "{module}");
        assert!(!module.contains("__block"), "{module}");

        let _ = fs::remove_dir_all(temp);
    }

    #[test]
    fn readable_java_keeps_anonymous_object_methods_structured() {
        let temp = temp_path("lume-java-readable-anonymous-interface");
        let source = temp.join("anonymous_interface.lum");
        let out = temp.join("out");
        fs::create_dir_all(&temp).expect("create temp dir");
        fs::write(
            &source,
            r#"
module demo/anonymousinterface

interface Handler {
    def handle(value Int) Int
}

def handler(offset Int) Handler = object with Handler {
    source Int = offset

    def handle(value Int) Int {
        if value > 0 {
            return value + source
        }
        source
    }
}
"#,
        )
        .expect("write source");

        let result = generate_java_path(&source, JavaBackendOptions::new(&out))
            .expect("generate readable Java");
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);

        let module =
            fs::read_to_string(out.join("demo/anonymousinterface/AnonymousinterfaceModule.java"))
                .expect("read module");
        assert!(module.contains("new __LumeObject_"), "{module}");
        assert!(
            module.contains("private final Long __field_source = offset;"),
            "{module}"
        );
        assert!(module.contains("if ((value > 0L))"), "{module}");
        assert!(
            module.contains("return (value + this.source());"),
            "{module}"
        );
        assert!(!module.contains("__block"), "{module}");

        if command_available("javac") {
            let classes = temp.join("classes");
            let mut sources = core_runtime_sources();
            collect_java_sources(&out, &mut sources).expect("collect generated Java");
            fs::create_dir_all(&classes).expect("create classes dir");
            run_checked(
                Command::new("javac").arg("-d").arg(&classes).args(&sources),
                "javac",
            );
        }

        let _ = fs::remove_dir_all(temp);
    }

    #[test]
    fn generated_java_runs_contextual_new_and_behavioral_shapes() {
        if !command_available("javac") || !command_available("java") {
            eprintln!("skipping contextual construction test because a JDK tool is unavailable");
            return;
        }

        let temp = temp_path("lume-java-contextual-new");
        let source = temp.join("contextual_new.lum");
        let out = temp.join("generated");
        let classes = temp.join("classes");
        fs::create_dir_all(&temp).expect("create temp dir");
        fs::write(
            &source,
            r#"
module demo/contextual_new

shape Point {
    x Int
    y Int
}

type Position = Point

class Worker {
    name Str
    age Int
}

class LabeledWorker {
    name Str

    new(label Str) {
        this.name = label + "!"
    }
}

interface Printable {
    def print() Str
}

def main() Unit {
    point Point = { x: 10, y: 20 }
    x = 3
    y = 4
    punned Point = new { x, y }
    worker Worker = { name: "Ada", age: 42 }
    labeled LabeledWorker = { label: "Ben" }
    position Position = new(3, 4)
    anonymous = new { x, y }
    empty = new {}
    widened Any = new { x, y }
    base = 10
    printable Printable = object with Printable {
        x Int = base
        y Int = 12

        def print() Str = "${this.x}:${this.y}"
    }

    println(point.x + point.y)
    println(worker.name, worker.age)
    println(labeled.name)
    println(position.x + position.y)
    println(printable.print())
    println(punned.x + anonymous.y)
}
"#,
        )
        .expect("write source");

        let result = generate_java_path(&source, JavaBackendOptions::new(&out))
            .expect("generate readable Java");
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);

        let module = fs::read_to_string(out.join("demo/contextual_new/Contextual_newModule.java"))
            .expect("read module Java");
        assert!(
            module.contains("Point point = new Point(10L, 20L)"),
            "{module}"
        );
        assert!(
            module.contains("Worker worker = new Worker(\"Ada\", 42L)"),
            "{module}"
        );
        assert!(
            module.contains("LabeledWorker labeled = new LabeledWorker(\"Ben\")"),
            "{module}"
        );
        assert!(
            module.contains("Object anonymous = LumeShape.of"),
            "{module}"
        );
        assert!(
            module.contains("private final Long __field_x = base"),
            "{module}"
        );
        assert!(
            module.contains("private final Long __field_y = 12L"),
            "{module}"
        );

        let mut sources = core_runtime_sources();
        collect_java_sources(&out, &mut sources).expect("collect generated Java");
        fs::create_dir_all(&classes).expect("create classes dir");
        run_checked(
            Command::new("javac").arg("-d").arg(&classes).args(&sources),
            "javac",
        );
        let output = run_checked(
            Command::new("java")
                .arg("-cp")
                .arg(&classes)
                .arg("demo.contextual_new.Contextual_newMain"),
            "java",
        );
        assert_eq!(
            String::from_utf8(output.stdout).expect("Java stdout utf8"),
            "30\nAda 42\nBen!\n7\n10:12\n7\n"
        );

        let _ = fs::remove_dir_all(temp);
    }

    #[test]
    fn readable_java_keeps_block_lambdas_structured() {
        let temp = temp_path("lume-java-readable-block-lambda");
        let source = temp.join("block_lambda.lum");
        let out = temp.join("generated");
        let classes = temp.join("classes");
        fs::create_dir_all(&temp).expect("create temp dir");
        fs::write(
            &source,
            r#"
module demo/block_lambda

def apply(value Int, mapper fn(Int) Int) Int = mapper(value)

def withOffset(offset Int) Int {
    mapper fn(Int) Int = (value Int) => {
        doubled Int = value * 2
        doubled + offset
    }
    apply(3, mapper)
}

def main() Int = withOffset(4)
"#,
        )
        .expect("write source");

        let result = generate_java_path(&source, JavaBackendOptions::new(&out))
            .expect("generate readable Java");
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);

        let module = fs::read_to_string(out.join("demo/block_lambda/Block_lambdaModule.java"))
            .expect("read module Java");
        assert!(module.contains("(value) -> {"), "{module}");
        assert!(module.contains("Long doubled"), "{module}");
        assert!(module.contains("return (doubled"), "{module}");
        assert!(!module.contains("__block"), "{module}");

        if command_available("javac") && command_available("java") {
            let mut sources = core_runtime_sources();
            collect_java_sources(&out, &mut sources).expect("collect generated Java");
            fs::create_dir_all(&classes).expect("create classes dir");
            run_checked(
                Command::new("javac").arg("-d").arg(&classes).args(&sources),
                "javac",
            );
            let output = run_checked(
                Command::new("java")
                    .arg("-cp")
                    .arg(&classes)
                    .arg("demo.block_lambda.Block_lambdaMain"),
                "java",
            );
            assert_eq!(
                String::from_utf8(output.stdout).expect("Java stdout utf8"),
                "10\n"
            );
        }

        let _ = fs::remove_dir_all(temp);
    }

    #[test]
    fn readable_java_uses_method_local_temporary_names_across_nested_lambdas() {
        if !command_available("javac") || !command_available("java") {
            eprintln!("skipping nested lambda temporary test because a JDK tool is unavailable");
            return;
        }

        let temp = temp_path("lume-java-readable-nested-lambda-temporaries");
        let source = temp.join("nested_lambda_temporaries.lum");
        let out = temp.join("generated");
        let classes = temp.join("classes");
        fs::create_dir_all(&temp).expect("create temp dir");
        fs::write(
            &source,
            r#"
module demo/nested_lambda_temporaries

def succeed(value Int) Result[Int, Str] = Ok(value)

def apply(value Int, mapper fn(Int) Result[Int, Str]) Result[Int, Str] =
    mapper(value)

def calculate(value Int) Result[Int, Str] = {
    outer Int = try succeed(value)
    apply(outer, current => {
        inner Int = try succeed(current + 1)
        Ok(inner)
    })
}

def main() Int = calculate(4).getOr(0)
"#,
        )
        .expect("write source");

        let result = generate_java_path(&source, JavaBackendOptions::new(&out))
            .expect("generate readable Java");
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);

        let module = fs::read_to_string(
            out.join("demo/nested_lambda_temporaries/Nested_lambda_temporariesModule.java"),
        )
        .expect("read module Java");
        assert!(module.contains("var __try1"), "{module}");
        assert!(module.contains("var __try2"), "{module}");
        assert!(module.contains("(current) -> {"), "{module}");
        assert!(module.contains("Long outer"), "{module}");
        assert!(module.contains("Long inner"), "{module}");

        let mut sources = core_runtime_sources();
        collect_java_sources(&out, &mut sources).expect("collect generated Java");
        fs::create_dir_all(&classes).expect("create classes dir");
        run_checked(
            Command::new("javac").arg("-d").arg(&classes).args(&sources),
            "javac",
        );
        let output = run_checked(
            Command::new("java")
                .arg("-cp")
                .arg(&classes)
                .arg("demo.nested_lambda_temporaries.Nested_lambda_temporariesMain"),
            "java",
        );
        assert_eq!(
            String::from_utf8(output.stdout).expect("Java stdout utf8"),
            "5\n"
        );

        let _ = fs::remove_dir_all(temp);
    }

    #[test]
    fn readable_java_keeps_mixed_let_conditions_structured() {
        let temp = temp_path("lume-java-readable-let-conditions");
        let source = temp.join("let_conditions.lum");
        let out = temp.join("generated");
        let classes = temp.join("classes");
        fs::create_dir_all(&temp).expect("create temp dir");
        fs::write(
            &source,
            r#"
module demo/let_conditions

def positive(ready Bool, value Int?) Int {
    if ready && let Some(item) = value && item > 0 {
        return item
    }
    0
}

def sum(items [Int]) Int {
    var index Int = 0
    var total Int = 0
    while let item <- items.at(index) && item < 4 {
        total += item
        index += 1
    }
    total
}

def main() Int = positive(true, Some(2)) + sum([1, 2, 5])
"#,
        )
        .expect("write source");

        let result = generate_java_path(&source, JavaBackendOptions::new(&out))
            .expect("generate readable Java");
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);

        let module = fs::read_to_string(out.join("demo/let_conditions/Let_conditionsModule.java"))
            .expect("read module Java");
        assert!(module.contains("if (ready && value instanceof"), "{module}");
        assert!(
            module.contains("while (items.at(index) instanceof"),
            "{module}"
        );
        assert!(!module.contains("__block"), "{module}");

        if command_available("javac") && command_available("java") {
            let mut sources = core_runtime_sources();
            collect_java_sources(&out, &mut sources).expect("collect generated Java");
            fs::create_dir_all(&classes).expect("create classes dir");
            run_checked(
                Command::new("javac").arg("-d").arg(&classes).args(&sources),
                "javac",
            );
            let output = run_checked(
                Command::new("java")
                    .arg("-cp")
                    .arg(&classes)
                    .arg("demo.let_conditions.Let_conditionsMain"),
                "java",
            );
            assert_eq!(
                String::from_utf8(output.stdout).expect("Java stdout utf8"),
                "5\n"
            );
        }

        let _ = fs::remove_dir_all(temp);
    }

    #[test]
    fn readable_java_preserves_by_name_arguments_and_forwarding() {
        let temp = temp_path("lume-java-readable-by-name");
        let source = temp.join("by_name.lum");
        let out = temp.join("generated");
        let classes = temp.join("classes");
        fs::create_dir_all(&temp).expect("create temp dir");
        fs::write(
            &source,
            r#"
module demo/by_name

def fallback() Int {
    println("fallback")
    9
}

def choose(useFallback Bool, value => Int) Int =
    if useFallback { value } else { 5 }

def forward(useFallback Bool, value => Int) Int =
    choose(useFallback, value)

def direct(useFallback Bool) Int =
    forward(useFallback, fallback())

def optionValue() Int {
    present Int? = Some(3)
    present.getOr(fallback())
}

def main() Unit {
    println(direct(false))
    println(direct(true))
    println(optionValue())
}
"#,
        )
        .expect("write source");

        let result = generate_java_path(&source, JavaBackendOptions::new(&out))
            .expect("generate readable Java");
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);

        let module = fs::read_to_string(out.join("demo/by_name/By_nameModule.java"))
            .expect("read module Java");
        assert!(module.contains("choose(useFallback, value)"), "{module}");
        assert!(module.contains("() -> fallback()"), "{module}");
        assert!(module.contains(".getOr(() -> fallback())"), "{module}");
        assert!(!module.contains("__block"), "{module}");

        if command_available("javac") && command_available("java") {
            let mut sources = core_runtime_sources();
            collect_java_sources(&out, &mut sources).expect("collect generated Java");
            fs::create_dir_all(&classes).expect("create classes dir");
            run_checked(
                Command::new("javac").arg("-d").arg(&classes).args(&sources),
                "javac",
            );
            let output = run_checked(
                Command::new("java")
                    .arg("-cp")
                    .arg(&classes)
                    .arg("demo.by_name.By_nameMain"),
                "java",
            );
            assert_eq!(
                String::from_utf8(output.stdout).expect("Java stdout utf8"),
                "5\nfallback\n9\n3\n"
            );
        }

        let _ = fs::remove_dir_all(temp);
    }

    #[test]
    fn readable_java_keeps_let_else_extraction_structured() {
        let temp = temp_path("lume-java-readable-let-else");
        let source = temp.join("let_else.lum");
        let out = temp.join("generated");
        let classes = temp.join("classes");
        fs::create_dir_all(&temp).expect("create temp dir");
        fs::write(
            &source,
            r#"
module demo/let_else

def explicit(value Int?) Int {
    let Some(item) = value else return 0
    item
}

def pair(left Int?, right Int?) Int {
    let {
        a <- left
        b <- right
    } else return -1
    a + b
}

def prefix(values [Int?]) Int {
    var total Int = 0
    for value <- values {
        let item <- value else break
        total += item
    }
    total
}

def main() Int =
    explicit(Some(4)) + pair(Some(2), Some(3)) + pair(Some(2), None) + prefix([Some(1), Some(2), None, Some(9)])
"#,
        )
        .expect("write source");

        let result = generate_java_path(&source, JavaBackendOptions::new(&out))
            .expect("generate readable Java");
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);

        let module = fs::read_to_string(out.join("demo/let_else/Let_elseModule.java"))
            .expect("read module Java");
        assert!(module.contains("if (!(__let"), "{module}");
        assert!(module.contains("instanceof Option.Some<?>"), "{module}");
        assert!(module.contains("break;"), "{module}");
        assert!(!module.contains("__block"), "{module}");

        if command_available("javac") && command_available("java") {
            let mut sources = core_runtime_sources();
            collect_java_sources(&out, &mut sources).expect("collect generated Java");
            fs::create_dir_all(&classes).expect("create classes dir");
            run_checked(
                Command::new("javac").arg("-d").arg(&classes).args(&sources),
                "javac",
            );
            let output = run_checked(
                Command::new("java")
                    .arg("-cp")
                    .arg(&classes)
                    .arg("demo.let_else.Let_elseMain"),
                "java",
            );
            assert_eq!(
                String::from_utf8(output.stdout).expect("Java stdout utf8"),
                "11\n"
            );
        }

        let _ = fs::remove_dir_all(temp);
    }

    #[test]
    fn readable_java_keeps_extract_or_defaults_structured_and_lazy() {
        let temp = temp_path("lume-java-readable-extract-or");
        let source = temp.join("extract_or.lum");
        let out = temp.join("generated");
        let classes = temp.join("classes");
        fs::create_dir_all(&temp).expect("create temp dir");
        fs::write(
            &source,
            r#"
module demo/extract_or

def fallback(label Str, value Int) Int {
    println(label)
    value
}

def main() Unit {
    some Int? = Some(5)
    none Int? = None
    ok Result[Int, Str] = Ok(7)
    left Either[Str, Int] = Left("missing")

    println(some ?? fallback("some fallback", 10))
    println(none ?? fallback("none fallback", 11))
    println(ok ?? fallback("ok fallback", 12))
    println(left ?? fallback("left fallback", 13))
}
"#,
        )
        .expect("write source");

        let result = generate_java_path(&source, JavaBackendOptions::new(&out))
            .expect("generate readable Java");
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);

        let module = fs::read_to_string(out.join("demo/extract_or/Extract_orModule.java"))
            .expect("read module Java");
        assert!(module.contains("instanceof Option.Some<?>"), "{module}");
        assert!(module.contains("instanceof Result.Ok<?, ?>"), "{module}");
        assert!(module.contains("instanceof Either.Right<?, ?>"), "{module}");
        assert!(!module.contains("__block"), "{module}");

        if command_available("javac") && command_available("java") {
            let mut sources = core_runtime_sources();
            collect_java_sources(&out, &mut sources).expect("collect generated Java");
            fs::create_dir_all(&classes).expect("create classes dir");
            run_checked(
                Command::new("javac").arg("-d").arg(&classes).args(&sources),
                "javac",
            );
            let output = run_checked(
                Command::new("java")
                    .arg("-cp")
                    .arg(&classes)
                    .arg("demo.extract_or.Extract_orMain"),
                "java",
            );
            assert_eq!(
                String::from_utf8(output.stdout).expect("Java stdout utf8"),
                "5\nnone fallback\n11\n7\nleft fallback\n13\n"
            );
        }

        let _ = fs::remove_dir_all(temp);
    }

    #[test]
    fn readable_java_keeps_try_bindings_structured() {
        let temp = temp_path("lume-java-readable-try-binding");
        let source = temp.join("try_binding.lum");
        let out = temp.join("generated");
        let classes = temp.join("classes");
        fs::create_dir_all(&temp).expect("create temp dir");
        fs::write(
            &source,
            r#"
module demo/try_binding

def parse(value Str) Result[Int, Str] =
    if value == "4" { Ok(4) } else { Err("invalid Int: " + value) }

def increment(value Str) Result[Int, Str] {
    number Int = try parse(value)
    Ok(number + 1)
}

def main() Unit {
    println(increment("4").getOr(0))
    println(increment("bad").getError())
}
"#,
        )
        .expect("write source");

        let result = generate_java_path(&source, JavaBackendOptions::new(&out))
            .expect("generate readable Java");
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);

        let module = fs::read_to_string(out.join("demo/try_binding/Try_bindingModule.java"))
            .expect("read module Java");
        assert!(module.contains("instanceof Result.Ok<?, ?>"), "{module}");
        assert!(
            module.contains("return (Result<Long, String>) (Object) __try1"),
            "{module}"
        );
        assert!(!module.contains("__block"), "{module}");

        if command_available("javac") && command_available("java") {
            let mut sources = core_runtime_sources();
            collect_java_sources(&out, &mut sources).expect("collect generated Java");
            fs::create_dir_all(&classes).expect("create classes dir");
            run_checked(
                Command::new("javac").arg("-d").arg(&classes).args(&sources),
                "javac",
            );
            let output = run_checked(
                Command::new("java")
                    .arg("-cp")
                    .arg(&classes)
                    .arg("demo.try_binding.Try_bindingMain"),
                "java",
            );
            assert_eq!(
                String::from_utf8(output.stdout).expect("Java stdout utf8"),
                "5\ninvalid Int: bad\n"
            );
        }

        let _ = fs::remove_dir_all(temp);
    }

    #[test]
    fn readable_java_keeps_nested_try_and_union_flow_structured() {
        if !command_available("javac") || !command_available("java") {
            eprintln!("skipping nested try Java test because a JDK tool is unavailable");
            return;
        }

        let temp = temp_path("lume-java-readable-nested-try");
        let source = temp.join("nested_try.lum");
        let out = temp.join("generated");
        let classes = temp.join("classes");
        fs::create_dir_all(&temp).expect("create temp dir");
        fs::write(
            &source,
            r#"
module demo/nested_try

class Box {
    value Int

    new(value Int) = {
        this.value = value
    }
}

type Choice =
    class First { value Int }
    | class Second { value Int }

def succeed(value Int) Result[Int, Str] = Ok(value)

def checked(value Int) Result[Bool, Str] = Ok(value > 0)

def choiceValue(choice Choice) Int = match choice {
    case Choice.First { value } if value > 0 => value
    case Choice.First { value } => -value
    case Choice.Second { value } => value
}

def observe(value Option[Int]) Unit = {
    match value {
        case Some(_) => ()
        case None => ()
    }
}

def captured(flag Bool, value Option[Int]) Int = {
    let Some { value as item } = value else return 0
    selected Int = if flag { item } else if item > 0 { item + 1 } else { item - 1 }
    result Option[Int] = Some(0).map(_ => selected + item)
    result.getOr(0)
}

def compute(flag Bool) Result[Int, Str] = {
    box Box = Box(try succeed(4))
    let (left Int, right Int) = if flag { (box.value, 2) } else { (1, 3) }
    selected Int = if flag {
        value Int = try succeed(left)
        value
    } else {
        value Int = try succeed(right)
        value
    }
    positive Bool = flag && try checked(selected)
    var total Int = selected
    for index <- Range(0, 2) {
        total += index
    }
    try checked(total)
    if positive { Ok(total) } else { Ok(total + 10) }
}

def main() Unit = {
    observe(None)
    println(compute(true).getOr(0))
    println(compute(false).getOr(0))
    println(choiceValue(Choice.First(7)))
    println(captured(true, Some(3)))
    println(captured(false, Some(3)))
}
"#,
        )
        .expect("write source");

        let result = generate_java_path(&source, JavaBackendOptions::new(&out))
            .expect("generate readable Java");
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);

        let module = fs::read_to_string(out.join("demo/nested_try/Nested_tryModule.java"))
            .expect("read module Java");
        assert!(!module.contains("__block"), "{module}");
        assert!(!module.contains("LumeUnit.INSTANCE;"), "{module}");
        assert!(module.contains("if (flag)"), "{module}");
        assert!(module.contains("instanceof Result.Ok<?, ?>"), "{module}");
        assert!(module.contains("switch (__match"), "{module}");
        assert!(module.contains(" when "), "{module}");
        let choice =
            fs::read_to_string(out.join("demo/nested_try/Choice.java")).expect("read union Java");
        assert!(choice.contains("final class First"), "{choice}");

        let mut sources = core_runtime_sources();
        collect_java_sources(&out, &mut sources).expect("collect generated Java");
        fs::create_dir_all(&classes).expect("create classes dir");
        run_checked(
            Command::new("javac").arg("-d").arg(&classes).args(&sources),
            "javac",
        );
        let output = run_checked(
            Command::new("java")
                .arg("-cp")
                .arg(&classes)
                .arg("demo.nested_try.Nested_tryMain"),
            "java",
        );
        assert_eq!(
            String::from_utf8(output.stdout).expect("Java stdout utf8"),
            "5\n14\n7\n6\n7\n"
        );

        let _ = fs::remove_dir_all(temp);
    }

    #[test]
    fn readable_java_uses_resolved_metadata_for_member_calls() {
        let temp = temp_path("lume-java-readable-resolved-member-call");
        let source = temp.join("member_call.lum");
        let out = temp.join("generated");
        fs::create_dir_all(&temp).expect("create temp dir");
        fs::write(
            &source,
            r#"
module demo/member_call

interface Calculator {
    def addOne(value Int) Int
}

def through(calc Calculator, value Int) Int = calc.addOne(value)
"#,
        )
        .expect("write source");

        let result = generate_java_path(&source, JavaBackendOptions::new(&out))
            .expect("generate readable Java");
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);

        let module = fs::read_to_string(out.join("demo/member_call/Member_callModule.java"))
            .expect("read module Java");
        assert!(module.contains("return calc.addOne(value);"), "{module}");
        assert!(!module.contains("__block"), "{module}");

        let _ = fs::remove_dir_all(temp);
    }

    #[test]
    fn readable_java_keeps_intrinsic_calls_source_shaped() {
        let temp = temp_path("lume-java-readable-intrinsics");
        let source = temp.join("intrinsics.lum");
        let out = temp.join("generated");
        fs::create_dir_all(&temp).expect("create temp dir");
        fs::write(
            &source,
            r#"
module demo/intrinsics

def echo(value Int) Int = identity(value)

def report(value Int) Unit {
    println("value", value)
}
"#,
        )
        .expect("write source");

        let result = generate_java_path(&source, JavaBackendOptions::new(&out))
            .expect("generate readable Java");
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);

        let module = fs::read_to_string(out.join("demo/intrinsics/IntrinsicsModule.java"))
            .expect("read module Java");
        assert!(module.contains("return value;"), "{module}");
        assert!(
            module.contains("LumeRuntime.println(\"value\", value);"),
            "{module}"
        );
        assert!(!module.contains("__block"), "{module}");

        let _ = fs::remove_dir_all(temp);
    }

    #[test]
    fn readable_java_emits_builtin_math_calls() {
        let temp = temp_path("lume-java-readable-math");
        let source = temp.join("math.lum");
        let out = temp.join("generated");
        let classes = temp.join("classes");
        fs::create_dir_all(&temp).expect("create temp dir");
        fs::write(
            &source,
            r#"
module demo/math

def main() Unit {
    println(Math.min(-3, 8), Math.max(-3, 8))
    println(Math.min(1.25, 7.5), Math.max(1.25, 7.5))
}
"#,
        )
        .expect("write source");

        let result = generate_java_path(&source, JavaBackendOptions::new(&out))
            .expect("generate readable Java");
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);

        let module =
            fs::read_to_string(out.join("demo/math/MathModule.java")).expect("read module Java");
        assert!(module.contains("Math.min((-3L), 8L)"), "{module}");
        assert!(module.contains("Math.max((-3L), 8L)"), "{module}");
        assert!(module.contains("Math.min(1.25, 7.5)"), "{module}");
        assert!(module.contains("Math.max(1.25, 7.5)"), "{module}");

        let mut sources = core_runtime_sources();
        collect_java_sources(&out, &mut sources).expect("collect generated Java");
        fs::create_dir_all(&classes).expect("create classes dir");
        run_checked(
            Command::new("javac").arg("-d").arg(&classes).args(&sources),
            "javac",
        );
        let output = run_checked(
            Command::new("java")
                .arg("-cp")
                .arg(&classes)
                .arg("demo.math.MathMain"),
            "java",
        );
        assert_eq!(
            String::from_utf8(output.stdout).expect("Java stdout utf8"),
            "-3 8\n1.25 7.5\n"
        );

        let _ = fs::remove_dir_all(temp);
    }

    #[test]
    fn readable_java_emits_typed_source_expressions() {
        let temp = temp_path("lume-java-readable-source-expressions");
        let source = temp.join("expressions.lum");
        let out = temp.join("generated");
        let classes = temp.join("classes");
        fs::create_dir_all(&temp).expect("create temp dir");
        fs::write(
            &source,
            r#"
module demo/expressions

shape Point {
    x Int
    y Int
}

class Cat {
    name Str
}

class Dog {
    name Str
}

type Pet = Cat | Dog

def pointX(point Point) Int = point.x
def makePoint() Point = Point { y: 2, x: 1 }
def at(values [Int], index Int) Int = values[index]
def isText(value Any) Bool = value is Str
def pair() (Int, Str) = (1, "one")
def mapping() [Str: Int] = ["one": 1]
def required(value Int?) Int = value!
def text(value Int) Str = value.toStr()
def widen(value Int) Any = Any(value)
def describe(value Int) Str = match value {
    case -1 => "missing"
    case 0 => "zero"
    case _ as other => other.toStr()
}
def optionValue(value Int?) Int = match value {
    case Some(item) => item
    case None => 0
}
def petName(value Pet) Str = match value {
    case Cat { name } => name
    case Dog { name } => name
}
"#,
        )
        .expect("write source");

        let result = generate_java_path(&source, JavaBackendOptions::new(&out))
            .expect("generate readable Java");
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);

        let module = fs::read_to_string(out.join("demo/expressions/ExpressionsModule.java"))
            .expect("read module Java");
        assert!(module.contains("return point.x();"), "{module}");
        assert!(module.contains("Long __argument1 = 2L;"), "{module}");
        assert!(module.contains("Long __argument2 = 1L;"), "{module}");
        assert!(
            module.contains("return new Point(__argument2, __argument1);"),
            "{module}"
        );
        assert!(
            module.contains("LumeRuntime.indexValue(values, index)"),
            "{module}"
        );
        assert!(module.contains("value instanceof String"), "{module}");
        assert!(
            module.contains("return new Tuple2<>(1L, \"one\");"),
            "{module}"
        );
        assert!(
            module.contains("LumeMap.fromParts(new Tuple2<>(\"one\", 1L))"),
            "{module}"
        );
        assert!(
            module.contains("LumeRuntime.extractSuccessValue(value)"),
            "{module}"
        );
        assert!(module.contains("return String.valueOf(value);"), "{module}");
        assert!(module.contains("return value;"), "{module}");
        assert!(module.contains("Objects.equals(__match1"), "{module}");
        assert!(module.contains("switch (__match1)"), "{module}");
        assert!(module.contains("case Option.Some<?> __case"), "{module}");
        assert!(module.contains("case Option.None<?> __case"), "{module}");
        assert!(module.contains("case Cat __case"), "{module}");
        assert!(module.contains("case Dog __case"), "{module}");
        assert!(!module.contains("__block"), "{module}");

        if command_available("javac") {
            let mut sources = core_runtime_sources();
            collect_java_sources(&out, &mut sources).expect("collect generated Java");
            fs::create_dir_all(&classes).expect("create classes dir");
            run_checked(
                Command::new("javac").arg("-d").arg(&classes).args(&sources),
                "javac",
            );
        }

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
    text.size
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
        assert!(module.contains("String text"));
        assert!(!module.contains("Object text"));
        assert!(module.contains("LumeUnit value"));
        assert!(module.contains("new Result.Ok<>(value"));

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
                    "switch (__match1)",
                    "case Some<?> __case",
                    "case None<?> __case",
                    "default <X> Option<X> map(Function<T, X> f)",
                    "default <X> Option<X> flatMap(Function<T, Option<X>> f)",
                    "default LumeIterator<T> iterator()",
                    "return None.instance();",
                ],
            ),
            (
                "Result",
                vec![
                    "switch (__match1)",
                    "case Ok<?, ?> __case",
                    "case Err<?, ?> __case",
                    "default <X> Result<X, E> map(Function<T, X> f)",
                    "default <X> Result<X, E> flatMap(Function<T, Result<X, E>> f)",
                    "return ((T) __case",
                ],
            ),
            (
                "Either",
                vec![
                    "switch (__match1)",
                    "case Left<?, ?> __case",
                    "case Right<?, ?> __case",
                    "default <X> Either<L, X> map(Function<R, X> f)",
                    "default <X> Either<L, X> flatMap(Function<R, Either<L, X>> f)",
                    "default L merge()",
                    "return ((R) __case",
                    "return ((L) ((Object) ((R) __case",
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
        assert!(event.contains("import java.time.Duration;"));
        assert!(event.contains("import java.time.Instant;"));
        assert!(event.contains("Instant at;"));
        assert!(event.contains("Duration duration;"));
        assert!(event.contains("Event(Instant at, Duration duration)"));
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
        assert!(module.contains("builder.append(text());"));
        assert!(!module.contains("__block"));

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
        assert!(holder.contains("import third.party.Widget;"));
        assert!(holder.contains("import third.party.GenericBox;"));
        assert!(holder.contains("import java.util.ArrayList;"));
        assert!(holder.contains("Widget widget;"));
        assert!(holder.contains("GenericBox<String> generic;"));
        assert!(holder.contains("ArrayList<String> list;"));

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
        assert!(module.contains("import third.party.Widget;"));
        assert!(module.contains("new Widget(\"Ada\", 7L)"));
        assert!(module.contains(".label()"));
        assert!(module.contains(".count()"));
        assert!(module.contains("Widget.create(\"Bob\")"));
        assert!(module.contains("new GenericBox<>(\"hello\")"));
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
        assert!(client.contains("queryRow(\"select ?\", LumeVector.of(sub"));
        assert!(!client.contains("((LumeVector<Object>) ((Object) sub"));

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
    if value is not Worker {
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

object Ready {}

type ReviewState =
    class Approved { label Str = "approved" }
    | class Rejected { label Str = "rejected" }

type Payload =
    class Item { value Int }
    | object Empty {}

def describe(value Any) Str = match value {
    case User {
        location as home
        age: 18
        name
    } as user => name + " " + home.city + " " + user.name
    case _ => "other"
}

def describeSingleton(value Any) Str = match value {
    case Ready => "ready"
    case _ as other => "other " + other.toStr()
}

def describeReview(value ReviewState) Str = match value {
    case Approved { label } => label
    case Rejected { label } => label
}

def describeMaybe(value Option[Int]) Str = match value {
    case Some(item) as some => some.value.toStr() + " " + item.toStr()
    case None => "none"
}

def describeNoneAlias(value Option[Int]) Str = match value {
    case Some(_) => "error"
    case None as none => none.toStr()
}

def describePayload(value Payload) Str = match value {
    case Payload.Item(item) as whole => whole.value.toStr() + " " + item.toStr()
    case Payload.Empty => "empty"
}

def describeList(values [Int]) Str = match values {
    case [first, second, ...rest] as list =>
        first.toStr() + " " + second.toStr() + " " + rest.size.toStr() + " " + list.size.toStr()
    case _ => "short"
}

def leadingPair(values [Int]) Int {
    let [first, second, ...] = values else return -1
    first + second
}

def main() Unit {
    println(describe(User { name: "Ada", location: Location { city: "Tampa" }, age: 18 }))
    println(describe(User { name: "Bob", location: Location { city: "Miami" }, age: 19 }))
    println(describe("not a user"))
    println(describeReview(ReviewState.Approved { label: "approved" }))
    println(describeSingleton(Ready))
    println(describeMaybe(Some(5)))
    println(describeNoneAlias(None))
    println(describePayload(Payload.Item(7)))
    println(describeList([10, 20, 30, 40]))
    println(leadingPair([10, 20, 30]))
    println(leadingPair([10]))
}
"#,
        )
        .expect("write source");

        let bundled =
            build_backend_bundle_with_load_options(&source, &ModuleLoadOptions::default())
                .expect("build backend bundle");
        assert!(bundled.diagnostics.is_empty(), "{:#?}", bundled.diagnostics);
        let ir = &bundled.bundle.expect("backend bundle").ir;
        let describe_payload = ir
            .functions
            .iter()
            .find(|function| function.name == "describePayload")
            .expect("describePayload function");
        let whole = describe_payload
            .locals
            .iter()
            .find(|local| local.name == "whole")
            .expect("whole alias local");
        assert_eq!(
            whole.ty,
            crate::ir::Type::Named {
                name: "Payload::Item".to_string(),
                args: Vec::new(),
            }
        );

        let result = generate_java_path(&source, JavaBackendOptions::new(&out)).expect("generate");
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);

        let module = fs::read_to_string(out.join("demo/recordpatterns/RecordpatternsModule.java"))
            .expect("read module");
        assert!(module.contains("case User __case"));
        assert!(module.contains(" when "));
        assert!(module.contains("LumeRuntime.listLen"));

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
            "Ada Tampa Ada\nother\nother\napproved\nready\n5 5\nNone\n7 7\n10 20 2 4\n30\n-1\n"
        );

        let _ = fs::remove_dir_all(temp);
    }

    #[test]
    fn generated_java_runs_strict_equality_and_reference_ids() {
        if !command_available("javac") || !command_available("java") {
            eprintln!("skipping Java strict equality test because javac/java is not available");
            return;
        }

        let temp = temp_path("lume-java-strict-equality");
        let source = temp.join("equality.lum");
        let out = temp.join("out");
        let classes = temp.join("classes");
        fs::create_dir_all(&temp).expect("create temp dir");
        fs::write(
            &source,
            r#"
module demo/equality

class Box with Eq[Box] {
    value Int

    def equals(other Box) Bool = this.value == other.value
}

def main() Unit {
    first = Box(1)
    alias = first
    separate = Box(1)
    different = Box(2)

    println(first === alias)
    println(first === separate)
    println(first !== different)
    println(first.referenceId == alias.referenceId)
    println(first.referenceId == separate.referenceId)

    visited Set[ReferenceId] = Set()
    visited.add(first.referenceId)
    println(visited.contains(alias.referenceId))
    println(visited.contains(separate.referenceId))
}
"#,
        )
        .expect("write source");

        let result = generate_java_path(&source, JavaBackendOptions::new(&out)).expect("generate");
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);

        let module =
            fs::read_to_string(out.join("demo/equality/EqualityModule.java")).expect("read module");
        assert!(module.contains("LumeRuntime.strictEquals"));
        assert!(module.contains("LumeRuntime.referenceIdOf"));

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
                .arg("demo.equality.EqualityMain"),
            "java",
        );
        assert_eq!(
            String::from_utf8(output.stdout).expect("java stdout utf8"),
            "true\ntrue\ntrue\ntrue\nfalse\ntrue\nfalse\n"
        );

        let _ = fs::remove_dir_all(temp);
    }

    #[test]
    fn generated_java_runs_transparent_aliases_and_union_widening() {
        if !command_available("javac") || !command_available("java") {
            eprintln!("skipping Java union test because javac/java is not available");
            return;
        }

        let temp = temp_path("lume-java-unions");
        let source = temp.join("unions.lum");
        let out = temp.join("out");
        let classes = temp.join("classes");
        fs::create_dir_all(&temp).expect("create temp dir");
        fs::write(
            &source,
            r#"
module demo/unions

class Cat {
    name Str
}

class Dog {
    name Str
}

class Bird {
    name Str
}

type Companion = Pet
type Pet = Cat | Dog
type Names = [Str]
type Labeler = fn(Str) Str
shape Span {
    start Int
    end Int
}

def describe(value Companion) Str = match value {
    case Cat { name } => "cat " + name
    case Dog { name } => "dog " + name
}

def widen(value Pet) Bird | Dog | Cat = value

def view() { name Str } = { name: "Milo" }
def copyView(value { name Str }) { name Str } = value

def main() Unit {
    pet Companion = Cat("Milo")
    reordered Dog | Cat = pet
    widened Bird | Dog | Cat = reordered
    names Names = ["Milo"]
    label Labeler = value => "name " + value
    span Span = Span(10, 30)
    println(describe(reordered))
    println(label(names[0]))
    println(match widened {
        case Cat { name } => name
        case Dog { name } => name
        case Bird { name } => name
    })
    println(copyView(view()).name)
    println(span.end - span.start)
    println("alice".compare("bob") < 0)
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

        let module = fs::read_to_string(out.join("demo/unions/UnionsModule.java"))
            .expect("read generated module");
        assert!(module.contains("static String describe(Object"));
        assert!(module.contains("static Object widen(Object"));

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
                .arg("demo.unions.UnionsMain"),
            "java",
        );
        assert_eq!(
            String::from_utf8(output.stdout).expect("java stdout utf8"),
            "cat Milo\nname Milo\nMilo\nMilo\n20\ntrue\n"
        );

        let _ = fs::remove_dir_all(temp);
    }

    #[test]
    fn generated_java_runs_declared_union_variants() {
        if !command_available("javac") || !command_available("java") {
            eprintln!("skipping Java declared-union test because javac/java is not available");
            return;
        }

        let temp = temp_path("lume-java-declared-union");
        let source = temp.join("declared_union.lum");
        let out = temp.join("out");
        let classes = temp.join("classes");
        fs::create_dir_all(&temp).expect("create temp dir");
        fs::write(
            &source,
            r#"
module demo/declared_union

type Outcome =
    class Success { value Str }
    | shape Failure { message Str }
    | object Cancelled {}

ext Outcome {
    def result() Str = match this {
        case Success { value } => value
        case Failure { message } => "Failed: " + message
        case Cancelled => "Cancelled"
    }
}

def main() Unit {
    success Outcome = Success { value: "Great" }
    failure Outcome = Failure { message: "nope" }
    cancelled Outcome = Cancelled
    println(success.result())
    println(failure.result())
    println(cancelled.result())
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

        let union = fs::read_to_string(out.join("demo/declared_union/Outcome.java"))
            .expect("read generated union");
        assert!(union.contains("final class Success implements Outcome"));
        assert!(union.contains("record Failure(String message) implements Outcome"));
        assert!(union.contains("final class Cancelled implements Outcome"));
        assert!(union.contains("public static Cancelled instance()"));

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
                .arg("demo.declared_union.Declared_unionMain"),
            "java",
        );
        assert_eq!(
            String::from_utf8(output.stdout).expect("java stdout utf8"),
            "Great\nFailed: nope\nCancelled\n"
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
    x = 1
    label = "one"
    first = Point { x, label }
    same = Point(1, "one")
    different = Point(2, "two")

    println(first == same)
    println(first == different)

    points [Point: Str] = [first: "found"]
    println(points[same] !)

    reordered = ReorderedPoint("one", 1)
    println(first == reordered)
    println(reordered == first)
    println(first.equals(reordered))
    println(first === reordered)
    println(Point(1, "one") === ReorderedPoint("one", 1))

    println(Marker {} == Marker {})

    println(Account(1) == Account(1))
    println(Account(1) != Account(2))
    left Identified = Entry(1)
    right Identified = AlternateEntry(1)
    sameClass Identified = Entry(1)
    println(left == right)
    println(left === right)
    println(left === sameClass)

    id = 3
    account = Account { id }
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
        assert!(point.contains("import java.util.Objects;"));
        assert!(point.contains("Objects.equals(this.x, that.x)"));
        assert!(point.contains("Objects.equals(this.label, that.label)"));
        assert!(point.contains("public int hashCode()"));
        assert!(point.contains("Objects.hash(this.label, this.x)"));
        assert!(point.contains("implements Hashed<Point>, LumeTyped"));

        let generic =
            fs::read_to_string(out.join("demo/shapevalue/Box.java")).expect("read generated Box");
        assert!(generic.contains("other instanceof Box<?> that"));
        assert!(generic.contains("implements Eq<Box<T>>, LumeTyped"));
        assert!(!generic.contains("Hashed<"));

        let reordered = fs::read_to_string(out.join("demo/shapevalue/ReorderedPoint.java"))
            .expect("read generated ReorderedPoint");
        assert!(reordered.contains("Objects.hash(this.label, this.x)"));

        let stable = fs::read_to_string(out.join("demo/shapevalue/StableReference.java"))
            .expect("read generated StableReference");
        assert!(stable.contains("implements Hashed<StableReference>"));
        assert!(stable.contains("public int hashCode()"));
        assert!(stable.contains("return Long.hashCode(this.hash())"));

        let account = fs::read_to_string(out.join("demo/shapevalue/Account.java"))
            .expect("read generated Account");
        assert!(account.contains("implements Eq<Account>, LumeTyped"));
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
        assert!(marker.contains("return Objects.hash();"));

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
            "true\nfalse\nfound\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\ntrue\nfalse\ntrue\ntrue\n"
        );

        let _ = fs::remove_dir_all(temp);
    }

    #[test]
    fn generated_java_runs_tuple_and_declared_union_map_keys() {
        if !command_available("javac") || !command_available("java") {
            eprintln!("skipping Java tuple/union hashing test because javac/java is not available");
            return;
        }

        let temp = temp_path("lume-java-tuple-union-hashing");
        let source = temp.join("tuple_union_hashing.lum");
        let out = temp.join("out");
        let classes = temp.join("classes");
        fs::create_dir_all(&temp).expect("create temp dir");
        fs::write(
            &source,
            r#"
module demo/tupleunionhashing

type LookupKey =
    class Number { value Int }
    | object Default {}

def main() Unit {
    positions [(Int, Int): Str] = [(10, 20): "start"]
    println(positions[(10, 20)]!)

    number LookupKey = LookupKey.Number(7)
    sameNumber LookupKey = LookupKey.Number(7)
    labels [LookupKey: Str] = [number: "seven"]
    println(labels[sameNumber]!)
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

        let key = fs::read_to_string(out.join("demo/tupleunionhashing/LookupKey.java"))
            .expect("read generated union");
        assert!(key.contains("public boolean equals(Object other)"), "{key}");
        assert!(key.contains("public int hashCode()"), "{key}");
        assert!(key.contains("Objects.hash(this.value)"), "{key}");

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
                .arg("demo.tupleunionhashing.TupleunionhashingMain"),
            "java",
        );
        assert_eq!(
            String::from_utf8(output.stdout).expect("java stdout utf8"),
            "start\nseven\n"
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
        assert!(
            generated.diagnostics.is_empty(),
            "{:#?}",
            generated.diagnostics
        );

        let module =
            fs::read_to_string(out.join("demo/runarray/RunarrayModule.java")).expect("read module");
        assert!(!module.contains("UnsupportedOperationException"));
        assert!(module.contains("LumeArray.ofRune(2L)"));

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
    fn emits_readable_function_bodies() {
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
        assert!(module.contains("static Long add(Long left, Long right)"));
        assert!(module.contains("Long result = (left + right);"));
        assert!(module.contains("return result;"));
        assert!(module.contains("if (flag)"));
        assert!(module.contains("return 10L;"));
        assert!(module.contains("Long value = add(2L, 3L);"));
        assert!(module.contains("LumeRuntime.println(value)"));

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
        assert!(module.contains("new Result.Ok<>(0L)"));
        assert!(module.contains("new Result.Ok<>(1L)"));

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

type Status =
    object Pending {}
    | object Complete {}

def ok() Result[HttpResponse, HttpError] =
    Ok({ body: "ok" })

def err() Result[HttpResponse, HttpError] =
    Err({ status: 400, body: "bad" })

def pending() Option[Status] =
    Some(Status.Pending)
"#,
        )
        .expect("write source");

        let result = generate_java_path(&source, JavaBackendOptions::new(&out)).expect("generate");

        assert!(result.diagnostics.is_empty());
        let module = fs::read_to_string(out.join("demo/result_shape/Result_shapeModule.java"))
            .expect("read module");
        assert!(!module.contains("UnsupportedOperationException"));
        assert!(module.contains("new HttpResponse(200L, \"ok\""));
        assert!(module.contains("new HttpError(400L"));
        assert!(module.contains("new Option.Some<>(Status.Pending.instance())"));
        assert!(module.contains("\"application/json\""));
        assert!(!module.contains("__block"), "{module}");

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
        targetType.name.getOr("?")

    def reifiedLabel[reified T]() Str =
        typeOf[T].name.getOr("?")
}


class Reader {

    def read[reified T]() Str {
        Cache.label(typeOf[T])
    }
}

def concrete() Str = Cache.reifiedLabel[Int]()

"#,
        )
        .expect("write source");

        let result = generate_java_path(&source, JavaBackendOptions::new(&out)).expect("generate");

        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        let reader =
            fs::read_to_string(out.join("demo/object_reified/Reader.java")).expect("read reader");
        assert!(!reader.contains("UnsupportedOperationException"));
        assert!(reader.contains("Cache.INSTANCE.label"));
        assert!(reader.contains("__type_T"));
        let module = fs::read_to_string(out.join("demo/object_reified/Object_reifiedModule.java"))
            .expect("read module");
        assert!(
            module.contains("Cache.INSTANCE.reifiedLabel(LumeType.primitive(\"Int\"))"),
            "{module}"
        );
        assert!(!reader.contains("__block"), "{reader}");
        assert!(!module.contains("__block"), "{module}");

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
    private var values Map[Str, Str] = Map()

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
        assert!(cache.contains("import lume.core.LumeMap;"));

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
        assert!(runner.contains("mapper.apply"));
        assert!(runner.contains("new Result.Ok<>"));
        assert!(runner.contains("new Result.Err<>"));
        assert!(runner.contains("new Option.Some<>"));
        assert!(runner.contains("Option.None.instance()"));

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
        assert!(module.contains("Greeter greeter = new Greeter();"));
        assert!(!module.contains("__block"));

        let _ = fs::remove_dir_all(temp);
    }

    #[test]
    fn emits_nested_named_declarations_and_qualified_object_access() {
        let temp = temp_path("lume-java-nested-declarations");
        let source = temp.join("nested_declarations.lum");
        let out = temp.join("out");
        fs::create_dir_all(&temp).expect("create temp dir");
        fs::write(
            &source,
            r#"
module demo/nested

class Namespace {
    shape Point { x Int }
    class Worker { name Str }
    interface Reader { def read() Str }
    object Defaults { prefix Str = "worker:" }
    annotation Label { value Str }
    type Name = Str
    type Outcome =
        class Present { value Name }
        | object Missing {}

    def worker(name Str) Worker = Worker(name)
    def outcome(name Name) Outcome = Outcome.Present(name)
}

class TextReader with Namespace.Reader {
    def read() Str = "!"
}

@Namespace.Label { value: "entry" }
def run() Str {
    namespace = Namespace()
    worker Namespace.Worker = namespace.worker("Ada")
    point Namespace.Point = Namespace.Point(7)
    outcome Namespace.Outcome = namespace.outcome("ok")
    text = match outcome {
        case Present { value } => value
        case Missing => "missing"
    }
    return Namespace.Defaults.prefix + worker.name + point.x.toStr() + text + TextReader().read()
}
"#,
        )
        .expect("write source");

        let result = generate_java_path(&source, JavaBackendOptions::new(&out)).expect("generate");

        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
        let module =
            fs::read_to_string(out.join("demo/nested/NestedModule.java")).expect("read module");
        assert!(module.contains("NamespaceDefaults.INSTANCE.prefix"));
        assert!(out.join("demo/nested/NamespacePoint.java").exists());
        assert!(out.join("demo/nested/NamespaceWorker.java").exists());
        assert!(out.join("demo/nested/NamespaceReader.java").exists());
        assert!(out.join("demo/nested/NamespaceLabel.java").exists());
        assert!(out.join("demo/nested/NamespaceOutcome.java").exists());

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

type Status =
    object Pending {}
    | class Done { label Str }

def main() Unit {
    user User = User("Ada", 42)

    declared Type[User] = typeOf[User]
    actual Type[User] = user.runtimeType

    println(declared.name !)
    println(actual.qualifiedName !)
    println(declared.kind)

    classType ClassType[User] = declared.asClass() !
    fields [Field] = classType.fields
    println(fields.size)

    nameField Field = fields.at(0) !
    ageField Field = fields.at(1) !

    println(nameField.name)
    println(nameField.fieldType.name !)
    println(ageField.name)
    println(ageField.fieldType.name !)

    enumType EnumType[Status] = typeOf[Status].asEnum() !
    println(enumType.name !)
    println(enumType.kind)
    println((enumType.case("Pending") !).name)
}
"#,
        )
        .expect("write source");

        let generated =
            generate_java_path(&source, JavaBackendOptions::new(&out)).expect("generate java");
        assert!(generated.diagnostics.is_empty());

        let user = fs::read_to_string(out.join("demo/metadata/User.java")).expect("read user");
        assert!(user.contains("static final LumeType TYPE"));
        assert!(user.contains("LumeField.of(\"name\""));
        assert!(user.contains("LumeAnnotationField.of(\"path\", \"/users\")"));

        let module =
            fs::read_to_string(out.join("demo/metadata/MetadataModule.java")).expect("read module");
        assert!(!module.contains("UnsupportedOperationException"));
        assert!(module.contains("= User.TYPE;"));
        assert!(module.contains("LumeRuntime.runtimeTypeOf(user"));
        assert!(!module.contains("__block"));

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
    fn generated_java_runs_explicit_and_contextual_generic_construction() {
        if !command_available("javac") || !command_available("java") {
            eprintln!(
                "skipping Java generic construction test because javac/java is not available"
            );
            return;
        }

        let temp = temp_path("lume-java-generic-construction");
        let source = temp.join("generic_construction.lum");
        let out = temp.join("out");
        let classes = temp.join("classes");
        fs::create_dir_all(&temp).expect("create temp dir");
        fs::write(
            &source,
            r#"
module demo/genericconstruct

class Box[T] {
    value T

    new(value T) {
        this.value = value
    }
}

def main() Unit {
    set Set[Str] = Set()
    map Map[Str, Int] = Map[Str, Int]()
    inferred = Box("hello")
    contextual Box[Str] = new("world")

    set.add("Ada")
    println(set.size, map.size, inferred.value, contextual.value)
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

        let module =
            fs::read_to_string(out.join("demo/genericconstruct/GenericconstructModule.java"))
                .expect("read module");
        assert!(module.contains("LumeSet<String> set = LumeSet.empty()"));
        assert!(module.contains("LumeMap<String, Long> map = LumeMap.empty()"));
        assert!(module.contains("Box<String> inferred = new Box<>(\"hello\")"));
        assert!(module.contains("Box<String> contextual = new Box<>(\"world\")"));

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
                .arg("demo.genericconstruct.GenericconstructMain"),
            "java",
        );
        assert_eq!(
            String::from_utf8(output.stdout).expect("java stdout utf8"),
            "1 0 hello world\n"
        );

        let _ = fs::remove_dir_all(temp);
    }

    #[test]
    fn generated_java_runs_contextual_shape_projection_inside_generic_map() {
        if !command_available("javac") || !command_available("java") {
            eprintln!("skipping Java shape projection test because javac/java is not available");
            return;
        }

        let temp = temp_path("lume-java-contextual-shape-projection");
        let source = temp.join("shape_projection.lum");
        let out = temp.join("out");
        let classes = temp.join("classes");
        fs::create_dir_all(&temp).expect("create temp dir");
        fs::write(
            &source,
            r#"
module demo/shapeprojection

shape StoredRollup {
    total Int
    label Str
    internalId Int
}

shape Rollup {
    total Int
    label Str
}

def source() StoredRollup? = Some(StoredRollup {
    total: 7
    label: "week"
    internalId: 99
})

def named() Rollup? = source().map(r => Rollup { ...r })
def bareFields() Rollup? = source().map(r => { ...r })
def implicitShape() Rollup? = source().map(r => { ...r })
def contextualNew() Rollup? = source().map(r => new { ...r })
def namedMembers() Rollup? = source().map(r => Rollup { total: r.total, label: r.label })
def bareFieldMembers() Rollup? = source().map(r => { total: r.total, label: r.label })
def contextualNewMembers() Rollup? = source().map(r => new { total: r.total, label: r.label })

def main() Unit {
    println(named()!.total)
    println(bareFields()!.total)
    println(implicitShape()!.total)
    println(contextualNew()!.total)
    println(namedMembers()!.total)
    println(bareFieldMembers()!.total)
    println(contextualNewMembers()!.total)
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
        let module =
            fs::read_to_string(out.join("demo/shapeprojection/ShapeprojectionModule.java"))
                .expect("read module");
        assert_eq!(module.matches("new Rollup(").count(), 7);

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
                .arg("demo.shapeprojection.ShapeprojectionMain"),
            "java",
        );
        assert_eq!(
            String::from_utf8(output.stdout).expect("java stdout utf8"),
            "7\n7\n7\n7\n7\n7\n7\n"
        );

        let _ = fs::remove_dir_all(temp);
    }

    #[test]
    fn generated_java_materializes_shape_width_projection() {
        if !command_available("javac") || !command_available("java") {
            eprintln!("skipping Java shape-projection test because javac/java is not available");
            return;
        }

        let temp = temp_path("lume-java-shape-projection-copy");
        let source = temp.join("shape_projection_copy.lum");
        let out = temp.join("out");
        let classes = temp.join("classes");
        fs::create_dir_all(&temp).expect("create temp dir");
        fs::write(
            &source,
            r#"
module demo/shapeprojectioncopy

shape Point {
    x Int
    y Int
}

shape Point3D {
    x Int
    y Int
    z Int
}

shape WiderItems {
    x Int
    items [Int]
    z Int
}

shape Items {
    x Int
    items [Int]
}

def view(point Point) Point = point

def main() Unit {
    first3d = Point3D(1, 2, 3)
    second3d = Point3D(1, 2, 99)
    first Point = view(first3d)
    second Point = second3d
    concrete = Point(1, 2)

    println(first == second)
    println(first === second)
    println(first == concrete)
    println(first === concrete)
    println(first.runtimeType.name !)
    match first {
        case Point3D { z } => println(z)
        case _ => println("missing")
    }
    if first is Point3D {
        println(first.z)
    }

    values [Point: Str] = []
    values[first3d] := "first"
    values[second3d] := "second"
    println(values.size, values[first3d]!)

    points Set[Point] = Set()
    points.add(first3d)
    points.add(second3d)
    println(points.size)
    println(values.contains(first3d))

    projected Point = Point { ...first }
    contextual Point = { ...first }
    forced Point = new { ...first }
    updated = first with { x: 4 }
    println(projected === concrete)
    println(contextual === concrete)
    println(forced === concrete)
    println(updated === Point(4, 2))

    wider = WiderItems(1, [1], 3)
    items Items = wider
    wider.items.add(2)
    println(items.items.size)
}
"#,
        )
        .expect("write source");

        let result = generate_java_path(&source, JavaBackendOptions::new(&out)).expect("generate");
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);

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
                .arg("demo.shapeprojectioncopy.ShapeprojectioncopyMain"),
            "java",
        );
        assert_eq!(
            String::from_utf8(output.stdout).expect("java stdout utf8"),
            "true\ntrue\ntrue\ntrue\nPoint\nmissing\n1 second\n1\ntrue\ntrue\ntrue\ntrue\ntrue\n2\n"
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
        assert!(module.contains("LumeVector<T> out"));
        assert!(module.contains(".add(mapper.apply(item"));
        assert!(!module.contains("__block"));

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
    fn generated_java_runs_indexed_callable_invocation() {
        if !command_available("javac") || !command_available("java") {
            eprintln!("skipping Java indexed callable test because javac/java is not available");
            return;
        }

        let temp = temp_path("lume-java-indexed-callable");
        let source = temp.join("indexed_callable.lum");
        let out = temp.join("out");
        let classes = temp.join("classes");
        fs::create_dir_all(&temp).expect("create temp dir");
        fs::write(
            &source,
            r#"
module demo/indexedcall

class User {}

class HandlerService {
    stored [fn() Int]

    def handlers [fn() Int] = stored

    def echo[T](value T) T = value
}

def metadata[reified T]() Type[T] = typeOf[T]

def main() Unit {
    handlers [fn() Int] = [() => 26]
    index = 0
    service = HandlerService(handlers)

    println(metadata[User]().name!)
    println(handlers[0]())
    println(handlers[index]())
    println(service.handlers[index]())
    println(service.echo[Int](27))
}
"#,
        )
        .expect("write source");

        let interpreted = run_path(&source, None).expect("run interpreter");
        assert!(interpreted.diagnostics.is_empty());
        let expected = interpreter_stdout(interpreted);

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
                .arg("demo.indexedcall.IndexedcallMain"),
            "java",
        );
        let actual = String::from_utf8(output.stdout).expect("java stdout utf8");
        assert_eq!(actual, expected);

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
    inserted Unit = users.insertAt(0, User { name: "Bob", cost: 2 }) !
    println(users.at(0)!.name)
    println(users.setAt(0, User { name: "Cara", cost: 4 })!.name)
    println(users.removeAt(1)!.name)
    println(users.fold(0, (cost, user) => cost + user.cost))

    values [Int] = [1, 2]
    println(values.setAt(0, 3) !)
    vectorInserted Unit = values.insertAt(1, 4) !
    println(values.removeAt(2) !)
    println(values[0])
    match values.removeAt(9) {
        case Err { error } => {
            println(error.index)
            println(error.size)
        }
        case Ok { value: _ } => ()
    }

    array Array[Int] = Array.fill(2, 5)
    println(array.at(1) !)
    println(array.setAt(1, 7) !)
    println(array[1])

    result Result[Int, Str] = Ok(7)
    println(result !)
    nested Option[Result[Int, Str]] = Some(Ok(11))
    println(nested!!)
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
        assert!(module.contains("import lume.core.LumeLinkedList;"));
        assert!(module.contains("LumeRuntime.extractSuccessValue"));

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
            "Bob\nBob\nAda\n4\n1\n2\n3\n9\n2\n5\n5\n7\n7\n11\n"
        );

        let _ = fs::remove_dir_all(temp);
    }

    #[test]
    fn generated_java_runs_string_split_and_trim() {
        if !command_available("javac") || !command_available("java") {
            eprintln!("skipping Java string methods test because javac/java is not available");
            return;
        }

        let temp = temp_path("lume-java-string-methods");
        let source = temp.join("string_methods.lum");
        let out = temp.join("out");
        let classes = temp.join("classes");
        fs::create_dir_all(&temp).expect("create temp dir");
        fs::write(
            &source,
            r##"
module demo/stringmethods

def main() Unit {
    literal Vector[Str] = "a.b".split(".")
    parts Vector[Str] = "  alpha, beta  ".trim().splitRegex("\s*,\s*")
    parts.add("gamma")
    println(literal.size)
    println(literal[0])
    println(literal[1])
    println(parts.size)
    println(parts[0])
    println(parts[1])
    println(parts[2])
    println("".isEmpty)
    println("lume".nonEmpty)
    println("".nonEmpty)
    println("lume".isEmpty)
    println("😀a".size)
    println("😀a".runeAt(1)! == "a".runeAt(0)!)
    text = "  LuMe  "
    println("[${text.trim()}]")
    println("[${text.trimLeft()}]")
    println("[${text.trimRight()}]")
    println(text.toLower().trim())
    println(text.toUpper().trim())
    println(text.contains("LuMe"))
    println(text.contains("missing"))
    println("😀ab".indexOf("a"))
    println("a1 b22".replaceFirstRegex("\d+", "#"))
    println("a1 b22".replaceAllRegex("\d+", "#"))
}
"##,
        )
        .expect("write source");

        let generated =
            generate_java_path(&source, JavaBackendOptions::new(&out)).expect("generate java");
        assert!(
            generated.diagnostics.is_empty(),
            "{:#?}",
            generated.diagnostics
        );

        let module = fs::read_to_string(out.join("demo/stringmethods/StringmethodsModule.java"))
            .expect("read module");
        assert!(module.contains("LumeRuntime.stringSplit"));
        assert!(module.contains("LumeRuntime.stringSplitRegex"));
        assert!(module.contains("LumeRuntime.stringRuneAt"));
        assert!(module.contains("LumeRuntime.stringSize"));
        assert!(module.contains("LumeRuntime.stringTrimLeft"));
        assert!(module.contains("LumeRuntime.stringTrimRight"));
        assert!(module.contains("LumeRuntime.stringToLower"));
        assert!(module.contains("LumeRuntime.stringToUpper"));
        assert!(module.contains("LumeRuntime.stringContains"));
        assert!(module.contains("LumeRuntime.stringIndexOf"));
        assert!(module.contains("LumeRuntime.stringReplaceFirstRegex"));
        assert!(module.contains("LumeRuntime.stringReplaceAllRegex"));
        assert!(module.contains("LumeVector<String> parts"));
        assert!(module.contains("(\"\").isEmpty()"));
        assert!(module.contains("!(\"lume\").isEmpty()"));

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
                .arg("demo.stringmethods.StringmethodsMain"),
            "java",
        );
        assert_eq!(
            String::from_utf8(output.stdout).expect("java stdout utf8"),
            "2\na\nb\n3\nalpha\nbeta\ngamma\ntrue\ntrue\nfalse\nfalse\n2\ntrue\n[LuMe]\n[LuMe  ]\n[  LuMe]\nlume\nLUME\ntrue\nfalse\n1\na# b22\na# b#\n"
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
def mapSize(values [Str : Int]) Int = values.size

class Cache {
    private var values [Str : Int] = []

    new() {}

    def currentValue() Int = 7

    def empty() [Str : Int] = []

    def store(key Str) Unit {
        values[key] := currentValue()
    }

    def adjust(key Str) Unit {
        values[key] += 5
        values[key] -= 2
    }

    def reset() Unit {
        this.values := empty()
    }

    def lookup(key Str) Int = values[key] ?? -1
}

def main() Unit {
    cache = Cache()
    cache.store("answer")
    cache.adjust("answer")
    println(cache.lookup("answer"))
    cache.reset()
    println(cache.lookup("answer"))
    println(emptyMap().size)
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
            "10\n-1\n0\n0\n"
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
        assert!(module.contains("Function7<"));
        assert!(module.contains(".apply(1L, 2L, 3L, 4L, 5L, 6L, 7L)"));
        assert!(module.contains("(a, b, c, d, e, g, h) ->"));
        assert!(!module.contains("__block"));

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
    fn generated_java_runs_intermingled_defaults() {
        if !command_available("javac") || !command_available("java") {
            eprintln!("skipping Java default-prefix test because javac/java is not available");
            return;
        }

        let temp = temp_path("lume-java-default-prefix");
        let source = temp.join("default_prefix.lum");
        let out = temp.join("out");
        let classes = temp.join("classes");
        fs::create_dir_all(&temp).expect("create temp dir");
        fs::write(
            &source,
            r#"
module demo/defaultprefix

class X1 {
    a Str = "A"
    b Str
}

class X2 {
    b Str
    a Str = "A"
}

class X3 {
    a Str
    b Str = "B"
    c Str
}

class X4 {
    a Str = "A"
    b Str = "B"
}

class Article {
    body Str
    title Str

    new(body Str = "body", title Str) {
        this.body = body
        this.title = title
    }
}

class Client {
    def connect(protocol Str = "https", host Str, port Int = 443) Str =
        protocol + "://" + host + ":" + port.toStr()
}

def connect(protocol Str = "https", host Str, port Int = 443) Str =
    protocol + "://" + host + ":" + port.toStr()

def main() Int {
    x1 X1 = X1 { b: "one" }
    x2 X2 = X2("two")
    x3 X3 = X3 { a: "three", c: "four" }
    x3Full X3 = X3("five", "six", "seven")
    x4Empty X4 = X4()
    x4One X4 = X4("eight")
    x4Full X4 = X4("nine", "ten")
    article Article = Article { title: "Intro" }
    client = Client {}
    println(x1.a, x1.b)
    println(x2.b, x2.a)
    println(x3.a, x3.b, x3.c)
    println(x3Full.a, x3Full.b, x3Full.c)
    println(x4Empty.a, x4Empty.b)
    println(x4One.a, x4One.b)
    println(x4Full.a, x4Full.b)
    println(article.body, article.title)
    println(connect(host = "example.com"))
    println(client.connect(host = "example.com", port = 8443))
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
                .arg("demo.defaultprefix.DefaultprefixMain"),
            "java",
        );
        let actual = String::from_utf8(output.stdout).expect("java stdout utf8");
        assert_eq!(actual, expected);

        let _ = fs::remove_dir_all(temp);
    }

    #[test]
    fn generated_java_preserves_source_evaluation_order_for_named_arguments() {
        if !command_available("javac") || !command_available("java") {
            eprintln!("skipping Java argument-order test because javac/java is not available");
            return;
        }

        let temp = temp_path("lume-java-argument-order");
        let source = temp.join("argument_order.lum");
        let out = temp.join("out");
        let classes = temp.join("classes");
        fs::create_dir_all(&temp).expect("create temp dir");
        fs::write(
            &source,
            r#"
module demo/argumentorder

class Recorder {
    values [Int] = []

    def mark(value Int) Int {
        this.values.add(value)
        value
    }
}

class Pair {
    first Int
    second Int
}

class ExplicitPair {
    first Int
    second Int

    new(first Int, second Int) {
        this.first = first
        this.second = second
    }
}

class Service {
    recorder Recorder

    def send(request Int) Int {
        this.recorder.mark(3)
        request
    }

    def combine(first Int, second Int) Int {
        this.recorder.mark(4)
        first * 10 + second
    }
}

class Harness {
    recorder Recorder

    def makeService() Service {
        this.recorder.mark(1)
        Service(this.recorder)
    }

    def makeRequest() Int = this.recorder.mark(2)
}

def combine(first Int, second Int) Int = first * 10 + second

def collect(prefix Int, values [Int] vararg) Int =
    prefix + values.fold(0, (sum, value) => sum + value)

def keep(first Int, second => Int) Int = first

def main() Int {
    callRecorder = Recorder {}
    combined = combine(
        second = callRecorder.mark(2),
        first = callRecorder.mark(1)
    )
    println(combined, callRecorder.values[0], callRecorder.values[1])

    explicitRecorder = Recorder {}
    explicit Pair = Pair {
        second: explicitRecorder.mark(2)
        first: explicitRecorder.mark(1)
    }
    println(explicit.first, explicit.second,
        explicitRecorder.values[0], explicitRecorder.values[1])

    contextualRecorder = Recorder {}
    contextual Pair = {
        second: contextualRecorder.mark(2)
        first: contextualRecorder.mark(1)
    }
    println(contextual.first, contextual.second,
        contextualRecorder.values[0], contextualRecorder.values[1])

    constructorRecorder = Recorder {}
    constructed ExplicitPair = ExplicitPair {
        second: constructorRecorder.mark(2)
        first: constructorRecorder.mark(1)
    }
    println(constructed.first, constructed.second,
        constructorRecorder.values[0], constructorRecorder.values[1])

    receiverRecorder = Recorder {}
    harness = Harness(receiverRecorder)
    sent = harness.makeService().send(harness.makeRequest())
    println(sent, receiverRecorder.values[0], receiverRecorder.values[1],
        receiverRecorder.values[2])

    namedReceiverRecorder = Recorder {}
    namedHarness = Harness(namedReceiverRecorder)
    namedResult = namedHarness.makeService().combine(
        second = namedHarness.makeRequest(),
        first = namedReceiverRecorder.mark(3)
    )
    println(namedResult, namedReceiverRecorder.values[0],
        namedReceiverRecorder.values[1], namedReceiverRecorder.values[2],
        namedReceiverRecorder.values[3])

    variadicRecorder = Recorder {}
    variadicResult = collect(
        values = [variadicRecorder.mark(2), variadicRecorder.mark(3)],
        prefix = variadicRecorder.mark(1)
    )
    println(variadicResult, variadicRecorder.values[0],
        variadicRecorder.values[1], variadicRecorder.values[2])

    lazyRecorder = Recorder {}
    lazyResult = keep(
        second = panic("must remain lazy"),
        first = lazyRecorder.mark(1)
    )
    println(lazyResult, lazyRecorder.values[0])
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
                .arg("demo.argumentorder.ArgumentorderMain"),
            "java",
        );
        let actual = String::from_utf8(output.stdout).expect("java stdout utf8");
        assert_eq!(actual, expected);

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

class Holder {
    payload { x Int }
}

class GuardTracker {
    var calls Int = 0

    def allowed() Bool {
        this.calls += 1
        false
    }
}

def add(left Int, right Int) Int {
    result Int = left + right
    result
}

def choose(tracker GuardTracker) Str = match (0, 0) {
    case (0, _) | (_, 0) if tracker.allowed() => "accepted"
    case _ => "rejected"
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

    println(match -1 {
        case -1 => "negative int"
        case _ => "other"
    })

    println(match -3.5 {
        case -3.5 => "negative float"
        case _ => "other"
    })

    low Int = 9007199254740992
    high Int = 9007199254740993
    println(low < high, high > low, high <= low)

    maximum Int = 9223372036854775807
    minimum Int = maximum + 1
    println(minimum, minimum / -1, minimum % -1)

    println(1.0 / 0.0 > 0.0, 0.0 / 0.0 < 0.0, 5.5 % 2.0)
    println(high > 9007199254740992.0)

    payload { x Int } = { x: 7 }
    fromValue Holder = Holder(payload)
    fromLiteral Holder = Holder({ x: 8 })
    println(fromValue.payload.x, fromLiteral.payload.x)

    expressionTracker = GuardTracker {}
    expressionResult = match (0, 0) {
        case (0, _) | (_, 0) if expressionTracker.allowed() => "accepted"
        case _ => "rejected"
    }
    println(expressionResult, expressionTracker.calls)

    returnTracker = GuardTracker {}
    println(choose(returnTracker), returnTracker.calls)

    statementTracker = GuardTracker {}
    match (0, 0) {
        case (0, _) | (_, 0) if statementTracker.allowed() => println("accepted")
        case _ => println("statement rejected", statementTracker.calls)
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
