use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
};

use crate::{
    Diagnostic,
    ast::{
        Annotation, AssignOp, AssignmentStmt, Binding, Block, CallableBody, ElseBranch,
        ElseExprBranch, Expr, ExprStmt, ForBinding, ForStmt, FunctionDecl, IfConditionClause,
        IfStmt, ImplBlock, ImplTargetKind, ImportSymbol, LambdaBody, LetElseStmt, MatchCase,
        MatchCaseBody, MethodDecl, Pattern, PatternBindingStmt, Program, RecordTypeField, Stmt,
        TypeDecl, TypeKind, TypeMember, TypeParam, TypeRef, Visibility, WhileStmt,
    },
    lexer::lex,
    parser::parse_program,
    render_diagnostic,
    source::SourceFile,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocatedDiagnostic {
    pub path: String,
    pub diagnostic: Diagnostic,
}

#[derive(Debug, Clone, Default)]
pub struct ResolveResult {
    pub diagnostics: Vec<LocatedDiagnostic>,
}

impl ResolveResult {
    pub fn has_errors(&self) -> bool {
        !self.diagnostics.is_empty()
    }
}

#[derive(Debug, Clone)]
pub struct CheckResult {
    pub diagnostics: Vec<Diagnostic>,
}

pub fn resolve_program(program: &Program) -> CheckResult {
    let ambient = AmbientRegistry::with_builtin_values();
    let modules = HashMap::new();
    let module = LoadedModule {
        path: PathBuf::from("<memory>"),
        display_path: "<memory>".to_string(),
        program: program.clone(),
        imports: HashMap::new(),
        symbol_imports: HashMap::new(),
        dependencies: Vec::new(),
    };
    let mut resolver = Resolver::new("<memory>", &module, &modules, &ambient);
    resolver.resolve();
    CheckResult {
        diagnostics: resolver.into_diagnostics(),
    }
}

pub fn resolve_path(path: impl AsRef<Path>) -> Result<ResolveResult, String> {
    let root = path.as_ref();
    let stdlib_dir = find_stdlib_dir(root.parent().unwrap_or_else(|| Path::new(".")))?;
    let ambient = AmbientRegistry::load_from_stdlib(&stdlib_dir)?;
    let mut graph = ModuleGraph::default();
    let root_path = load_module(root, &mut graph, &mut HashSet::new())?;

    let mut visited = HashSet::new();
    let mut order = Vec::new();
    collect_module_order(&graph, &root_path, &mut visited, &mut order);

    let mut diagnostics = Vec::new();
    for module_path in order {
        let Some(module) = graph.modules.get(&module_path) else {
            continue;
        };
        let mut resolver = Resolver::new(
            module.display_path.as_str(),
            module,
            &graph.modules,
            &ambient,
        );
        resolver.resolve();
        diagnostics.extend(resolver.into_diagnostics().into_iter().map(|diagnostic| {
            LocatedDiagnostic {
                path: module.display_path.clone(),
                diagnostic,
            }
        }));
    }

    Ok(ResolveResult { diagnostics })
}

pub(crate) fn load_module_graph(path: impl AsRef<Path>) -> Result<(ModuleGraph, PathBuf), String> {
    let mut graph = ModuleGraph::default();
    let root = load_module(path.as_ref(), &mut graph, &mut HashSet::new())?;
    Ok((graph, root))
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ModuleGraph {
    pub(crate) modules: HashMap<PathBuf, LoadedModule>,
}

#[derive(Debug, Clone)]
pub(crate) struct LoadedModule {
    pub(crate) path: PathBuf,
    pub(crate) display_path: String,
    pub(crate) program: Program,
    pub(crate) imports: HashMap<String, PathBuf>,
    pub(crate) symbol_imports: HashMap<String, ImportedSymbol>,
    pub(crate) dependencies: Vec<PathBuf>,
}

#[derive(Debug, Clone)]
pub(crate) struct ImportedSymbol {
    pub(crate) original_name: String,
    pub(crate) object_name: Option<String>,
    pub(crate) kind: ImportedKind,
    pub(crate) module_path: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ImportedKind {
    Function,
    Value,
    Type,
    Interface,
    Object,
}

#[derive(Debug, Clone, Default)]
struct AmbientRegistry {
    types: HashMap<String, TypeInfo>,
    values: HashSet<String>,
}

impl AmbientRegistry {
    fn with_builtin_values() -> Self {
        let mut registry = AmbientRegistry::default();
        for value in [
            "List", "Map", "Set", "Array", "Range", "Int", "Int64", "Bool", "Rune", "Float",
            "Float64", "Str", "Unit", "Never", "print", "println", "printf", "panic",
        ] {
            registry.values.insert(value.to_string());
        }
        registry
    }

    fn load_from_stdlib(stdlib_dir: &Path) -> Result<Self, String> {
        let mut registry = AmbientRegistry::with_builtin_values();
        let mut entries = fs::read_dir(stdlib_dir)
            .map_err(|err| format!("read stdlib {}: {err}", stdlib_dir.display()))?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "lum"))
            .collect::<Vec<_>>();
        entries.sort();

        for path in entries {
            let directives = read_directives(&path)?;
            if !directives.interpreter {
                continue;
            }
            let program = parse_program_from_path(&path)?;
            let decls = collect_top_level_decls(&program);
            for (name, info) in decls.types {
                registry.values.insert(name.clone());
                registry.types.insert(name.clone(), info.clone());
                if info.kind == TypeKind::Enum {
                    for case in info.enum_cases.keys() {
                        registry.values.insert(case.clone());
                    }
                }
            }
            for (name, info) in decls.objects {
                registry.values.insert(name.clone());
                registry.types.insert(name, info);
            }
        }

        Ok(registry)
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct FileDirectives {
    pub(crate) interpreter: bool,
}

#[derive(Debug, Clone, Default)]
struct TopLevelDecls {
    functions: HashMap<String, DeclSpan>,
    globals: HashMap<String, Symbol>,
    types: HashMap<String, TypeInfo>,
    objects: HashMap<String, TypeInfo>,
}

#[derive(Debug, Clone, Copy)]
struct DeclSpan {
    visibility: Visibility,
    span: crate::source::Span,
}

#[derive(Debug, Clone, Copy)]
struct Symbol {
    span: crate::source::Span,
    mutable: bool,
}

#[derive(Debug, Clone)]
struct TypeInfo {
    kind: TypeKind,
    visibility: Visibility,
    arity: usize,
    span: crate::source::Span,
    fields: Vec<SymbolicField>,
    methods: HashMap<String, DeclSpan>,
    enum_cases: HashMap<String, crate::source::Span>,
}

#[derive(Debug, Clone, Copy)]
struct SymbolicField {
    name: &'static str,
}

impl SymbolicField {
    fn from_binding(name: &str) -> Self {
        Self {
            name: Box::leak(name.to_string().into_boxed_str()),
        }
    }
}

#[derive(Debug, Clone, Default)]
struct ModuleNamespace {
    functions: HashMap<String, crate::source::Span>,
    globals: HashMap<String, crate::source::Span>,
    types: HashMap<String, TypeInfo>,
    objects: HashMap<String, TypeInfo>,
}

pub(crate) fn load_module(
    path: &Path,
    graph: &mut ModuleGraph,
    loading: &mut HashSet<PathBuf>,
) -> Result<PathBuf, String> {
    let abs = fs::canonicalize(path).map_err(|err| format!("resolve {}: {err}", path.display()))?;
    if graph.modules.contains_key(&abs) {
        return Ok(abs);
    }
    if !loading.insert(abs.clone()) {
        return Err(format!("use cycle detected at {}", abs.display()));
    }

    let program = parse_program_from_path(&abs)?;
    let display_path = abs.display().to_string();
    let base_dir = abs
        .parent()
        .ok_or_else(|| format!("resolve module base for {}", abs.display()))?;

    let mut module = LoadedModule {
        path: abs.clone(),
        display_path,
        program,
        imports: HashMap::new(),
        symbol_imports: HashMap::new(),
        dependencies: Vec::new(),
    };

    let mut import_paths = HashMap::<String, String>::new();
    let module_name = module
        .program
        .module
        .as_ref()
        .map(|module| module.name.as_str());
    let imports = module.program.imports.clone();
    for import in imports {
        let child_path = base_dir.join(format!("{}.lum", import.path));
        let child_abs = load_module(&child_path, graph, loading)?;
        let child = graph
            .modules
            .get(&child_abs)
            .ok_or_else(|| format!("loaded module missing {}", child_abs.display()))?;

        if !module.dependencies.contains(&child_abs) {
            module.dependencies.push(child_abs.clone());
        }
        if import.object_name.is_none() && import.symbols.is_empty() && !import.wildcard {
            let alias = module_alias(&import.path);
            if let Some(existing) = import_paths.get(&alias) {
                if existing != &import.path {
                    return Err(format!(
                        "duplicate use alias '{}' for paths '{}' and '{}'",
                        alias, existing, import.path
                    ));
                }
            }
            if module.symbol_imports.contains_key(&alias) {
                return Err(format!(
                    "module use alias '{}' conflicts with used symbol",
                    alias
                ));
            }
            if let Some(child_module) = child.program.module.as_ref() {
                if child_module.name != alias {
                    return Err(format!(
                        "use '{}' expected module '{}', got '{}'",
                        import.path, alias, child_module.name
                    ));
                }
            }
            import_paths.insert(alias.clone(), import.path.clone());
            module.imports.insert(alias, child_abs.clone());
            continue;
        }

        let same_module = module_name.is_some_and(|current| {
            child
                .program
                .module
                .as_ref()
                .is_some_and(|module| module.name == current)
        });
        let mut symbols = import.symbols.clone();
        if let Some(object_name) = import.object_name.as_deref() {
            if import.wildcard {
                symbols = exported_object_members(child, object_name, same_module);
            }
        } else if import.wildcard {
            symbols = exported_symbols(child, same_module);
        }

        for symbol in symbols {
            let resolved = if let Some(object_name) = import.object_name.as_deref() {
                resolve_imported_object_member(
                    child,
                    object_name,
                    symbol.name.as_str(),
                    same_module,
                )
                .ok_or_else(|| {
                    format!(
                        "use '{}' has no visible member '{}' on object '{}'",
                        import.path, symbol.name, object_name
                    )
                })?
            } else {
                resolve_imported_symbol(child, symbol.name.as_str(), same_module).ok_or_else(
                    || {
                        format!(
                            "use '{}' has no public symbol '{}'",
                            import.path, symbol.name
                        )
                    },
                )?
            };

            let local_name = symbol.alias.clone().unwrap_or(symbol.name.clone());
            if module.imports.contains_key(&local_name) {
                return Err(format!(
                    "used symbol '{}' conflicts with module use alias",
                    local_name
                ));
            }
            if let Some(existing) = module.symbol_imports.get(&local_name) {
                if existing.module_path != resolved.module_path
                    || existing.original_name != resolved.original_name
                    || existing.object_name != resolved.object_name
                {
                    return Err(format!("duplicate used symbol '{}'", local_name));
                }
            }
            module.symbol_imports.insert(local_name, resolved);
        }
    }

    loading.remove(&abs);
    graph.modules.insert(abs.clone(), module);
    Ok(abs)
}

pub(crate) fn parse_program_from_path(path: &Path) -> Result<Program, String> {
    let text = fs::read_to_string(path).map_err(|err| format!("read {}: {err}", path.display()))?;
    let file = SourceFile::new(path.display().to_string(), text);
    let lexed = lex(&file);
    if !lexed.diagnostics.is_empty() {
        return Err(format_path_diagnostics(
            path,
            Some(&file.text),
            &lexed.diagnostics,
        ));
    }
    let parsed = parse_program(&lexed.tokens);
    if !parsed.diagnostics.is_empty() {
        return Err(format_path_diagnostics(
            path,
            Some(&file.text),
            &parsed.diagnostics,
        ));
    }
    parsed
        .program
        .ok_or_else(|| format!("parse {}: parser did not produce a program", path.display()))
}

fn format_path_diagnostics(
    path: &Path,
    source: Option<&str>,
    diagnostics: &[Diagnostic],
) -> String {
    let display = path.display().to_string();
    diagnostics
        .iter()
        .map(|diagnostic| render_diagnostic(&display, source, diagnostic))
        .collect::<Vec<_>>()
        .join("\n")
}

fn module_alias(path: &str) -> String {
    path.rsplit('/').next().unwrap_or(path).to_string()
}

pub(crate) fn read_directives(path: &Path) -> Result<FileDirectives, String> {
    let text = fs::read_to_string(path).map_err(|err| format!("read {}: {err}", path.display()))?;
    let mut directives = FileDirectives::default();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if !line.starts_with('#') {
            break;
        }
        match line {
            "# INTERPRETER" => directives.interpreter = true,
            _ => {}
        }
    }
    Ok(directives)
}

pub(crate) fn find_stdlib_dir(start: &Path) -> Result<PathBuf, String> {
    let mut dir =
        fs::canonicalize(start).map_err(|err| format!("resolve {}: {err}", start.display()))?;
    loop {
        let candidate = dir.join("stdlib");
        if candidate.is_dir() {
            return Ok(candidate);
        }
        let Some(parent) = dir.parent() else {
            return Err(format!(
                "could not find stdlib directory from {}",
                start.display()
            ));
        };
        if parent == dir {
            return Err(format!(
                "could not find stdlib directory from {}",
                start.display()
            ));
        }
        dir = parent.to_path_buf();
    }
}

pub(crate) fn collect_module_order(
    graph: &ModuleGraph,
    root: &Path,
    seen: &mut HashSet<PathBuf>,
    out: &mut Vec<PathBuf>,
) {
    if !seen.insert(root.to_path_buf()) {
        return;
    }
    let Some(module) = graph.modules.get(root) else {
        return;
    };
    for dependency in &module.dependencies {
        collect_module_order(graph, dependency, seen, out);
    }
    out.push(root.to_path_buf());
}

fn exported_symbols(module: &LoadedModule, same_module: bool) -> Vec<ImportSymbol> {
    let decls = collect_top_level_decls(&module.program);
    let mut out = Vec::new();
    for (name, decl) in decls.functions {
        if decl.visibility == Visibility::Public {
            out.push(ImportSymbol {
                name,
                alias: None,
                span: decl.span,
            });
        }
    }
    for (name, symbol) in decls.globals {
        if symbol.mutable {
            continue;
        }
        out.push(ImportSymbol {
            name,
            alias: None,
            span: symbol.span,
        });
    }
    for (name, info) in decls.types {
        if info.visibility != Visibility::Hidden || same_module {
            out.push(ImportSymbol {
                name,
                alias: None,
                span: info.span,
            });
        }
    }
    for (name, info) in decls.objects {
        if info.visibility != Visibility::Hidden || same_module {
            out.push(ImportSymbol {
                name,
                alias: None,
                span: info.span,
            });
        }
    }
    out
}

fn exported_object_members(
    module: &LoadedModule,
    object_name: &str,
    same_module: bool,
) -> Vec<ImportSymbol> {
    let decls = collect_top_level_decls(&module.program);
    let Some(info) = decls.objects.get(object_name) else {
        return Vec::new();
    };
    if info.visibility == Visibility::Hidden && !same_module {
        return Vec::new();
    }
    let mut out = Vec::new();
    for (name, method) in &info.methods {
        if method.visibility != Visibility::Hidden || same_module {
            out.push(ImportSymbol {
                name: name.clone(),
                alias: None,
                span: method.span,
            });
        }
    }
    out
}

fn resolve_imported_symbol(
    module: &LoadedModule,
    name: &str,
    same_module: bool,
) -> Option<ImportedSymbol> {
    let decls = collect_top_level_decls(&module.program);
    if let Some(decl) = decls.functions.get(name) {
        if decl.visibility == Visibility::Public {
            return Some(ImportedSymbol {
                original_name: name.to_string(),
                object_name: None,
                kind: ImportedKind::Function,
                module_path: module.path.clone(),
            });
        }
    }
    if let Some(symbol) = decls.globals.get(name) {
        if !symbol.mutable {
            return Some(ImportedSymbol {
                original_name: name.to_string(),
                object_name: None,
                kind: ImportedKind::Value,
                module_path: module.path.clone(),
            });
        }
    }
    if let Some(info) = decls.types.get(name) {
        if info.visibility != Visibility::Hidden || same_module {
            return Some(ImportedSymbol {
                original_name: name.to_string(),
                object_name: None,
                kind: if info.kind == TypeKind::Interface {
                    ImportedKind::Interface
                } else {
                    ImportedKind::Type
                },
                module_path: module.path.clone(),
            });
        }
    }
    if let Some(info) = decls.objects.get(name) {
        if info.visibility != Visibility::Hidden || same_module {
            return Some(ImportedSymbol {
                original_name: name.to_string(),
                object_name: None,
                kind: ImportedKind::Object,
                module_path: module.path.clone(),
            });
        }
    }
    None
}

fn resolve_imported_object_member(
    module: &LoadedModule,
    object_name: &str,
    member_name: &str,
    same_module: bool,
) -> Option<ImportedSymbol> {
    let decls = collect_top_level_decls(&module.program);
    let info = decls.objects.get(object_name)?;
    if info.visibility == Visibility::Hidden && !same_module {
        return None;
    }
    let method = info.methods.get(member_name)?;
    if method.visibility == Visibility::Hidden && !same_module {
        return None;
    }
    Some(ImportedSymbol {
        original_name: member_name.to_string(),
        object_name: Some(object_name.to_string()),
        kind: ImportedKind::Function,
        module_path: module.path.clone(),
    })
}

fn collect_top_level_decls(program: &Program) -> TopLevelDecls {
    let mut decls = TopLevelDecls::default();
    for item in &program.items {
        match item {
            crate::ast::Item::Function(function) => {
                decls.functions.insert(
                    function.name.clone(),
                    DeclSpan {
                        visibility: function.visibility,
                        span: function.span,
                    },
                );
            }
            crate::ast::Item::Type(decl) => {
                let info = summarize_type(decl);
                if decl.kind == TypeKind::Object {
                    decls.objects.insert(decl.name.clone(), info);
                } else {
                    decls.types.insert(decl.name.clone(), info);
                }
            }
            crate::ast::Item::Statement(Stmt::Binding(binding)) => {
                for local in &binding.bindings {
                    if local.name == "_" {
                        continue;
                    }
                    decls.globals.insert(
                        local.name.clone(),
                        Symbol {
                            span: local.span,
                            mutable: local.mutable,
                        },
                    );
                }
            }
            _ => {}
        }
    }
    for item in &program.items {
        if let crate::ast::Item::Impl(block) = item {
            merge_impl_decl_into_infos(&mut decls.types, &mut decls.objects, block);
        }
    }
    decls
}

fn synthetic_single_type_info(target: &TypeInfo) -> TypeInfo {
    TypeInfo {
        kind: TypeKind::Object,
        visibility: target.visibility,
        arity: 0,
        span: target.span,
        fields: Vec::new(),
        methods: HashMap::new(),
        enum_cases: HashMap::new(),
    }
}

fn standalone_single_type_info(span: crate::source::Span) -> TypeInfo {
    TypeInfo {
        kind: TypeKind::Object,
        visibility: Visibility::Default,
        arity: 0,
        span,
        fields: Vec::new(),
        methods: HashMap::new(),
        enum_cases: HashMap::new(),
    }
}

fn merge_impl_decl_into_infos(
    types: &mut HashMap<String, TypeInfo>,
    objects: &mut HashMap<String, TypeInfo>,
    block: &ImplBlock,
) {
    let Some(target_name) = type_ref_name(&block.target) else {
        return;
    };
    let target = match block.target_kind {
        ImplTargetKind::Instance => types.get_mut(target_name),
        ImplTargetKind::Single => {
            if !objects.contains_key(target_name) {
                if matches!(&block.target, TypeRef::Named { args, .. } if !args.is_empty()) {
                    return;
                }
                let info = types
                    .get(target_name)
                    .cloned()
                    .map(|target_type| synthetic_single_type_info(&target_type))
                    .unwrap_or_else(|| standalone_single_type_info(block.span));
                objects.insert(target_name.to_string(), info);
            }
            objects.get_mut(target_name)
        }
    };
    let Some(target) = target else {
        return;
    };
    for method in &block.methods {
        target.methods.insert(
            method.name.clone(),
            DeclSpan {
                visibility: method.visibility,
                span: method.span,
            },
        );
    }
}

fn summarize_type(decl: &TypeDecl) -> TypeInfo {
    let mut fields = Vec::new();
    let mut methods = HashMap::new();
    let mut enum_cases = HashMap::new();
    for member in &decl.members {
        match member {
            TypeMember::Field(field) => {
                fields.push(SymbolicField::from_binding(field.name.as_str()));
            }
            TypeMember::Method(method) => {
                methods.insert(
                    method.name.clone(),
                    DeclSpan {
                        visibility: method.visibility,
                        span: method.span,
                    },
                );
            }
            TypeMember::Case(case) => {
                enum_cases.insert(case.name.clone(), case.span);
            }
        }
    }
    TypeInfo {
        kind: decl.kind,
        visibility: decl.visibility,
        arity: decl.type_params.len(),
        span: decl.span,
        fields,
        methods,
        enum_cases,
    }
}

struct Resolver<'a> {
    module: &'a LoadedModule,
    modules: &'a HashMap<PathBuf, LoadedModule>,
    ambient: &'a AmbientRegistry,
    diagnostics: Vec<Diagnostic>,
    scopes: Vec<HashMap<String, Symbol>>,
    type_scopes: Vec<HashMap<String, crate::source::Span>>,
    globals: HashMap<String, Symbol>,
    functions: HashMap<String, crate::source::Span>,
    types: HashMap<String, TypeInfo>,
    objects: HashMap<String, TypeInfo>,
    enum_case_values: HashMap<String, crate::source::Span>,
    imported_values: HashMap<String, Symbol>,
    imported_functions: HashMap<String, crate::source::Span>,
    imported_types: HashMap<String, TypeInfo>,
    imported_objects: HashMap<String, TypeInfo>,
    modules_by_alias: HashMap<String, ModuleNamespace>,
    field_hint_scopes: Vec<HashSet<String>>,
    method_hint_scopes: Vec<HashSet<String>>,
    loop_depth: usize,
    current_constructor: bool,
}

impl<'a> Resolver<'a> {
    fn new(
        _path: &'a str,
        module: &'a LoadedModule,
        modules: &'a HashMap<PathBuf, LoadedModule>,
        ambient: &'a AmbientRegistry,
    ) -> Self {
        Self {
            module,
            modules,
            ambient,
            diagnostics: Vec::new(),
            scopes: Vec::new(),
            type_scopes: Vec::new(),
            globals: HashMap::new(),
            functions: HashMap::new(),
            types: HashMap::new(),
            objects: HashMap::new(),
            enum_case_values: HashMap::new(),
            imported_values: HashMap::new(),
            imported_functions: HashMap::new(),
            imported_types: HashMap::new(),
            imported_objects: HashMap::new(),
            modules_by_alias: HashMap::new(),
            field_hint_scopes: Vec::new(),
            method_hint_scopes: Vec::new(),
            loop_depth: 0,
            current_constructor: false,
        }
    }

    // resolve performs the main structural pass: collect top-level declarations,
    // install imports, resolve globals in declaration order, then walk bodies.
    fn resolve(&mut self) {
        self.install_imports();
        self.collect_top_level_decls();
        self.resolve_globals();
        self.resolve_items();
    }

    fn into_diagnostics(self) -> Vec<Diagnostic> {
        self.diagnostics
    }

    fn collect_top_level_decls(&mut self) {
        for item in &self.module.program.items {
            match item {
                crate::ast::Item::Function(function) => {
                    self.resolve_annotations(&function.annotations);
                    if let Some(previous) = self.functions.get(&function.name) {
                        self.add_duplicate(
                            "duplicate_function",
                            format!("duplicate function '{}'", function.name),
                            function.span,
                            *previous,
                        );
                    } else {
                        self.functions.insert(function.name.clone(), function.span);
                    }
                }
                crate::ast::Item::Type(decl) => {
                    self.resolve_annotations(&decl.annotations);
                    let info = summarize_type(decl);
                    let previous = if decl.kind == TypeKind::Object {
                        self.objects.get(&decl.name).map(|info| info.span)
                    } else {
                        self.types.get(&decl.name).map(|info| info.span)
                    };
                    if let Some(previous) = previous {
                        self.add_duplicate(
                            "duplicate_type",
                            format!("duplicate type '{}'", decl.name),
                            decl.span,
                            previous,
                        );
                    } else {
                        if decl.kind == TypeKind::Object {
                            self.objects.insert(decl.name.clone(), info);
                        } else {
                            if decl.kind == TypeKind::Enum {
                                for member in &decl.members {
                                    if let TypeMember::Case(case) = member {
                                        self.enum_case_values.insert(case.name.clone(), case.span);
                                    }
                                }
                            }
                            self.types.insert(decl.name.clone(), info);
                        }
                    }
                }
                crate::ast::Item::Statement(Stmt::Binding(binding)) => {
                    for annotation_binding in &binding.bindings {
                        self.resolve_type_ref(annotation_binding.ty.as_ref());
                    }
                }
                _ => {}
            }
        }
        for item in &self.module.program.items {
            if let crate::ast::Item::Impl(block) = item {
                self.merge_impl_decl(block);
            }
        }
    }

    fn merge_impl_decl(&mut self, block: &ImplBlock) {
        let Some(target_name) = type_ref_name(&block.target) else {
            return;
        };
        match block.target_kind {
            ImplTargetKind::Instance => {
                let Some(target) = self.types.get_mut(target_name) else {
                    return;
                };
                for method in &block.methods {
                    target.methods.insert(
                        method.name.clone(),
                        DeclSpan {
                            visibility: method.visibility,
                            span: method.span,
                        },
                    );
                }
            }
            ImplTargetKind::Single => {
                if !self.objects.contains_key(target_name) {
                    if matches!(&block.target, TypeRef::Named { args, .. } if !args.is_empty()) {
                        return;
                    }
                    let info = self
                        .types
                        .get(target_name)
                        .cloned()
                        .map(|target_type| synthetic_single_type_info(&target_type))
                        .unwrap_or_else(|| standalone_single_type_info(block.span));
                    self.objects.insert(target_name.to_string(), info);
                }
                let Some(target) = self.objects.get_mut(target_name) else {
                    return;
                };
                for method in &block.methods {
                    target.methods.insert(
                        method.name.clone(),
                        DeclSpan {
                            visibility: method.visibility,
                            span: method.span,
                        },
                    );
                }
            }
        }
    }

    // install_imports exposes module aliases and direct imports before
    // resolution so later identifier lookup can treat them as ordinary names.
    fn install_imports(&mut self) {
        for (alias, module_path) in &self.module.imports {
            let Some(module) = self.modules.get(module_path) else {
                continue;
            };
            let decls = collect_top_level_decls(&module.program);
            let namespace = ModuleNamespace {
                functions: decls
                    .functions
                    .into_iter()
                    .filter_map(|(name, decl)| {
                        (decl.visibility == Visibility::Public).then_some((name, decl.span))
                    })
                    .collect(),
                globals: decls
                    .globals
                    .into_iter()
                    .map(|(name, sym)| (name, sym.span))
                    .collect(),
                types: decls.types,
                objects: decls.objects,
            };
            self.modules_by_alias.insert(alias.clone(), namespace);
        }

        for (local_name, symbol) in &self.module.symbol_imports {
            let Some(module) = self.modules.get(&symbol.module_path) else {
                continue;
            };
            let decls = collect_top_level_decls(&module.program);
            match symbol.kind {
                ImportedKind::Function => {
                    let span = if let Some(object_name) = symbol.object_name.as_deref() {
                        decls
                            .objects
                            .get(object_name)
                            .and_then(|info| info.methods.get(&symbol.original_name))
                            .map(|decl| decl.span)
                    } else {
                        decls
                            .functions
                            .get(&symbol.original_name)
                            .map(|decl| decl.span)
                    };
                    if let Some(span) = span {
                        self.imported_functions.insert(local_name.clone(), span);
                    }
                }
                ImportedKind::Value => {
                    if let Some(sym) = decls.globals.get(&symbol.original_name) {
                        self.imported_values.insert(local_name.clone(), *sym);
                    }
                }
                ImportedKind::Type | ImportedKind::Interface => {
                    if let Some(info) = decls.types.get(&symbol.original_name) {
                        self.imported_types.insert(local_name.clone(), info.clone());
                    }
                }
                ImportedKind::Object => {
                    if let Some(info) = decls.objects.get(&symbol.original_name) {
                        self.imported_objects
                            .insert(local_name.clone(), info.clone());
                    }
                }
            }
        }
    }

    // resolve_globals keeps top-level value initializers declaration-ordered so
    // earlier globals are visible and later ones are not.
    fn resolve_globals(&mut self) {
        self.push_scope();
        for item in &self.module.program.items {
            let crate::ast::Item::Statement(Stmt::Binding(binding)) = item else {
                continue;
            };
            for value in &binding.values {
                self.resolve_expr(value);
            }
            for local in &binding.bindings {
                self.resolve_type_ref(local.ty.as_ref());
                if local.name == "_" {
                    continue;
                }
                if let Some(previous) = self.globals.get(&local.name) {
                    self.add_duplicate(
                        "duplicate_binding",
                        format!("duplicate binding '{}'", local.name),
                        local.span,
                        previous.span,
                    );
                    continue;
                }
                self.globals.insert(
                    local.name.clone(),
                    Symbol {
                        span: local.span,
                        mutable: local.mutable,
                    },
                );
            }
        }
        self.pop_scope();
    }

    fn resolve_items(&mut self) {
        self.push_scope();
        for item in &self.module.program.items {
            match item {
                crate::ast::Item::Function(function) => self.resolve_function(function),
                crate::ast::Item::Type(decl) => self.resolve_type_decl(decl),
                crate::ast::Item::Impl(block) => self.resolve_impl(block),
                crate::ast::Item::Statement(Stmt::Binding(_)) => {}
                crate::ast::Item::Statement(statement) => self.resolve_stmt(statement),
            }
        }
        self.pop_scope();
    }

    fn resolve_annotations(&mut self, annotations: &[Annotation]) {
        for annotation in annotations {
            self.resolve_expr(&annotation.value);
        }
    }

    fn resolve_function(&mut self, function: &FunctionDecl) {
        self.push_type_scope();
        for param in &function.type_params {
            self.define_type_param(param);
        }
        self.resolve_type_parameter_bounds(&function.type_params);
        self.resolve_type_ref(function.return_type.as_ref());
        self.push_scope();
        for param in &function.params {
            self.resolve_type_ref(param.ty.as_ref());
            self.define_value(
                param.name.as_str(),
                param.span,
                false,
                "duplicate_parameter",
                format!("duplicate parameter '{}'", param.name),
                true,
            );
        }
        self.resolve_callable_body(&function.body);
        self.pop_scope();
        self.pop_type_scope();
    }

    fn resolve_type_decl(&mut self, decl: &TypeDecl) {
        self.push_type_scope();
        for param in &decl.type_params {
            self.define_type_param(param);
        }
        self.resolve_type_parameter_bounds(&decl.type_params);
        for bound in &decl.with_bounds {
            self.resolve_type_ref(Some(bound));
        }
        for member in &decl.members {
            match member {
                TypeMember::Field(field) => {
                    self.resolve_annotations(&field.annotations);
                    self.resolve_type_ref(field.ty.as_ref());
                }
                TypeMember::Case(case) => {
                    self.resolve_annotations(&case.annotations);
                    for field in &case.fields {
                        self.resolve_annotations(&field.annotations);
                        self.resolve_type_ref(field.ty.as_ref());
                        if let Some(initializer) = &field.initializer {
                            self.resolve_expr(initializer);
                        }
                    }
                }
                TypeMember::Method(_) => {}
            }
        }

        self.push_scope();
        self.push_field_hints(decl.members.iter().filter_map(|member| match member {
            TypeMember::Field(field) => Some(field.name.as_str()),
            _ => None,
        }));
        if decl.kind != TypeKind::Enum {
            self.define_value(
                "this",
                decl.span,
                false,
                "duplicate_binding",
                "duplicate binding 'this'".to_string(),
                true,
            );
            for member in &decl.members {
                if let TypeMember::Field(field) = member {
                    if let Some(initializer) = &field.initializer {
                        self.resolve_expr(initializer);
                    }
                }
            }
        }
        if decl.kind == TypeKind::Enum {
            for member in &decl.members {
                if let TypeMember::Case(case) = member {
                    self.define_value(
                        case.name.as_str(),
                        case.span,
                        false,
                        "duplicate_binding",
                        format!("duplicate binding '{}'", case.name),
                        false,
                    );
                }
            }
        }
        for member in &decl.members {
            match member {
                TypeMember::Method(method) => self.resolve_method(method),
                TypeMember::Case(case) => {
                    if !case.fields.is_empty() {
                        self.push_scope();
                        for field in &case.fields {
                            self.define_value(
                                field.name.as_str(),
                                field.span,
                                field.mutable,
                                "duplicate_binding",
                                format!("duplicate binding '{}'", field.name),
                                true,
                            );
                        }
                        self.pop_scope();
                    }
                }
                TypeMember::Field(_) => {}
            }
        }
        self.pop_field_hints();
        self.pop_scope();
        self.pop_type_scope();
    }

    fn resolve_impl(&mut self, block: &ImplBlock) {
        self.push_type_scope();
        self.install_impl_target_type_params(&block.target);
        self.resolve_impl_target(block);
        let target_name = type_ref_name(&block.target);
        let target_fields = target_name.and_then(|name| match block.target_kind {
            ImplTargetKind::Instance => self.types.get(name).cloned(),
            ImplTargetKind::Single => self.objects.get(name).cloned(),
        });

        self.push_scope();
        self.push_field_hints(
            target_fields
                .iter()
                .flat_map(|info| info.fields.iter().map(|field| field.name)),
        );
        self.push_method_hints(
            target_fields
                .iter()
                .flat_map(|info| info.methods.keys().map(String::as_str)),
        );
        for method in &block.methods {
            self.resolve_method(method);
        }
        self.pop_method_hints();
        self.pop_field_hints();
        self.pop_scope();
        self.pop_type_scope();
    }

    fn install_impl_target_type_params(&mut self, target: &TypeRef) {
        if let TypeRef::Named { args, .. } = target {
            for arg in args {
                if let TypeRef::Named { name, args, span } = arg {
                    if args.is_empty() {
                        self.current_type_scope().insert(name.clone(), *span);
                    }
                }
            }
        }
    }

    fn resolve_impl_target(&mut self, block: &ImplBlock) {
        let target = &block.target;
        match target {
            TypeRef::Named { name, args, span } => {
                for arg in args {
                    self.resolve_type_ref(Some(arg));
                }
                match block.target_kind {
                    ImplTargetKind::Instance => {
                        if let Some(info) = self.types.get(name) {
                            if args.len() != info.arity {
                                self.add_error(
                                    "invalid_type_arity",
                                    format!(
                                        "type '{}' expects {} type arguments",
                                        name,
                                        arity_label(info.arity)
                                    ),
                                    *span,
                                );
                            }
                        } else {
                            self.add_error(
                                "undefined_type",
                                format!("undefined type '{}'", name),
                                *span,
                            );
                        }
                    }
                    ImplTargetKind::Single => {
                        if let Some(info) = self.objects.get(name) {
                            if args.len() != info.arity {
                                self.add_error(
                                    "invalid_type_arity",
                                    format!(
                                        "single '{}' expects {} type arguments",
                                        name,
                                        arity_label(info.arity)
                                    ),
                                    *span,
                                );
                            }
                        } else if self.types.contains_key(name) {
                            if !args.is_empty() {
                                self.add_error(
                                    "invalid_type_arity",
                                    format!(
                                        "single '{}' expects no type arguments; write 'impl single {}'",
                                        name, name
                                    ),
                                    *span,
                                );
                            }
                        } else if !args.is_empty() {
                            self.add_error(
                                "invalid_type_arity",
                                format!(
                                    "single '{}' expects no type arguments; write 'impl single {}'",
                                    name, name
                                ),
                                *span,
                            );
                        }
                    }
                }
            }
            other => self.resolve_type_ref(Some(other)),
        }
    }

    fn define_implicit_this(&mut self, span: Option<crate::source::Span>) {
        if let Some(span) = span {
            self.define_value(
                "this",
                span,
                false,
                "duplicate_binding",
                "duplicate binding 'this'".to_string(),
                true,
            );
        }
    }

    fn resolve_method(&mut self, method: &MethodDecl) {
        self.resolve_annotations(&method.annotations);
        let is_constructor = method.name == "new";
        let previous_constructor = self.current_constructor;
        self.current_constructor = is_constructor;
        self.push_type_scope();
        for param in &method.type_params {
            self.define_type_param(param);
        }
        self.resolve_type_parameter_bounds(&method.type_params);
        self.resolve_type_ref(method.return_type.as_ref());
        self.push_scope();
        self.define_implicit_this(Some(method.span));
        for param in &method.params {
            self.resolve_type_ref(param.ty.as_ref());
            self.define_value(
                param.name.as_str(),
                param.span,
                false,
                "duplicate_parameter",
                format!("duplicate parameter '{}'", param.name),
                is_constructor,
            );
        }
        if let Some(body) = &method.body {
            self.resolve_callable_body(body);
        }
        self.pop_scope();
        self.pop_type_scope();
        self.current_constructor = previous_constructor;
    }

    fn resolve_callable_body(&mut self, body: &CallableBody) {
        match body {
            CallableBody::Block(block) => self.resolve_block(block),
            CallableBody::Expr(expr) => self.resolve_expr(expr),
        }
    }

    fn resolve_block(&mut self, block: &Block) {
        self.push_scope();
        for statement in &block.statements {
            self.resolve_stmt(statement);
        }
        self.pop_scope();
    }

    // resolve_stmt owns lexical-scope creation and the declaration-before-use
    // rules for bindings, loops, match arms, and local functions.
    fn resolve_stmt(&mut self, statement: &Stmt) {
        match statement {
            Stmt::Binding(binding) => {
                for value in &binding.values {
                    self.resolve_expr(value);
                }
                for local in &binding.bindings {
                    self.resolve_type_ref(local.ty.as_ref());
                    self.define_binding(local, "duplicate_binding");
                }
            }
            Stmt::PatternBinding(stmt) => self.resolve_pattern_binding(stmt),
            Stmt::ExpectCondition(stmt) => self.resolve_expr(&stmt.condition),
            Stmt::Assignment(assignment) => self.resolve_assignment(assignment),
            Stmt::If(stmt) => self.resolve_if_stmt(stmt),
            Stmt::Match(stmt) => {
                self.resolve_expr(&stmt.value);
                for case in &stmt.cases {
                    self.resolve_match_case(case);
                }
            }
            Stmt::While(WhileStmt {
                condition, body, ..
            }) => {
                self.resolve_expr(condition);
                self.loop_depth += 1;
                self.resolve_block(body);
                self.loop_depth -= 1;
            }
            Stmt::For(ForStmt { bindings, body, .. }) => {
                self.push_scope();
                for binding in bindings {
                    self.resolve_for_binding(binding);
                }
                self.loop_depth += 1;
                self.resolve_block(body);
                self.loop_depth -= 1;
                self.pop_scope();
            }
            Stmt::Defer(stmt) => match &stmt.action {
                crate::ast::DeferAction::Call(expr) => self.resolve_expr(expr),
                crate::ast::DeferAction::Block(block) => self.resolve_block(block),
            },
            Stmt::LetElse(stmt) => self.resolve_let_else(stmt),
            Stmt::Return(return_stmt) => {
                if let Some(value) = &return_stmt.value {
                    self.resolve_expr(value);
                }
            }
            Stmt::Break(break_stmt) => {
                if self.loop_depth == 0 {
                    self.add_error(
                        "invalid_break",
                        "break used outside of a loop",
                        break_stmt.span,
                    );
                }
            }
            Stmt::Continue(continue_stmt) => {
                if self.loop_depth == 0 {
                    self.add_error(
                        "invalid_continue",
                        "continue used outside of a loop",
                        continue_stmt.span,
                    );
                }
            }
            Stmt::Expr(ExprStmt { expr, .. }) => self.resolve_expr(expr),
            Stmt::LocalFunction(function) => {
                self.define_value(
                    function.name.as_str(),
                    function.span,
                    false,
                    "duplicate_binding",
                    format!("duplicate binding '{}'", function.name),
                    false,
                );
                self.resolve_local_function(function);
            }
        }
    }

    fn resolve_local_function(&mut self, function: &FunctionDecl) {
        self.push_type_scope();
        for param in &function.type_params {
            self.define_type_param(param);
        }
        self.resolve_type_parameter_bounds(&function.type_params);
        self.resolve_type_ref(function.return_type.as_ref());
        self.push_scope();
        for param in &function.params {
            self.resolve_type_ref(param.ty.as_ref());
            self.define_value(
                param.name.as_str(),
                param.span,
                false,
                "duplicate_parameter",
                format!("duplicate parameter '{}'", param.name),
                false,
            );
        }
        self.resolve_callable_body(&function.body);
        self.pop_scope();
        self.pop_type_scope();
    }

    fn resolve_if_stmt(&mut self, stmt: &IfStmt) {
        if !stmt.condition_clauses.is_empty() {
            self.push_scope();
            for clause in &stmt.condition_clauses {
                match clause {
                    IfConditionClause::Let(clause) => {
                        self.resolve_expr(&clause.value);
                        self.resolve_pattern(&clause.pattern);
                    }
                    IfConditionClause::Expr(condition) => self.resolve_expr(condition),
                }
            }
            for statement in &stmt.then_block.statements {
                self.resolve_stmt(statement);
            }
            self.pop_scope();
        } else if !stmt.pattern_clauses.is_empty() {
            self.push_scope();
            for clause in &stmt.pattern_clauses {
                self.resolve_expr(&clause.value);
                self.resolve_pattern(&clause.pattern);
            }
            for statement in &stmt.then_block.statements {
                self.resolve_stmt(statement);
            }
            self.pop_scope();
        } else if let Some(value) = &stmt.pattern_value {
            self.resolve_expr(value);
            self.push_scope();
            if let Some(pattern) = &stmt.pattern {
                self.resolve_pattern(pattern);
            }
            for statement in &stmt.then_block.statements {
                self.resolve_stmt(statement);
            }
            self.pop_scope();
        } else if let Some(value) = &stmt.binding_value {
            self.resolve_expr(value);
            self.push_scope();
            for binding in &stmt.bindings {
                self.define_binding(binding, "duplicate_binding");
            }
            for statement in &stmt.then_block.statements {
                self.resolve_stmt(statement);
            }
            self.pop_scope();
        } else if let Some(condition) = &stmt.condition {
            self.resolve_expr(condition);
            self.resolve_block(&stmt.then_block);
        }
        if let Some(else_branch) = &stmt.else_branch {
            self.resolve_else_branch(else_branch);
        }
    }

    fn resolve_let_else(&mut self, stmt: &LetElseStmt) {
        self.resolve_block(&stmt.else_block);
        if !stmt.clauses.is_empty() {
            for clause in &stmt.clauses {
                self.resolve_expr(&clause.value);
                self.resolve_pattern(&clause.pattern);
            }
            return;
        }
        self.resolve_expr(&stmt.value);
        self.resolve_pattern(&stmt.pattern);
    }

    fn resolve_pattern_binding(&mut self, stmt: &PatternBindingStmt) {
        if !stmt.clauses.is_empty() {
            for clause in &stmt.clauses {
                self.resolve_expr(&clause.value);
                self.resolve_pattern(&clause.pattern);
            }
            return;
        }
        self.resolve_expr(&stmt.value);
        self.resolve_pattern(&stmt.pattern);
    }

    fn resolve_else_branch(&mut self, branch: &ElseBranch) {
        match branch {
            ElseBranch::If(stmt) => self.resolve_if_stmt(stmt),
            ElseBranch::Block(block) => self.resolve_block(block),
        }
    }

    fn resolve_match_case(&mut self, case: &MatchCase) {
        self.push_scope();
        self.resolve_pattern(&case.pattern);
        if let Some(guard) = &case.guard {
            self.resolve_expr(guard);
        }
        match &case.body {
            MatchCaseBody::Block(block) => {
                for statement in &block.statements {
                    self.resolve_stmt(statement);
                }
            }
            MatchCaseBody::Expr(expr) => self.resolve_expr(expr),
        }
        self.pop_scope();
    }

    fn resolve_for_binding(&mut self, binding: &ForBinding) {
        if let Some(iterable) = &binding.iterable {
            self.resolve_expr(iterable);
        }
        for value in &binding.values {
            self.resolve_expr(value);
        }
        if let Some(pattern) = &binding.pattern {
            self.resolve_pattern(pattern);
            return;
        }
        for local in &binding.bindings {
            self.resolve_type_ref(local.ty.as_ref());
            self.define_binding(local, "duplicate_binding");
        }
    }

    fn resolve_assignment(&mut self, assignment: &AssignmentStmt) {
        for target in &assignment.targets {
            self.resolve_assignment_target(target, assignment.operator);
        }
        for value in &assignment.values {
            self.resolve_expr(value);
        }
    }

    fn resolve_assignment_target(&mut self, target: &Expr, operator: AssignOp) {
        match target {
            Expr::Identifier { name, span } => {
                if let Some(symbol) = self.lookup_value(name) {
                    if !symbol.mutable {
                        self.add_error(
                            "assign_immutable",
                            format!("cannot assign to immutable binding '{}'", name),
                            *span,
                        );
                    } else if operator == AssignOp::Reassign {
                        // plain reassign is allowed in the Rust resolver for now;
                        // operator-shape validation stays a later typecheck concern.
                    }
                } else {
                    self.add_error("undefined_name", self.undefined_value_message(name), *span);
                }
            }
            Expr::Member { receiver, .. } => self.resolve_expr(receiver),
            Expr::Index {
                receiver, index, ..
            } => {
                self.resolve_expr(receiver);
                self.resolve_expr(index);
            }
            _ => self.add_error(
                "invalid_assignment_target",
                "invalid assignment target",
                target.span(),
            ),
        }
    }

    // resolve_expr validates identifier and type-name usage without attempting
    // to compute final expression types or overload picks.
    fn resolve_expr(&mut self, expr: &Expr) {
        match expr {
            Expr::Identifier { name, span } => {
                if !self.is_name_defined(name) {
                    self.add_error("undefined_name", self.undefined_value_message(name), *span);
                }
            }
            Expr::Placeholder { .. }
            | Expr::Integer { .. }
            | Expr::Float { .. }
            | Expr::String { .. }
            | Expr::Bool { .. }
            | Expr::Unit { .. } => {}
            Expr::ListLiteral { items, .. } | Expr::TupleLiteral { items, .. } => {
                for item in items {
                    self.resolve_expr(item);
                }
            }
            Expr::Call { callee, args, .. } => {
                let skip_init = matches!(
                    callee.as_ref(),
                    Expr::Identifier { name, .. } if self.current_constructor && name == "new"
                );
                if !skip_init && !self.is_implicit_method_call(callee) {
                    self.resolve_expr(callee);
                }
                for arg in args {
                    self.resolve_expr(&arg.value);
                }
            }
            Expr::Member {
                receiver,
                name,
                span,
            } => {
                if let Some(result) = self.resolve_module_member_chain(expr) {
                    if let Err(message) = result {
                        self.add_error(
                            "unknown_member",
                            format!("unknown imported member '{}' ({})", name, message),
                            *span,
                        );
                    }
                    return;
                }
                self.resolve_expr(receiver);
            }
            Expr::Index {
                receiver, index, ..
            } => {
                self.resolve_expr(receiver);
                self.resolve_expr(index);
            }
            Expr::RecordUpdate {
                receiver, updates, ..
            } => {
                self.resolve_expr(receiver);
                for update in updates {
                    self.resolve_expr(&update.value);
                }
            }
            Expr::RecordLiteral { fields, values, .. } => {
                let mut seen = HashMap::new();
                for field in fields {
                    if let Some(name) = &field.name {
                        if let Some(previous) = seen.get(name) {
                            self.add_duplicate(
                                "duplicate_record_field",
                                format!("duplicate record field '{}'", name),
                                field.span,
                                *previous,
                            );
                        } else {
                            seen.insert(name.clone(), field.span);
                        }
                    }
                    self.resolve_expr(&field.value);
                }
                for value in values {
                    self.resolve_expr(value);
                }
            }
            Expr::AnonymousInterface {
                interfaces,
                methods,
                ..
            } => {
                for interface in interfaces {
                    self.resolve_type_ref(Some(interface));
                }
                for method in methods {
                    self.resolve_method(method);
                }
            }
            Expr::Try { value, .. } => self.resolve_expr(value),
            Expr::Unary { expr, .. } => self.resolve_expr(expr),
            Expr::Binary { left, right, .. } => {
                self.resolve_expr(left);
                self.resolve_expr(right);
            }
            Expr::Is { left, target, .. } => {
                self.resolve_expr(left);
                self.resolve_type_ref(Some(target));
            }
            Expr::If {
                condition,
                then_block,
                else_branch,
                ..
            } => {
                self.resolve_expr(condition);
                self.resolve_block(then_block);
                self.resolve_else_expr_branch(else_branch);
            }
            Expr::Block { body, .. } => self.resolve_block(body),
            Expr::Match { value, cases, .. } => {
                self.resolve_expr(value);
                for case in cases {
                    self.resolve_match_case(case);
                }
            }
            Expr::ForYield {
                bindings,
                yield_body,
                ..
            } => {
                self.push_scope();
                for binding in bindings {
                    self.resolve_for_binding(binding);
                }
                self.loop_depth += 1;
                self.resolve_block(yield_body);
                self.loop_depth -= 1;
                self.pop_scope();
            }
            Expr::Lambda { params, body, .. } => {
                self.push_scope();
                for param in params {
                    self.resolve_type_ref(param.ty.as_ref());
                    self.define_value(
                        param.name.as_str(),
                        param.span,
                        false,
                        "duplicate_parameter",
                        format!("duplicate parameter '{}'", param.name),
                        false,
                    );
                }
                match body {
                    LambdaBody::Expr(expr) => self.resolve_expr(expr),
                    LambdaBody::Block(block) => self.resolve_block(block),
                }
                self.pop_scope();
            }
            Expr::Group { inner, .. } => self.resolve_expr(inner),
        }
    }

    fn resolve_else_expr_branch(&mut self, branch: &ElseExprBranch) {
        match branch {
            ElseExprBranch::If(expr) => self.resolve_expr(expr),
            ElseExprBranch::Block(block) => self.resolve_block(block),
        }
    }

    fn resolve_pattern(&mut self, pattern: &Pattern) {
        match pattern {
            Pattern::Wildcard { .. } => {}
            Pattern::Extract { inner, .. } => self.resolve_pattern(inner),
            Pattern::Binding { name, span } => {
                if name != "_" {
                    self.define_value(
                        name,
                        *span,
                        false,
                        "duplicate_binding",
                        format!("duplicate binding '{}'", name),
                        false,
                    );
                }
            }
            Pattern::Type { name, target, span } => {
                self.resolve_type_pattern_ref(target);
                if let Some(name) = name {
                    if name != "_" {
                        self.define_value(
                            name,
                            *span,
                            false,
                            "duplicate_binding",
                            format!("duplicate binding '{}'", name),
                            false,
                        );
                    }
                }
            }
            Pattern::Literal { value, .. } => self.resolve_expr(value),
            Pattern::Tuple { elements, .. } => {
                for element in elements {
                    self.resolve_pattern(element);
                }
            }
            Pattern::Constructor { path, args, span } => {
                self.resolve_pattern_path(path, *span);
                for arg in args {
                    self.resolve_pattern(arg);
                }
            }
        }
    }

    fn resolve_pattern_path(&mut self, path: &[String], span: crate::source::Span) {
        if path.is_empty() {
            return;
        }
        let base = &path[0];
        if self.modules_by_alias.contains_key(base) {
            if let Some(message) = self.validate_module_segments(path) {
                self.add_error("unknown_member", message, span);
            }
            return;
        }
        if !self.is_name_defined(base) {
            self.add_error("undefined_name", format!("undefined name '{}'", base), span);
        }
    }

    // resolve_type_ref validates structural type syntax and generic arity, but
    // intentionally stops short of producing any typed representation.
    fn resolve_type_ref(&mut self, reference: Option<&TypeRef>) {
        let Some(reference) = reference else {
            return;
        };
        match reference {
            TypeRef::Function { params, ret, .. } => {
                for param in params {
                    self.resolve_type_ref(Some(param));
                }
                self.resolve_type_ref(Some(ret));
            }
            TypeRef::Tuple { fields, .. } => {
                for field in fields {
                    self.resolve_type_ref(Some(&field.ty));
                }
            }
            TypeRef::Record { fields, .. } => {
                let mut seen = HashMap::new();
                for RecordTypeField { name, ty, span } in fields {
                    if let Some(previous) = seen.get(name) {
                        self.add_duplicate(
                            "duplicate_record_field",
                            format!("duplicate record field '{}'", name),
                            *span,
                            *previous,
                        );
                    } else {
                        seen.insert(name.clone(), *span);
                    }
                    self.resolve_type_ref(Some(ty));
                }
            }
            TypeRef::Named { name, args, span } => {
                for arg in args {
                    self.resolve_type_ref(Some(arg));
                }
                if self.is_type_param(name) {
                    if !args.is_empty() {
                        self.add_error(
                            "invalid_type_arity",
                            format!("type parameter '{}' cannot have type arguments", name),
                            *span,
                        );
                    }
                    return;
                }
                if let Some(arity) = builtin_type_arity(name) {
                    if args.len() != arity {
                        self.add_error(
                            "invalid_type_arity",
                            format!(
                                "type '{}' expects {} type arguments",
                                name,
                                arity_label(arity)
                            ),
                            *span,
                        );
                    }
                    return;
                }
                if let Some(info) = self.lookup_type(name) {
                    if args.len() != info.arity {
                        self.add_error(
                            "invalid_type_arity",
                            format!(
                                "type '{}' expects {} type arguments",
                                name,
                                arity_label(info.arity)
                            ),
                            *span,
                        );
                    }
                    return;
                }
                self.add_error(
                    "undefined_type",
                    format!("undefined type '{}'", name),
                    *span,
                );
            }
        }
    }

    fn resolve_type_parameter_bounds(&mut self, params: &[TypeParam]) {
        for param in params {
            for bound in &param.bounds {
                self.resolve_type_ref(Some(bound));
            }
        }
    }

    fn resolve_type_pattern_ref(&mut self, reference: &TypeRef) {
        match reference {
            TypeRef::Function { .. } | TypeRef::Tuple { .. } | TypeRef::Record { .. } => {
                self.resolve_type_ref(Some(reference));
            }
            TypeRef::Named { name, args, span } => {
                for arg in args {
                    self.resolve_type_ref(Some(arg));
                }
                if self.type_pattern_uses_erased_generic(name) {
                    if !args.is_empty() {
                        self.add_error(
                            "invalid_match_pattern",
                            "runtime type patterns cannot specify generic arguments; use the erased outer type",
                            *span,
                        );
                    }
                    return;
                }
                self.resolve_type_ref(Some(reference));
            }
        }
    }

    fn type_pattern_uses_erased_generic(&self, name: &str) -> bool {
        if let Some(arity) = builtin_type_arity(name) {
            return arity > 0;
        }
        self.lookup_type(name).is_some_and(|info| info.arity > 0)
    }

    fn define_binding(&mut self, binding: &Binding, code: &'static str) {
        self.define_value(
            binding.name.as_str(),
            binding.span,
            binding.mutable,
            code,
            format!("duplicate binding '{}'", binding.name),
            false,
        );
    }

    fn define_type_param(&mut self, param: &TypeParam) {
        let previous = self
            .type_scopes
            .last()
            .and_then(|scope| scope.get(&param.name))
            .copied();
        if let Some(previous) = previous {
            self.add_duplicate(
                "duplicate_type_parameter",
                format!("duplicate type parameter '{}'", param.name),
                param.span,
                previous,
            );
            return;
        }
        self.current_type_scope()
            .insert(param.name.clone(), param.span);
    }

    // define_value centralizes duplicate and outer-shadow checks so the many
    // binding forms in the language behave consistently.
    fn define_value(
        &mut self,
        name: &str,
        span: crate::source::Span,
        mutable: bool,
        code: &'static str,
        message: String,
        allow_outer_shadow: bool,
    ) {
        if name == "_" {
            return;
        }
        let current_previous = self
            .scopes
            .last()
            .and_then(|scope| scope.get(name))
            .copied();
        if let Some(previous) = current_previous {
            if code == "duplicate_binding" {
                self.add_duplicate(
                    "shadowing_binding",
                    format!(
                        "binding '{}' shadows an existing variable; use a different name",
                        name
                    ),
                    span,
                    previous.span,
                );
            } else {
                self.add_duplicate(code, message, span, previous.span);
            }
            return;
        }
        if !allow_outer_shadow {
            if let Some(previous) = self.lookup_outer(name) {
                self.add_duplicate(
                    "shadowing_binding",
                    format!(
                        "binding '{}' shadows an existing variable; use a different name",
                        name
                    ),
                    span,
                    previous.span,
                );
                return;
            }
        }
        self.current_scope()
            .insert(name.to_string(), Symbol { span, mutable });
    }

    fn lookup_value(&self, name: &str) -> Option<Symbol> {
        for scope in self.scopes.iter().rev() {
            if let Some(symbol) = scope.get(name) {
                return Some(*symbol);
            }
        }
        self.globals
            .get(name)
            .copied()
            .or_else(|| self.imported_values.get(name).copied())
    }

    fn lookup_outer(&self, name: &str) -> Option<Symbol> {
        if self.scopes.len() > 1 {
            for scope in self.scopes[..self.scopes.len() - 1].iter().rev() {
                if let Some(symbol) = scope.get(name) {
                    return Some(*symbol);
                }
            }
        }
        self.globals
            .get(name)
            .copied()
            .or_else(|| self.imported_values.get(name).copied())
    }

    fn is_name_defined(&self, name: &str) -> bool {
        self.lookup_value(name).is_some()
            || self.functions.contains_key(name)
            || self.types.contains_key(name)
            || self.objects.contains_key(name)
            || self.enum_case_values.contains_key(name)
            || self.imported_functions.contains_key(name)
            || self.imported_types.contains_key(name)
            || self.imported_objects.contains_key(name)
            || self.modules_by_alias.contains_key(name)
            || self.ambient.values.contains(name)
            || self.ambient.types.contains_key(name)
    }

    fn push_field_hints<'b>(&mut self, fields: impl Iterator<Item = &'b str>) {
        self.field_hint_scopes
            .push(fields.map(|name| name.to_string()).collect());
    }

    fn pop_field_hints(&mut self) {
        self.field_hint_scopes.pop();
    }

    fn push_method_hints<'b>(&mut self, methods: impl Iterator<Item = &'b str>) {
        self.method_hint_scopes
            .push(methods.map(|name| name.to_string()).collect());
    }

    fn pop_method_hints(&mut self) {
        self.method_hint_scopes.pop();
    }

    fn is_implicit_method_call(&self, expr: &Expr) -> bool {
        let Expr::Identifier { name, .. } = expr else {
            return false;
        };
        !self.is_name_defined(name)
            && self
                .method_hint_scopes
                .iter()
                .rev()
                .any(|scope| scope.contains(name))
    }

    fn undefined_value_message(&self, name: &str) -> String {
        if self
            .field_hint_scopes
            .iter()
            .rev()
            .any(|scope| scope.contains(name))
        {
            format!(
                "undefined name '{}'; if you meant the field, write 'this.{}'",
                name, name
            )
        } else {
            format!("undefined name '{}'", name)
        }
    }

    fn lookup_type(&self, name: &str) -> Option<&TypeInfo> {
        self.types
            .get(name)
            .or_else(|| self.imported_types.get(name))
            .or_else(|| self.ambient.types.get(name))
    }

    fn is_type_param(&self, name: &str) -> bool {
        self.type_scopes
            .iter()
            .rev()
            .any(|scope| scope.contains_key(name))
    }

    fn current_scope(&mut self) -> &mut HashMap<String, Symbol> {
        if self.scopes.is_empty() {
            self.push_scope();
        }
        self.scopes.last_mut().expect("scope")
    }

    fn current_type_scope(&mut self) -> &mut HashMap<String, crate::source::Span> {
        if self.type_scopes.is_empty() {
            self.push_type_scope();
        }
        self.type_scopes.last_mut().expect("type scope")
    }

    fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    fn push_type_scope(&mut self) {
        self.type_scopes.push(HashMap::new());
    }

    fn pop_type_scope(&mut self) {
        self.type_scopes.pop();
    }

    fn add_error(
        &mut self,
        code: &'static str,
        message: impl Into<String>,
        span: crate::source::Span,
    ) {
        self.diagnostics
            .push(Diagnostic::error(code, message, span));
    }

    fn add_duplicate(
        &mut self,
        code: &'static str,
        message: impl Into<String>,
        span: crate::source::Span,
        previous: crate::source::Span,
    ) {
        self.diagnostics
            .push(Diagnostic::error(code, message, span));
        self.diagnostics.push(Diagnostic::error(
            code,
            "previous declaration here",
            previous,
        ));
    }

    fn resolve_module_member_chain(&mut self, expr: &Expr) -> Option<Result<(), String>> {
        let segments = member_segments(expr)?;
        let first = segments.first()?;
        self.modules_by_alias
            .contains_key(first)
            .then(|| self.validate_module_segments(&segments).map_or(Ok(()), Err))
    }

    fn validate_module_segments(&self, segments: &[String]) -> Option<String> {
        let namespace = self.modules_by_alias.get(segments.first()?)?;
        match segments {
            [module, member] => {
                if namespace.functions.contains_key(member)
                    || namespace.globals.contains_key(member)
                    || namespace.types.contains_key(member)
                    || namespace.objects.contains_key(member)
                {
                    None
                } else {
                    Some(format!(
                        "module '{}' has no visible member '{}'",
                        module, member
                    ))
                }
            }
            [module, object_name, member] => {
                let object = namespace.objects.get(object_name)?;
                if object.methods.contains_key(member) {
                    None
                } else {
                    Some(format!(
                        "object '{}.{}' has no visible member '{}'",
                        module, object_name, member
                    ))
                }
            }
            _ => Some(format!(
                "module '{}' access is only supported for direct members and object methods",
                segments.first().unwrap_or(&"<unknown>".to_string())
            )),
        }
    }
}

