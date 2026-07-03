use std::{
    collections::{HashMap, HashSet},
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

mod emit;

use crate::{
    Diagnostic,
    ast::TypeRef,
    backend::{ExternalDescriptors, bundle::build_backend_bundle_with_load_options},
    resolver::{
        JavaExternalCallable, JavaExternalClass, JavaExternalParam, LocatedDiagnostic,
        ModuleLoadOptions, load_module_graph_with_options,
    },
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
    let discovery_options = ModuleLoadOptions {
        allow_unresolved_java_imports: true,
        java_external_type_params: HashMap::new(),
        java_external_classes: HashMap::new(),
    };
    let (discovery_graph, _) = load_module_graph_with_options(path, &discovery_options)?;
    let discovered_externals = ExternalDescriptors::from_module_graph(&discovery_graph);
    let external_resolution = resolve_external_classes(&discovered_externals, &options)?;
    if !external_resolution.diagnostics.is_empty() {
        return Ok(JavaBackendResult {
            diagnostics: external_resolution.diagnostics,
            written_files: Vec::new(),
        });
    }

    let load_options = ModuleLoadOptions {
        allow_unresolved_java_imports: true,
        java_external_type_params: external_resolution.type_params,
        java_external_classes: external_resolution.classes,
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
    let mut written_files = Vec::new();
    for source in emit::render_declaration_skeletons(&bundle) {
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

#[derive(Debug, Clone, Default)]
struct ExternalClassResolution {
    diagnostics: Vec<LocatedDiagnostic>,
    type_params: HashMap<String, Vec<String>>,
    classes: HashMap<String, JavaExternalClass>,
}

fn resolve_external_classes(
    externals: &ExternalDescriptors,
    options: &JavaBackendOptions,
) -> Result<ExternalClassResolution, String> {
    let classpath_entries = effective_java_classpath(options);
    let index = JavaClasspathIndex::from_entries(&classpath_entries)?;
    let classpath = java_classpath(&classpath_entries)?;
    let local_type_names = externals
        .symbols
        .iter()
        .filter(|symbol| matches!(symbol.kind, crate::backend::ExternalSymbolKind::Type))
        .map(|symbol| (symbol.qualified_name.clone(), symbol.local_name.clone()))
        .collect::<HashMap<_, _>>();
    let mut diagnostics = Vec::new();
    let mut type_params = HashMap::new();
    let mut classes = HashMap::new();
    for symbol in &externals.symbols {
        if !matches!(symbol.kind, crate::backend::ExternalSymbolKind::Type) {
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
        type_params.insert(
            symbol.qualified_name.clone(),
            descriptor.class.type_params.clone(),
        );
        classes.insert(symbol.qualified_name.clone(), descriptor.class);
    }
    flatten_inherited_java_methods(&mut classes, &local_type_names);
    Ok(ExternalClassResolution {
        diagnostics,
        type_params,
        classes,
    })
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

fn missing_java_class_diagnostic(symbol: &crate::backend::ExternalSymbol) -> LocatedDiagnostic {
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
        .arg("-public")
        .arg(qualified_name)
        .output()
        .map_err(|err| format!("run javap to inspect Java classpath: {err}"))?;
    if !output.status.success() {
        return Ok(None);
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(Some(JavaClassDescriptor {
        class: parse_javap_class(&stdout, qualified_name, local_type_names, span),
    }))
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
    let header = output
        .lines()
        .find(|line| line.contains(" class ") || line.contains(" interface "))
        .unwrap_or_default();
    let kind = if output.contains("extends java.lang.annotation.Annotation") {
        crate::ast::TypeKind::Annotation
    } else if header.contains(" interface ") {
        crate::ast::TypeKind::Interface
    } else {
        crate::ast::TypeKind::Class
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
        match parse_javap_callable_line(line, qualified_name, ctx) {
            Some(ParsedJavaCallable::Constructor(constructor)) => constructors.push(constructor),
            Some(ParsedJavaCallable::Method(method)) => methods.push(method),
            None => {}
        }
    }

    JavaExternalClass {
        kind,
        type_params,
        with_bounds,
        constructors,
        methods,
    }
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
            "Any" | "Bool" | "Int" | "Int32" | "Float" | "Float32" | "Rune" | "Str" | "Unit"
        )
        || matches!(name, "List" | "Set" | "Map" | "Option")
}

fn parse_javap_callable_line(
    line: &str,
    qualified_name: &str,
    mut ctx: JavaTypeContext<'_>,
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

    let params = parse_javap_params(&line[open + 1..close], &ctx);
    if java_constructor_name_matches(before, qualified_name) {
        return Some(ParsedJavaCallable::Constructor(JavaExternalCallable {
            name: "new".to_string(),
            type_params: method_type_params,
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
        params,
        return_type,
    }))
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

fn parse_javap_params(params: &str, ctx: &JavaTypeContext<'_>) -> Vec<JavaExternalParam> {
    if params.trim().is_empty() {
        return Vec::new();
    }
    split_java_signature_list(params)
        .into_iter()
        .enumerate()
        .map(|(index, raw)| {
            let raw = raw.trim();
            let variadic = raw.ends_with("...");
            let raw_ty = raw.strip_suffix("...").map(str::trim).unwrap_or(raw);
            let ty = java_type_to_lume_type_ref(raw_ty, ctx).map(|ty| {
                if variadic {
                    TypeRef::Named {
                        name: "List".to_string(),
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
            }
        })
        .collect()
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
        _ => None,
    }
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
        | "Byte" | "Short" | "Integer" => Some("Int32"),
        "long" | "java.lang.Long" | "Long" => Some("Int"),
        "float" | "java.lang.Float" | "Float" => Some("Float32"),
        "double" | "java.lang.Double" | "Double" => Some("Float"),
        "char" | "java.lang.Character" | "Character" => Some("Rune"),
        "java.lang.String" | "String" => Some("Str"),
        "java.util.List"
        | "java.util.Collection"
        | "java.lang.Iterable"
        | "lume.core.LumeList"
        | "List"
        | "Collection"
        | "Iterable"
            if arg_count == 1 =>
        {
            Some("List")
        }
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
            return ctx.local_type_names.get(base).cloned();
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
            "lume.core.Result<lume.core.LumeList<lume.db.Row>, lume.db.DbError>",
            &ctx,
        )
        .expect("rows result type");
        assert_eq!(
            rows,
            TypeRef::Named {
                name: "Result".to_string(),
                args: vec![
                    TypeRef::Named {
                        name: "List".to_string(),
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

single Routes {
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

        let class = fs::read_to_string(out.join("demo/app/User.java")).expect("read class");
        assert!(class.contains("class User"));
        assert!(class.contains("String name;"));
        assert!(class.contains("Long age;"));

        let runtime_box =
            fs::read_to_string(out.join("demo/app/RuntimeBox.java")).expect("read runtime box");
        assert!(runtime_box.contains("lume.core.LumeList<Long> items;"));
        assert!(runtime_box.contains("lume.core.LumeSet<String> names;"));
        assert!(runtime_box.contains("lume.core.LumeMap<String, lume.core.LumeList<Long>> index;"));
        assert!(runtime_box.contains("lume.core.Option<String> maybe;"));
        assert!(runtime_box.contains("lume.core.Result<Long, String> result;"));
        assert!(runtime_box.contains("lume.core.Either<String, Long> either;"));
        assert!(runtime_box.contains("lume.core.Tuple2<Long, String> pair;"));

        let single = fs::read_to_string(out.join("demo/app/Routes.java")).expect("read single");
        assert!(single.contains("final class Routes"));
        assert!(single.contains("static final Routes INSTANCE"));
        assert!(single.contains("String healthPath()"));

        let interface =
            fs::read_to_string(out.join("demo/app/Named.java")).expect("read interface");
        assert!(interface.contains("interface Named"));
        assert!(interface.contains("String name();"));

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
}

impl Maybe[T] {
    def isDefined() Bool = match this {
        case Some(_) => true
        case None => false
    }

    def orPanic() T = match this {
        case Some(value) => value
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
                    "return ((R) __case1.value());",
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
        assert!(module.contains("result_2 = ((Long) ((Object) tmp3_3));"));
        assert!(module.contains("return result_2;"));
        assert!(module.contains("if (flag_0)"));
        assert!(module.contains("return ((Long) ((Object) tmp1_1));"));
        assert!(module.contains("tmp1_1 = add(2L, 3L);"));
        assert!(module.contains("value_0 = tmp1_1;"));
        assert!(module.contains("tmp2_2 = lume.core.LumeRuntime.println(value_0);"));

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
}

impl Runner {
    def call(value Int, mapper (Int) -> Result[Int, Str]) Result[Int, Str] {
        mapped Result[Int, Str] = mapper(value)
        match mapped {
            case Ok(item) => Ok(item)
            case Err(error) => Err(error)
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
        assert!(module.contains("tmp1_1 = new Greeter();"));
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

    println(declared.name().orPanic())
    println(actual.qualifiedName().orPanic())
    println(declared.kind())

    classType ClassType[User] = declared.asClass().orPanic()
    fields [Field] = classType.fields()
    println(fields.size())

    nameField Field = fields.get(0).orPanic()
    ageField Field = fields.get(1).orPanic()

    println(nameField.name())
    println(nameField.fieldType().name().orPanic())
    println(ageField.name())
    println(ageField.fieldType().name().orPanic())

    enumType EnumType[Status] = typeOf[Status].asEnum().orPanic()
    println(enumType.name().orPanic())
    println(enumType.kind())
    println(enumType.case("Pending").orPanic().name())
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