fn member_segments(expr: &Expr) -> Option<Vec<String>> {
    match expr {
        Expr::Identifier { name, .. } => Some(vec![name.clone()]),
        Expr::Member { receiver, name, .. } => {
            let mut parts = member_segments(receiver)?;
            parts.push(name.clone());
            Some(parts)
        }
        _ => None,
    }
}

fn type_ref_name(reference: &TypeRef) -> Option<&str> {
    match reference {
        TypeRef::Named { name, .. } => Some(name.as_str()),
        _ => None,
    }
}

fn builtin_type_arity(name: &str) -> Option<usize> {
    match name {
        "Int" | "Int64" | "Bool" | "Rune" | "Float" | "Float64" | "Str" | "Unit" | "Never" => {
            Some(0)
        }
        _ => None,
    }
}

fn arity_label(arity: usize) -> &'static str {
    match arity {
        0 => "0",
        1 => "1",
        2 => "2",
        _ => "multiple",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::SourceFile;
    use std::path::{Path, PathBuf};

    fn workspace_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../..")
            .canonicalize()
            .expect("workspace root")
    }

    fn parse_inline(src: &str) -> Program {
        let file = SourceFile::new("test.lum", src);
        let lexed = lex(&file);
        assert!(lexed.diagnostics.is_empty(), "{:#?}", lexed.diagnostics);
        let parsed = parse_program(&lexed.tokens);
        assert!(parsed.diagnostics.is_empty(), "{:#?}", parsed.diagnostics);
        parsed.program.expect("program")
    }

    #[test]
    fn reports_undefined_local_name() {
        let program = parse_inline(
            r#"
def main() Int {
    value = missing + 1
    return 0
}
"#,
        );
        let result = resolve_program(&program);
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diag| diag.code == "undefined_name" && diag.message.contains("missing")),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn resolves_import_forms_example() {
        let result =
            resolve_path(workspace_root().join("examples/import_forms.lum")).expect("resolve");
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn resolves_selected_repo_sources() {
        let root = workspace_root();
        let files = [
            root.join("stdlib/option.lum"),
            root.join("stdlib/range.lum"),
            root.join("examples/import_forms.lum"),
            root.join("examples/classes.lum"),
            root.join("examples/controll_flow/match_enums.lum"),
            root.join("examples/random_code/bumper.lum"),
        ];
        let mut failures = Vec::new();
        for path in files {
            match resolve_path(&path) {
                Ok(result) if result.diagnostics.is_empty() => {}
                Ok(result) => failures.push(format!(
                    "resolve {}: {:#?}",
                    path.strip_prefix(&root).unwrap_or(&path).display(),
                    result.diagnostics
                )),
                Err(err) => failures.push(format!(
                    "load {}: {}",
                    path.strip_prefix(&root).unwrap_or(&path).display(),
                    err
                )),
            }
        }

        assert!(
            failures.is_empty(),
            "repo resolve failures:\n{}",
            failures.join("\n\n")
        );
    }

    #[test]
    fn formats_path_diagnostics_one_per_line() {
        let diagnostics = vec![
            Diagnostic::error(
                "first",
                "one",
                crate::source::Span::new(
                    0,
                    1,
                    crate::source::LineColumn::new(2, 3),
                    crate::source::LineColumn::new(2, 4),
                ),
            ),
            Diagnostic::error(
                "second",
                "two",
                crate::source::Span::new(
                    1,
                    2,
                    crate::source::LineColumn::new(4, 5),
                    crate::source::LineColumn::new(4, 6),
                ),
            ),
        ];

        let rendered = format_path_diagnostics(
            Path::new("/tmp/test.lum"),
            Some("abc\n12345\n\n123456\n"),
            &diagnostics,
        );
        assert_eq!(
            rendered,
            "error[first]: one\n  --> /tmp/test.lum:2:3\n  |\n2 | 12345\n  |   ^\nerror[second]: two\n  --> /tmp/test.lum:4:5\n  |\n4 | 123456\n  |     ^"
        );
    }
}
