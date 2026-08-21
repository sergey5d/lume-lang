use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
};

use crate::{
    Diagnostic,
    ast::{
        AssignOp, AssignmentStmt, BinaryOp, BindingStmt, Block, CallableBody, DestructureKind,
        ElseBranch, ElseExprBranch, Expr, ExtensionBlock, FieldDecl, ForBinding, FunctionDecl,
        GenericCondition, IfConditionClause, IfStmt, Item, LambdaBody, MatchCase, MatchCaseBody,
        MatchStmt, MethodDecl, Param, Pattern, PatternBindingStmt, Program, Stmt, TypeDecl,
        TypeKind, TypeMember, TypeParam, TypeRef, Visibility,
    },
    resolver::{
        ImportedKind, ImportedSymbol, LoadedModule, ModuleGraph, ModuleLoadOptions,
        collect_module_order, find_stdlib_dir, load_module_graph_with_options,
        parse_program_from_path, read_directives, resolve_path_with_options,
    },
    typecheck_diagnostics,
};

#[derive(Debug, Clone)]
pub struct CheckResult {
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, Default)]
pub struct PathCheckResult {
    pub diagnostics: Vec<crate::resolver::LocatedDiagnostic>,
}

pub fn check_program(program: &Program) -> CheckResult {
    let world = World::from_program(program);
    let Some(module) = world.root_module.as_ref() else {
        return CheckResult {
            diagnostics: Vec::new(),
        };
    };
    let mut checker = Checker::new(&world, module);
    checker.check_module();
    CheckResult {
        diagnostics: checker.diagnostics,
    }
}

pub fn check_path(path: impl AsRef<Path>) -> Result<PathCheckResult, String> {
    check_path_with_load_options(path, &ModuleLoadOptions::default())
}

pub(crate) fn check_path_with_load_options(
    path: impl AsRef<Path>,
    options: &ModuleLoadOptions,
) -> Result<PathCheckResult, String> {
    let resolved = resolve_path_with_options(path.as_ref(), options)?;
    if !resolved.diagnostics.is_empty() {
        return Ok(PathCheckResult {
            diagnostics: resolved.diagnostics,
        });
    }

    let (graph, root_path) = load_module_graph_with_options(path.as_ref(), options)?;
    let stdlib_dir = find_stdlib_dir(path.as_ref().parent().unwrap_or_else(|| Path::new(".")))?;
    let mut world = World::from_graph(graph, root_path, stdlib_dir)?;

    let mut diagnostics = Vec::new();
    for module_path in world.order.clone() {
        let Some(module) = world.modules.get(&module_path) else {
            continue;
        };
        let display_path = module.display_path.clone();
        let (module_diagnostics, checked_globals) = {
            let mut checker = Checker::new(&world, module);
            checker.check_module();
            (checker.diagnostics, checker.globals.clone())
        };
        world
            .checked_globals
            .insert(module_path.clone(), checked_globals);
        diagnostics.extend(module_diagnostics.into_iter().map(|diagnostic| {
            crate::resolver::LocatedDiagnostic {
                path: display_path.clone(),
                diagnostic,
            }
        }));
    }

    Ok(PathCheckResult { diagnostics })
}

fn default_stdlib_dir() -> Option<PathBuf> {
    find_stdlib_dir(Path::new(env!("CARGO_MANIFEST_DIR")))
        .ok()
        .or_else(|| find_stdlib_dir(Path::new(".")).ok())
}

fn default_inline_ambient() -> AmbientInfo {
    default_stdlib_dir()
        .and_then(|stdlib_dir| AmbientInfo::load(&stdlib_dir).ok())
        .unwrap_or_default()
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Ty {
    Unknown,
    Wildcard,
    Capture(usize),
    Never,
    Named(String, Vec<Ty>),
    Tuple(Vec<Ty>),
    Record(Vec<(String, Ty)>),
    Function(Vec<Ty>, Box<Ty>),
    TypeParam(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ForYieldFamily {
    Iterable,
    Option,
    Result { error: Ty },
    Either { left: Ty },
    Unknown,
}

#[derive(Debug, Clone)]
struct ShapeFieldProvider {
    source: String,
    conflicting_source: Option<String>,
    span: crate::source::Span,
}

fn describe_for_yield_family(family: &ForYieldFamily) -> &'static str {
    match family {
        ForYieldFamily::Iterable => "iterable",
        ForYieldFamily::Option => "Option",
        ForYieldFamily::Result { .. } => "Result",
        ForYieldFamily::Either { .. } => "Either",
        ForYieldFamily::Unknown => "unknown",
    }
}

impl Ty {
    fn named(name: impl Into<String>) -> Self {
        Self::Named(name.into(), Vec::new())
    }

    fn never() -> Self {
        Self::Never
    }

    fn list(item: Ty) -> Self {
        Self::Named("Vector".to_string(), vec![item])
    }

    fn option(item: Ty) -> Self {
        Self::Named("Option".to_string(), vec![item])
    }

    fn exact_runtime_type(represented: Ty) -> Self {
        Self::Named("Type".to_string(), vec![runtime_type_arg(represented)])
    }

    fn value_runtime_type(represented: Ty) -> Self {
        Self::Named(
            "Type".to_string(),
            vec![runtime_value_type_arg(represented)],
        )
    }

    fn any() -> Self {
        Self::named("Any")
    }

    fn bool() -> Self {
        Self::named("Bool")
    }

    fn int() -> Self {
        Self::named("Int")
    }

    fn float() -> Self {
        Self::named("Float")
    }

    fn str() -> Self {
        Self::named("Str")
    }

    fn rune() -> Self {
        Self::named("Rune")
    }

    fn unit() -> Self {
        Self::named("Unit")
    }

    fn describe(&self) -> String {
        match self {
            Ty::Unknown => "<unknown>".to_string(),
            Ty::Wildcard => "_".to_string(),
            Ty::Capture(_) => "_".to_string(),
            Ty::Never => "Never".to_string(),
            Ty::Named(name, args) if args.is_empty() => name.clone(),
            Ty::Named(name, args) => format!(
                "{}[{}]",
                name,
                args.iter().map(Ty::describe).collect::<Vec<_>>().join(", ")
            ),
            Ty::Tuple(items) => format!(
                "({})",
                items
                    .iter()
                    .map(Ty::describe)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Ty::Record(fields) => format!(
                "{{{}}}",
                fields
                    .iter()
                    .map(|(name, ty)| format!("{name} {}", ty.describe()))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Ty::Function(params, ret) => format!(
                "fn({}) {}",
                params
                    .iter()
                    .map(Ty::describe)
                    .collect::<Vec<_>>()
                    .join(", "),
                ret.describe()
            ),
            Ty::TypeParam(name) => name.clone(),
        }
    }

    fn is_bool(&self) -> bool {
        matches!(self, Ty::Named(name, args) if name == "Bool" && args.is_empty())
    }

    fn is_any(&self) -> bool {
        matches!(self, Ty::Named(name, args) if name == "Any" && args.is_empty())
    }

    fn is_str(&self) -> bool {
        matches!(self, Ty::Named(name, args) if name == "Str" && args.is_empty())
    }

    fn is_int_like(&self) -> bool {
        matches!(self, Ty::Named(name, args) if args.is_empty() && name == "Int")
    }

    fn is_float_like(&self) -> bool {
        matches!(self, Ty::Named(name, args) if args.is_empty() && name == "Float")
    }

    fn is_numeric(&self) -> bool {
        self.is_int_like() || self.is_float_like()
    }
}

#[derive(Debug, Clone)]
struct ValueInfo {
    ty: Ty,
    mutable: bool,
    stable: bool,
}

#[derive(Debug, Clone)]
struct TypeNarrowing {
    name: String,
    ty: Ty,
}

#[derive(Debug, Clone)]
struct ParamSig {
    name: String,
    ty: Ty,
    variadic: bool,
    lazy: bool,
    has_initializer: bool,
}

#[derive(Debug, Clone)]
struct FunctionSig {
    type_params: Vec<String>,
    reified_type_params: Vec<String>,
    generic_conditions: Vec<GenericConditionSig>,
    params: Vec<ParamSig>,
    ret: Ty,
    visibility: Visibility,
    has_body: bool,
}

#[derive(Debug, Clone)]
enum GenericConditionSig {
    Bound { subject: Ty, bound: Ty },
    Equal { left: Ty, right: Ty },
}

#[derive(Debug, Clone)]
struct CallableSelection {
    sig: FunctionSig,
    explicit_type_args: Vec<Ty>,
}

#[derive(Debug, Clone)]
struct ConstructorCycleNode {
    method: MethodDecl,
    sig: FunctionSig,
}

#[derive(Debug, Clone, Default)]
struct TypeParamScope {
    names: HashSet<String>,
    reified: HashSet<String>,
    conditions: Vec<GenericConditionSig>,
}

fn universal_member_type(name: &str) -> Option<Ty> {
    universal_method_sigs(name).and_then(|methods| {
        let first = methods.into_iter().next()?;
        Some(Ty::Function(
            first.params.into_iter().map(|param| param.ty).collect(),
            Box::new(first.ret),
        ))
    })
}

fn hash_method_sig() -> FunctionSig {
    FunctionSig {
        type_params: Vec::new(),
        reified_type_params: Vec::new(),
        generic_conditions: Vec::new(),
        params: Vec::new(),
        ret: Ty::int(),
        visibility: Visibility::Default,
        has_body: true,
    }
}

fn universal_method_sigs(name: &str) -> Option<Vec<FunctionSig>> {
    let sig = match name {
        "toStr" => FunctionSig {
            type_params: Vec::new(),
            reified_type_params: Vec::new(),
            generic_conditions: Vec::new(),
            params: Vec::new(),
            ret: Ty::str(),
            visibility: Visibility::Default,
            has_body: true,
        },
        "equals" => FunctionSig {
            type_params: Vec::new(),
            reified_type_params: Vec::new(),
            generic_conditions: Vec::new(),
            params: vec![ParamSig {
                name: "other".to_string(),
                ty: Ty::any(),
                variadic: false,
                lazy: false,
                has_initializer: false,
            }],
            ret: Ty::bool(),
            visibility: Visibility::Default,
            has_body: true,
        },
        "sameValue" => FunctionSig {
            type_params: Vec::new(),
            reified_type_params: Vec::new(),
            generic_conditions: Vec::new(),
            params: vec![ParamSig {
                name: "other".to_string(),
                ty: Ty::any(),
                variadic: false,
                lazy: false,
                has_initializer: false,
            }],
            ret: Ty::bool(),
            visibility: Visibility::Default,
            has_body: true,
        },
        _ => return None,
    };
    Some(vec![sig])
}

#[derive(Debug, Clone)]
struct FieldSig {
    name: String,
    ty: Ty,
    mutable: bool,
    hidden: bool,
    has_initializer: bool,
    variadic: bool,
}

#[derive(Debug, Clone)]
struct EnumCaseSig {
    params: Vec<FieldSig>,
    field_count: usize,
    result: Ty,
}

#[derive(Debug, Clone)]
struct TypeSig {
    kind: TypeKind,
    name: String,
    type_params: Vec<String>,
    generic_conditions: Vec<GenericConditionSig>,
    with_bounds: Vec<Ty>,
    fields: Vec<FieldSig>,
    methods: HashMap<String, Vec<FunctionSig>>,
    enum_cases: HashMap<String, EnumCaseSig>,
}

#[derive(Debug, Clone, Default)]
struct ModuleInfo {
    path: PathBuf,
    display_path: String,
    program: Program,
    imports: HashMap<String, PathBuf>,
    symbol_imports: HashMap<String, ImportedSymbol>,
    extension_imports: Vec<PathBuf>,
    functions: HashMap<String, Vec<FunctionSig>>,
    types: HashMap<String, TypeSig>,
    objects: HashMap<String, TypeSig>,
    extensions: HashMap<String, HashMap<String, Vec<FunctionSig>>>,
    global_binding_stmts: Vec<crate::ast::BindingStmt>,
}

impl ModuleInfo {
    fn from_loaded(module: &LoadedModule) -> Self {
        let mut info = Self {
            path: module.path.clone(),
            display_path: module.display_path.clone(),
            program: module.program.clone(),
            imports: module.imports.clone(),
            symbol_imports: module.symbol_imports.clone(),
            extension_imports: module.extension_imports.clone(),
            functions: HashMap::new(),
            types: HashMap::new(),
            objects: HashMap::new(),
            extensions: HashMap::new(),
            global_binding_stmts: Vec::new(),
        };
        info.collect_items();
        info
    }

    fn from_program(display_path: &str, program: &Program) -> Self {
        let mut info = Self {
            path: PathBuf::from(display_path),
            display_path: display_path.to_string(),
            program: program.clone(),
            imports: HashMap::new(),
            symbol_imports: HashMap::new(),
            extension_imports: Vec::new(),
            functions: HashMap::new(),
            types: HashMap::new(),
            objects: HashMap::new(),
            extensions: HashMap::new(),
            global_binding_stmts: Vec::new(),
        };
        info.collect_items();
        info
    }

    fn collect_items(&mut self) {
        let items = self.program.items.clone();
        for item in &items {
            match item {
                Item::Function(function) => {
                    self.functions
                        .entry(function.name.clone())
                        .or_default()
                        .push(function_sig_from_function(function, &[]));
                }
                Item::Type(decl) => {
                    let sig = type_sig_from_decl(decl);
                    if decl.kind == TypeKind::Object {
                        self.objects.insert(decl.name.clone(), sig);
                    } else {
                        self.types.insert(decl.name.clone(), sig);
                    }
                }
                Item::Statement(Stmt::Binding(binding)) => {
                    self.global_binding_stmts.push(binding.clone());
                }
                Item::Extension(block) => {
                    self.collect_extension(block);
                }
                _ => {}
            }
        }
    }

    fn collect_extension(&mut self, block: &ExtensionBlock) {
        let Some(target_name) = type_ref_named_name(&block.target) else {
            return;
        };
        let target_type_params = impl_target_type_params(&block.target);
        let methods = self.extensions.entry(target_name.to_string()).or_default();
        for method in &block.methods {
            methods
                .entry(method.name.clone())
                .or_default()
                .push(function_sig_from_method(method, &target_type_params));
        }
    }
}

#[derive(Debug, Clone, Default)]
struct AmbientInfo {
    functions: HashMap<String, Vec<FunctionSig>>,
    types: HashMap<String, TypeSig>,
    objects: HashMap<String, TypeSig>,
    enum_cases: HashMap<String, EnumCaseSig>,
    extensions: HashMap<String, HashMap<String, Vec<FunctionSig>>>,
}

impl AmbientInfo {
    fn load(stdlib_dir: &Path) -> Result<Self, String> {
        let mut entries = fs::read_dir(stdlib_dir)
            .map_err(|err| format!("read stdlib {}: {err}", stdlib_dir.display()))?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "lum"))
            .collect::<Vec<_>>();
        entries.sort();

        let mut ambient = AmbientInfo::default();
        for path in entries {
            let directives = read_directives(&path)?;
            if !directives.interpreter {
                continue;
            }
            let program = parse_program_from_path(&path)?;
            let module = ModuleInfo::from_program(&path.display().to_string(), &program);
            ambient.functions.extend(module.functions.clone());
            for (target, methods) in module.extensions.clone() {
                let target_methods = ambient.extensions.entry(target).or_default();
                for (name, overloads) in methods {
                    target_methods.entry(name).or_default().extend(overloads);
                }
            }
            for (name, sig) in module.types {
                for (case_name, case_sig) in &sig.enum_cases {
                    ambient
                        .enum_cases
                        .insert(case_name.clone(), case_sig.clone());
                }
                ambient.types.insert(name, sig);
            }
            for (name, sig) in module.objects {
                ambient.objects.insert(name, sig);
            }
        }

        if let Some(os) = ambient.objects.get("OS") {
            for builtin in ["print", "println", "printf"] {
                if let Some(sigs) = os.methods.get(builtin) {
                    ambient.functions.insert(builtin.to_string(), sigs.clone());
                }
            }
        }

        Ok(ambient)
    }
}

fn type_kind_label(kind: TypeKind) -> &'static str {
    match kind {
        TypeKind::Annotation => "annotation",
        TypeKind::Class => "class",
        TypeKind::Record => "shape",
        TypeKind::Object => "object",
        TypeKind::Interface => "interface",
        TypeKind::Enum => "enum",
    }
}

fn custom_constructor_error(sig: &TypeSig) -> String {
    match sig.kind {
        TypeKind::Annotation => format!(
            "only classes can declare custom constructors; annotation '{}' cannot declare constructors",
            sig.name
        ),
        TypeKind::Record => format!(
            "only classes can declare custom constructors; shape '{}' uses structural brace construction",
            sig.name
        ),
        TypeKind::Enum => format!(
            "only classes can declare custom constructors; enum '{}' uses enum cases for construction",
            sig.name
        ),
        TypeKind::Interface => format!(
            "only classes can declare custom constructors; interface '{}' cannot declare constructors",
            sig.name
        ),
        TypeKind::Object => format!(
            "only classes can declare custom constructors; object '{}' declares one object value",
            sig.name
        ),
        TypeKind::Class => format!("class '{}' can declare custom constructors", sig.name),
    }
}

fn builtin_extension_type_sig(name: &str) -> Option<TypeSig> {
    if !matches!(name, "Bool" | "Float" | "Int" | "Rune" | "Str") {
        return None;
    }
    Some(TypeSig {
        kind: TypeKind::Class,
        name: name.to_string(),
        type_params: Vec::new(),
        generic_conditions: Vec::new(),
        with_bounds: Vec::new(),
        fields: Vec::new(),
        methods: HashMap::new(),
        enum_cases: HashMap::new(),
    })
}

#[derive(Debug, Clone, Default)]
struct World {
    modules: HashMap<PathBuf, ModuleInfo>,
    order: Vec<PathBuf>,
    ambient: AmbientInfo,
    checked_globals: HashMap<PathBuf, HashMap<String, ValueInfo>>,
    root_module: Option<ModuleInfo>,
}

impl World {
    fn from_program(program: &Program) -> Self {
        Self {
            ambient: default_inline_ambient(),
            root_module: Some(ModuleInfo::from_program("<memory>", program)),
            ..Self::default()
        }
    }

    fn from_graph(graph: ModuleGraph, root: PathBuf, stdlib_dir: PathBuf) -> Result<Self, String> {
        let ambient = AmbientInfo::load(&stdlib_dir)?;
        let mut modules = HashMap::new();
        for (path, loaded) in graph.modules {
            modules.insert(path, ModuleInfo::from_loaded(&loaded));
        }
        let mut visited = HashSet::new();
        let mut order = Vec::new();
        collect_module_order(
            &ModuleGraph {
                modules: modules
                    .iter()
                    .map(|(path, module)| {
                        (
                            path.clone(),
                            LoadedModule {
                                path: module.path.clone(),
                                display_path: module.display_path.clone(),
                                program: module.program.clone(),
                                source: crate::resolver::ModuleSource::Source,
                                typecheck_only_types: HashSet::new(),
                                imports: module.imports.clone(),
                                symbol_imports: module.symbol_imports.clone(),
                                extension_imports: module.extension_imports.clone(),
                                dependencies: Vec::new(),
                            },
                        )
                    })
                    .collect(),
            },
            &root,
            &mut visited,
            &mut order,
        );
        // The helper above depends on dependency edges on LoadedModule. Rebuild a
        // correct order directly from the already-loaded modules.
        if order.len() != modules.len() {
            order.clear();
            let mut seen = HashSet::new();
            visit_order(&modules, &root, &mut seen, &mut order);
        }
        Ok(Self {
            modules,
            order,
            ambient,
            checked_globals: HashMap::new(),
            root_module: None,
        })
    }

    fn lookup_module_alias<'a>(
        &'a self,
        module: &ModuleInfo,
        alias: &str,
    ) -> Option<&'a ModuleInfo> {
        let path = module.imports.get(alias)?;
        self.modules.get(path)
    }

    fn lookup_imported_function(
        &self,
        module: &ModuleInfo,
        name: &str,
    ) -> Option<Vec<FunctionSig>> {
        let imported = module.symbol_imports.get(name)?;
        if imported.kind != ImportedKind::Function {
            return None;
        }
        let source = self.modules.get(&imported.module_path)?;
        if let Some(object_name) = imported.object_name.as_deref() {
            let object = source.objects.get(object_name)?;
            return object.methods.get(&imported.original_name).cloned();
        }
        source.functions.get(&imported.original_name).cloned()
    }

    fn lookup_imported_type(&self, module: &ModuleInfo, name: &str) -> Option<TypeSig> {
        let imported = module.symbol_imports.get(name)?;
        match imported.kind {
            ImportedKind::Type | ImportedKind::Interface => self
                .modules
                .get(&imported.module_path)?
                .types
                .get(&imported.original_name)
                .cloned(),
            ImportedKind::Object => self
                .modules
                .get(&imported.module_path)?
                .objects
                .get(&imported.original_name)
                .cloned(),
            _ => None,
        }
    }

    fn lookup_imported_global(&self, module: &ModuleInfo, name: &str) -> Option<ValueInfo> {
        let imported = module.symbol_imports.get(name)?;
        if imported.kind != ImportedKind::Value {
            return None;
        }
        self.checked_globals
            .get(&imported.module_path)?
            .get(&imported.original_name)
            .cloned()
    }

    fn lookup_enum_case(&self, module: &ModuleInfo, name: &str) -> Option<EnumCaseSig> {
        for sig in module.types.values() {
            if let Some(case) = sig.enum_cases.get(name) {
                return Some(case.clone());
            }
        }
        self.ambient.enum_cases.get(name).cloned()
    }

    fn extension_method_sigs(
        &self,
        module: &ModuleInfo,
        type_name: &str,
        method: &str,
    ) -> Vec<FunctionSig> {
        let mut out = Vec::new();
        if let Some(methods) = self
            .ambient
            .extensions
            .get(type_name)
            .and_then(|methods| methods.get(method))
        {
            out.extend(methods.clone());
        }
        if let Some(methods) = module
            .extensions
            .get(type_name)
            .and_then(|methods| methods.get(method))
        {
            out.extend(methods.clone());
        }
        for import_path in &module.extension_imports {
            let Some(imported) = self.modules.get(import_path) else {
                continue;
            };
            if let Some(methods) = imported
                .extensions
                .get(type_name)
                .and_then(|methods| methods.get(method))
            {
                out.extend(methods.clone());
            }
        }
        out
    }
}

fn visit_order(
    modules: &HashMap<PathBuf, ModuleInfo>,
    root: &Path,
    seen: &mut HashSet<PathBuf>,
    out: &mut Vec<PathBuf>,
) {
    let root_buf = root.to_path_buf();
    if !seen.insert(root_buf.clone()) {
        return;
    }
    let Some(module) = modules.get(&root_buf) else {
        return;
    };
    for dep in module.imports.values() {
        visit_order(modules, dep, seen, out);
    }
    for imported in module.symbol_imports.values() {
        visit_order(modules, &imported.module_path, seen, out);
    }
    out.push(root_buf);
}

struct Checker<'a> {
    world: &'a World,
    module: &'a ModuleInfo,
    diagnostics: Vec<Diagnostic>,
    scopes: Vec<HashMap<String, ValueInfo>>,
    type_params: Vec<TypeParamScope>,
    current_return: Ty,
    current_owner: Option<TypeSig>,
    current_method: Option<String>,
    current_extension_target: Option<String>,
    callable_depth: usize,
    loop_depth: usize,
    defer_depth: usize,
    next_capture_id: usize,
    capture_labels: HashMap<usize, String>,
    globals: HashMap<String, ValueInfo>,
    anonymous_types: HashMap<String, TypeSig>,
}

impl<'a> Checker<'a> {
    fn new(world: &'a World, module: &'a ModuleInfo) -> Self {
        Self {
            world,
            module,
            diagnostics: Vec::new(),
            scopes: Vec::new(),
            type_params: Vec::new(),
            current_return: Ty::Unknown,
            current_owner: None,
            current_method: None,
            current_extension_target: None,
            callable_depth: 0,
            loop_depth: 0,
            defer_depth: 0,
            next_capture_id: 0,
            capture_labels: HashMap::new(),
            globals: HashMap::new(),
            anonymous_types: HashMap::new(),
        }
    }

    fn check_module(&mut self) {
        self.check_global_bindings();
        for item in &self.module.program.items {
            match item {
                Item::Function(function) => self.check_function(function),
                Item::Type(decl) => self.check_type_decl(decl),
                Item::Extension(block) => self.check_extension(block),
                _ => {}
            }
        }
        self.check_constructor_delegation_cycles();
    }

    fn check_global_bindings(&mut self) {
        self.push_scope();
        for binding_stmt in &self.module.global_binding_stmts {
            for binding in &binding_stmt.bindings {
                if let Some(ty) = &binding.ty {
                    self.validate_type_ref_generic_applications(ty);
                }
            }
            let value_types = binding_stmt
                .values
                .iter()
                .enumerate()
                .map(|(index, expr)| {
                    if binding_stmt.bindings.len() == 1 {
                        if let Some(expected) = binding_stmt.bindings[0]
                            .ty
                            .as_ref()
                            .map(|ty| self.ty_from_type_ref(ty))
                        {
                            return self.check_expr_against(expr, &expected);
                        }
                    }
                    if let Some(expected) = binding_stmt
                        .bindings
                        .get(index)
                        .and_then(|binding| binding.ty.as_ref())
                        .map(|ty| self.ty_from_type_ref(ty))
                    {
                        self.check_expr_against(expr, &expected)
                    } else {
                        self.check_expr(expr)
                    }
                })
                .collect::<Vec<_>>();
            let slot_types = self.binding_slot_types(binding_stmt, &value_types);
            for (index, binding) in binding_stmt.bindings.iter().enumerate() {
                let explicit = binding.ty.as_ref().map(|ty| self.ty_from_type_ref(ty));
                let inferred = slot_types.get(index).cloned().unwrap_or(Ty::Unknown);
                let ty = explicit.clone().unwrap_or_else(|| inferred.clone());
                if let Some(expected) = explicit {
                    self.require_assignable(
                        &inferred,
                        &expected,
                        binding.span,
                        "invalid_binding_type",
                        format!(
                            "cannot assign value of type {} to binding '{}' of type {}",
                            self.diagnostic_type_phrase(&inferred),
                            binding.name,
                            self.diagnostic_type_phrase(&expected)
                        ),
                    );
                }
                let ty = self.capture_wildcards(ty);
                self.globals.insert(
                    binding.name.clone(),
                    ValueInfo {
                        ty,
                        mutable: binding.mutable,
                        stable: !binding.mutable,
                    },
                );
            }
        }
        self.pop_scope();
    }

    fn check_function(&mut self, function: &FunctionDecl) {
        let previous_return = self.current_return.clone();
        let previous_defer_depth = self.defer_depth;
        let previous_callable_depth = self.callable_depth;
        self.push_ast_type_params(&function.type_params, &function.type_conditions);
        self.validate_generic_clause(&function.type_params, &function.type_conditions);
        if let Some(return_type) = &function.return_type {
            self.validate_type_ref_generic_applications(return_type);
        }
        for param in &function.params {
            if let Some(ty) = &param.ty {
                self.validate_type_ref_generic_applications(ty);
            }
        }
        let expected_return = function
            .return_type
            .as_ref()
            .map(|ty| self.ty_from_type_ref(ty))
            .unwrap_or(Ty::Unknown);
        self.current_return = expected_return.clone();
        self.defer_depth = 0;
        self.callable_depth += 1;
        self.push_scope();
        self.check_param_list_rules(&function.params, false);
        for param in &function.params {
            let elem_ty = param
                .ty
                .as_ref()
                .map(|value| self.ty_from_type_ref(value))
                .unwrap_or(Ty::Unknown);
            if let Some(initializer) = &param.initializer {
                self.check_param_initializer(param, &elem_ty, initializer);
            }
            let ty = self.param_local_type(param, elem_ty);
            self.define_local(&param.name, ty, false);
            if param.lazy {
                self.mark_current_local_unstable(&param.name);
            }
        }
        let actual = self.check_callable_body(&function.body);
        self.require_assignable(
            &actual,
            &expected_return,
            function.span,
            "invalid_return_type",
            format!(
                "function '{}' returns '{}' but is declared as '{}'",
                function.name,
                actual.describe(),
                expected_return.describe()
            ),
        );
        self.pop_scope();
        self.pop_type_params();
        self.current_return = previous_return;
        self.defer_depth = previous_defer_depth;
        self.callable_depth = previous_callable_depth;
    }

    fn check_type_decl(&mut self, decl: &TypeDecl) {
        let Some(type_sig) = self.lookup_type_local(&decl.name) else {
            return;
        };
        for param in &decl.type_params {
            if param.reified {
                self.add_error(
                    "invalid_reified_type_parameter",
                    format!(
                        "{} '{}' cannot declare reified type parameter '{}'; use reified on generic functions or methods",
                        type_kind_label(decl.kind),
                        decl.name,
                        param.name
                    ),
                    param.span,
                );
            }
        }
        self.push_ast_type_params(&decl.type_params, &decl.type_conditions);
        self.validate_generic_clause(&decl.type_params, &decl.type_conditions);
        for member in &decl.members {
            match member {
                TypeMember::Field(field) => {
                    if let Some(ty) = &field.ty {
                        self.validate_type_ref_generic_applications(ty);
                    }
                }
                TypeMember::Case(case) => {
                    for field in &case.fields {
                        if let Some(ty) = &field.ty {
                            self.validate_type_ref_generic_applications(ty);
                        }
                    }
                }
                TypeMember::Method(_) => {}
            }
        }

        if decl.kind == TypeKind::Enum && type_sig.enum_cases.is_empty() {
            self.add_error(
                "empty_enum",
                format!("enum '{}' must declare at least one case", decl.name),
                decl.span,
            );
        }

        for member in &decl.members {
            match member {
                TypeMember::Field(field) => {
                    if decl.kind == TypeKind::Annotation {
                        if field.visibility == Visibility::Hidden {
                            self.add_error(
                                "invalid_annotation_field",
                                format!(
                                    "annotation '{}' cannot declare hidden field '{}'",
                                    decl.name, field.name
                                ),
                                field.span,
                            );
                        }
                        if field.mutable {
                            self.add_error(
                                "invalid_annotation_field",
                                format!(
                                    "annotation '{}' cannot declare mutable field '{}'",
                                    decl.name, field.name
                                ),
                                field.span,
                            );
                        }
                    }
                    if decl.kind == TypeKind::Record {
                        if field.visibility == Visibility::Hidden {
                            self.add_error(
                                "invalid_shape_field",
                                format!(
                                    "shape '{}' cannot declare hidden field '{}'",
                                    decl.name, field.name
                                ),
                                field.span,
                            );
                        }
                        if field.mutable {
                            self.add_error(
                                "invalid_shape_field",
                                format!(
                                    "shape '{}' cannot declare mutable field '{}'",
                                    decl.name, field.name
                                ),
                                field.span,
                            );
                        }
                    }
                    if decl.kind == TypeKind::Enum {
                        if field.visibility == Visibility::Hidden {
                            self.add_error(
                                "invalid_enum_field",
                                format!(
                                    "enum '{}' cannot declare private field '{}'",
                                    decl.name, field.name
                                ),
                                field.span,
                            );
                        }
                        if field.mutable {
                            self.add_error(
                                "invalid_enum_field",
                                format!(
                                    "enum '{}' cannot declare mutable field '{}'",
                                    decl.name, field.name
                                ),
                                field.span,
                            );
                        }
                    }
                }
                TypeMember::Method(method) => {
                    if method.name == "new" && decl.kind != TypeKind::Class {
                        self.add_error(
                            "invalid_constructor_decl",
                            custom_constructor_error(&type_sig),
                            method.span,
                        );
                        continue;
                    }
                    if decl.kind == TypeKind::Annotation {
                        self.add_error(
                            "invalid_annotation_method",
                            format!(
                                "annotation '{}': annotations cannot declare methods",
                                decl.name
                            ),
                            method.span,
                        );
                    }
                    self.check_method(method, &type_sig)
                }
                TypeMember::Case(case) => {
                    for field in &case.fields {
                        if field.visibility == Visibility::Hidden {
                            self.add_error(
                                "invalid_enum_case_field",
                                format!(
                                    "enum case '{}' cannot declare private field '{}'",
                                    case.name, field.name
                                ),
                                field.span,
                            );
                        }
                        if field.mutable {
                            self.add_error(
                                "invalid_enum_case_field",
                                format!(
                                    "enum case '{}' cannot declare mutable field '{}'",
                                    case.name, field.name
                                ),
                                field.span,
                            );
                        }
                    }
                }
            }
        }

        self.check_interface_implementation(&type_sig, decl.span);
        self.check_type_field_initializers(decl, &type_sig);

        self.pop_type_params();
    }

    fn check_interface_implementation(&mut self, sig: &TypeSig, span: crate::source::Span) {
        if sig.kind == TypeKind::Interface {
            return;
        }
        if sig
            .methods
            .values()
            .flatten()
            .any(|method| !method.has_body)
        {
            return;
        }

        for bound in &sig.with_bounds {
            let Ty::Named(interface_name, _) = bound else {
                continue;
            };
            let Some(interface_sig) = self.lookup_any_type(interface_name) else {
                continue;
            };
            if interface_sig.kind != TypeKind::Interface {
                continue;
            }

            let mut required = HashSet::new();
            let mut seen = HashSet::new();
            self.collect_required_interface_method_names(&interface_sig, &mut seen, &mut required);
            let mut missing = required
                .into_iter()
                .filter(|name| !self.type_declares_method_body(sig, name))
                .collect::<Vec<_>>();
            missing.sort();
            for name in missing {
                self.add_error(
                    "missing_interface_member",
                    format!(
                        "{} '{}' declares interface '{}' but does not implement required method '{}'",
                        type_kind_label(sig.kind),
                        sig.name,
                        interface_sig.name,
                        name
                    ),
                    span,
                );
            }
        }
    }

    fn check_type_field_initializers(&mut self, decl: &TypeDecl, owner: &TypeSig) {
        if decl.kind == TypeKind::Enum {
            return;
        }

        let previous_owner = self.current_owner.clone();
        self.current_owner = Some(owner.clone());
        self.push_scope();
        self.define_local("this", self.owner_self_ty(owner), false);

        let mut initialized_fields = HashSet::new();
        for member in &decl.members {
            let TypeMember::Field(field) = member else {
                continue;
            };
            if let Some(initializer) = &field.initializer {
                self.check_field_initializer_expr(initializer, owner, &initialized_fields);
                let expected = field.ty.as_ref().map(|ty| self.ty_from_type_ref(ty));
                let actual = match &expected {
                    Some(expected) => self.check_expr_against(initializer, expected),
                    None => self.check_expr(initializer),
                };
                let expected = expected.unwrap_or_else(|| actual.clone());
                self.require_assignable(
                    &actual,
                    &expected,
                    field.span,
                    "invalid_field_initializer",
                    format!(
                        "field '{}' expects '{}' but initializer has type '{}'",
                        field.name,
                        expected.describe(),
                        actual.describe()
                    ),
                );
            }
            initialized_fields.insert(field.name.clone());
        }

        self.pop_scope();
        self.current_owner = previous_owner;
    }

    fn check_constructor_delegation_cycles(&mut self) {
        let class_names = self
            .module
            .types
            .values()
            .filter(|sig| sig.kind == TypeKind::Class)
            .map(|sig| sig.name.clone())
            .collect::<Vec<_>>();

        for class_name in class_names {
            let Some(type_sig) = self.lookup_type_local(&class_name) else {
                continue;
            };
            let nodes = self.constructor_cycle_nodes_for_type(&class_name);
            if nodes.is_empty() {
                continue;
            }
            self.check_constructor_delegation_cycles_for_type(&type_sig, &nodes);
        }
    }

    fn constructor_cycle_nodes_for_type(&self, class_name: &str) -> Vec<ConstructorCycleNode> {
        let mut nodes = Vec::new();
        for item in &self.module.program.items {
            match item {
                Item::Type(decl) if decl.kind == TypeKind::Class && decl.name == class_name => {
                    let owner_type_params = decl
                        .type_params
                        .iter()
                        .map(|param| param.name.clone())
                        .collect::<Vec<_>>();
                    nodes.extend(
                        decl.members
                            .iter()
                            .filter_map(|member| match member {
                                TypeMember::Method(method) if method.name == "new" => Some(method),
                                _ => None,
                            })
                            .map(|method| ConstructorCycleNode {
                                method: method.clone(),
                                sig: function_sig_from_method(method, &owner_type_params),
                            }),
                    );
                }
                _ => {}
            }
        }
        nodes
    }

    fn check_constructor_delegation_cycles_for_type(
        &mut self,
        owner: &TypeSig,
        nodes: &[ConstructorCycleNode],
    ) {
        let edges = nodes
            .iter()
            .enumerate()
            .map(|(index, _)| self.constructor_delegation_target_index(owner, nodes, index))
            .collect::<Vec<_>>();
        let mut reported = HashSet::new();

        for start in 0..nodes.len() {
            let mut path = Vec::new();
            let mut seen = HashMap::new();
            let mut current = start;

            loop {
                if let Some(cycle_start) = seen.get(&current).copied() {
                    let cycle = path[cycle_start..].to_vec();
                    if cycle.iter().any(|index| !reported.contains(index)) {
                        for index in &cycle {
                            reported.insert(*index);
                        }
                        let mut labels = cycle
                            .iter()
                            .map(|index| constructor_cycle_node_label(&nodes[*index]))
                            .collect::<Vec<_>>();
                        if let Some(first) = labels.first().cloned() {
                            labels.push(first);
                        }
                        self.add_error(
                            "constructor_delegation_cycle",
                            format!(
                                "constructor delegation cycle in class '{}': {}",
                                owner.name,
                                labels.join(" -> ")
                            ),
                            nodes[current].method.span,
                        );
                    }
                    break;
                }

                if reported.contains(&current) {
                    break;
                }

                seen.insert(current, path.len());
                path.push(current);

                let Some(next) = edges[current] else {
                    break;
                };
                current = next;
            }
        }
    }

    fn constructor_delegation_target_index(
        &mut self,
        owner: &TypeSig,
        nodes: &[ConstructorCycleNode],
        node_index: usize,
    ) -> Option<usize> {
        let method = &nodes[node_index].method;
        let body = method.body.as_ref()?;
        let (raw_args, uses_brace_syntax, _) = constructor_delegation_call(body)?;
        let normalized_args;
        let args = if uses_brace_syntax {
            normalized_args =
                brace_record_constructor_args(raw_args).unwrap_or_else(|| raw_args.to_vec());
            normalized_args.as_slice()
        } else {
            raw_args
        };

        let previous_owner = self.current_owner.clone();
        let previous_method = self.current_method.clone();
        self.current_owner = Some(owner.clone());
        self.current_method = Some("new".to_string());
        self.push_scope();
        self.define_local("this", self.owner_self_ty(owner), false);
        for param in &nodes[node_index].sig.params {
            self.define_local(&param.name, param.ty.clone(), false);
        }
        let target = self.choose_constructor_node(nodes, args);
        self.pop_scope();
        self.current_owner = previous_owner;
        self.current_method = previous_method;

        target
    }

    fn choose_constructor_node(
        &self,
        nodes: &[ConstructorCycleNode],
        args: &[crate::ast::CallArg],
    ) -> Option<usize> {
        let arg_types = args
            .iter()
            .map(|arg| self.probe_expr_type(call_arg_value_expr(arg)))
            .collect::<Vec<_>>();
        nodes
            .iter()
            .enumerate()
            .filter_map(|(index, node)| {
                let arrangement = arrange_param_args(&node.sig.params, args);
                if arrangement.overflow > 0 || arrangement.missing_required > 0 {
                    return None;
                }
                let mut score = 0usize;
                for (param_index, param) in node.sig.params.iter().enumerate() {
                    for arg in arrangement
                        .slots
                        .get(param_index)
                        .map(Vec::as_slice)
                        .unwrap_or(&[])
                    {
                        let arg_index = args
                            .iter()
                            .position(|candidate| std::ptr::eq(candidate, *arg))
                            .unwrap_or(0);
                        let actual = &arg_types[arg_index];
                        let expected = call_arg_expected_ty_for_arg(param.variadic, &param.ty, arg);
                        if !matches!(actual, Ty::Unknown) {
                            if self.arg_matches_expected(arg, actual, &expected) {
                                score += 2;
                            } else if type_contains_type_param(&expected) {
                                score += 1;
                            } else {
                                return None;
                            }
                        }
                        if arg.name.as_deref() == Some(param.name.as_str()) {
                            score += 1;
                        }
                    }
                }
                if !node.sig.params.iter().any(|param| param.variadic) {
                    score += 1;
                }
                Some((score, index))
            })
            .max_by_key(|(score, _)| *score)
            .map(|(_, index)| index)
    }

    fn check_extension(&mut self, block: &ExtensionBlock) {
        let Some(target_name) = type_ref_named_name(&block.target) else {
            return;
        };
        let Some(type_sig) = self
            .lookup_any_type(target_name)
            .or_else(|| builtin_extension_type_sig(target_name))
        else {
            self.add_error(
                "unknown_extension_target",
                format!("unknown extension target '{}'", target_name),
                block.span,
            );
            return;
        };
        if matches!(type_sig.kind, TypeKind::Annotation | TypeKind::Object) {
            self.add_error(
                "invalid_extension_target",
                format!(
                    "extension target '{}' must be a class, shape, enum, or interface",
                    target_name
                ),
                block.span,
            );
            return;
        }
        let previous_extension_target = self.current_extension_target.clone();
        self.current_extension_target = Some(target_name.to_string());
        self.push_type_params_with_conditions(
            type_sig.type_params.iter().map(String::as_str),
            type_sig.generic_conditions.clone(),
        );
        for method in &block.methods {
            if method.name == "new" {
                self.add_error(
                    "invalid_extension_constructor",
                    "extension blocks cannot declare constructors",
                    method.span,
                );
                continue;
            }
            self.check_method(method, &type_sig);
        }
        self.pop_type_params();
        self.current_extension_target = previous_extension_target;
    }

    fn check_method(&mut self, method: &MethodDecl, owner: &TypeSig) {
        let previous_return = self.current_return.clone();
        let previous_owner = self.current_owner.clone();
        let previous_method = self.current_method.clone();
        let previous_defer_depth = self.defer_depth;
        let previous_callable_depth = self.callable_depth;
        self.push_ast_type_params(&method.type_params, &method.type_conditions);
        self.validate_generic_clause(&method.type_params, &method.type_conditions);
        if let Some(return_type) = &method.return_type {
            self.validate_type_ref_generic_applications(return_type);
        }
        for param in &method.params {
            if let Some(ty) = &param.ty {
                self.validate_type_ref_generic_applications(ty);
            }
        }
        let expected_return = method
            .return_type
            .as_ref()
            .map(|ty| self.ty_from_type_ref(ty))
            .unwrap_or(Ty::Unknown);
        self.current_return = expected_return.clone();
        self.defer_depth = 0;
        self.callable_depth += 1;
        self.current_owner = Some(owner.clone());
        self.current_method = Some(method.name.clone());
        self.push_scope();
        self.define_local("this", self.owner_self_ty(owner), false);
        self.check_param_list_rules(&method.params, method.name == "new");
        for param in &method.params {
            let elem_ty = param
                .ty
                .as_ref()
                .map(|value| self.ty_from_type_ref(value))
                .unwrap_or(Ty::Unknown);
            if let Some(initializer) = &param.initializer {
                self.check_param_initializer(param, &elem_ty, initializer);
            }
            let ty = self.param_local_type(param, elem_ty);
            self.define_local(&param.name, ty, false);
            if param.lazy {
                self.mark_current_local_unstable(&param.name);
            }
        }
        if let Some(body) = &method.body {
            let actual = self.check_callable_body(body);
            self.require_assignable(
                &actual,
                &expected_return,
                method.span,
                "invalid_return_type",
                format!(
                    "method '{}' returns '{}' but is declared as '{}'",
                    method.name,
                    actual.describe(),
                    expected_return.describe()
                ),
            );
        }
        self.check_constructor_initializes_required_fields(method, owner);

        self.pop_scope();
        self.pop_type_params();
        self.current_return = previous_return;
        self.current_owner = previous_owner;
        self.current_method = previous_method;
        self.defer_depth = previous_defer_depth;
        self.callable_depth = previous_callable_depth;
    }

    fn check_constructor_initializes_required_fields(
        &mut self,
        method: &MethodDecl,
        owner: &TypeSig,
    ) {
        if method.name != "new" || owner.kind != TypeKind::Class {
            return;
        }
        if let Some(body) = &method.body {
            if constructor_body_contains_this_delegation_call(body)
                && !constructor_body_delegates(body)
            {
                self.add_error(
                    "invalid_constructor_delegation",
                    "constructor delegation must be the entire expression body; write `new(...) = this(...)`, `new(...) = this { ... }`, or initialize fields directly",
                    method.span,
                );
            }
        }
        let required_fields = owner
            .fields
            .iter()
            .filter(|field| !field.has_initializer)
            .collect::<Vec<_>>();
        if required_fields.is_empty() {
            return;
        }
        let Some(body) = &method.body else {
            self.report_constructor_missing_fields(owner, &required_fields, method.span);
            return;
        };
        if constructor_body_delegates(body) {
            return;
        }
        if constructor_body_contains_delegation_attempt(body) {
            return;
        }
        let assigned = constructor_assigned_fields(body, owner, method);
        let missing = required_fields
            .into_iter()
            .filter(|field| !assigned.contains(&field.name))
            .collect::<Vec<_>>();
        if !missing.is_empty() {
            self.report_constructor_missing_fields(owner, &missing, method.span);
        }
    }

    fn report_constructor_missing_fields(
        &mut self,
        owner: &TypeSig,
        fields: &[&FieldSig],
        span: crate::source::Span,
    ) {
        let names = fields
            .iter()
            .map(|field| field.name.as_str())
            .collect::<Vec<_>>()
            .join("', '");
        self.add_error(
            "uninitialized_field",
            format!(
                "constructor 'new' for {} '{}' must initialize field '{}' or delegate to another constructor",
                type_kind_label(owner.kind),
                owner.name,
                names
            ),
            span,
        );
    }

    fn owner_self_ty(&self, owner: &TypeSig) -> Ty {
        Ty::Named(
            owner.name.clone(),
            owner
                .type_params
                .iter()
                .map(|name| Ty::TypeParam(name.clone()))
                .collect(),
        )
    }

    fn param_local_type(&self, _param: &Param, ty: Ty) -> Ty {
        ty
    }

    fn fresh_capture(&mut self) -> Ty {
        self.fresh_capture_with_label(None)
    }

    fn fresh_capture_with_label(&mut self, label: Option<String>) -> Ty {
        let id = self.next_capture_id;
        self.next_capture_id += 1;
        if let Some(label) = label {
            self.capture_labels.insert(id, label);
        }
        Ty::Capture(id)
    }

    fn capture_wildcards(&mut self, ty: Ty) -> Ty {
        match ty {
            Ty::Wildcard => self.fresh_capture(),
            Ty::Named(name, args) => {
                let labels = wildcard_capture_labels(&name, &args);
                Ty::Named(
                    name,
                    args.into_iter()
                        .enumerate()
                        .map(|(index, arg)| match arg {
                            Ty::Wildcard => self
                                .fresh_capture_with_label(labels.get(index).and_then(Clone::clone)),
                            other => self.capture_wildcards(other),
                        })
                        .collect(),
                )
            }
            Ty::Tuple(items) => Ty::Tuple(
                items
                    .into_iter()
                    .map(|item| self.capture_wildcards(item))
                    .collect(),
            ),
            Ty::Record(fields) => Ty::Record(
                fields
                    .into_iter()
                    .map(|(name, ty)| (name, self.capture_wildcards(ty)))
                    .collect(),
            ),
            Ty::Function(params, ret) => Ty::Function(params, ret),
            other => other,
        }
    }

    fn check_param_list_rules(&mut self, params: &[Param], is_constructor: bool) {
        let mut seen_default = false;
        let mut seen_variadic = false;
        for (index, param) in params.iter().enumerate() {
            if !is_constructor {
                if let Some(ty) = &param.ty {
                    if is_unit_type_ref(ty) {
                        self.add_error(
                            "invalid_parameter_type",
                            format!(
                                "parameter '{}' cannot have type Unit; omit the parameter or use '() => T' for a no-argument callback",
                                param.name
                            ),
                            ty.span(),
                        );
                    } else if let Some(unit_span) = unit_function_param_span(ty) {
                        self.add_error(
                            "invalid_parameter_type",
                            format!(
                                "function parameter '{}' cannot use Unit as a callback parameter; write '() => T' for a no-argument function type",
                                param.name
                            ),
                            unit_span,
                        );
                    }
                }
            }
            if param.lazy && is_constructor {
                self.add_error(
                    "invalid_by_name_param",
                    "constructor parameters cannot be by-name; use 'name => Type' only on function and method parameters",
                    param.span,
                );
            }
            if param.lazy && param.variadic {
                self.add_error(
                    "invalid_by_name_param",
                    "by-name parameters cannot be vararg",
                    param.span,
                );
            }
            if param.variadic && seen_variadic {
                self.add_error(
                    "invalid_variadic_param",
                    if is_constructor {
                        "only one variadic constructor parameter is allowed"
                    } else {
                        "only one variadic parameter is allowed"
                    },
                    param.span,
                );
            }
            if param.variadic && index + 1 != params.len() {
                self.add_error(
                    "invalid_variadic_param",
                    if is_constructor {
                        "variadic constructor parameter must be last"
                    } else {
                        "variadic parameter must be last"
                    },
                    param.span,
                );
            }
            if param.variadic && seen_default && !is_constructor {
                self.add_error(
                    "invalid_variadic_param",
                    "variadic parameter cannot follow defaulted parameters",
                    param.span,
                );
            }
            if param.variadic {
                let is_list_type = param.ty.as_ref().is_some_and(is_list_type_ref);
                if !is_list_type {
                    self.add_error(
                        "invalid_variadic_param",
                        if is_constructor {
                            "variadic constructor parameter must use a vector type like 'args [T] vararg'"
                        } else {
                            "variadic parameter must use a vector type like 'args [T] vararg'"
                        },
                        param.span,
                    );
                }
                seen_variadic = true;
            }
            if param.initializer.is_some() {
                seen_default = true;
            } else if seen_default && !param.variadic && !is_constructor {
                self.add_error(
                    "invalid_constructor_default",
                    "parameters without defaults cannot follow defaulted parameters",
                    param.span,
                );
            }
        }
    }

    fn check_param_initializer(&mut self, param: &Param, expected: &Ty, initializer: &Expr) {
        let actual = self.check_expr_against(initializer, expected);
        self.require_assignable(
            &actual,
            expected,
            initializer.span(),
            "invalid_argument_type",
            format!(
                "default value for '{}' has type '{}' but expects '{}'",
                param.name,
                actual.describe(),
                expected.describe()
            ),
        );
        if infer_literal_type(initializer).is_none() {
            self.add_error(
                "invalid_constructor_default",
                "parameter defaults must be literal constants for now",
                initializer.span(),
            );
        }
    }

    fn check_field_initializer_expr(
        &mut self,
        expr: &Expr,
        owner: &TypeSig,
        initialized_fields: &HashSet<String>,
    ) {
        match expr {
            Expr::Identifier { .. }
            | Expr::Placeholder { .. }
            | Expr::Integer { .. }
            | Expr::Float { .. }
            | Expr::String { .. }
            | Expr::Bool { .. }
            | Expr::Unit { .. } => {}
            Expr::Spread { value, .. } => {
                self.check_field_initializer_expr(value, owner, initialized_fields);
            }
            Expr::ListLiteral { items, .. }
            | Expr::TupleLiteral { items, .. }
            | Expr::ShapeLiteral { items, .. } => {
                for item in items {
                    self.check_field_initializer_expr(item, owner, initialized_fields);
                }
            }
            Expr::Call {
                callee, args, span, ..
            } => {
                if let Expr::Member { receiver, name, .. } = callee.as_ref() {
                    if matches!(receiver.as_ref(), Expr::Identifier { name: receiver_name, .. } if receiver_name == "this")
                    {
                        self.add_error(
                            "invalid_field_initializer",
                            format!(
                                "field initializer cannot call instance method '{}'; move this logic to 'new()'",
                                name
                            ),
                            *span,
                        );
                        for arg in args {
                            self.check_field_initializer_expr(
                                &arg.value,
                                owner,
                                initialized_fields,
                            );
                        }
                        return;
                    }
                }
                self.check_field_initializer_expr(callee, owner, initialized_fields);
                for arg in args {
                    self.check_field_initializer_expr(&arg.value, owner, initialized_fields);
                }
            }
            Expr::Member {
                receiver,
                name,
                span,
            } => {
                if matches!(receiver.as_ref(), Expr::Identifier { name: receiver_name, .. } if receiver_name == "this")
                    && owner.fields.iter().any(|field| field.name == *name)
                    && !initialized_fields.contains(name)
                {
                    self.add_error(
                        "invalid_field_initializer",
                        format!(
                            "field initializer can read only fields declared earlier; '{}' is not available yet",
                            name
                        ),
                        *span,
                    );
                }
                self.check_field_initializer_expr(receiver, owner, initialized_fields);
            }
            Expr::Index {
                receiver, index, ..
            } => {
                self.check_field_initializer_expr(receiver, owner, initialized_fields);
                self.check_field_initializer_expr(index, owner, initialized_fields);
            }
            Expr::RecordUpdate {
                receiver, patch, ..
            } => {
                self.check_field_initializer_expr(receiver, owner, initialized_fields);
                self.check_field_initializer_expr(patch, owner, initialized_fields);
            }
            Expr::RecordLiteral { fields, values, .. } => {
                for field in fields {
                    self.check_field_initializer_expr(&field.value, owner, initialized_fields);
                }
                for value in values {
                    self.check_field_initializer_expr(value, owner, initialized_fields);
                }
            }
            Expr::AnonymousInterface { .. }
            | Expr::AnonymousObject { .. }
            | Expr::Lambda { .. } => {}
            Expr::Try { value, .. }
            | Expr::Unary { expr: value, .. }
            | Expr::Group { inner: value, .. } => {
                self.check_field_initializer_expr(value, owner, initialized_fields);
            }
            Expr::ExtractOr {
                value, fallback, ..
            } => {
                self.check_field_initializer_expr(value, owner, initialized_fields);
                self.check_field_initializer_expr(fallback, owner, initialized_fields);
            }
            Expr::Return { value, .. } => {
                if let Some(value) = value {
                    self.check_field_initializer_expr(value, owner, initialized_fields);
                }
            }
            Expr::Break { .. } | Expr::Continue { .. } => {}
            Expr::Binary { left, right, .. } => {
                self.check_field_initializer_expr(left, owner, initialized_fields);
                self.check_field_initializer_expr(right, owner, initialized_fields);
            }
            Expr::Is { left, .. } => {
                self.check_field_initializer_expr(left, owner, initialized_fields);
            }
            Expr::TypeOf { .. } => {}
            Expr::If {
                condition,
                then_block,
                else_branch,
                ..
            } => {
                self.check_field_initializer_expr(condition, owner, initialized_fields);
                self.check_field_initializer_block(then_block, owner, initialized_fields);
                self.check_field_initializer_else_expr_branch(
                    else_branch,
                    owner,
                    initialized_fields,
                );
            }
            Expr::Block { body, .. } => {
                self.check_field_initializer_block(body, owner, initialized_fields);
            }
            Expr::Match { value, cases, .. } => {
                self.check_field_initializer_expr(value, owner, initialized_fields);
                for case in cases {
                    if let Some(guard) = &case.guard {
                        self.check_field_initializer_expr(guard, owner, initialized_fields);
                    }
                    match &case.body {
                        MatchCaseBody::Block(block) => {
                            self.check_field_initializer_block(block, owner, initialized_fields);
                        }
                        MatchCaseBody::Expr(expr) => {
                            self.check_field_initializer_expr(expr, owner, initialized_fields);
                        }
                    }
                }
            }
            Expr::ForYield {
                bindings,
                yield_body,
                ..
            } => {
                for binding in bindings {
                    if let Some(iterable) = &binding.iterable {
                        self.check_field_initializer_expr(iterable, owner, initialized_fields);
                    }
                    for value in &binding.values {
                        self.check_field_initializer_expr(value, owner, initialized_fields);
                    }
                }
                self.check_field_initializer_block(yield_body, owner, initialized_fields);
            }
        }
    }

    fn check_field_initializer_block(
        &mut self,
        block: &Block,
        owner: &TypeSig,
        initialized_fields: &HashSet<String>,
    ) {
        for statement in &block.statements {
            self.check_field_initializer_stmt(statement, owner, initialized_fields);
        }
    }

    fn check_field_initializer_stmt(
        &mut self,
        stmt: &Stmt,
        owner: &TypeSig,
        initialized_fields: &HashSet<String>,
    ) {
        match stmt {
            Stmt::Binding(binding) => {
                for value in &binding.values {
                    self.check_field_initializer_expr(value, owner, initialized_fields);
                }
            }
            Stmt::PatternBinding(stmt) => {
                for clause in &stmt.clauses {
                    self.check_field_initializer_expr(&clause.value, owner, initialized_fields);
                }
                self.check_field_initializer_expr(&stmt.value, owner, initialized_fields);
            }
            Stmt::Assignment(stmt) => {
                for target in &stmt.targets {
                    self.check_field_initializer_expr(target, owner, initialized_fields);
                }
                for value in &stmt.values {
                    self.check_field_initializer_expr(value, owner, initialized_fields);
                }
            }
            Stmt::Defer(stmt) => {
                self.add_error(
                    "invalid_field_initializer",
                    "field initializer block cannot contain 'defer'; move cleanup logic to 'new()'",
                    stmt.span,
                );
                match &stmt.action {
                    crate::ast::DeferAction::Call(expr) => {
                        self.check_field_initializer_expr(expr, owner, initialized_fields);
                    }
                    crate::ast::DeferAction::Block(block) => {
                        self.check_field_initializer_block(block, owner, initialized_fields);
                    }
                }
            }
            Stmt::If(stmt) => {
                self.check_field_initializer_if_stmt(stmt, owner, initialized_fields);
            }
            Stmt::Match(stmt) => {
                self.check_field_initializer_expr(&stmt.value, owner, initialized_fields);
                for case in &stmt.cases {
                    if let Some(guard) = &case.guard {
                        self.check_field_initializer_expr(guard, owner, initialized_fields);
                    }
                    match &case.body {
                        MatchCaseBody::Block(block) => {
                            self.check_field_initializer_block(block, owner, initialized_fields);
                        }
                        MatchCaseBody::Expr(expr) => {
                            self.check_field_initializer_expr(expr, owner, initialized_fields);
                        }
                    }
                }
            }
            Stmt::While(stmt) => {
                for condition in &stmt.condition_clauses {
                    match condition {
                        IfConditionClause::Expr(condition) => {
                            self.check_field_initializer_expr(condition, owner, initialized_fields);
                        }
                        IfConditionClause::Let(clause) => {
                            self.check_field_initializer_expr(
                                &clause.value,
                                owner,
                                initialized_fields,
                            );
                        }
                    }
                }
                self.check_field_initializer_block(&stmt.body, owner, initialized_fields);
            }
            Stmt::For(stmt) => {
                for binding in &stmt.bindings {
                    if let Some(iterable) = &binding.iterable {
                        self.check_field_initializer_expr(iterable, owner, initialized_fields);
                    }
                    for value in &binding.values {
                        self.check_field_initializer_expr(value, owner, initialized_fields);
                    }
                }
                self.check_field_initializer_block(&stmt.body, owner, initialized_fields);
            }
            Stmt::LetElse(stmt) => {
                for clause in &stmt.clauses {
                    self.check_field_initializer_expr(&clause.value, owner, initialized_fields);
                }
                self.check_field_initializer_expr(&stmt.value, owner, initialized_fields);
                self.check_field_initializer_block(&stmt.else_block, owner, initialized_fields);
            }
            Stmt::Return(stmt) => {
                if let Some(value) = &stmt.value {
                    self.check_field_initializer_expr(value, owner, initialized_fields);
                }
            }
            Stmt::Break(_) | Stmt::Continue(_) | Stmt::LocalFunction(_) => {}
            Stmt::Expr(stmt) => {
                self.check_field_initializer_expr(&stmt.expr, owner, initialized_fields);
            }
        }
    }

    fn check_field_initializer_if_stmt(
        &mut self,
        stmt: &IfStmt,
        owner: &TypeSig,
        initialized_fields: &HashSet<String>,
    ) {
        for clause in &stmt.condition_clauses {
            match clause {
                IfConditionClause::Let(clause) => {
                    self.check_field_initializer_expr(&clause.value, owner, initialized_fields);
                }
                IfConditionClause::Expr(expr) => {
                    self.check_field_initializer_expr(expr, owner, initialized_fields);
                }
            }
        }
        for clause in &stmt.pattern_clauses {
            self.check_field_initializer_expr(&clause.value, owner, initialized_fields);
        }
        if let Some(condition) = &stmt.condition {
            self.check_field_initializer_expr(condition, owner, initialized_fields);
        }
        if let Some(value) = &stmt.pattern_value {
            self.check_field_initializer_expr(value, owner, initialized_fields);
        }
        if let Some(value) = &stmt.binding_value {
            self.check_field_initializer_expr(value, owner, initialized_fields);
        }
        self.check_field_initializer_block(&stmt.then_block, owner, initialized_fields);
        if let Some(branch) = &stmt.else_branch {
            self.check_field_initializer_else_branch(branch, owner, initialized_fields);
        }
    }

    fn check_field_initializer_else_branch(
        &mut self,
        branch: &ElseBranch,
        owner: &TypeSig,
        initialized_fields: &HashSet<String>,
    ) {
        match branch {
            ElseBranch::If(stmt) => {
                self.check_field_initializer_if_stmt(stmt, owner, initialized_fields);
            }
            ElseBranch::Block(block) => {
                self.check_field_initializer_block(block, owner, initialized_fields);
            }
        }
    }

    fn check_field_initializer_else_expr_branch(
        &mut self,
        branch: &ElseExprBranch,
        owner: &TypeSig,
        initialized_fields: &HashSet<String>,
    ) {
        match branch {
            ElseExprBranch::If(expr) => {
                self.check_field_initializer_expr(expr, owner, initialized_fields);
            }
            ElseExprBranch::Block(block) => {
                self.check_field_initializer_block(block, owner, initialized_fields);
            }
        }
    }

    fn check_callable_body(&mut self, body: &CallableBody) -> Ty {
        match body {
            CallableBody::Expr(expr) => {
                let actual = self.check_expr_against(expr, &self.current_return.clone());
                if self.current_return == Ty::unit() {
                    self.check_discarded_expr_after_unit_expected(expr);
                    return Ty::unit();
                }
                actual
            }
            CallableBody::Block(block) => {
                self.check_block_against(block, &self.current_return.clone())
            }
        }
    }

    fn check_record_update(&mut self, base: &Ty, patch: &Expr, span: crate::source::Span) {
        let patch_ty = self.check_expr(patch);
        let Some(base_fields) = self.record_update_shape_fields(base, "base", span) else {
            return;
        };
        let Some(patch_fields) = self.record_update_shape_fields(&patch_ty, "patch", patch.span())
        else {
            return;
        };
        self.check_record_update_fields(&base_fields, &patch_fields, patch.span());
    }

    fn record_update_shape_fields(
        &mut self,
        ty: &Ty,
        role: &str,
        span: crate::source::Span,
    ) -> Option<Vec<(String, Ty)>> {
        match ty {
            Ty::Record(fields) => Some(fields.clone()),
            Ty::Named(name, args) => {
                let Some(sig) = self.lookup_any_type(name) else {
                    self.add_error(
                        "invalid_shape_update",
                        format!(
                            "shape update {role} must be a class, shape, or anonymous shape value, got '{}'",
                            ty.describe()
                        ),
                        span,
                    );
                    return None;
                };
                if !matches!(sig.kind, TypeKind::Class | TypeKind::Record) {
                    self.add_error(
                        "invalid_shape_update",
                        format!(
                            "shape update {role} must be a class, shape, or anonymous shape value, got '{}'",
                            ty.describe()
                        ),
                        span,
                    );
                    return None;
                }
                let subst = sig
                    .type_params
                    .iter()
                    .cloned()
                    .zip(args.iter().cloned())
                    .collect::<HashMap<_, _>>();
                Some(
                    sig.fields
                        .iter()
                        .filter(|field| !field.hidden)
                        .map(|field| (field.name.clone(), substitute_type(&field.ty, &subst)))
                        .collect(),
                )
            }
            Ty::Unknown => None,
            _ => {
                self.add_error(
                    "invalid_shape_update",
                    format!(
                        "shape update {role} must be a class, shape, or anonymous shape value, got '{}'",
                        ty.describe()
                    ),
                    span,
                );
                None
            }
        }
    }

    fn check_record_update_fields(
        &mut self,
        fields: &[(String, Ty)],
        updates: &[(String, Ty)],
        span: crate::source::Span,
    ) {
        for (name, actual) in updates {
            let Some((_, expected)) = fields.iter().find(|(field, _)| field == name) else {
                self.add_error(
                    "invalid_shape_update",
                    format!("update field '{}' does not exist on left-hand shape", name),
                    span,
                );
                continue;
            };
            self.require_assignable(
                actual,
                expected,
                span,
                "invalid_shape_update",
                format!(
                    "update field '{}' expects '{}', got '{}'",
                    name,
                    expected.describe(),
                    actual.describe()
                ),
            );
        }
    }

    fn record_spread_shape_fields(
        &mut self,
        ty: &Ty,
        span: crate::source::Span,
    ) -> Option<Vec<(String, Ty)>> {
        match ty {
            Ty::Record(fields) => Some(fields.clone()),
            Ty::Named(name, args) => {
                let Some(sig) = self.lookup_any_type(name) else {
                    self.add_error(
                        "invalid_shape_spread",
                        format!(
                            "shape spread requires a shape value, got '{}'",
                            ty.describe()
                        ),
                        span,
                    );
                    return None;
                };
                if !matches!(sig.kind, TypeKind::Class | TypeKind::Record) || sig.fields.is_empty()
                {
                    self.add_error(
                        "invalid_shape_spread",
                        format!(
                            "shape spread requires a shape value, got '{}'",
                            ty.describe()
                        ),
                        span,
                    );
                    return None;
                }
                let subst = sig
                    .type_params
                    .iter()
                    .cloned()
                    .zip(args.iter().cloned())
                    .collect::<HashMap<_, _>>();
                Some(
                    sig.fields
                        .iter()
                        .filter(|field| !field.hidden)
                        .map(|field| (field.name.clone(), substitute_type(&field.ty, &subst)))
                        .collect(),
                )
            }
            Ty::Unknown => None,
            _ => {
                self.add_error(
                    "invalid_shape_spread",
                    format!(
                        "shape spread requires a shape value, got '{}'",
                        ty.describe()
                    ),
                    span,
                );
                None
            }
        }
    }

    fn check_block(&mut self, block: &Block) -> Ty {
        self.check_block_against(block, &Ty::Unknown)
    }

    fn check_block_with_narrowing(
        &mut self,
        block: &Block,
        narrowing: Option<&TypeNarrowing>,
    ) -> Ty {
        self.push_scope();
        if let Some(narrowing) = narrowing {
            self.define_local(&narrowing.name, narrowing.ty.clone(), false);
        }
        let result = self.check_block(block);
        self.pop_scope();
        result
    }

    fn check_block_against(&mut self, block: &Block, expected: &Ty) -> Ty {
        self.push_scope();
        let mut last = Ty::unit();
        for (index, statement) in block.statements.iter().enumerate() {
            let ty = if index + 1 == block.statements.len() {
                match statement {
                    Stmt::Expr(expr_stmt) => self.check_expr_against(&expr_stmt.expr, expected),
                    Stmt::If(stmt) => self.check_if_stmt_value(stmt, expected),
                    Stmt::Match(stmt) => self.check_match_stmt_value(stmt, expected),
                    _ => self.check_stmt(statement),
                }
            } else {
                self.check_stmt(statement)
            };
            if index + 1 == block.statements.len() {
                if *expected == Ty::unit() {
                    if let Stmt::Expr(expr_stmt) = statement {
                        self.check_discarded_expr_after_unit_expected(&expr_stmt.expr);
                    }
                    last = Ty::unit();
                } else {
                    last = ty;
                }
            }
        }
        self.pop_scope();
        last
    }

    fn check_if_stmt_value(&mut self, stmt: &IfStmt, expected: &Ty) -> Ty {
        if !stmt.condition_clauses.is_empty() {
            self.push_scope();
            for clause in &stmt.condition_clauses {
                match clause {
                    IfConditionClause::Let(clause) => {
                        let value_ty = self.check_expr(&clause.value);
                        self.require_refutable_if_pattern(
                            &clause.pattern,
                            &value_ty,
                            clause.pattern.span(),
                        );
                        self.bind_pattern(&clause.pattern, &value_ty);
                    }
                    IfConditionClause::Expr(condition) => {
                        let condition_ty = self.check_expr(condition);
                        self.require_bool(
                            &condition_ty,
                            condition.span(),
                            "if condition must be Bool",
                        );
                    }
                }
            }
            let then_ty = self.check_block_against(&stmt.then_block, expected);
            self.pop_scope();
            let else_ty = stmt
                .else_branch
                .as_ref()
                .map(|branch| self.check_else_branch_value(branch, expected))
                .unwrap_or_else(Ty::unit);
            return join_types(&then_ty, &else_ty);
        } else if !stmt.pattern_clauses.is_empty() {
            self.push_scope();
            for clause in &stmt.pattern_clauses {
                let value_ty = self.check_expr(&clause.value);
                self.require_refutable_if_pattern(
                    &clause.pattern,
                    &value_ty,
                    clause.pattern.span(),
                );
                self.bind_pattern(&clause.pattern, &value_ty);
            }
            let then_ty = self.check_block_against(&stmt.then_block, expected);
            self.pop_scope();
            let else_ty = stmt
                .else_branch
                .as_ref()
                .map(|branch| self.check_else_branch_value(branch, expected))
                .unwrap_or_else(Ty::unit);
            return join_types(&then_ty, &else_ty);
        }
        if let Some(value) = &stmt.pattern_value {
            let value_ty = self.check_expr(value);
            self.push_scope();
            if let Some(pattern) = &stmt.pattern {
                self.require_refutable_if_pattern(pattern, &value_ty, pattern.span());
                self.bind_pattern(pattern, &value_ty);
            }
            let then_ty = self.check_block_against(&stmt.then_block, expected);
            self.pop_scope();
            let else_ty = stmt
                .else_branch
                .as_ref()
                .map(|branch| self.check_else_branch_value(branch, expected))
                .unwrap_or_else(Ty::unit);
            return join_types(&then_ty, &else_ty);
        }
        if let Some(value) = &stmt.binding_value {
            let value_ty = self.check_expr(value);
            let inner = self.unwrap_inner_type(&value_ty);
            self.check_destructure_source(
                &inner,
                DestructureKind::Tuple,
                stmt.bindings.len(),
                value.span(),
            );
            let slot_types =
                self.destructure_slots(&inner, stmt.bindings.len(), DestructureKind::Tuple);
            self.push_scope();
            for (index, binding) in stmt.bindings.iter().enumerate() {
                let inferred = slot_types.get(index).cloned().unwrap_or(Ty::Unknown);
                let explicit = binding.ty.as_ref().map(|ty| self.ty_from_type_ref(ty));
                let ty = explicit.clone().unwrap_or_else(|| inferred.clone());
                self.define_local(&binding.name, ty, false);
            }
            let then_ty = self.check_block_against(&stmt.then_block, expected);
            self.pop_scope();
            let else_ty = stmt
                .else_branch
                .as_ref()
                .map(|branch| self.check_else_branch_value(branch, expected))
                .unwrap_or_else(Ty::unit);
            return join_types(&then_ty, &else_ty);
        }
        if let Some(condition) = &stmt.condition {
            let condition_ty = self.check_expr(condition);
            self.require_bool(&condition_ty, condition.span(), "if condition must be Bool");
        }
        let then_narrowing = stmt
            .condition
            .as_ref()
            .and_then(|condition| self.type_narrowing_for_condition(condition, true));
        let else_narrowing = stmt
            .condition
            .as_ref()
            .and_then(|condition| self.type_narrowing_for_condition(condition, false));
        let then_ty = self.check_block_against_with_narrowing(
            &stmt.then_block,
            expected,
            then_narrowing.as_ref(),
        );
        let else_ty = stmt
            .else_branch
            .as_ref()
            .map(|branch| {
                self.check_else_branch_value_with_narrowing(
                    branch,
                    expected,
                    else_narrowing.as_ref(),
                )
            })
            .unwrap_or_else(Ty::unit);

        let then_exits = self.block_guarantees_control_exit(&stmt.then_block);
        let else_exits = stmt
            .else_branch
            .as_ref()
            .is_some_and(|branch| self.else_branch_guarantees_control_exit(branch));
        if then_exits && !else_exits {
            if let Some(narrowing) = else_narrowing {
                self.define_local(&narrowing.name, narrowing.ty, false);
            }
        } else if else_exits && !then_exits {
            if let Some(narrowing) = then_narrowing {
                self.define_local(&narrowing.name, narrowing.ty, false);
            }
        }

        join_types(&then_ty, &else_ty)
    }

    fn check_block_against_with_narrowing(
        &mut self,
        block: &Block,
        expected: &Ty,
        narrowing: Option<&TypeNarrowing>,
    ) -> Ty {
        self.push_scope();
        if let Some(narrowing) = narrowing {
            self.define_local(&narrowing.name, narrowing.ty.clone(), false);
        }
        let result = self.check_block_against(block, expected);
        self.pop_scope();
        result
    }

    fn check_else_branch_value(&mut self, branch: &ElseBranch, expected: &Ty) -> Ty {
        match branch {
            ElseBranch::If(stmt) => self.check_if_stmt_value(stmt, expected),
            ElseBranch::Block(block) => self.check_block_against(block, expected),
        }
    }

    fn check_else_branch_value_with_narrowing(
        &mut self,
        branch: &ElseBranch,
        expected: &Ty,
        narrowing: Option<&TypeNarrowing>,
    ) -> Ty {
        self.push_scope();
        if let Some(narrowing) = narrowing {
            self.define_local(&narrowing.name, narrowing.ty.clone(), false);
        }
        let result = self.check_else_branch_value(branch, expected);
        self.pop_scope();
        result
    }

    fn check_match_stmt_value(&mut self, stmt: &crate::ast::MatchStmt, expected: &Ty) -> Ty {
        let value_ty = self.check_expr(&stmt.value);
        let mut result = Ty::Unknown;
        for case in &stmt.cases {
            self.push_scope();
            self.bind_pattern(&case.pattern, &value_ty);
            if let Some(guard) = &case.guard {
                let guard_ty = self.check_expr(guard);
                self.require_bool(&guard_ty, guard.span(), "match guard must be Bool");
            }
            let ty = match &case.body {
                MatchCaseBody::Block(block) => self.check_block_against(block, expected),
                MatchCaseBody::Expr(expr) => self.check_expr_against(expr, expected),
            };
            self.pop_scope();
            result = join_types(&result, &ty);
        }
        if !stmt.partial {
            self.check_match_exhaustiveness(&value_ty, &stmt.cases, stmt.span);
        }
        if stmt.partial {
            Ty::option(result)
        } else {
            result
        }
    }

    fn check_stmt(&mut self, statement: &Stmt) -> Ty {
        match statement {
            Stmt::Binding(binding_stmt) => {
                for binding in &binding_stmt.bindings {
                    if let Some(ty) = &binding.ty {
                        self.validate_type_ref_generic_applications(ty);
                    }
                }
                let value_types = binding_stmt
                    .values
                    .iter()
                    .enumerate()
                    .map(|(index, expr)| {
                        if binding_stmt.bindings.len() == 1 {
                            if let Some(expected) = binding_stmt.bindings[0]
                                .ty
                                .as_ref()
                                .map(|ty| self.ty_from_type_ref(ty))
                            {
                                return self.check_expr_against(expr, &expected);
                            }
                        }
                        if let Some(expected) = binding_stmt
                            .bindings
                            .get(index)
                            .and_then(|binding| binding.ty.as_ref())
                            .map(|ty| self.ty_from_type_ref(ty))
                        {
                            self.check_expr_against(expr, &expected)
                        } else {
                            self.check_expr(expr)
                        }
                    })
                    .collect::<Vec<_>>();
                let slot_types = self.binding_slot_types(binding_stmt, &value_types);
                for (index, binding) in binding_stmt.bindings.iter().enumerate() {
                    let explicit = binding.ty.as_ref().map(|ty| self.ty_from_type_ref(ty));
                    let inferred = slot_types.get(index).cloned().unwrap_or(Ty::Unknown);
                    let ty = explicit.clone().unwrap_or_else(|| inferred.clone());
                    if let Some(expected) = explicit {
                        self.require_assignable(
                            &inferred,
                            &expected,
                            binding.span,
                            "invalid_binding_type",
                            format!(
                                "cannot assign value of type {} to binding '{}' of type {}",
                                self.diagnostic_type_phrase(&inferred),
                                binding.name,
                                self.diagnostic_type_phrase(&expected)
                            ),
                        );
                    }
                    self.define_local(&binding.name, ty, binding.mutable);
                }
                Ty::unit()
            }
            Stmt::PatternBinding(stmt) => {
                self.check_pattern_binding_stmt(stmt);
                Ty::unit()
            }
            Stmt::Assignment(assignment) => {
                self.check_assignment(assignment);
                Ty::unit()
            }
            Stmt::Defer(stmt) => {
                self.check_defer_stmt(stmt);
                Ty::unit()
            }
            Stmt::If(stmt) => {
                let _ = self.check_if_stmt_value(stmt, &Ty::unit());
                Ty::unit()
            }
            Stmt::Match(stmt) => {
                let _ = self.check_match_stmt_value(stmt, &Ty::unit());
                Ty::unit()
            }
            Stmt::While(stmt) => {
                self.push_scope();
                for condition in &stmt.condition_clauses {
                    match condition {
                        IfConditionClause::Expr(condition) => {
                            let condition_ty = self.check_expr(condition);
                            self.require_bool(
                                &condition_ty,
                                condition.span(),
                                "while condition must be Bool",
                            );
                        }
                        IfConditionClause::Let(clause) => {
                            let value_ty = self.check_expr(&clause.value);
                            self.require_refutable_while_pattern(
                                &clause.pattern,
                                &value_ty,
                                clause.pattern.span(),
                            );
                            self.bind_pattern(&clause.pattern, &value_ty);
                        }
                    }
                }
                self.loop_depth += 1;
                self.check_block(&stmt.body);
                self.loop_depth -= 1;
                self.pop_scope();
                Ty::unit()
            }
            Stmt::For(stmt) => {
                self.push_scope();
                self.loop_depth += 1;
                for binding in &stmt.bindings {
                    self.check_for_binding(binding, false);
                }
                self.check_block(&stmt.body);
                self.loop_depth -= 1;
                self.pop_scope();
                Ty::unit()
            }
            Stmt::LetElse(stmt) => {
                self.check_let_else_stmt(stmt);
                Ty::unit()
            }
            Stmt::Return(return_stmt) => {
                self.check_return_control_expr(return_stmt.value.as_ref(), return_stmt.span)
            }
            Stmt::Break(break_stmt) => self.check_break_control_expr(break_stmt.span),
            Stmt::Continue(continue_stmt) => self.check_continue_control_expr(continue_stmt.span),
            Stmt::Expr(expr_stmt) => {
                let ty = self.check_expr(&expr_stmt.expr);
                self.check_discarded_expr_in_statement(&expr_stmt.expr);
                ty
            }
            Stmt::LocalFunction(function) => {
                let sig = function_sig_from_function(function, &[]);
                self.define_local(
                    &function.name,
                    Ty::Function(
                        sig.params.iter().map(|param| param.ty.clone()).collect(),
                        Box::new(sig.ret.clone()),
                    ),
                    false,
                );
                self.check_function(function);
                Ty::unit()
            }
        }
    }

    fn check_defer_stmt(&mut self, stmt: &crate::ast::DeferStmt) {
        if self.callable_depth == 0 {
            self.add_error(
                "invalid_defer",
                "defer used outside callable body",
                stmt.span,
            );
            return;
        }
        match &stmt.action {
            crate::ast::DeferAction::Call(expr) => {
                let _ = self.check_expr(expr);
            }
            crate::ast::DeferAction::Block(block) => {
                self.defer_depth += 1;
                let _ = self.check_block_against(block, &Ty::unit());
                self.defer_depth -= 1;
            }
        }
    }

    fn check_let_else_stmt(&mut self, stmt: &crate::ast::LetElseStmt) {
        if self.current_return == Ty::Unknown {
            self.add_error(
                "invalid_let_else",
                "let else used outside callable body",
                stmt.span,
            );
            return;
        }
        self.check_block(&stmt.else_block);
        if !self.block_guarantees_control_exit(&stmt.else_block) {
            self.add_error(
                "non_diverging_let_else",
                "let else fallback must exit control flow with 'return', 'break', 'continue', or a call returning Never",
                stmt.else_block.span,
            );
        }
        if !stmt.clauses.is_empty() {
            for clause in &stmt.clauses {
                let value_ty = self.check_expr(&clause.value);
                self.bind_pattern(&clause.pattern, &value_ty);
            }
            return;
        }
        let value_ty = self.check_expr(&stmt.value);
        self.bind_pattern(&stmt.pattern, &value_ty);
    }

    fn check_pattern_binding_stmt(&mut self, stmt: &PatternBindingStmt) {
        if !stmt.clauses.is_empty() {
            for clause in &stmt.clauses {
                let value_ty = self.check_expr(&clause.value);
                self.require_safe_let_pattern(
                    &clause.pattern,
                    &value_ty,
                    &clause.value,
                    clause.pattern.span(),
                );
                self.bind_pattern(&clause.pattern, &value_ty);
            }
            return;
        }
        let value_ty = self.check_expr(&stmt.value);
        self.require_safe_let_pattern(&stmt.pattern, &value_ty, &stmt.value, stmt.pattern.span());
        self.bind_pattern(&stmt.pattern, &value_ty);
    }

    fn block_guarantees_control_exit(&self, block: &Block) -> bool {
        block
            .statements
            .iter()
            .any(|statement| self.stmt_guarantees_control_exit(statement))
    }

    fn stmt_guarantees_control_exit(&self, stmt: &Stmt) -> bool {
        match stmt {
            Stmt::Return(_) | Stmt::Break(_) | Stmt::Continue(_) => true,
            Stmt::Expr(expr_stmt) => self.expr_guarantees_control_exit(&expr_stmt.expr),
            Stmt::If(stmt) => self.if_stmt_guarantees_control_exit(stmt),
            Stmt::Match(stmt) => self.match_stmt_guarantees_control_exit(stmt),
            _ => false,
        }
    }

    fn expr_guarantees_control_exit(&self, expr: &Expr) -> bool {
        match expr {
            Expr::Group { inner, .. } => self.expr_guarantees_control_exit(inner),
            Expr::Return { .. } | Expr::Break { .. } | Expr::Continue { .. } => true,
            Expr::Call {
                callee,
                args,
                uses_brace_syntax,
                ..
            } => self.call_returns_never(callee, args, *uses_brace_syntax),
            _ => false,
        }
    }

    fn call_returns_never(
        &self,
        callee: &Expr,
        args: &[crate::ast::CallArg],
        uses_brace_syntax: bool,
    ) -> bool {
        if self.is_builtin_panic_call(callee) {
            return true;
        }

        if let Some((_, ret)) =
            self.callable_signature_for_args_probe(callee, args, uses_brace_syntax)
        {
            return matches!(ret, Ty::Never);
        }

        matches!(self.probe_expr_type(callee), Ty::Function(_, ret) if matches!(*ret, Ty::Never))
    }

    fn if_stmt_guarantees_control_exit(&self, stmt: &IfStmt) -> bool {
        self.block_guarantees_control_exit(&stmt.then_block)
            && stmt
                .else_branch
                .as_ref()
                .is_some_and(|branch| self.else_branch_guarantees_control_exit(branch))
    }

    fn else_branch_guarantees_control_exit(&self, branch: &ElseBranch) -> bool {
        match branch {
            ElseBranch::If(stmt) => self.if_stmt_guarantees_control_exit(stmt),
            ElseBranch::Block(block) => self.block_guarantees_control_exit(block),
        }
    }

    fn match_stmt_guarantees_control_exit(&self, stmt: &MatchStmt) -> bool {
        !stmt.partial
            && stmt
                .cases
                .iter()
                .all(|case| self.match_case_body_guarantees_control_exit(&case.body))
    }

    fn match_case_body_guarantees_control_exit(&self, body: &MatchCaseBody) -> bool {
        match body {
            MatchCaseBody::Block(block) => self.block_guarantees_control_exit(block),
            MatchCaseBody::Expr(_) => false,
        }
    }

    fn require_refutable_if_pattern(
        &mut self,
        pattern: &Pattern,
        scrutinee: &Ty,
        span: crate::source::Span,
    ) {
        if self.pattern_is_irrefutable(pattern, scrutinee) {
            self.add_error(
                "irrefutable_if_let",
                format!(
                    "if let pattern is irrefutable for value of type '{}'; use 'let' instead",
                    scrutinee.describe()
                ),
                span,
            );
        }
    }

    fn require_refutable_while_pattern(
        &mut self,
        pattern: &Pattern,
        scrutinee: &Ty,
        span: crate::source::Span,
    ) {
        if self.pattern_is_irrefutable(pattern, scrutinee) {
            self.add_error(
                "irrefutable_while_let",
                format!(
                    "while let pattern is irrefutable for value of type '{}'; use a Boolean while condition or bind inside the loop body",
                    scrutinee.describe()
                ),
                span,
            );
        }
    }

    fn require_irrefutable_for_pattern(
        &mut self,
        pattern: &Pattern,
        scrutinee: &Ty,
        span: crate::source::Span,
    ) {
        if !self.for_pattern_is_irrefutable(pattern, scrutinee) {
            self.add_error(
                "refutable_for_pattern",
                format!(
                    "for pattern must be irrefutable for value of type '{}'",
                    scrutinee.describe()
                ),
                span,
            );
        }
    }

    fn for_pattern_is_irrefutable(&self, pattern: &Pattern, scrutinee: &Ty) -> bool {
        match pattern {
            Pattern::Wildcard { .. } | Pattern::Binding { .. } => true,
            Pattern::Extract { .. } | Pattern::Literal { .. } | Pattern::Type { .. } => false,
            Pattern::Tuple { elements, .. } => match scrutinee {
                Ty::Tuple(items) if items.len() == elements.len() => elements
                    .iter()
                    .zip(items.iter())
                    .all(|(pattern, item)| self.for_pattern_is_irrefutable(pattern, item)),
                _ => false,
            },
            Pattern::List { elements, rest, .. } => {
                elements.is_empty() && rest.is_some() && self.list_element_type(scrutinee).is_some()
            }
            Pattern::Record { path, fields, .. } => {
                let Some((is_enum_case, target_ty, target_fields)) =
                    self.lookup_record_pattern_target(path, scrutinee)
                else {
                    return false;
                };
                !is_enum_case
                    && self.is_assignable(scrutinee, &target_ty)
                    && fields.iter().all(|field| {
                        target_fields
                            .iter()
                            .find(|(name, _)| name == &field.name)
                            .is_some_and(|(_, ty)| {
                                self.for_pattern_is_irrefutable(&field.pattern, ty)
                            })
                    })
            }
            Pattern::Constructor { path, args, .. } => {
                if self.lookup_case_by_pattern(path, scrutinee).is_some() {
                    return false;
                }
                let Some((destructured_ty, field_tys)) =
                    self.lookup_destructured_type_pattern(path)
                else {
                    return false;
                };
                self.is_assignable(scrutinee, &destructured_ty)
                    && args.len() == field_tys.len()
                    && args
                        .iter()
                        .zip(field_tys.iter())
                        .all(|(pattern, field_ty)| {
                            self.for_pattern_is_irrefutable(pattern, field_ty)
                        })
            }
        }
    }

    fn require_safe_let_pattern(
        &mut self,
        pattern: &Pattern,
        scrutinee: &Ty,
        source: &Expr,
        span: crate::source::Span,
    ) {
        if !self.pattern_is_irrefutable(pattern, scrutinee)
            && !self.source_expr_proves_pattern_match(pattern, source)
        {
            self.add_error(
                "refutable_let_pattern",
                format!(
                    "plain 'let' pattern may fail for value of type '{}'; add an 'else' fallback",
                    scrutinee.describe()
                ),
                span,
            );
        }
    }

    fn source_expr_proves_pattern_match(&self, pattern: &Pattern, source: &Expr) -> bool {
        match (pattern, source) {
            (pattern, Expr::Group { inner, .. }) => {
                self.source_expr_proves_pattern_match(pattern, inner)
            }
            (Pattern::Wildcard { .. } | Pattern::Binding { .. }, _) => true,
            // `<-` extraction always needs an explicit fallback. Otherwise a
            // harmless refactor from `Some(5)` to `value Option[Int] = Some(5)`
            // changes whether control flow is required.
            (Pattern::Extract { .. }, _) => false,
            (Pattern::Tuple { elements, .. }, Expr::TupleLiteral { items, .. })
                if elements.len() == items.len() =>
            {
                elements
                    .iter()
                    .zip(items.iter())
                    .all(|(pattern, item)| self.source_expr_proves_pattern_match(pattern, item))
            }
            (Pattern::List { elements, rest, .. }, Expr::ListLiteral { items, .. })
                if !items.iter().any(|item| matches!(item, Expr::Spread { .. }))
                    && (rest.is_some() || elements.len() == items.len()) =>
            {
                items.len() >= elements.len()
                    && elements
                        .iter()
                        .zip(items.iter())
                        .all(|(pattern, item)| self.source_expr_proves_pattern_match(pattern, item))
            }
            (Pattern::Record { .. }, _) => false,
            (Pattern::Constructor { path, args, .. }, source) => {
                let Some((source_path, source_args)) = self.constructor_expr_parts(source) else {
                    return false;
                };
                source_path.as_slice() == path.as_slice()
                    && source_args.len() == args.len()
                    && args
                        .iter()
                        .zip(source_args.iter())
                        .all(|(pattern, item)| self.source_expr_proves_pattern_match(pattern, item))
            }
            (Pattern::Literal { value, .. }, source) => self.literal_exprs_equal(value, source),
            (Pattern::Type { .. }, _) => false,
            _ => false,
        }
    }

    fn constructor_expr_parts<'b>(&self, source: &'b Expr) -> Option<(Vec<String>, Vec<&'b Expr>)> {
        match source {
            Expr::Group { inner, .. } => self.constructor_expr_parts(inner),
            Expr::Call { callee, args, .. } => {
                let path = expr_path_for_known_value(callee)?;
                if self.lookup_case_by_path(&path).is_none()
                    && self.lookup_destructured_type_pattern(&path).is_none()
                {
                    return None;
                }
                Some((path, args.iter().map(|arg| &arg.value).collect()))
            }
            other => {
                let path = expr_path_for_known_value(other)?;
                let case = self.lookup_case_by_path(&path)?;
                case.params.is_empty().then_some((path, Vec::new()))
            }
        }
    }

    fn literal_exprs_equal(&self, left: &Expr, right: &Expr) -> bool {
        match (left, right) {
            (left, Expr::Group { inner, .. }) => self.literal_exprs_equal(left, inner),
            (Expr::Unit { .. }, Expr::Unit { .. }) => true,
            (Expr::Bool { value: left, .. }, Expr::Bool { value: right, .. }) => left == right,
            (Expr::Integer { raw: left, .. }, Expr::Integer { raw: right, .. }) => {
                left.parse::<i64>().ok() == right.parse::<i64>().ok()
            }
            (Expr::Float { raw: left, .. }, Expr::Float { raw: right, .. }) => left == right,
            (Expr::String { raw: left, .. }, Expr::String { raw: right, .. }) => left == right,
            _ => false,
        }
    }

    fn check_match_case_against(&mut self, case: &MatchCase, value_ty: &Ty, expected: &Ty) -> Ty {
        self.push_scope();
        self.bind_pattern(&case.pattern, value_ty);
        if let Some(guard) = &case.guard {
            let guard_ty = self.check_expr(guard);
            self.require_bool(&guard_ty, guard.span(), "match guard must be Bool");
        }
        let ty = match &case.body {
            MatchCaseBody::Block(block) => self.check_block_against(block, expected),
            MatchCaseBody::Expr(expr) => self.check_expr_against(expr, expected),
        };
        self.pop_scope();
        ty
    }

    fn check_for_binding(
        &mut self,
        binding: &ForBinding,
        allow_lifted: bool,
    ) -> Option<ForYieldFamily> {
        if let Some(pattern) = &binding.pattern {
            let (value_ty, family) = if let Some(iterable) = &binding.iterable {
                let iterable_ty = self.check_expr(iterable);
                self.for_generator_source(&iterable_ty, allow_lifted, iterable.span())
            } else {
                (
                    binding
                        .values
                        .first()
                        .map(|expr| self.check_expr(expr))
                        .unwrap_or(Ty::Unknown),
                    None,
                )
            };
            self.require_irrefutable_for_pattern(pattern, &value_ty, pattern.span());
            self.bind_pattern(pattern, &value_ty);
            return family;
        }
        let mut generator_family = None;
        let slot_types = if let Some(iterable) = &binding.iterable {
            let iterable_ty = self.check_expr(iterable);
            let (item_ty, family) =
                self.for_generator_source(&iterable_ty, allow_lifted, iterable.span());
            generator_family = family;
            if let Some(kind) = binding.destructure {
                self.check_destructure_source(&item_ty, kind, binding.bindings.len(), binding.span);
                if kind == DestructureKind::Record {
                    self.record_binding_slot_types(&item_ty, &binding.bindings, binding.span)
                } else {
                    self.destructure_slots(&item_ty, binding.bindings.len(), kind)
                }
            } else {
                vec![item_ty]
            }
        } else {
            let value_types = binding
                .values
                .iter()
                .enumerate()
                .map(|(index, expr)| {
                    if binding.bindings.len() == 1 {
                        if let Some(expected) = binding.bindings[0]
                            .ty
                            .as_ref()
                            .map(|ty| self.ty_from_type_ref(ty))
                        {
                            return self.check_expr_against(expr, &expected);
                        }
                    }
                    if let Some(expected) = binding
                        .bindings
                        .get(index)
                        .and_then(|binding| binding.ty.as_ref())
                        .map(|ty| self.ty_from_type_ref(ty))
                    {
                        self.check_expr_against(expr, &expected)
                    } else {
                        self.check_expr(expr)
                    }
                })
                .collect::<Vec<_>>();
            if let Some(kind) = binding.destructure {
                let value_ty = value_types.first().cloned().unwrap_or(Ty::Unknown);
                self.check_destructure_source(
                    &value_ty,
                    kind,
                    binding.bindings.len(),
                    binding.span,
                );
                if kind == DestructureKind::Record {
                    self.record_binding_slot_types(&value_ty, &binding.bindings, binding.span)
                } else {
                    self.destructure_slots(&value_ty, binding.bindings.len(), kind)
                }
            } else {
                (0..binding.bindings.len())
                    .map(|index| value_types.get(index).cloned().unwrap_or(Ty::Unknown))
                    .collect()
            }
        };
        for (index, local) in binding.bindings.iter().enumerate() {
            let inferred = slot_types.get(index).cloned().unwrap_or(Ty::Unknown);
            let explicit = local.ty.as_ref().map(|ty| self.ty_from_type_ref(ty));
            let ty = explicit.clone().unwrap_or_else(|| inferred.clone());
            self.define_local(&local.name, ty, local.mutable);
        }
        generator_family
    }

    fn for_generator_source(
        &mut self,
        source_ty: &Ty,
        allow_lifted: bool,
        span: crate::source::Span,
    ) -> (Ty, Option<ForYieldFamily>) {
        if let Some(item_ty) = self.known_iterable_item_type(source_ty) {
            return (item_ty, Some(ForYieldFamily::Iterable));
        }
        if allow_lifted {
            if let Some((family, item_ty)) = self.unwrap_known_lifted_type(source_ty) {
                return (item_ty, Some(family));
            }
        }
        if matches!(source_ty, Ty::Unknown) {
            return (Ty::Unknown, Some(ForYieldFamily::Unknown));
        }
        let allowed = if allow_lifted {
            "Iterable, Iterator, Option, Result, or Either"
        } else {
            "Iterable or Iterator"
        };
        self.add_error(
            "invalid_for_generator_source",
            format!(
                "for generator source must be {allowed}, got '{}'",
                source_ty.describe()
            ),
            span,
        );
        (Ty::Unknown, None)
    }

    fn unwrap_known_lifted_type(&self, ty: &Ty) -> Option<(ForYieldFamily, Ty)> {
        match ty {
            Ty::Named(name, args) if name == "Option" && args.len() == 1 => {
                Some((ForYieldFamily::Option, args[0].clone()))
            }
            Ty::Named(name, args) if name == "Result" && args.len() == 2 => Some((
                ForYieldFamily::Result {
                    error: args[1].clone(),
                },
                args[0].clone(),
            )),
            Ty::Named(name, args) if name == "Either" && args.len() == 2 => Some((
                ForYieldFamily::Either {
                    left: args[0].clone(),
                },
                args[1].clone(),
            )),
            _ => None,
        }
    }

    fn choose_for_yield_family(
        &mut self,
        current: Option<ForYieldFamily>,
        next: ForYieldFamily,
        span: crate::source::Span,
    ) -> Option<ForYieldFamily> {
        let Some(current_family) = current else {
            return Some(next);
        };
        match (&current_family, &next) {
            (ForYieldFamily::Unknown, _) => Some(next),
            (_, ForYieldFamily::Unknown) => Some(current_family),
            (ForYieldFamily::Iterable, ForYieldFamily::Iterable) => Some(current_family),
            (ForYieldFamily::Option, ForYieldFamily::Option) => Some(current_family),
            (ForYieldFamily::Result { error }, ForYieldFamily::Result { error: next_error }) => {
                if self.is_assignable(next_error, error) {
                    Some(current_family)
                } else {
                    self.add_error(
                        "incompatible_for_yield_generator",
                        format!(
                            "for-yield Result generator has failure type '{}', but the comprehension already uses Result failure type '{}'; convert the failure explicitly before the generator",
                            next_error.describe(),
                            error.describe()
                        ),
                        span,
                    );
                    Some(current_family)
                }
            }
            (ForYieldFamily::Either { left }, ForYieldFamily::Either { left: next_left }) => {
                if self.is_assignable(next_left, left) {
                    Some(current_family)
                } else {
                    self.add_error(
                        "incompatible_for_yield_generator",
                        format!(
                            "for-yield Either generator has left type '{}', but the comprehension already uses Either left type '{}'; convert the left value explicitly before the generator",
                            next_left.describe(),
                            left.describe()
                        ),
                        span,
                    );
                    Some(current_family)
                }
            }
            _ => {
                self.add_error(
                    "incompatible_for_yield_generator",
                    format!(
                        "for-yield generators must use one source family; cannot mix {} with {}",
                        describe_for_yield_family(&current_family),
                        describe_for_yield_family(&next)
                    ),
                    span,
                );
                Some(current_family)
            }
        }
    }

    fn wrap_for_yield_type(&self, family: Option<&ForYieldFamily>, inner: Ty) -> Ty {
        match family {
            Some(ForYieldFamily::Option) => Ty::Named("Option".to_string(), vec![inner]),
            Some(ForYieldFamily::Result { error }) => {
                Ty::Named("Result".to_string(), vec![inner, error.clone()])
            }
            Some(ForYieldFamily::Either { left }) => {
                Ty::Named("Either".to_string(), vec![left.clone(), inner])
            }
            _ => Ty::list(inner),
        }
    }

    fn binding_slot_types(&mut self, binding_stmt: &BindingStmt, value_types: &[Ty]) -> Vec<Ty> {
        if let Some(kind) = binding_stmt.destructure {
            let value_ty = value_types.first().cloned().unwrap_or(Ty::Unknown);
            self.check_destructure_source(
                &value_ty,
                kind,
                binding_stmt.bindings.len(),
                binding_stmt.span,
            );
            if kind == DestructureKind::Record {
                return self.record_binding_slot_types(
                    &value_ty,
                    &binding_stmt.bindings,
                    binding_stmt.span,
                );
            }
            return self.destructure_slots(&value_ty, binding_stmt.bindings.len(), kind);
        }
        (0..binding_stmt.bindings.len())
            .map(|index| value_types.get(index).cloned().unwrap_or(Ty::Unknown))
            .collect()
    }

    fn record_binding_slot_types(
        &mut self,
        ty: &Ty,
        bindings: &[crate::ast::Binding],
        _span: crate::source::Span,
    ) -> Vec<Ty> {
        if matches!(ty, Ty::Unknown) {
            return vec![Ty::Unknown; bindings.len()];
        }
        let Some(fields) = self.destructure_record_fields(ty) else {
            return vec![Ty::Unknown; bindings.len()];
        };
        let field_map = fields
            .into_iter()
            .map(|(name, ty, hidden)| (name, (ty, hidden)))
            .collect::<HashMap<_, _>>();
        let mut seen = HashSet::new();
        bindings
            .iter()
            .map(|binding| {
                let field_name = binding
                    .field_name
                    .clone()
                    .or_else(|| {
                        if binding.name == "_" {
                            self.add_error(
                                "invalid_destructure",
                                "brace destructuring matches by field name; omit fields you do not need",
                                binding.span,
                            );
                            None
                        } else {
                            Some(binding.name.clone())
                        }
                    });
                let Some(field_name) = field_name else {
                    return Ty::Unknown;
                };
                if !seen.insert(field_name.clone()) {
                    self.add_error(
                        "invalid_destructure",
                        format!("duplicate named destructuring field '{}'", field_name),
                        binding.span,
                    );
                }
                let Some((field_ty, hidden)) = field_map.get(&field_name).cloned() else {
                    self.add_error(
                        "invalid_destructure",
                        format!(
                            "type '{}' does not have a field named '{}'",
                            ty.describe(),
                            field_name
                        ),
                        binding.span,
                    );
                    return Ty::Unknown;
                };
                if hidden {
                    self.add_error(
                        "invalid_destructure",
                        format!("cannot destructure hidden field '{}'", field_name),
                        binding.span,
                    );
                    return Ty::Unknown;
                }
                field_ty
            })
            .collect()
    }

    fn destructure_record_fields(&self, ty: &Ty) -> Option<Vec<(String, Ty, bool)>> {
        match ty {
            Ty::Record(fields) => Some(
                fields
                    .iter()
                    .map(|(name, ty)| (name.clone(), ty.clone(), false))
                    .collect(),
            ),
            Ty::Named(name, _) => self.lookup_any_type(name).map(|sig| {
                sig.fields
                    .iter()
                    .map(|field| (field.name.clone(), field.ty.clone(), field.hidden))
                    .collect()
            }),
            _ => None,
        }
    }

    fn check_destructure_source(
        &mut self,
        ty: &Ty,
        kind: DestructureKind,
        count: usize,
        span: crate::source::Span,
    ) {
        if count <= 1 || matches!(ty, Ty::Unknown) {
            return;
        }

        let valid = match kind {
            DestructureKind::Tuple => matches!(ty, Ty::Tuple(_)),
            DestructureKind::Record => matches!(ty, Ty::Record(_) | Ty::Named(_, _)),
        };
        if valid {
            return;
        }

        let message = match kind {
            DestructureKind::Tuple => {
                format!(
                    "tuple destructuring requires a tuple value, got '{}'",
                    ty.describe()
                )
            }
            DestructureKind::Record => format!(
                "brace destructuring requires a class or anonymous shape value, got '{}'",
                ty.describe()
            ),
        };
        self.add_error("invalid_destructure", message, span);
    }

    fn destructure_slots(&self, ty: &Ty, count: usize, kind: DestructureKind) -> Vec<Ty> {
        if count <= 1 {
            return vec![ty.clone()];
        }
        let mut slots = match ty {
            Ty::Tuple(items) if kind == DestructureKind::Tuple => items.clone(),
            Ty::Record(fields) if kind == DestructureKind::Record => {
                fields.iter().map(|(_, ty)| ty.clone()).collect()
            }
            Ty::Named(name, _) if kind == DestructureKind::Record => self
                .lookup_any_type(name)
                .map(|sig| {
                    sig.fields
                        .iter()
                        .filter(|field| !field.hidden)
                        .map(|field| field.ty.clone())
                        .collect()
                })
                .unwrap_or_default(),
            Ty::Unknown => vec![Ty::Unknown; count],
            _ => Vec::new(),
        };
        if slots.len() < count {
            slots.resize(count, Ty::Unknown);
        }
        slots
    }

    fn check_assignment(&mut self, assignment: &AssignmentStmt) {
        let expected_types = assignment
            .targets
            .iter()
            .map(|target| self.assignment_target_type(target, assignment.operator))
            .collect::<Vec<_>>();
        let value_types = assignment
            .values
            .iter()
            .enumerate()
            .map(|(index, expr)| match expected_types.get(index) {
                Some(expected)
                    if matches!(assignment.operator, AssignOp::Assign | AssignOp::Reassign) =>
                {
                    self.check_expr_against(expr, expected)
                }
                _ => self.check_expr(expr),
            })
            .collect::<Vec<_>>();
        for (index, target) in assignment.targets.iter().enumerate() {
            let actual = value_types.get(index).cloned().unwrap_or(Ty::Unknown);
            let expected = expected_types.get(index).cloned().unwrap_or(Ty::Unknown);
            self.require_assignable(
                &actual,
                &expected,
                target.span(),
                "invalid_assignment_type",
                format!(
                    "cannot assign value of type {} to target of type {}",
                    self.diagnostic_type_phrase(&actual),
                    self.diagnostic_type_phrase(&expected)
                ),
            );
            if !matches!(assignment.operator, AssignOp::Assign | AssignOp::Reassign)
                && !expected.is_numeric()
                && !expected.is_str()
                && !matches!(expected, Ty::Unknown)
            {
                self.add_error(
                    "invalid_assignment_operator",
                    "operator assignment currently expects a numeric or string target",
                    target.span(),
                );
            }
        }
    }

    fn assignment_target_type(&mut self, target: &Expr, operator: AssignOp) -> Ty {
        match target {
            Expr::Identifier { name, span } => {
                if let Some(value) = self.lookup_scoped_value(name) {
                    if operator == AssignOp::Assign {
                        self.add_error(
                            "unexpected_token",
                            "use ':=' for reassignment; '=' is only for bindings and constructor initialization",
                            *span,
                        );
                    } else if !value.mutable {
                        self.add_error(
                            "assign_immutable",
                            format!("cannot assign to immutable binding '{}'", name),
                            *span,
                        );
                    }
                    value.ty
                } else if let Some(field) = self.lookup_implicit_field(name) {
                    let can_initialize = self.current_method.as_deref() == Some("new");
                    if operator == AssignOp::Assign {
                        if !can_initialize {
                            self.add_error(
                                "unexpected_token",
                                "use ':=' for reassignment; '=' is only for bindings and constructor initialization",
                                *span,
                            );
                        }
                    } else if can_initialize {
                        self.add_error(
                            "unexpected_token",
                            "use '=' for field initialization in constructors; reassignment operators are only for mutation after construction",
                            *span,
                        );
                    } else if !field.mutable {
                        self.add_error(
                            "assign_immutable",
                            format!("cannot reassign immutable field '{}'", name),
                            *span,
                        );
                    }
                    field.ty
                } else if let Some(value) = self.lookup_global_value(name) {
                    if operator == AssignOp::Assign {
                        self.add_error(
                            "unexpected_token",
                            "use ':=' for reassignment; '=' is only for bindings and constructor initialization",
                            *span,
                        );
                    } else if !value.mutable {
                        self.add_error(
                            "assign_immutable",
                            format!("cannot assign to immutable binding '{}'", name),
                            *span,
                        );
                    }
                    value.ty
                } else {
                    self.add_error("undefined_name", self.undefined_value_message(name), *span);
                    Ty::Unknown
                }
            }
            Expr::Member {
                receiver,
                name,
                span,
            } => {
                let receiver_ty = self.check_expr(receiver);
                if let Some(field) = self.field_sig_for_member(&receiver_ty, name) {
                    if self.extension_this_hidden_field(receiver, &receiver_ty, name) {
                        self.add_extension_hidden_access_error("field", name, *span);
                        return Ty::Unknown;
                    }
                    let can_initialize =
                        self.can_initialize_field_in_constructor(receiver, &receiver_ty, name);
                    if operator == AssignOp::Assign {
                        if !can_initialize {
                            self.add_error(
                                "unexpected_token",
                                "use ':=' for reassignment; '=' is only for bindings and constructor initialization",
                                *span,
                            );
                        }
                    } else if can_initialize {
                        self.add_error(
                            "unexpected_token",
                            "use '=' for field initialization in constructors; reassignment operators are only for mutation after construction",
                            *span,
                        );
                    } else if !field.mutable {
                        self.add_error(
                            "assign_immutable",
                            format!("cannot reassign immutable field '{}'", name),
                            *span,
                        );
                    }
                    return field.ty.clone();
                }
                self.member_type(&receiver_ty, name).unwrap_or_else(|| {
                    self.add_error(
                        "unknown_member",
                        self.unknown_member_message(receiver, &receiver_ty, name),
                        *span,
                    );
                    Ty::Unknown
                })
            }
            Expr::Index {
                receiver,
                index,
                span,
            } => {
                if operator == AssignOp::Assign {
                    self.add_error(
                        "unexpected_token",
                        "use ':=' for reassignment; '=' is only for bindings and constructor initialization",
                        *span,
                    );
                }
                let receiver_ty = self.check_expr(receiver);
                let index_ty = self.check_expr(index);
                match &receiver_ty {
                    Ty::Named(name, args) if name == "LinkedList" && args.len() == 1 => {
                        self.add_error(
                            "linked_list_indexed_assignment",
                            "LinkedList does not support indexed assignment; use 'setAt(index, value)'",
                            *span,
                        );
                        Ty::Unknown
                    }
                    Ty::Named(name, args)
                        if (name == "Vector" || name == "Array") && args.len() == 1 =>
                    {
                        if !index_ty.is_int_like() && !matches!(index_ty, Ty::Unknown) {
                            self.add_error(
                                "invalid_index_type",
                                format!(
                                    "index assignment expects Int, got '{}'",
                                    index_ty.describe()
                                ),
                                index.span(),
                            );
                        }
                        args[0].clone()
                    }
                    Ty::Named(name, args) if name == "Map" && args.len() == 2 => {
                        if !self.is_assignable(&index_ty, &args[0])
                            && !matches!(index_ty, Ty::Unknown)
                        {
                            self.add_error(
                                "invalid_index_type",
                                format!(
                                    "map index assignment expects '{}', got '{}'",
                                    args[0].describe(),
                                    index_ty.describe()
                                ),
                                index.span(),
                            );
                        }
                        args[1].clone()
                    }
                    Ty::Unknown => Ty::Unknown,
                    _ => {
                        self.add_error(
                            "invalid_assignment_target",
                            format!(
                                "indexed assignment is not supported for '{}'",
                                receiver_ty.describe()
                            ),
                            *span,
                        );
                        Ty::Unknown
                    }
                }
            }
            other => {
                self.add_error(
                    "invalid_assignment_target",
                    "invalid assignment target",
                    other.span(),
                );
                Ty::Unknown
            }
        }
    }

    fn check_expr(&mut self, expr: &Expr) -> Ty {
        self.check_expr_against(expr, &Ty::Unknown)
    }

    fn reject_tuple_literal_against_shape_like(
        &mut self,
        items: &[Expr],
        expected: &Ty,
        span: crate::source::Span,
    ) -> Option<Ty> {
        let message = match expected {
            Ty::Record(_) => {
                "tuple values cannot construct anonymous shape; use shape(...) with an expected anonymous shape type or named fields like '{ field: value }'".to_string()
            }
            Ty::Named(name, _) => {
                let sig = self.lookup_any_type(name)?;
                match sig.kind {
                    TypeKind::Record => format!(
                        "tuple values cannot construct shape '{}'; use '{}(...)' or '{} {{ ... }}'",
                        sig.name, sig.name, sig.name
                    ),
                    TypeKind::Class => format!(
                        "tuple values cannot construct class '{}'; use '{}' constructors",
                        sig.name, sig.name
                    ),
                    _ => return None,
                }
            }
            _ => return None,
        };
        for item in items {
            self.check_expr(item);
        }
        self.add_error("invalid_tuple_shape_conversion", message, span);
        Some(materialize_type(expected))
    }

    fn check_shape_literal_expr(
        &mut self,
        items: &[Expr],
        expected: &Ty,
        span: crate::source::Span,
    ) -> Ty {
        match expected {
            Ty::Record(fields) => {
                if items.len() != fields.len() {
                    self.add_error(
                        "invalid_shape_argument_count",
                        format!(
                            "shape(...) for anonymous shape expects {} values, got {}; values map to fields in written order",
                            fields.len(),
                            items.len()
                        ),
                        span,
                    );
                }

                for (index, item) in items.iter().enumerate() {
                    if let Some((name, expected_ty)) = fields.get(index) {
                        let actual = self.check_expr_against(item, expected_ty);
                        self.require_assignable(
                            &actual,
                            expected_ty,
                            item.span(),
                            "invalid_argument_type",
                            format!(
                                "shape field '{}' expects '{}' but got '{}'",
                                name,
                                expected_ty.describe(),
                                actual.describe()
                            ),
                        );
                    } else {
                        self.check_expr(item);
                    }
                }

                Ty::Record(
                    fields
                        .iter()
                        .map(|(name, ty)| (name.clone(), materialize_type(ty)))
                        .collect(),
                )
            }
            Ty::Named(name, _) => {
                let kind = self.lookup_any_type(name).map(|sig| sig.kind);
                for item in items {
                    self.check_expr(item);
                }
                let message = match kind {
                    Some(TypeKind::Record) => format!(
                        "shape(...) constructs anonymous shapes only; use '{}(...)' for named shape construction",
                        name
                    ),
                    Some(TypeKind::Class) => format!(
                        "shape(...) constructs anonymous shapes only; use '{}' constructors for class construction",
                        name
                    ),
                    _ => "shape(...) requires an expected anonymous shape type".to_string(),
                };
                self.add_error("missing_shape_context", message, span);
                Ty::Unknown
            }
            _ => {
                for item in items {
                    self.check_expr(item);
                }
                self.add_error(
                    "missing_shape_context",
                    "shape(...) requires an expected anonymous shape type; add an anonymous shape annotation like `value { name Str, age Int } = shape(...)`",
                    span,
                );
                Ty::Unknown
            }
        }
    }

    fn check_expr_against(&mut self, expr: &Expr, expected: &Ty) -> Ty {
        match expr {
            Expr::Identifier { name, span } => self
                .lookup_scoped_value(name)
                .map(|value| value.ty)
                .or_else(|| self.lookup_implicit_field(name).map(|field| field.ty))
                .or_else(|| self.lookup_global_value(name).map(|value| value.ty))
                .or_else(|| self.lookup_function_type(name))
                .or_else(|| self.lookup_bare_enum_case_value_type(name, expected))
                .or_else(|| self.lookup_named_constructor_type(name))
                .unwrap_or_else(|| {
                    self.add_error("undefined_name", self.undefined_value_message(name), *span);
                    Ty::Unknown
                }),
            Expr::Placeholder { span } => {
                self.add_error(
                    "invalid_placeholder_expr",
                    "'_' is not a value; it only marks an ignored pattern or explicit lambda parameter slot",
                    *span,
                );
                let _ = expected;
                Ty::Unknown
            }
            Expr::Integer { .. } => Ty::int(),
            Expr::Float { .. } => Ty::float(),
            Expr::String { .. } => Ty::str(),
            Expr::Bool { .. } => Ty::bool(),
            Expr::Unit { .. } => Ty::unit(),
            Expr::ListLiteral { items, span } => {
                if items.is_empty() {
                    return match expected {
                        Ty::Named(name, args) if name == "Vector" && args.len() == 1 => {
                            Ty::Named(name.clone(), args.iter().map(materialize_type).collect())
                        }
                        Ty::Named(name, args) if name == "Map" && args.len() == 2 => {
                            Ty::Named(name.clone(), args.iter().map(materialize_type).collect())
                        }
                        Ty::Unknown => {
                            self.add_error(
                                "cannot_infer_empty_collection_type",
                                "cannot infer the type of empty collection '[]'; add a vector or map type annotation",
                                *span,
                            );
                            Ty::Unknown
                        }
                        other => {
                            self.add_error(
                                "invalid_empty_collection_context",
                                format!(
                                    "empty collection '[]' requires a vector or map type, got '{}'",
                                    other.describe()
                                ),
                                *span,
                            );
                            Ty::Unknown
                        }
                    };
                }
                if let Some(collection_ty) =
                    self.check_spread_only_collection_literal(items, expected)
                {
                    return collection_ty;
                }
                let mut item_ty = Ty::Unknown;
                for item in items {
                    let current = if let Expr::Spread { value, span, .. } = item {
                        let spread_ty = self.check_expr(value);
                        if is_map_ty(&spread_ty) {
                            self.add_error(
                                "invalid_vector_spread",
                                "cannot spread a map into a vector literal",
                                *span,
                            );
                            Ty::Unknown
                        } else if let Some(item_ty) = self.known_iterable_item_type(&spread_ty) {
                            item_ty
                        } else {
                            if !matches!(spread_ty, Ty::Unknown) {
                                self.add_error(
                                    "invalid_vector_spread",
                                    format!(
                                        "vector spread requires an iterable value, got '{}'",
                                        spread_ty.describe()
                                    ),
                                    *span,
                                );
                            }
                            Ty::Unknown
                        }
                    } else {
                        self.check_expr(item)
                    };
                    item_ty = join_types(&item_ty, &current);
                }
                Ty::list(item_ty)
            }
            Expr::Spread { span, .. } => {
                self.add_error(
                    "invalid_spread",
                    "spread syntax is only valid inside vector or map literals and positional vararg call arguments",
                    *span,
                );
                Ty::Unknown
            }
            Expr::TupleLiteral { items, span } => {
                if let Some(ty) =
                    self.reject_tuple_literal_against_shape_like(items, expected, *span)
                {
                    ty
                } else {
                    Ty::Tuple(items.iter().map(|item| self.check_expr(item)).collect())
                }
            }
            Expr::ShapeLiteral { items, span } => {
                self.check_shape_literal_expr(items, expected, *span)
            }
            Expr::Call {
                callee,
                args,
                uses_brace_syntax,
                span,
            } => self.check_call(callee, args, *uses_brace_syntax, *span, expected),
            Expr::Member {
                receiver,
                name,
                span,
            } => {
                if let Some(ty) = self.module_member_value_type(expr) {
                    return ty;
                }
                if let Some(ty) = self.static_member_value_type(receiver, name, expected) {
                    return ty;
                }
                let receiver_ty = self.check_expr(receiver);
                if name == "runtimeType" {
                    return Ty::value_runtime_type(receiver_ty);
                }
                if self.extension_this_hidden_field(receiver, &receiver_ty, name) {
                    self.add_extension_hidden_access_error("field", name, *span);
                    return Ty::Unknown;
                }
                if self.extension_this_hidden_method(receiver, &receiver_ty, name) {
                    self.add_extension_hidden_access_error("method", name, *span);
                    return Ty::Unknown;
                }
                self.member_type(&receiver_ty, name).unwrap_or_else(|| {
                    self.add_error(
                        "unknown_member",
                        self.unknown_member_message(receiver, &receiver_ty, name),
                        *span,
                    );
                    Ty::Unknown
                })
            }
            Expr::Index {
                receiver,
                index,
                span,
            } => {
                let receiver_ty = self.check_expr(receiver);
                let index_ty = self.check_expr(index);
                if matches!(&receiver_ty, Ty::Named(name, args) if name == "LinkedList" && args.len() == 1)
                {
                    self.add_error(
                        "linked_list_indexed_access",
                        "LinkedList does not support indexed access; use 'at(index)'",
                        *span,
                    );
                    return Ty::Unknown;
                }
                let valid_index = match &receiver_ty {
                    Ty::Named(name, args)
                        if (name == "Vector" || name == "Array") && args.len() == 1 =>
                    {
                        index_ty.is_int_like() || matches!(index_ty, Ty::Unknown)
                    }
                    Ty::Named(name, args) if name == "Map" && args.len() == 2 => {
                        self.is_assignable(&index_ty, &args[0]) || matches!(index_ty, Ty::Unknown)
                    }
                    _ => index_ty.is_int_like() || matches!(index_ty, Ty::Unknown),
                };
                if !valid_index {
                    self.add_error(
                        "invalid_index_type",
                        match &receiver_ty {
                            Ty::Named(name, args) if name == "Map" && args.len() == 2 => format!(
                                "index expression expects '{}', got '{}'",
                                args[0].describe(),
                                index_ty.describe()
                            ),
                            _ => format!(
                                "index expression expects Int, got '{}'",
                                index_ty.describe()
                            ),
                        },
                        index.span(),
                    );
                }
                self.index_result_type(&receiver_ty)
            }
            Expr::RecordUpdate {
                receiver,
                patch,
                span,
            } => {
                let base = self.check_expr(receiver);
                self.check_record_update(&base, patch, *span);
                base
            }
            Expr::RecordLiteral { fields, values, .. } => {
                if !fields.is_empty() {
                    let expected_fields = match expected {
                        Ty::Record(fields) => fields.as_slice(),
                        _ => &[],
                    };
                    let explicit_names = fields
                        .iter()
                        .filter_map(|field| field.name.clone())
                        .collect::<HashSet<_>>();
                    let mut explicit_seen = HashSet::new();
                    let mut providers = HashMap::<String, ShapeFieldProvider>::new();
                    let mut actual_fields = Vec::new();
                    for field in fields {
                        if let Some(name) = &field.name {
                            let annotated_ty =
                                field.ty.as_ref().map(|ty| self.ty_from_type_ref(ty));
                            let expected_ty = annotated_ty.clone().unwrap_or_else(|| {
                                expected_fields
                                    .iter()
                                    .find(|(expected_name, _)| expected_name == name)
                                    .map(|(_, ty)| ty.clone())
                                    .unwrap_or(Ty::Unknown)
                            });
                            let actual = self.check_expr_against(&field.value, &expected_ty);
                            let field_ty = if let Some(annotated_ty) = annotated_ty {
                                self.require_assignable(
                                    &actual,
                                    &annotated_ty,
                                    field.span,
                                    "invalid_field_initializer_type",
                                    format!(
                                        "field '{}' is annotated as '{}' but initializer has type '{}'",
                                        name,
                                        annotated_ty.describe(),
                                        actual.describe()
                                    ),
                                );
                                annotated_ty
                            } else {
                                actual
                            };
                            if !explicit_seen.insert(name.clone()) {
                                self.add_error(
                                    "duplicate_shape_field",
                                    format!("duplicate shape field '{}'", name),
                                    field.span,
                                );
                            }
                            upsert_shape_field(&mut actual_fields, name.clone(), field_ty);
                            providers.insert(
                                name.clone(),
                                ShapeFieldProvider {
                                    source: format!("explicit field '{}'", name),
                                    conflicting_source: None,
                                    span: field.span,
                                },
                            );
                        } else {
                            let Expr::Spread {
                                value,
                                override_existing,
                                ..
                            } = &field.value
                            else {
                                self.add_error(
                                    "invalid_shape_spread",
                                    "internal shape spread representation is invalid",
                                    field.span,
                                );
                                continue;
                            };
                            let spread_ty = self.check_expr(value);
                            if let Some(spread_fields) =
                                self.record_spread_shape_fields(&spread_ty, field.span)
                            {
                                let source = self
                                    .describe_member_path(value)
                                    .unwrap_or_else(|| "spread expression".to_string());
                                for (name, ty) in spread_fields {
                                    if explicit_names.contains(&name) {
                                        if !actual_fields
                                            .iter()
                                            .any(|(existing, _)| existing == &name)
                                        {
                                            actual_fields.push((name, ty));
                                        }
                                        continue;
                                    }

                                    if *override_existing {
                                        upsert_shape_field(&mut actual_fields, name.clone(), ty);
                                        providers.insert(
                                            name,
                                            ShapeFieldProvider {
                                                source: source.clone(),
                                                conflicting_source: None,
                                                span: field.span,
                                            },
                                        );
                                    } else if let Some(provider) = providers.get_mut(&name) {
                                        provider.conflicting_source = Some(source.clone());
                                        provider.span = field.span;
                                    } else {
                                        actual_fields.push((name.clone(), ty));
                                        providers.insert(
                                            name,
                                            ShapeFieldProvider {
                                                source: source.clone(),
                                                conflicting_source: None,
                                                span: field.span,
                                            },
                                        );
                                    }
                                }
                            }
                        }
                    }
                    for (name, provider) in providers {
                        if let Some(conflicting_source) = provider.conflicting_source {
                            self.add_error(
                                "ambiguous_shape_field",
                                format!(
                                    "field '{}' is provided by both '{}' and '{}'; select a value explicitly or use 'override' on one spread",
                                    name, provider.source, conflicting_source
                                ),
                                provider.span,
                            );
                        }
                    }
                    return Ty::Record(actual_fields);
                }
                if let Ty::Record(expected_fields) = expected {
                    let mut actual_fields = Vec::new();
                    for (index, value) in values.iter().enumerate() {
                        let expected_ty = expected_fields
                            .get(index)
                            .map(|(_, ty)| ty.clone())
                            .unwrap_or(Ty::Unknown);
                        let actual = self.check_expr_against(value, &expected_ty);
                        let name = expected_fields
                            .get(index)
                            .map(|(name, _)| name.clone())
                            .unwrap_or_else(|| format!("_{}", index + 1));
                        actual_fields.push((name, actual));
                    }
                    Ty::Record(actual_fields)
                } else {
                    for value in values {
                        self.check_expr(value);
                    }
                    if !values.is_empty() {
                        self.add_error(
	                            "missing_shape_context",
                            "anonymous shape literals require field labels; map literals use '[key: value]'",
	                            expr.span(),
	                        );
                    }
                    Ty::Record(Vec::new())
                }
            }
            Expr::AnonymousInterface {
                interfaces,
                methods,
                span,
            } => self.check_anonymous_interface_expr(interfaces, methods, *span, expected),
            Expr::AnonymousObject {
                fields,
                methods,
                span,
            } => self.check_anonymous_object_expr(fields, methods, *span),
            Expr::Try { value, span } => self.check_try_expr(value, *span),
            Expr::ExtractOr {
                value,
                fallback,
                span,
            } => self.check_extract_or_expr(value, fallback, *span),
            Expr::Return { value, span } => self.check_return_control_expr(value.as_deref(), *span),
            Expr::Break { span } => self.check_break_control_expr(*span),
            Expr::Continue { span } => self.check_continue_control_expr(*span),
            Expr::Unary { op, expr, span } => {
                let inner = self.check_expr(expr);
                match op {
                    crate::ast::UnaryOp::Neg => {
                        if inner.is_numeric() || matches!(inner, Ty::Unknown) {
                            inner
                        } else if let Some(methods) = self.member_method_sigs(&inner, "-") {
                            methods
                                .first()
                                .map(|method| method.ret.clone())
                                .unwrap_or(Ty::Unknown)
                        } else {
                            self.add_error(
                                "invalid_unary_operand",
                                format!(
                                    "unary '-' expects numeric operand, got '{}'",
                                    inner.describe()
                                ),
                                *span,
                            );
                            Ty::Unknown
                        }
                    }
                    crate::ast::UnaryOp::Not => {
                        self.require_bool(&inner, *span, "unary '!' expects Bool");
                        Ty::bool()
                    }
                    crate::ast::UnaryOp::UnsafeExtract => {
                        let extracted = self.unwrap_inner_type(&inner);
                        if matches!(extracted, Ty::Unknown) && !matches!(inner, Ty::Unknown) {
                            self.add_error(
                                "invalid_unsafe_extract",
                                format!(
                                    "unsafe extraction '!!' requires Option[T], Result[T, E], or Either[L, R], got '{}'",
                                    inner.describe()
                                ),
                                *span,
                            );
                        }
                        extracted
                    }
                }
            }
            Expr::Binary {
                left,
                op,
                right,
                span,
            } => {
                let left_ty = self.check_expr(left);
                let right_ty = self.check_expr(right);
                self.check_binary_expr(&left_ty, *op, &right_ty, *span)
            }
            Expr::Is { left, target, .. } => {
                self.check_expr(left);
                let target_ty = self.ty_from_type_ref(target);
                self.validate_runtime_type_ref(target, &target_ty, "type tests");
                Ty::bool()
            }
            Expr::TypeOf { ty, .. } => {
                let represented = self.ty_from_type_ref(ty);
                self.check_typeof_type_ref(ty, &represented);
                Ty::exact_runtime_type(represented)
            }
            Expr::If {
                condition,
                then_block,
                else_branch,
                ..
            } => {
                let cond_ty = self.check_expr(condition);
                self.require_bool(&cond_ty, condition.span(), "if condition must be Bool");
                let then_narrowing = self.type_narrowing_for_condition(condition, true);
                let else_narrowing = self.type_narrowing_for_condition(condition, false);
                let then_ty = self.check_block_with_narrowing(then_block, then_narrowing.as_ref());
                let else_ty = self
                    .check_else_expr_branch_with_narrowing(else_branch, else_narrowing.as_ref());
                join_types(&then_ty, &else_ty)
            }
            Expr::Block { body, .. } => self.check_block_against(body, expected),
            Expr::Match {
                partial,
                value,
                cases,
                span,
            } => {
                let value_ty = self.check_expr(value);
                let mut result = Ty::Unknown;
                let case_expected = if *partial {
                    self.unwrap_inner_type(expected)
                } else {
                    expected.clone()
                };
                for case in cases {
                    let current = self.check_match_case_against(case, &value_ty, &case_expected);
                    result = join_types(&result, &current);
                }
                if !*partial {
                    self.check_match_exhaustiveness(&value_ty, cases, *span);
                }
                if *partial { Ty::option(result) } else { result }
            }
            Expr::ForYield {
                bindings,
                yield_body,
                ..
            } => {
                self.push_scope();
                self.loop_depth += 1;
                let mut family = None;
                for binding in bindings {
                    if let Some(next) = self.check_for_binding(binding, true) {
                        family = self.choose_for_yield_family(family, next, binding.span);
                    }
                }
                if let Some(family) = &family
                    && !matches!(family, ForYieldFamily::Iterable | ForYieldFamily::Unknown)
                    && let Some(control) = loop_control_targeting_current_loop_in_block(yield_body)
                {
                    self.add_error(
                        control.kind.diagnostic_code(),
                        format!(
                            "`{}` inside `for ... yield` is only supported for iterable comprehensions; {} comprehensions have no {} state",
                            control.kind.keyword(),
                            describe_for_yield_family(family),
                            control.kind.state_name()
                        ),
                        control.span,
                    );
                }
                let yield_ty = self.check_block(yield_body);
                self.loop_depth -= 1;
                self.pop_scope();
                self.wrap_for_yield_type(family.as_ref(), yield_ty)
            }
            Expr::Lambda { params, body, .. } => self.check_lambda_expr(params, body, expected),
            Expr::Group { inner, .. } => self.check_expr(inner),
        }
    }

    fn check_call_arg_expr_against(&mut self, expr: &Expr, expected: &Ty) -> Ty {
        self.check_expr_against(expr, expected)
    }

    fn check_lambda_expr(
        &mut self,
        params: &[crate::ast::LambdaParam],
        body: &LambdaBody,
        expected: &Ty,
    ) -> Ty {
        let (expected_params, expected_ret) = match expected {
            Ty::Function(params, ret) => (Some(params.clone()), Some(ret.as_ref().clone())),
            _ => (None, None),
        };
        let hinted_params = match expected_params {
            Some(ref params_hint) if params_hint.len() == params.len() => params_hint.clone(),
            _ => vec![Ty::Unknown; params.len()],
        };

        self.push_scope();
        let previous_return = self.current_return.clone();
        let previous_callable_depth = self.callable_depth;
        let previous_loop_depth = self.loop_depth;
        self.current_return = expected_ret.clone().unwrap_or(Ty::Unknown);
        self.callable_depth += 1;
        self.loop_depth = 0;
        let mut param_types = Vec::new();
        for (index, param) in params.iter().enumerate() {
            let hint = hinted_params.get(index).cloned().unwrap_or(Ty::Unknown);
            param_types.push(self.check_lambda_param(param, hint));
        }
        let ret = match body {
            LambdaBody::Expr(expr) => expected_ret
                .as_ref()
                .map(|ret| self.check_expr_against(expr, ret))
                .unwrap_or_else(|| self.check_expr(expr)),
            LambdaBody::Block(block) => {
                self.check_block_against(block, expected_ret.as_ref().unwrap_or(&Ty::Unknown))
            }
        };
        if expected_ret.as_ref().is_some_and(|ret| *ret == Ty::unit()) {
            if let LambdaBody::Expr(expr) = body {
                self.check_discarded_expr_after_unit_expected(expr);
            }
        }
        self.current_return = previous_return;
        self.callable_depth = previous_callable_depth;
        self.loop_depth = previous_loop_depth;
        self.pop_scope();
        let ret = if expected_ret
            .as_ref()
            .is_some_and(|expected| *expected == Ty::unit())
        {
            Ty::unit()
        } else {
            ret
        };
        Ty::Function(param_types, Box::new(ret))
    }

    fn check_lambda_param(&mut self, param: &crate::ast::LambdaParam, hint: Ty) -> Ty {
        if let Some(destructure) = &param.destructure {
            let ty = param
                .ty
                .as_ref()
                .map(|ty| self.ty_from_type_ref(ty))
                .unwrap_or_else(|| {
                    if !matches!(hint, Ty::Unknown) {
                        hint
                    } else {
                        self.infer_destructured_lambda_param_type(destructure)
                    }
                });
            self.check_destructure_source(
                &ty,
                destructure.kind,
                destructure.bindings.len(),
                param.span,
            );
            let slot_types = if destructure.kind == DestructureKind::Record {
                self.record_binding_slot_types(&ty, &destructure.bindings, param.span)
            } else {
                self.destructure_slots(&ty, destructure.bindings.len(), destructure.kind)
            };
            for (index, binding) in destructure.bindings.iter().enumerate() {
                let inferred = slot_types.get(index).cloned().unwrap_or(Ty::Unknown);
                let explicit = binding.ty.as_ref().map(|ty| self.ty_from_type_ref(ty));
                let local_ty = explicit.unwrap_or(inferred);
                self.define_local(&binding.name, local_ty, false);
            }
            return ty;
        }

        let ty = param
            .ty
            .as_ref()
            .map(|ty| self.ty_from_type_ref(ty))
            .unwrap_or(hint);
        self.define_local(&param.name, ty.clone(), false);
        ty
    }

    fn infer_destructured_lambda_param_type(
        &mut self,
        destructure: &crate::ast::LambdaParamDestructure,
    ) -> Ty {
        match destructure.kind {
            DestructureKind::Tuple => Ty::Tuple(
                destructure
                    .bindings
                    .iter()
                    .map(|binding| {
                        binding
                            .ty
                            .as_ref()
                            .map(|ty| self.ty_from_type_ref(ty))
                            .unwrap_or(Ty::Unknown)
                    })
                    .collect(),
            ),
            DestructureKind::Record => Ty::Record(
                destructure
                    .bindings
                    .iter()
                    .map(|binding| {
                        let name = binding
                            .field_name
                            .clone()
                            .unwrap_or_else(|| binding.name.clone());
                        let ty = binding
                            .ty
                            .as_ref()
                            .map(|ty| self.ty_from_type_ref(ty))
                            .unwrap_or(Ty::Unknown);
                        (name, ty)
                    })
                    .collect(),
            ),
        }
    }

    fn check_call(
        &mut self,
        callee: &Expr,
        args: &[crate::ast::CallArg],
        uses_brace_syntax: bool,
        span: crate::source::Span,
        expected: &Ty,
    ) -> Ty {
        if matches!(callee, Expr::Identifier { name, .. } if name == "Any") {
            return self.check_explicit_any_widening(args, uses_brace_syntax, span);
        }
        if uses_brace_syntax
            && matches!(
                callee,
                Expr::Call {
                    uses_brace_syntax: false,
                    ..
                }
            )
        {
            self.add_error(
                "invalid_trailing_brace_call",
                "trailing block cannot follow an already completed call; pass the callback inside the same argument list",
                span,
            );
            return Ty::Unknown;
        }
        if uses_brace_syntax
            && self.brace_call_type_sig(callee).is_none()
            && !self.brace_call_targets_current_constructor(callee)
            && !self.brace_call_uses_constructor_delegation_syntax(callee)
            && !self.brace_call_targets_enum_case(callee)
            && !trailing_brace_call_has_lambda_arg(args)
        {
            self.add_error(
                "invalid_trailing_brace_call",
                "trailing brace call syntax only accepts lambda arguments; use parentheses for ordinary arguments",
                span,
            );
            return Ty::Unknown;
        }

        let normalized_args =
            self.normalize_trailing_brace_call_args(callee, args, uses_brace_syntax);
        if let Expr::Member { receiver, name, .. } = callee
            && normalized_args.len() == 1
            && matches!(name.as_str(), "equals" | "sameValue")
        {
            let left = self.check_expr(receiver);
            let right = self.check_expr(&normalized_args[0].value);
            if name == "equals" {
                self.check_equality_operands(&left, &right, span);
            }
            return Ty::bool();
        }
        if self.is_builtin_panic_call(callee) {
            for arg in &normalized_args {
                self.check_expr(&arg.value);
            }
            return Ty::never();
        }
        if self.is_builtin_print_call(callee) {
            for arg in &normalized_args {
                self.check_expr(&arg.value);
            }
            return Ty::unit();
        }
        if self.is_builtin_assert_call(callee) {
            return self.check_builtin_assert_call(&normalized_args, span);
        }
        if let Some(ty) = self.check_builtin_static_method_call(callee, &normalized_args, span) {
            return ty;
        }
        if let Some(ty) = self.check_builtin_static_factory_call(callee, &normalized_args, span) {
            return ty;
        }
        if self.check_extension_hidden_method_call(callee, &normalized_args) {
            return Ty::Unknown;
        }
        if let Some(ty) = self.try_check_constructor_call(
            callee,
            &normalized_args,
            uses_brace_syntax,
            span,
            expected,
        ) {
            return ty;
        }
        if let Some(selection) =
            self.callable_signature_for_args(callee, &normalized_args, uses_brace_syntax, span)
        {
            return self.check_callable_selection_call(&selection, callee, &normalized_args, span);
        }
        if let Expr::Index { span, .. } = callee {
            self.add_error(
                "indexed_function_call_requires_grouping",
                "callee[...](...) is explicit generic application syntax; to call an indexed function value, write '(callee[key])()'",
                *span,
            );
            return Ty::Unknown;
        }
        let callee_ty = self.check_expr(callee);
        match callee_ty {
            Ty::Function(params, ret) => {
                let sig_params = params
                    .into_iter()
                    .map(|ty| ParamSig {
                        name: String::new(),
                        ty,
                        variadic: false,
                        lazy: false,
                        has_initializer: false,
                    })
                    .collect::<Vec<_>>();
                self.check_signature_call(&sig_params, &ret, &normalized_args, span)
            }
            Ty::Unknown => Ty::Unknown,
            other => {
                self.add_error(
                    "invalid_call_target",
                    format!("cannot call value of type '{}'", other.describe()),
                    span,
                );
                Ty::Unknown
            }
        }
    }

    fn check_explicit_any_widening(
        &mut self,
        args: &[crate::ast::CallArg],
        uses_brace_syntax: bool,
        span: crate::source::Span,
    ) -> Ty {
        for arg in args {
            self.check_expr(&arg.value);
        }

        if uses_brace_syntax {
            self.add_error(
                "invalid_any_widening_syntax",
                "'Any' is an explicit widening expression, not a constructor; use 'Any(value)'",
                span,
            );
            return Ty::any();
        }

        if args.len() != 1 {
            self.add_error(
                "invalid_any_widening_arity",
                format!("'Any(...)' expects exactly one value, got {}", args.len()),
                span,
            );
        }

        if args
            .iter()
            .any(|arg| arg.name.is_some() || arg.ty.is_some())
        {
            self.add_error(
                "invalid_any_widening_argument",
                "'Any(...)' accepts one positional value; named or typed arguments are not allowed",
                span,
            );
        }

        Ty::any()
    }

    fn check_anonymous_interface_expr(
        &mut self,
        interfaces: &[TypeRef],
        methods: &[MethodDecl],
        span: crate::source::Span,
        expected: &Ty,
    ) -> Ty {
        let provided = methods
            .iter()
            .map(|method| method.name.clone())
            .collect::<HashSet<_>>();
        let mut result_tys = Vec::new();

        for interface in interfaces {
            let interface_ty = self.ty_from_type_ref(interface);
            let Some(interface_sig) = self.interface_sig_from_type_ref(interface) else {
                continue;
            };
            let mut required = HashSet::new();
            let mut seen = HashSet::new();
            self.collect_required_interface_method_names(&interface_sig, &mut seen, &mut required);

            let mut missing = required
                .into_iter()
                .filter(|name| !provided.contains(name))
                .collect::<Vec<_>>();
            missing.sort();
            for name in missing {
                self.add_error(
                    "missing_interface_member",
                    format!(
                        "anonymous implementation of interface '{}' is missing method '{}'",
                        interface_sig.name, name
                    ),
                    span,
                );
            }
            result_tys.push(interface_ty);
        }

        if result_tys
            .iter()
            .any(|interface_ty| self.is_assignable(interface_ty, expected))
        {
            expected.clone()
        } else if result_tys.len() == 1 {
            result_tys.pop().unwrap_or(Ty::Unknown)
        } else {
            Ty::Unknown
        }
    }

    fn check_anonymous_object_expr(
        &mut self,
        fields: &[crate::ast::FieldDecl],
        methods: &[MethodDecl],
        span: crate::source::Span,
    ) -> Ty {
        let name = crate::source::anonymous_object_type_name(span);
        let mut field_sigs = Vec::new();

        self.push_scope();
        for field in fields {
            if field.mutable {
                self.add_error(
                    "mutable_anonymous_object_field",
                    format!(
                        "anonymous object field '{}' cannot be mutable; use a named class when the object owns mutable state",
                        field.name
                    ),
                    field.span,
                );
            }
            let declared = field.ty.as_ref().map(|ty| self.ty_from_type_ref(ty));
            let actual = field
                .initializer
                .as_ref()
                .map(|initializer| {
                    declared
                        .as_ref()
                        .map(|expected| self.check_expr_against(initializer, expected))
                        .unwrap_or_else(|| self.check_expr(initializer))
                })
                .unwrap_or(Ty::Unknown);
            let ty = declared.unwrap_or(actual.clone());
            if !matches!(actual, Ty::Unknown) {
                self.require_assignable(
                    &actual,
                    &ty,
                    field.span,
                    "invalid_field_initializer_type",
                    format!(
                        "anonymous object field '{}' has type '{}' but its initializer has type '{}'",
                        field.name,
                        ty.describe(),
                        actual.describe()
                    ),
                );
            }
            field_sigs.push(FieldSig {
                name: field.name.clone(),
                ty: ty.clone(),
                mutable: false,
                hidden: field.visibility == Visibility::Hidden,
                has_initializer: true,
                variadic: false,
            });
            self.define_local(&field.name, ty, false);
        }
        self.pop_scope();

        let mut method_sigs = HashMap::<String, Vec<FunctionSig>>::new();
        for method in methods {
            method_sigs
                .entry(method.name.clone())
                .or_default()
                .push(function_sig_from_method(method, &[]));
        }
        let sig = TypeSig {
            kind: TypeKind::Object,
            name: name.clone(),
            type_params: Vec::new(),
            generic_conditions: Vec::new(),
            with_bounds: Vec::new(),
            fields: field_sigs,
            methods: method_sigs,
            enum_cases: HashMap::new(),
        };
        self.anonymous_types.insert(name.clone(), sig.clone());
        for method in methods {
            self.check_method(method, &sig);
        }

        Ty::Named(name, Vec::new())
    }

    fn interface_sig_from_type_ref(&mut self, interface: &TypeRef) -> Option<TypeSig> {
        let TypeRef::Named { name, .. } = interface else {
            self.add_error(
                "invalid_anonymous_interface",
                "anonymous implementation target must be an interface name",
                interface.span(),
            );
            return None;
        };
        let Some(sig) = self.lookup_any_type(name) else {
            self.add_error(
                "unknown_type",
                format!("unknown interface '{}'", name),
                interface.span(),
            );
            return None;
        };
        if sig.kind != TypeKind::Interface {
            self.add_error(
                "invalid_anonymous_interface",
                format!(
                    "anonymous implementation target '{}' must be an interface",
                    sig.name
                ),
                interface.span(),
            );
            return None;
        }
        Some(sig)
    }

    fn collect_required_interface_method_names(
        &self,
        sig: &TypeSig,
        seen: &mut HashSet<String>,
        methods: &mut HashSet<String>,
    ) {
        if !seen.insert(sig.name.clone()) {
            return;
        }
        for bound in &sig.with_bounds {
            let Ty::Named(bound_name, _) = bound else {
                continue;
            };
            let Some(bound_sig) = self.lookup_any_type(bound_name) else {
                continue;
            };
            if bound_sig.kind == TypeKind::Interface {
                self.collect_required_interface_method_names(&bound_sig, seen, methods);
            }
        }
        for (name, overloads) in &sig.methods {
            if overloads.iter().any(|method| !method.has_body) {
                methods.insert(name.clone());
            } else {
                methods.remove(name);
            }
        }
    }

    fn type_declares_method_body(&self, sig: &TypeSig, name: &str) -> bool {
        sig.methods
            .get(name)
            .is_some_and(|methods| methods.iter().any(|method| method.has_body))
    }

    fn reject_parenthesized_constructor_fields(
        &mut self,
        args: &[crate::ast::CallArg],
        uses_brace_syntax: bool,
        span: crate::source::Span,
    ) -> bool {
        if uses_brace_syntax || args.iter().all(|arg| arg.name.is_none()) {
            return false;
        }
        self.add_error(
            "invalid_constructor_call",
            "constructor parentheses accept positional arguments only; use braces for construction fields",
            span,
        );
        true
    }

    fn check_builtin_static_factory_call(
        &mut self,
        callee: &Expr,
        args: &[crate::ast::CallArg],
        span: crate::source::Span,
    ) -> Option<Ty> {
        let Expr::Member { receiver, name, .. } = callee else {
            return None;
        };
        if name != "from" {
            return None;
        }
        let Expr::Identifier {
            name: type_name, ..
        } = receiver.as_ref()
        else {
            return None;
        };

        let result_name = match type_name.as_str() {
            "Vector" => "Vector",
            "Set" => "Set",
            _ => return None,
        };

        if args.len() != 1 {
            self.add_error(
                "invalid_argument_count",
                format!("{result_name}.from expects 1 argument, got {}", args.len()),
                span,
            );
        }

        let source_ty = args
            .first()
            .map(|arg| self.check_expr(&arg.value))
            .unwrap_or(Ty::Unknown);
        let item_ty = self.iterable_item_type(&source_ty);
        if matches!(item_ty, Ty::Unknown) && !matches!(source_ty, Ty::Unknown) {
            let arg_span = args.first().map(|arg| arg.span).unwrap_or(span);
            self.add_error(
                "invalid_argument_type",
                format!(
                    "{result_name}.from expects iterable argument, got '{}'",
                    source_ty.describe()
                ),
                arg_span,
            );
        }
        Some(Ty::Named(result_name.to_string(), vec![item_ty]))
    }

    fn check_builtin_static_method_call(
        &mut self,
        callee: &Expr,
        args: &[crate::ast::CallArg],
        span: crate::source::Span,
    ) -> Option<Ty> {
        let Expr::Member { receiver, name, .. } = callee else {
            return None;
        };
        let Expr::Identifier {
            name: type_name, ..
        } = receiver.as_ref()
        else {
            return None;
        };

        match (type_name.as_str(), name.as_str()) {
            ("Array", factory @ ("ofInt" | "ofFloat" | "ofBool" | "ofStr" | "ofRune")) => {
                if args.len() != 1 {
                    self.add_error(
                        "invalid_argument_count",
                        format!("Array.{factory} expects 1 argument, got {}", args.len()),
                        span,
                    );
                }
                if let Some(arg) = args.first() {
                    let ty = self.check_expr(&arg.value);
                    self.require_assignable(
                        &ty,
                        &Ty::int(),
                        arg.span,
                        "invalid_argument_type",
                        format!(
                            "Array.{factory} expects Int capacity, got '{}'",
                            ty.describe()
                        ),
                    );
                }
                let item_ty = match factory {
                    "ofInt" => Ty::int(),
                    "ofFloat" => Ty::float(),
                    "ofBool" => Ty::bool(),
                    "ofStr" => Ty::str(),
                    "ofRune" => Ty::rune(),
                    _ => unreachable!(),
                };
                Some(Ty::Named("Array".to_string(), vec![item_ty]))
            }
            ("Array", "fill") => {
                if args.len() != 2 {
                    self.add_error(
                        "invalid_argument_count",
                        format!("Array.fill expects 2 arguments, got {}", args.len()),
                        span,
                    );
                }
                if let Some(arg) = args.first() {
                    let ty = self.check_expr(&arg.value);
                    self.require_assignable(
                        &ty,
                        &Ty::int(),
                        arg.span,
                        "invalid_argument_type",
                        format!("Array.fill expects Int length, got '{}'", ty.describe()),
                    );
                }
                let item_ty = args
                    .get(1)
                    .map(|arg| self.check_expr(&arg.value))
                    .unwrap_or(Ty::Unknown);
                Some(Ty::Named("Array".to_string(), vec![item_ty]))
            }
            ("Array", "generate") => {
                if args.len() != 2 {
                    self.add_error(
                        "invalid_argument_count",
                        format!("Array.generate expects 2 arguments, got {}", args.len()),
                        span,
                    );
                }
                if let Some(arg) = args.first() {
                    let ty = self.check_expr(&arg.value);
                    self.require_assignable(
                        &ty,
                        &Ty::int(),
                        arg.span,
                        "invalid_argument_type",
                        format!("Array.generate expects Int length, got '{}'", ty.describe()),
                    );
                }
                let item_ty = args
                    .get(1)
                    .map(|arg| {
                        let expected = Ty::Function(vec![Ty::int()], Box::new(Ty::Unknown));
                        let generator_ty = self.check_expr_against(&arg.value, &expected);
                        match generator_ty {
                            Ty::Function(params, ret) if params.len() == 1 => {
                                if !matches!(params[0], Ty::Unknown)
                                    && !self.is_assignable(&Ty::int(), &params[0])
                                {
                                    self.add_error(
                                        "invalid_argument_type",
                                        format!(
                                            "Array.generate expects fn(Int) T generator, got '{}'",
                                            Ty::Function(params.clone(), ret.clone()).describe()
                                        ),
                                        arg.span,
                                    );
                                }
                                *ret
                            }
                            Ty::Unknown => Ty::Unknown,
                            other => {
                                self.add_error(
                                    "invalid_argument_type",
                                    format!(
                                        "Array.generate expects fn(Int) T generator, got '{}'",
                                        other.describe()
                                    ),
                                    arg.span,
                                );
                                Ty::Unknown
                            }
                        }
                    })
                    .unwrap_or(Ty::Unknown);
                Some(Ty::Named("Array".to_string(), vec![item_ty]))
            }
            ("Int", "parse") => {
                if args.len() != 1 {
                    self.add_error(
                        "invalid_argument_count",
                        format!("Int.parse expects 1 argument, got {}", args.len()),
                        span,
                    );
                }
                if let Some(arg) = args.first() {
                    let ty = self.check_expr(&arg.value);
                    self.require_assignable(
                        &ty,
                        &Ty::str(),
                        arg.span,
                        "invalid_argument_type",
                        format!("Int.parse expects Str argument, got '{}'", ty.describe()),
                    );
                }
                Some(Ty::option(Ty::int()))
            }
            ("Float", "parse") => {
                if args.len() != 1 {
                    self.add_error(
                        "invalid_argument_count",
                        format!("Float.parse expects 1 argument, got {}", args.len()),
                        span,
                    );
                }
                if let Some(arg) = args.first() {
                    let ty = self.check_expr(&arg.value);
                    self.require_assignable(
                        &ty,
                        &Ty::str(),
                        arg.span,
                        "invalid_argument_type",
                        format!("Float.parse expects Str argument, got '{}'", ty.describe()),
                    );
                }
                Some(Ty::option(Ty::float()))
            }
            ("Option", "when") => {
                if args.len() != 2 {
                    self.add_error(
                        "invalid_argument_count",
                        format!("Option.when expects 2 arguments, got {}", args.len()),
                        span,
                    );
                }
                if let Some(arg) = args.first() {
                    let ty = self.check_expr(&arg.value);
                    self.require_assignable(
                        &ty,
                        &Ty::bool(),
                        arg.span,
                        "invalid_argument_type",
                        format!(
                            "Option.when expects Bool condition, got '{}'",
                            ty.describe()
                        ),
                    );
                }
                let value_ty = args
                    .get(1)
                    .map(|arg| self.check_expr(&arg.value))
                    .unwrap_or(Ty::Unknown);
                Some(Ty::option(value_ty))
            }
            _ => None,
        }
    }

    fn check_discarded_expr_in_statement(&mut self, expr: &Expr) {
        match expr {
            Expr::Call { .. }
            | Expr::Try { .. }
            | Expr::ExtractOr { .. }
            | Expr::Return { .. }
            | Expr::Break { .. }
            | Expr::Continue { .. }
            | Expr::Unit { .. }
            | Expr::ForYield { .. } => {}
            Expr::Group { inner, .. } => self.check_discarded_expr_in_statement(inner),
            Expr::Block { body, .. } => self.check_discarded_block_tail_in_statement(body),
            Expr::If {
                then_block,
                else_branch,
                ..
            } => {
                self.check_discarded_block_tail_in_statement(then_block);
                self.check_discarded_else_expr_branch(else_branch);
            }
            Expr::Match { cases, .. } => {
                for case in cases {
                    match &case.body {
                        MatchCaseBody::Block(block) => {
                            self.check_discarded_block_tail_in_statement(block)
                        }
                        MatchCaseBody::Expr(expr) => self.check_discarded_expr_in_statement(expr),
                    }
                }
            }
            _ => self.add_error(
                "discarded_expression",
                "discarded expression has no effect; did you mean to assign it or return it?",
                expr.span(),
            ),
        }
    }

    fn check_discarded_expr_after_unit_expected(&mut self, expr: &Expr) {
        match expr {
            Expr::Call { .. }
            | Expr::Try { .. }
            | Expr::ExtractOr { .. }
            | Expr::Return { .. }
            | Expr::Break { .. }
            | Expr::Continue { .. }
            | Expr::Unit { .. }
            | Expr::ForYield { .. } => {}
            Expr::Group { inner, .. } => self.check_discarded_expr_after_unit_expected(inner),
            Expr::Block { .. } => {}
            Expr::If {
                then_block,
                else_branch,
                ..
            } => {
                self.check_discarded_block_tail_in_statement(then_block);
                self.check_discarded_else_expr_branch(else_branch);
            }
            Expr::Match { cases, .. } => {
                for case in cases {
                    match &case.body {
                        MatchCaseBody::Block(block) => {
                            self.check_discarded_block_tail_in_statement(block)
                        }
                        MatchCaseBody::Expr(expr) => self.check_discarded_expr_in_statement(expr),
                    }
                }
            }
            _ => self.add_error(
                "discarded_expression",
                "discarded expression has no effect; did you mean to assign it or return it?",
                expr.span(),
            ),
        }
    }

    fn check_discarded_block_tail_in_statement(&mut self, block: &Block) {
        let Some(statement) = block.statements.last() else {
            return;
        };
        match statement {
            Stmt::Expr(expr_stmt) => self.check_discarded_expr_in_statement(&expr_stmt.expr),
            Stmt::If(stmt) => self.check_discarded_if_stmt_in_statement(stmt),
            Stmt::Match(stmt) => self.check_discarded_match_stmt_in_statement(stmt),
            _ => {}
        }
    }

    fn check_discarded_if_stmt_in_statement(&mut self, stmt: &IfStmt) {
        self.check_discarded_block_tail_in_statement(&stmt.then_block);
        if let Some(branch) = &stmt.else_branch {
            self.check_discarded_else_branch_in_statement(branch);
        }
    }

    fn check_discarded_else_branch_in_statement(&mut self, branch: &ElseBranch) {
        match branch {
            ElseBranch::If(stmt) => self.check_discarded_if_stmt_in_statement(stmt),
            ElseBranch::Block(block) => self.check_discarded_block_tail_in_statement(block),
        }
    }

    fn check_discarded_match_stmt_in_statement(&mut self, stmt: &crate::ast::MatchStmt) {
        for case in &stmt.cases {
            match &case.body {
                MatchCaseBody::Block(block) => self.check_discarded_block_tail_in_statement(block),
                MatchCaseBody::Expr(expr) => self.check_discarded_expr_in_statement(expr),
            }
        }
    }

    fn check_discarded_else_expr_branch(&mut self, branch: &ElseExprBranch) {
        match branch {
            ElseExprBranch::If(expr) => self.check_discarded_expr_in_statement(expr),
            ElseExprBranch::Block(block) => self.check_discarded_block_tail_in_statement(block),
        }
    }

    fn check_signature_call(
        &mut self,
        params: &[ParamSig],
        ret: &Ty,
        args: &[crate::ast::CallArg],
        span: crate::source::Span,
    ) -> Ty {
        self.check_signature_call_with_subst(params, ret, args, span, HashMap::new())
            .0
    }

    fn check_callable_selection_call(
        &mut self,
        selection: &CallableSelection,
        callee: &Expr,
        args: &[crate::ast::CallArg],
        span: crate::source::Span,
    ) -> Ty {
        let subst = self.explicit_type_arg_subst(
            &selection.sig.type_params,
            &selection.explicit_type_args,
            span,
        );
        let (ret, subst, argument_shape_valid) = self.check_signature_call_with_subst(
            &selection.sig.params,
            &selection.sig.ret,
            args,
            span,
            subst,
        );
        if argument_shape_valid {
            self.check_call_generic_conditions(&selection.sig.generic_conditions, &subst, span);
        }
        let missing = if argument_shape_valid {
            selection
                .sig
                .reified_type_params
                .iter()
                .filter(|name| {
                    let ty = subst.get(*name).cloned().unwrap_or(Ty::Unknown);
                    matches!(ty, Ty::Unknown | Ty::TypeParam(_))
                })
                .cloned()
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        for name in missing {
            let callee_name = callable_name_for_diagnostic(callee);
            self.add_error(
                "cannot_infer_reified_type",
                format!(
                    "cannot infer reified type parameter '{}'; pass an explicit type argument, for example {}[{}](...)",
                    name, callee_name, name
                ),
                span,
            );
        }
        ret
    }

    fn check_call_generic_conditions(
        &mut self,
        conditions: &[GenericConditionSig],
        subst: &HashMap<String, Ty>,
        span: crate::source::Span,
    ) {
        for condition in conditions {
            match substitute_generic_condition(condition, subst) {
                GenericConditionSig::Bound { subject, bound } => {
                    if matches!(subject, Ty::Unknown | Ty::TypeParam(_))
                        && !self.is_assignable(&subject, &bound)
                    {
                        self.add_error(
                            "cannot_infer_bounded_type",
                            format!(
                                "cannot prove that generic type '{}' satisfies bound '{}'",
                                subject.describe(),
                                bound.describe()
                            ),
                            span,
                        );
                    } else if !self.is_assignable(&subject, &bound) {
                        self.add_error(
                            "generic_bound_not_satisfied",
                            format!(
                                "type '{}' does not satisfy generic bound '{}'",
                                subject.describe(),
                                bound.describe()
                            ),
                            span,
                        );
                    }
                }
                GenericConditionSig::Equal { left, right } => {
                    if matches!(left, Ty::Unknown) || matches!(right, Ty::Unknown) {
                        self.add_error(
                            "cannot_infer_generic_equality",
                            format!(
                                "cannot prove generic equality condition '{} = {}'",
                                left.describe(),
                                right.describe()
                            ),
                            span,
                        );
                    } else if !self.generic_types_are_equal(&left, &right) {
                        self.add_error(
                            "generic_equality_not_satisfied",
                            format!(
                                "generic equality condition '{} = {}' is not satisfied",
                                left.describe(),
                                right.describe()
                            ),
                            span,
                        );
                    }
                }
            }
        }
    }

    fn generic_types_are_equal(&self, left: &Ty, right: &Ty) -> bool {
        match (left, right) {
            (Ty::TypeParam(left), Ty::TypeParam(right)) => self.type_params_are_equal(left, right),
            _ => left == right,
        }
    }

    fn check_signature_call_with_subst(
        &mut self,
        params: &[ParamSig],
        ret: &Ty,
        args: &[crate::ast::CallArg],
        span: crate::source::Span,
        mut subst: HashMap<String, Ty>,
    ) -> (Ty, HashMap<String, Ty>, bool) {
        let arrangement = arrange_param_args(params, args);
        let min_required = params
            .iter()
            .filter(|param| !param.variadic && !param.has_initializer)
            .count();
        let max_allowed = if params.last().is_some_and(|param| param.variadic) {
            usize::MAX
        } else {
            params.len()
        };
        let argument_shape_valid = arrangement.overflow == 0
            && arrangement.missing_required == 0
            && args.len() >= min_required
            && args.len() <= max_allowed;
        if !argument_shape_valid {
            self.add_error(
                "invalid_argument_count",
                format!(
                    "call expects {}..{} arguments, got {}",
                    min_required,
                    if max_allowed == usize::MAX {
                        "many".to_string()
                    } else {
                        max_allowed.to_string()
                    },
                    args.len()
                ),
                span,
            );
        }
        let mut checked_args = Vec::new();
        for (index, param) in params.iter().enumerate() {
            let slot = arrangement
                .slots
                .get(index)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            for arg in slot {
                if param.lazy {
                    if let Some(reason_span) = lazy_arg_forbidden_control_flow_span(&arg.value) {
                        self.add_error(
                            "invalid_by_name_argument",
                            "by-name argument expressions cannot contain return, break, continue, or try; make the control flow explicit before the call",
                            reason_span,
                        );
                    }
                }
                if matches!(arg.value, Expr::Spread { .. })
                    && (!param.variadic || arg.name.is_some())
                {
                    self.add_error(
                        "invalid_spread_argument",
                        "spread arguments are only valid as positional arguments for a vararg parameter",
                        arg.span,
                    );
                }
                let raw_expected = call_arg_expected_ty_for_arg(param.variadic, &param.ty, arg);
                let expected = substitute_type(&raw_expected, &subst);
                // Unresolved call type parameters are inference holes, not concrete
                // contextual types. In particular, a lambda returning `Ok(())`
                // must be able to infer `T = Unit` for a `(…) => Result[T, E]`
                // parameter.
                let check_expected = materialize_type(&expected);
                let actual =
                    self.check_call_arg_expr_against(call_arg_value_expr(arg), &check_expected);
                infer_type_subst(&expected, &actual, &mut subst);
                checked_args.push((arg.span, actual, raw_expected.clone()));
            }
        }

        for (arg_span, actual, raw_expected) in checked_args {
            let expected = materialize_type(&substitute_type(&raw_expected, &subst));
            self.require_assignable(
                &actual,
                &expected,
                arg_span,
                "invalid_argument_type",
                if matches!(actual, Ty::Record(_)) && matches!(expected, Ty::Record(_)) {
                    format!(
                        "cannot pass {} to parameter of type {}",
                        actual.describe(),
                        expected.describe()
                    )
                } else {
                    self.diagnostic_type_mismatch_message(
                        "argument",
                        &actual,
                        "parameter",
                        &expected,
                    )
                },
            );
        }

        let ret = materialize_type(&substitute_type(ret, &subst));
        (self.capture_wildcards(ret), subst, argument_shape_valid)
    }

    fn explicit_type_arg_subst(
        &mut self,
        type_params: &[String],
        explicit_type_args: &[Ty],
        span: crate::source::Span,
    ) -> HashMap<String, Ty> {
        let mut subst = HashMap::new();
        if explicit_type_args.is_empty() {
            return subst;
        }
        if explicit_type_args.len() != type_params.len() {
            self.add_error(
                "invalid_type_argument_count",
                format!(
                    "call expects {} type arguments, got {}",
                    type_params.len(),
                    explicit_type_args.len()
                ),
                span,
            );
        }
        for (name, ty) in type_params.iter().zip(explicit_type_args.iter()) {
            subst.insert(name.clone(), ty.clone());
        }
        subst
    }

    fn try_check_constructor_call(
        &mut self,
        callee: &Expr,
        args: &[crate::ast::CallArg],
        uses_brace_syntax: bool,
        span: crate::source::Span,
        expected: &Ty,
    ) -> Option<Ty> {
        let structural_record_arg = call_uses_structural_record_arg(args, uses_brace_syntax);
        let parenthesized_record_arg =
            constructor_uses_parenthesized_record_arg(self, args, uses_brace_syntax);
        match callee {
            Expr::Identifier { name, .. } => {
                if name == "this" {
                    if self.current_method.as_deref() == Some("new") && self.current_owner.is_some()
                    {
                        return self.current_owner.clone().map(|owner| {
                            self.check_named_type_constructor(
                                &owner,
                                args,
                                span,
                                uses_brace_syntax,
                                structural_record_arg,
                                parenthesized_record_arg,
                            )
                        });
                    }
                    self.add_error(
                        "invalid_constructor_delegation",
                        "constructor delegation with `this(...)` or `this { ... }` is only valid inside a class constructor",
                        span,
                    );
                    return Some(Ty::Unknown);
                }
                if name == "new"
                    && self.current_method.as_deref() == Some("new")
                    && self.current_owner.is_some()
                {
                    let replacement = if uses_brace_syntax {
                        "`this { ... }`"
                    } else {
                        "`this(...)`"
                    };
                    self.add_error(
                        "invalid_constructor_delegation",
                        format!(
                            "constructor delegation uses {replacement}; `new` only declares constructors"
                        ),
                        span,
                    );
                    return Some(Ty::Unknown);
                }
                if !structural_record_arg {
                    if let Some(ty) = self.check_intrinsic_collection_constructor(
                        name,
                        args,
                        span,
                        uses_brace_syntax,
                    ) {
                        return Some(ty);
                    }
                }
                if let Some(case) = self.world.lookup_enum_case(self.module, name) {
                    return Some(self.check_enum_case_constructor_signature(
                        name,
                        &case,
                        args,
                        span,
                        uses_brace_syntax,
                        expected,
                    ));
                }
                if let Some(sig) = self.lookup_type_local(name) {
                    return Some(self.check_named_type_constructor(
                        &sig,
                        args,
                        span,
                        uses_brace_syntax,
                        structural_record_arg,
                        parenthesized_record_arg,
                    ));
                }
                if let Some(sig) = self.world.lookup_imported_type(self.module, name) {
                    return Some(self.check_named_type_constructor(
                        &sig,
                        args,
                        span,
                        uses_brace_syntax,
                        structural_record_arg,
                        parenthesized_record_arg,
                    ));
                }
                if let Some(sig) = self.world.ambient.types.get(name).cloned() {
                    return Some(self.check_named_type_constructor(
                        &sig,
                        args,
                        span,
                        uses_brace_syntax,
                        structural_record_arg,
                        parenthesized_record_arg,
                    ));
                }
                if let Some(sig) = self.lookup_any_object(name) {
                    return Some(self.check_named_type_constructor(
                        &sig,
                        args,
                        span,
                        uses_brace_syntax,
                        structural_record_arg,
                        parenthesized_record_arg,
                    ));
                }
                None
            }
            Expr::Member { receiver, name, .. } => {
                if let Some(module) = module_alias_and_member(callee).and_then(|(alias, member)| {
                    self.world
                        .lookup_module_alias(self.module, &alias)
                        .map(|module| (module, member))
                }) {
                    let (module_info, member) = module;
                    if let Some(sig) = module_info.types.get(&member).cloned() {
                        return Some(self.check_named_type_constructor(
                            &sig,
                            args,
                            span,
                            uses_brace_syntax,
                            structural_record_arg,
                            parenthesized_record_arg,
                        ));
                    }
                    if let Some(sig) = module_info.objects.get(&member).cloned() {
                        return Some(self.check_named_type_constructor(
                            &sig,
                            args,
                            span,
                            uses_brace_syntax,
                            structural_record_arg,
                            parenthesized_record_arg,
                        ));
                    }
                }
                if let Expr::Identifier {
                    name: type_name, ..
                } = receiver.as_ref()
                {
                    if let Some(sig) = self.lookup_type_local(type_name) {
                        if let Some(case) = sig.enum_cases.get(name).cloned() {
                            let display_name = format!("{type_name}.{name}");
                            return Some(self.check_enum_case_constructor_signature(
                                &display_name,
                                &case,
                                args,
                                span,
                                uses_brace_syntax,
                                expected,
                            ));
                        }
                        if sig.kind == TypeKind::Object {
                            return None;
                        }
                    }
                    if let Some(sig) = self.world.lookup_imported_type(self.module, type_name) {
                        if let Some(case) = sig.enum_cases.get(name).cloned() {
                            let display_name = format!("{type_name}.{name}");
                            return Some(self.check_enum_case_constructor_signature(
                                &display_name,
                                &case,
                                args,
                                span,
                                uses_brace_syntax,
                                expected,
                            ));
                        }
                    }
                    if let Some(sig) = self.world.ambient.types.get(type_name) {
                        if let Some(case) = sig.enum_cases.get(name).cloned() {
                            let display_name = format!("{type_name}.{name}");
                            return Some(self.check_enum_case_constructor_signature(
                                &display_name,
                                &case,
                                args,
                                span,
                                uses_brace_syntax,
                                expected,
                            ));
                        }
                    }
                }
                None
            }
            _ => None,
        }
    }

    fn check_intrinsic_collection_constructor(
        &mut self,
        name: &str,
        args: &[crate::ast::CallArg],
        span: crate::source::Span,
        uses_brace_syntax: bool,
    ) -> Option<Ty> {
        match name {
            "Vector" => {
                self.reject_parenthesized_constructor_fields(args, uses_brace_syntax, span);
                let mut item = Ty::Unknown;
                for arg in args {
                    item = join_types(&item, &self.check_expr(&arg.value));
                }
                Some(Ty::Named(name.to_string(), vec![item]))
            }
            "Map" => {
                self.reject_parenthesized_constructor_fields(args, uses_brace_syntax, span);
                let mut key = Ty::Unknown;
                let mut value = Ty::Unknown;
                for arg in args {
                    if let Expr::Spread { value: spread, .. } = &arg.value {
                        match self.check_expr(spread) {
                            Ty::Named(name, args) if name == "Map" && args.len() == 2 => {
                                key = join_types(&key, &args[0]);
                                value = join_types(&value, &args[1]);
                            }
                            Ty::Unknown => {}
                            other => {
                                self.add_error(
                                    "invalid_map_spread",
                                    format!(
                                        "map spread requires a map value, got '{}'",
                                        other.describe()
                                    ),
                                    arg.span,
                                );
                            }
                        }
                        continue;
                    }
                    match self.check_expr(&arg.value) {
                        Ty::Tuple(items) if items.len() == 2 => {
                            key = join_types(&key, &items[0]);
                            value = join_types(&value, &items[1]);
                        }
                        other => {
                            self.add_error(
                                "invalid_argument_type",
                                format!(
                                    "Map constructor expects tuple pair arguments, got '{}'",
                                    other.describe()
                                ),
                                arg.span,
                            );
                        }
                    }
                }
                self.require_hashable_map_key(&key, span);
                Some(Ty::Named("Map".to_string(), vec![key, value]))
            }
            _ => None,
        }
    }

    fn require_hashable_map_key(&mut self, key: &Ty, span: crate::source::Span) {
        if matches!(key, Ty::Unknown | Ty::Wildcard | Ty::Capture(_)) {
            return;
        }
        if !self.is_hashable_type(key, &mut HashSet::new()) {
            self.add_error(
                "map_key_not_hashable",
                format!(
                    "map key type '{}' does not satisfy 'Hashed[{}]'; map keys must provide equality-compatible hashing",
                    key.describe(),
                    key.describe()
                ),
                span,
            );
        }
    }

    fn check_named_type_constructor(
        &mut self,
        sig: &TypeSig,
        args: &[crate::ast::CallArg],
        span: crate::source::Span,
        uses_brace_syntax: bool,
        structural_record_arg: bool,
        parenthesized_record_arg: bool,
    ) -> Ty {
        let ret = Ty::Named(
            sig.name.clone(),
            sig.type_params
                .iter()
                .map(|name| Ty::TypeParam(name.clone()))
                .collect(),
        );

        if parenthesized_record_arg {
            self.add_error(
                "no_matching_overload",
                format!(
                    "constructor syntax for '{}' does not accept anonymous shape arguments in '(...)'; use construction fields in braces or positional values directly",
                    sig.name
                ),
                span,
            );
            return ret;
        }

        self.reject_parenthesized_constructor_fields(args, uses_brace_syntax, span);

        if sig.kind == TypeKind::Annotation {
            self.add_error(
                "invalid_annotation_construction",
                format!(
                    "annotation '{}' cannot be constructed as a value; use '@{} {{ ... }}' as metadata",
                    sig.name, sig.name
                ),
                span,
            );
            return ret;
        }

        if sig.kind == TypeKind::Object {
            self.add_error(
                "invalid_object_construction",
                format!(
                    "object '{}' cannot be constructed; reference '{}' directly",
                    sig.name, sig.name
                ),
                span,
            );
            return ret;
        }

        if sig.kind == TypeKind::Interface {
            self.add_error(
                "invalid_interface_construction",
                format!(
                    "interface '{}' cannot be constructed directly; use 'object with {} {{ ... }}' and define all required members",
                    sig.name, sig.name
                ),
                span,
            );
            return ret;
        }

        let constructor_overloads = if sig.kind == TypeKind::Record {
            None
        } else {
            sig.methods.get("new")
        };

        let explicit_constructor_args;
        let args = if structural_record_arg && constructor_overloads.is_some() {
            explicit_constructor_args = brace_record_constructor_args(args).unwrap_or_default();
            explicit_constructor_args.as_slice()
        } else {
            args
        };

        if structural_record_arg {
            if constructor_overloads.is_none() {
                return self.check_record_constructor_conversion(sig, &ret, &args[0].value, span);
            }
        }

        if let Some(overloads) = constructor_overloads {
            let can_access_hidden = self.can_access_hidden_constructor(sig);
            let visible = overloads
                .iter()
                .filter(|ctor| ctor.visibility != Visibility::Hidden || can_access_hidden)
                .cloned()
                .collect::<Vec<_>>();
            self.diagnose_ambiguous_shape_context_call(&visible, args, span);
            if let Some(ctor) = self.choose_overload(&visible, args) {
                let params = constructor_field_sigs_from_params(&ctor.params);
                return self.check_constructor_signature(&params, &ret, args, span);
            }
            if args.iter().all(|arg| arg.name.is_none()) {
                for ctor in &visible {
                    let params = constructor_field_sigs_from_params(&ctor.params);
                    if let Some(message) =
                        positional_constructor_prefix_message(&sig.name, &params, args.len())
                    {
                        self.add_error("invalid_argument_count", message, span);
                        return ret;
                    }
                }
            }
            let hidden = overloads
                .iter()
                .filter(|ctor| ctor.visibility == Visibility::Hidden && !can_access_hidden)
                .cloned()
                .collect::<Vec<_>>();
            if self.choose_overload(&hidden, args).is_some() {
                let help = self.hidden_constructor_factory_help(&sig.name, args);
                self.diagnostics
                    .push(typecheck_diagnostics::hidden_field_constructor(
                        span, &sig.name, help,
                    ));
                return ret;
            }
            self.add_error(
                "no_matching_overload",
                format!(
                    "no constructor overload for {} '{}' matches {} arguments",
                    type_kind_label(sig.kind),
                    sig.name,
                    args.len()
                ),
                span,
            );
            return ret;
        }

        if matches!(sig.kind, TypeKind::Class | TypeKind::Record) {
            if uses_brace_syntax {
                self.add_error(
                    "no_matching_overload",
                    format!(
                        "{} '{}' brace construction requires construction fields",
                        type_kind_label(sig.kind),
                        sig.name
                    ),
                    span,
                );
                return ret;
            }
            return self.check_positional_record_constructor_conversion(sig, &ret, args, span);
        }

        self.add_error(
            "no_matching_overload",
            format!(
                "{} '{}' cannot be constructed with {} arguments{}",
                type_kind_label(sig.kind),
                sig.name,
                args.len(),
                if sig.kind == TypeKind::Record {
                    ""
                } else {
                    " or define 'new'"
                }
            ),
            span,
        );
        ret
    }

    fn check_constructor_signature(
        &mut self,
        params: &[FieldSig],
        ret: &Ty,
        args: &[crate::ast::CallArg],
        span: crate::source::Span,
    ) -> Ty {
        if args.iter().all(|arg| arg.name.is_none()) {
            if let Some(message) = positional_constructor_prefix_message(
                constructor_target_name(ret).unwrap_or("constructor"),
                params,
                args.len(),
            ) {
                self.add_error("invalid_argument_count", message, span);
                return materialize_type(ret);
            }
            let arrangement = arrange_constructor_args(params, args);
            let min_required = params
                .iter()
                .filter(|param| !param.variadic && !param.has_initializer)
                .count();
            let max_allowed = if params.last().is_some_and(|param| param.variadic) {
                usize::MAX
            } else {
                params.len()
            };
            if arrangement.overflow > 0
                || arrangement.missing_required > 0
                || args.len() < min_required
                || args.len() > max_allowed
            {
                self.add_error(
                    "invalid_argument_count",
                    format!(
                        "call expects {}..{} arguments, got {}",
                        min_required,
                        if max_allowed == usize::MAX {
                            "many".to_string()
                        } else {
                            max_allowed.to_string()
                        },
                        args.len()
                    ),
                    span,
                );
            }
            let mut subst = HashMap::new();
            let mut checked_args = Vec::new();
            for (index, param) in params.iter().enumerate() {
                for arg in arrangement
                    .slots
                    .get(index)
                    .map(Vec::as_slice)
                    .unwrap_or(&[])
                {
                    if matches!(arg.value, Expr::Spread { .. })
                        && (!param.variadic || arg.name.is_some())
                    {
                        self.add_error(
                            "invalid_spread_argument",
                            "spread arguments are only valid as positional arguments for a vararg parameter",
                            arg.span,
                        );
                    }
                    let raw_expected = call_arg_expected_ty_for_arg(param.variadic, &param.ty, arg);
                    let expected = substitute_type(&raw_expected, &subst);
                    let actual = self.check_expr_against(call_arg_value_expr(arg), &expected);
                    infer_type_subst(&expected, &actual, &mut subst);
                    checked_args.push((arg.span, actual, raw_expected, String::new()));
                }
            }
            for (arg_span, actual, raw_expected, _) in checked_args {
                let expected = materialize_type(&substitute_type(&raw_expected, &subst));
                self.require_assignable(
                    &actual,
                    &expected,
                    arg_span,
                    "invalid_argument_type",
                    self.diagnostic_type_mismatch_message(
                        "constructor argument",
                        &actual,
                        "constructor parameter",
                        &expected,
                    ),
                );
            }
            let ret = materialize_type(&substitute_type(ret, &subst));
            return self.capture_wildcards(ret);
        }

        let arrangement = arrange_constructor_args(params, args);
        if arrangement.overflow > 0 || arrangement.missing_required > 0 {
            let min_required = params
                .iter()
                .filter(|param| !param.variadic && !param.has_initializer)
                .count();
            let max_allowed = if params.last().is_some_and(|param| param.variadic) {
                usize::MAX
            } else {
                params.len()
            };
            self.add_error(
                "invalid_argument_count",
                format!(
                    "call expects {}..{} arguments, got {}",
                    min_required,
                    if max_allowed == usize::MAX {
                        "many".to_string()
                    } else {
                        max_allowed.to_string()
                    },
                    args.len()
                ),
                span,
            );
        }
        let mut subst = HashMap::new();
        let mut checked_args = Vec::new();
        for (index, param) in params.iter().enumerate() {
            for arg in arrangement
                .slots
                .get(index)
                .map(Vec::as_slice)
                .unwrap_or(&[])
            {
                if matches!(arg.value, Expr::Spread { .. })
                    && (!param.variadic || arg.name.is_some())
                {
                    self.add_error(
                        "invalid_spread_argument",
                        "spread arguments are only valid as positional arguments for a vararg parameter",
                        arg.span,
                    );
                }
                let raw_expected = call_arg_expected_ty_for_arg(param.variadic, &param.ty, arg);
                let expected = substitute_type(&raw_expected, &subst);
                let actual = self.check_expr_against(call_arg_value_expr(arg), &expected);
                infer_type_subst(&expected, &actual, &mut subst);
                checked_args.push((arg.span, actual, expected, param.name.clone()));
            }
        }

        for (arg_span, actual, expected, field_name) in checked_args {
            self.require_assignable(
                &actual,
                &materialize_type(&expected),
                arg_span,
                "invalid_argument_type",
                if field_name.is_empty() {
                    self.diagnostic_type_mismatch_message(
                        "constructor argument",
                        &actual,
                        "constructor parameter",
                        &materialize_type(&expected),
                    )
                } else {
                    format!(
                        "argument for '{}' has type {} but expects {}",
                        field_name,
                        self.diagnostic_type_phrase(&actual),
                        self.diagnostic_type_phrase(&materialize_type(&expected))
                    )
                },
            );
        }

        let ret = materialize_type(&substitute_type(ret, &subst));
        self.capture_wildcards(ret)
    }

    fn check_enum_case_constructor_signature(
        &mut self,
        case_name: &str,
        case: &EnumCaseSig,
        args: &[crate::ast::CallArg],
        span: crate::source::Span,
        uses_brace_syntax: bool,
        expected: &Ty,
    ) -> Ty {
        let (params, ret) = self.materialized_enum_case_signature_against(case, expected);
        if case.field_count == 0 && args.is_empty() {
            self.add_error(
                "invalid_enum_case_call",
                format!("enum case '{case_name}' does not accept call syntax; use '{case_name}'"),
                span,
            );
            return ret;
        }
        self.reject_parenthesized_constructor_fields(args, uses_brace_syntax, span);
        self.check_constructor_signature(&params, &ret, args, span)
    }

    fn materialized_enum_case_signature_against(
        &self,
        case: &EnumCaseSig,
        expected: &Ty,
    ) -> (Vec<FieldSig>, Ty) {
        let mut subst = HashMap::new();
        if !matches!(expected, Ty::Unknown) {
            infer_type_subst(&case.result, expected, &mut subst);
        }
        let params = case
            .params
            .iter()
            .map(|param| FieldSig {
                name: param.name.clone(),
                ty: substitute_type(&param.ty, &subst),
                mutable: param.mutable,
                hidden: param.hidden,
                has_initializer: param.has_initializer,
                variadic: param.variadic,
            })
            .collect();
        // Preserve enum type parameters that the expected type leaves unknown so
        // constructor arguments can still infer them. For example, contextual
        // `Result[<unknown>, E]` plus `Ok(())` must produce `Result[Unit, E]`.
        let ret = substitute_type(&case.result, &subst);
        (params, ret)
    }

    fn callable_signature_for_args(
        &mut self,
        callee: &Expr,
        args: &[crate::ast::CallArg],
        _uses_brace_syntax: bool,
        span: crate::source::Span,
    ) -> Option<CallableSelection> {
        let (callee, explicit_type_args) = self.split_generic_call_callee(callee);
        match callee {
            Expr::Identifier { name, .. } => {
                if let Some(functions) = self.lookup_functions(name) {
                    self.diagnose_ambiguous_shape_context_call(&functions, args, span);
                    let sig = self
                        .choose_overload(&functions, args)
                        .or_else(|| functions.first())?
                        .clone();
                    return Some(CallableSelection {
                        sig,
                        explicit_type_args,
                    });
                }
                if let Some(methods) = self.lookup_implicit_method_functions(name) {
                    self.diagnose_ambiguous_shape_context_call(&methods, args, span);
                    let sig = self
                        .choose_overload(&methods, args)
                        .or_else(|| methods.first())?
                        .clone();
                    return Some(CallableSelection {
                        sig,
                        explicit_type_args,
                    });
                }
                None
            }
            Expr::Member { receiver, name, .. } => {
                if let Some((module, member)) =
                    module_alias_and_member(callee).and_then(|(alias, member)| {
                        self.world
                            .lookup_module_alias(self.module, &alias)
                            .map(|module| (module, member))
                    })
                {
                    if let Some(functions) = module.functions.get(&member) {
                        self.diagnose_ambiguous_shape_context_call(functions, args, span);
                        let sig = self
                            .choose_overload(functions, args)
                            .or_else(|| functions.first())?
                            .clone();
                        return Some(CallableSelection {
                            sig,
                            explicit_type_args,
                        });
                    }
                }
                if let Some(sigs) = self.static_method_sigs(receiver, name) {
                    self.diagnose_ambiguous_shape_context_call(&sigs, args, span);
                    let sig = self
                        .choose_overload(&sigs, args)
                        .or_else(|| sigs.first())?
                        .clone();
                    return Some(CallableSelection {
                        sig,
                        explicit_type_args,
                    });
                }
                let receiver_ty = self.check_expr(receiver);
                let methods = self.member_method_sigs(&receiver_ty, name)?;
                self.diagnose_ambiguous_shape_context_call(&methods, args, span);
                let method = self
                    .choose_overload(&methods, args)
                    .or_else(|| methods.first())?
                    .clone();
                Some(CallableSelection {
                    sig: method,
                    explicit_type_args,
                })
            }
            _ => None,
        }
    }

    fn diagnose_ambiguous_shape_context_call(
        &mut self,
        overloads: &[FunctionSig],
        args: &[crate::ast::CallArg],
        span: crate::source::Span,
    ) {
        let shape_span = args
            .iter()
            .find_map(|arg| first_shape_literal_span(&arg.value));
        let empty_collection_span = args
            .iter()
            .find_map(|arg| empty_collection_literal_span(&arg.value));
        if shape_span.is_none() && empty_collection_span.is_none() {
            return;
        }
        let candidate_count = self.shape_context_candidate_count(overloads, args);
        if candidate_count <= 1 {
            return;
        }
        if let Some(diagnostic_span) = empty_collection_span {
            self.add_error(
                "ambiguous_empty_collection",
                "empty collection '[]' matches multiple vector/map overloads; add an intermediate typed binding",
                diagnostic_span,
            );
        } else {
            self.add_error(
                "ambiguous_shape_context",
                "shape(...) in an overloaded call needs a unique expected anonymous shape type; add an intermediate anonymous shape annotation",
                shape_span.unwrap_or(span),
            );
        }
    }

    fn shape_context_candidate_count(
        &self,
        overloads: &[FunctionSig],
        args: &[crate::ast::CallArg],
    ) -> usize {
        let arg_types = args
            .iter()
            .map(|arg| self.probe_expr_type(call_arg_value_expr(arg)))
            .collect::<Vec<_>>();
        overloads
            .iter()
            .filter(|sig| {
                let arrangement = arrange_param_args(&sig.params, args);
                if arrangement.overflow > 0 || arrangement.missing_required > 0 {
                    return false;
                }
                for (index, param) in sig.params.iter().enumerate() {
                    for arg in arrangement
                        .slots
                        .get(index)
                        .map(Vec::as_slice)
                        .unwrap_or(&[])
                    {
                        let raw_expected =
                            call_arg_expected_ty_for_arg(param.variadic, &param.ty, arg);
                        let arg_index = args
                            .iter()
                            .position(|candidate| std::ptr::eq(candidate, *arg))
                            .unwrap_or(0);
                        if empty_collection_literal_span(&arg.value).is_some() {
                            if !is_list_or_map_ty(&raw_expected) {
                                return false;
                            }
                            continue;
                        }
                        if first_shape_literal_span(&arg.value).is_some() {
                            if !self.shape_expr_can_use_expected(&arg.value, &raw_expected) {
                                return false;
                            }
                            continue;
                        }
                        let actual = &arg_types[arg_index];
                        if matches!(actual, Ty::Unknown) {
                            continue;
                        }
                        if !self.arg_matches_expected(arg, actual, &raw_expected)
                            && !type_contains_type_param(&raw_expected)
                        {
                            return false;
                        }
                    }
                }
                true
            })
            .count()
    }

    fn shape_expr_can_use_expected(&self, expr: &Expr, expected: &Ty) -> bool {
        match expr {
            Expr::ShapeLiteral { items, .. } => {
                let Ty::Record(fields) = expected else {
                    return false;
                };
                items.len() == fields.len()
                    && items.iter().zip(fields.iter()).all(|(item, (_, ty))| {
                        let actual = self.probe_expr_type(item);
                        matches!(actual, Ty::Unknown)
                            || self.is_assignable(&actual, ty)
                            || type_contains_type_param(ty)
                    })
            }
            Expr::Group { inner, .. } => self.shape_expr_can_use_expected(inner, expected),
            _ => true,
        }
    }

    fn callable_signature_for_args_probe(
        &self,
        callee: &Expr,
        args: &[crate::ast::CallArg],
        _uses_brace_syntax: bool,
    ) -> Option<(Vec<ParamSig>, Ty)> {
        let (callee, explicit_type_args) = self.split_generic_call_callee(callee);
        match callee {
            Expr::Identifier { name, .. } => {
                if let Some(functions) = self.lookup_functions(name) {
                    let sig = self
                        .choose_overload(&functions, args)
                        .or_else(|| functions.first())?
                        .clone();
                    return Some(function_sig_parts_for_probe(sig, &explicit_type_args));
                }
                let methods = self.lookup_implicit_method_functions(name)?;
                let sig = self
                    .choose_overload(&methods, args)
                    .or_else(|| methods.first())?
                    .clone();
                Some(function_sig_parts_for_probe(sig, &explicit_type_args))
            }
            Expr::Member { receiver, name, .. } => {
                if let Some((module, member)) =
                    module_alias_and_member(callee).and_then(|(alias, member)| {
                        self.world
                            .lookup_module_alias(self.module, &alias)
                            .map(|module| (module, member))
                    })
                {
                    if let Some(functions) = module.functions.get(&member) {
                        let sig = self
                            .choose_overload(functions, args)
                            .or_else(|| functions.first())?
                            .clone();
                        return Some(function_sig_parts_for_probe(sig, &explicit_type_args));
                    }
                }
                if let Some(sigs) = self.static_method_sigs(receiver, name) {
                    let sig = self
                        .choose_overload(&sigs, args)
                        .or_else(|| sigs.first())?
                        .clone();
                    return Some(function_sig_parts_for_probe(sig, &explicit_type_args));
                }
                let receiver_ty = self.probe_expr_type(receiver);
                let methods = self.member_method_sigs(&receiver_ty, name)?;
                let method = self
                    .choose_overload(&methods, args)
                    .or_else(|| methods.first())?
                    .clone();
                Some(function_sig_parts_for_probe(method, &explicit_type_args))
            }
            _ => None,
        }
    }

    fn split_generic_call_callee<'expr>(&self, callee: &'expr Expr) -> (&'expr Expr, Vec<Ty>) {
        let Expr::Index {
            receiver, index, ..
        } = callee
        else {
            return (callee, Vec::new());
        };
        let Some(type_args) = type_arg_refs_from_expr(index) else {
            return (callee, Vec::new());
        };
        (
            receiver.as_ref(),
            type_args
                .iter()
                .map(|arg| self.ty_from_type_ref(arg))
                .collect(),
        )
    }

    fn normalize_trailing_brace_call_args(
        &self,
        callee: &Expr,
        args: &[crate::ast::CallArg],
        uses_brace_syntax: bool,
    ) -> Vec<crate::ast::CallArg> {
        if uses_brace_syntax
            && (self.brace_call_targets_explicit_constructor(callee)
                || self.brace_call_targets_current_constructor(callee)
                || self.brace_call_uses_constructor_delegation_syntax(callee)
                || self.brace_call_targets_enum_case(callee))
        {
            if let Some(args) = brace_record_constructor_args(args) {
                return args;
            }
        }
        args.to_vec()
    }

    fn brace_call_targets_explicit_constructor(&self, callee: &Expr) -> bool {
        self.brace_call_type_sig(callee)
            .is_some_and(|sig| sig.kind == TypeKind::Class && sig.methods.contains_key("new"))
    }

    fn brace_call_targets_current_constructor(&self, callee: &Expr) -> bool {
        matches!(callee, Expr::Identifier { name, .. } if name == "this")
            && self.current_method.as_deref() == Some("new")
            && self.current_owner.is_some()
    }

    fn brace_call_uses_constructor_delegation_syntax(&self, callee: &Expr) -> bool {
        matches!(callee, Expr::Identifier { name, .. } if name == "this")
            || (matches!(callee, Expr::Identifier { name, .. } if name == "new")
                && self.current_method.as_deref() == Some("new")
                && self.current_owner.is_some())
    }

    fn brace_call_targets_enum_case(&self, callee: &Expr) -> bool {
        match callee {
            Expr::Identifier { name, .. } => {
                self.world.lookup_enum_case(self.module, name).is_some()
            }
            Expr::Member { receiver, name, .. } => {
                let Expr::Identifier {
                    name: type_name, ..
                } = receiver.as_ref()
                else {
                    return false;
                };
                self.lookup_type_local(type_name)
                    .or_else(|| self.world.lookup_imported_type(self.module, type_name))
                    .or_else(|| self.world.ambient.types.get(type_name).cloned())
                    .is_some_and(|sig| sig.enum_cases.contains_key(name))
            }
            _ => false,
        }
    }

    fn brace_call_type_sig(&self, callee: &Expr) -> Option<TypeSig> {
        let class_sig = match callee {
            Expr::Identifier { name, .. } => self
                .lookup_type_local(name)
                .or_else(|| self.world.lookup_imported_type(self.module, name))
                .or_else(|| self.lookup_unique_module_type(name))
                .or_else(|| self.lookup_any_object(name))
                .or_else(|| self.world.ambient.types.get(name).cloned()),
            Expr::Member { .. } => module_alias_and_member(callee).and_then(|(alias, member)| {
                self.world
                    .lookup_module_alias(self.module, &alias)
                    .and_then(|module| {
                        module
                            .types
                            .get(&member)
                            .cloned()
                            .or_else(|| module.objects.get(&member).cloned())
                    })
            }),
            _ => None,
        };

        class_sig
    }

    fn static_member_value_type(&self, receiver: &Expr, name: &str, expected: &Ty) -> Option<Ty> {
        let Expr::Identifier {
            name: type_name, ..
        } = receiver
        else {
            return None;
        };
        if let Some(sig) = self.lookup_any_object(type_name) {
            if let Some(methods) = self.method_sigs_for_type(&sig, name) {
                let first = methods.first()?;
                return Some(Ty::Function(
                    first.params.iter().map(|param| param.ty.clone()).collect(),
                    Box::new(first.ret.clone()),
                ));
            }
        }
        let sig = self.lookup_any_non_object_type(type_name)?;
        if let Some(case) = sig.enum_cases.get(name) {
            if case.params.is_empty() {
                return Some(self.materialize_enum_case_result_against(&case.result, expected));
            }
        }
        None
    }

    fn static_method_sigs(&self, receiver: &Expr, name: &str) -> Option<Vec<FunctionSig>> {
        let Expr::Identifier {
            name: type_name, ..
        } = receiver
        else {
            return None;
        };
        if let Some(sig) = self.lookup_any_object(type_name) {
            return self.method_sigs_for_type(&sig, name);
        }
        let sig = self.lookup_any_non_object_type(type_name)?;
        self.method_sigs_for_type(&sig, name)
    }

    fn hidden_constructor_factory_help(
        &self,
        class_name: &str,
        args: &[crate::ast::CallArg],
    ) -> Option<String> {
        let object = self.lookup_any_object(class_name)?;
        self.method_sigs_for_type(&object, "create")?;
        let args = format_factory_help_args(args)?;
        Some(format!("use {class_name}.create({args})"))
    }

    fn check_binary_expr(
        &mut self,
        left: &Ty,
        op: BinaryOp,
        right: &Ty,
        span: crate::source::Span,
    ) -> Ty {
        match op {
            BinaryOp::Add => {
                if left.is_str() || right.is_str() {
                    Ty::str()
                } else if left.is_float_like() || right.is_float_like() {
                    if !left.is_numeric() && !matches!(left, Ty::Unknown) {
                        self.add_error(
                            "invalid_binary_operand",
                            "left operand must be numeric",
                            span,
                        );
                    }
                    if !right.is_numeric() && !matches!(right, Ty::Unknown) {
                        self.add_error(
                            "invalid_binary_operand",
                            "right operand must be numeric",
                            span,
                        );
                    }
                    Ty::float()
                } else if left.is_int_like() && right.is_int_like() {
                    Ty::int()
                } else {
                    Ty::Unknown
                }
            }
            BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Mod => {
                if !left.is_numeric() && !matches!(left, Ty::Unknown) {
                    self.add_error(
                        "invalid_binary_operand",
                        "left operand must be numeric",
                        span,
                    );
                }
                if !right.is_numeric() && !matches!(right, Ty::Unknown) {
                    self.add_error(
                        "invalid_binary_operand",
                        "right operand must be numeric",
                        span,
                    );
                }
                if left.is_float_like() || right.is_float_like() {
                    Ty::float()
                } else {
                    Ty::int()
                }
            }
            BinaryOp::Eq | BinaryOp::NotEq => {
                self.check_equality_operands(left, right, span);
                Ty::bool()
            }
            BinaryOp::Less
            | BinaryOp::LessEq
            | BinaryOp::Greater
            | BinaryOp::GreaterEq
            | BinaryOp::And
            | BinaryOp::Or => {
                if matches!(op, BinaryOp::And | BinaryOp::Or) {
                    self.require_bool(left, span, "logical operator expects Bool operands");
                    self.require_bool(right, span, "logical operator expects Bool operands");
                }
                Ty::bool()
            }
            BinaryOp::IdentityEq | BinaryOp::IdentityNotEq => {
                let left_is_reference = self.is_identity_reference_type(left);
                let right_is_reference = self.is_identity_reference_type(right);
                let unknown = matches!(left, Ty::Unknown) || matches!(right, Ty::Unknown);

                if !unknown && (!left_is_reference || !right_is_reference) {
                    self.add_error(
                        "invalid_identity_operand",
                        format!(
                            "identity operators require class, object, or concrete collection references; got '{}' and '{}'",
                            left.describe(),
                            right.describe()
                        ),
                        span,
                    );
                } else if left_is_reference
                    && right_is_reference
                    && !self.is_assignable(left, right)
                    && !self.is_assignable(right, left)
                {
                    self.add_error(
                        "incompatible_identity_operands",
                        format!(
                            "identity comparison requires compatible reference types; '{}' and '{}' cannot reference the same instance",
                            left.describe(),
                            right.describe()
                        ),
                        span,
                    );
                }

                Ty::bool()
            }
            BinaryOp::Colon => {
                self.add_error(
                    "removed_pair_expression",
                    "':' pair expressions are no longer supported; use '(left, right)' for tuple pairs or '[key: value]' for maps",
                    span,
                );
                Ty::Unknown
            }
        }
    }

    fn check_equality_operands(&mut self, left: &Ty, right: &Ty, span: crate::source::Span) {
        if matches!(left, Ty::Unknown) || matches!(right, Ty::Unknown) {
            return;
        }
        if left.is_any() || right.is_any() {
            self.add_error(
                "dynamic_equality_requires_same_value",
                format!(
                    "'==' and '!=' do not compare Any values; use 'sameValue(...)' for strict dynamic equality or narrow the value before comparing it (got '{}' and '{}')",
                    left.describe(),
                    right.describe()
                ),
                span,
            );
            return;
        }

        let left_is_shape = self.shape_target_fields(left).is_some();
        let right_is_shape = self.shape_target_fields(right).is_some();
        if left_is_shape || right_is_shape {
            self.check_shape_equality_operands(left, right, span);
            return;
        }

        let same_domain = left == right
            || matches!((left, right), (Ty::TypeParam(a), Ty::TypeParam(b)) if self.type_params_are_equal(a, b));
        if !same_domain {
            self.add_error(
                "incompatible_equality_operands",
                format!(
                    "equality requires the same static equality domain; '{}' and '{}' are different types",
                    left.describe(),
                    right.describe()
                ),
                span,
            );
            return;
        }

        let intrinsic_domain = matches!(
            left,
            Ty::Named(name, args)
                if args.is_empty()
                    && matches!(
                        name.as_str(),
                        "Bool" | "Float" | "Int" | "Rune" | "Str" | "Unit"
                    )
        );
        let requires_contract = !intrinsic_domain
            && match left {
                Ty::Named(name, _) => self
                    .lookup_any_type(name)
                    .is_some_and(|sig| matches!(sig.kind, TypeKind::Class | TypeKind::Interface)),
                Ty::TypeParam(_) => true,
                _ => false,
            };
        if requires_contract {
            let equality_contract = Ty::Named("Eq".to_string(), vec![left.clone()]);
            if !self.is_assignable(left, &equality_contract) {
                self.add_error(
                    "missing_equality_contract",
                    format!(
                        "type '{}' has no equality contract; declare 'with Eq[{}]' before using '==' or '!='",
                        left.describe(),
                        left.describe()
                    ),
                    span,
                );
            }
        }
    }

    fn check_shape_equality_operands(&mut self, left: &Ty, right: &Ty, span: crate::source::Span) {
        let left_fields = self.shape_target_fields(left);
        let right_fields = self.shape_target_fields(right);
        let unknown = matches!(left, Ty::Unknown) || matches!(right, Ty::Unknown);

        let (Some(left_fields), Some(right_fields)) = (left_fields, right_fields) else {
            if !unknown
                && (self.shape_target_fields(left).is_some()
                    || self.shape_target_fields(right).is_some())
            {
                self.add_error(
                    "incompatible_shape_equality",
                    format!(
                        "shape equality requires matching shape operands; got '{}' and '{}'",
                        left.describe(),
                        right.describe()
                    ),
                    span,
                );
            }
            return;
        };

        if left_fields.len() != right_fields.len() {
            self.add_error(
                "incompatible_shape_equality",
                format!(
                    "shape equality requires identical fields; '{}' has {} field(s) but '{}' has {}",
                    left.describe(),
                    left_fields.len(),
                    right.describe(),
                    right_fields.len()
                ),
                span,
            );
            return;
        }

        for left_field in &left_fields {
            let Some(right_field) = right_fields
                .iter()
                .find(|right_field| right_field.name == left_field.name)
            else {
                self.add_error(
                    "incompatible_shape_equality",
                    format!(
                        "shape equality requires identical fields; '{}' has no field '{}' required by '{}'",
                        right.describe(),
                        left_field.name,
                        left.describe()
                    ),
                    span,
                );
                return;
            };
            if !self.is_assignable(&right_field.ty, &left_field.ty)
                || !self.is_assignable(&left_field.ty, &right_field.ty)
            {
                self.add_error(
                    "incompatible_shape_equality",
                    format!(
                        "shape equality field '{}' has different types: '{}' in '{}' and '{}' in '{}'",
                        left_field.name,
                        left_field.ty.describe(),
                        left.describe(),
                        right_field.ty.describe(),
                        right.describe()
                    ),
                    span,
                );
                return;
            }
        }
    }

    fn is_identity_reference_type(&self, ty: &Ty) -> bool {
        let Ty::Named(name, _) = ty else {
            return false;
        };
        if matches!(
            name.as_str(),
            "Any" | "Bool" | "Float" | "Int" | "Rune" | "Str" | "Unit"
        ) {
            return false;
        }
        self.lookup_any_type(name)
            .is_some_and(|sig| matches!(sig.kind, TypeKind::Class | TypeKind::Object))
    }

    fn check_else_expr_branch(&mut self, branch: &ElseExprBranch) -> Ty {
        match branch {
            ElseExprBranch::If(expr) => self.check_expr(expr),
            ElseExprBranch::Block(block) => self.check_block(block),
        }
    }

    fn check_else_expr_branch_with_narrowing(
        &mut self,
        branch: &ElseExprBranch,
        narrowing: Option<&TypeNarrowing>,
    ) -> Ty {
        self.push_scope();
        if let Some(narrowing) = narrowing {
            self.define_local(&narrowing.name, narrowing.ty.clone(), false);
        }
        let result = self.check_else_expr_branch(branch);
        self.pop_scope();
        result
    }

    fn type_narrowing_for_condition(
        &self,
        condition: &Expr,
        condition_is_true: bool,
    ) -> Option<TypeNarrowing> {
        match condition {
            Expr::Group { inner, .. } => {
                self.type_narrowing_for_condition(inner, condition_is_true)
            }
            Expr::Unary {
                op: crate::ast::UnaryOp::Not,
                expr,
                ..
            } => self.type_narrowing_for_condition(expr, !condition_is_true),
            Expr::Is { left, target, .. } if condition_is_true => {
                let Expr::Identifier { name, .. } = left.as_ref() else {
                    return None;
                };
                let value = self.lookup_scoped_value(name)?;
                if !value.stable || runtime_type_ref_has_arguments(target) {
                    return None;
                }
                let ty = self.ty_from_type_ref(target);
                if matches!(ty, Ty::TypeParam(_) | Ty::Wildcard) {
                    return None;
                }
                Some(TypeNarrowing {
                    name: name.clone(),
                    ty,
                })
            }
            _ => None,
        }
    }

    fn validate_runtime_type_ref(&mut self, reference: &TypeRef, ty: &Ty, construct: &str) {
        match reference {
            TypeRef::Named { args, span, .. } if !args.is_empty() => {
                self.add_error(
                    "erased_runtime_type_arguments",
                    format!(
                        "{construct} cannot specify generic arguments; use the erased outer type"
                    ),
                    *span,
                );
            }
            TypeRef::Named { args, .. } => {
                for arg in args {
                    let arg_ty = self.ty_from_type_ref(arg);
                    self.validate_runtime_type_ref(arg, &arg_ty, construct);
                }
            }
            TypeRef::Tuple { fields, .. } => {
                for field in fields {
                    let field_ty = self.ty_from_type_ref(&field.ty);
                    self.validate_runtime_type_ref(&field.ty, &field_ty, construct);
                }
            }
            TypeRef::Record { fields, .. } => {
                for field in fields {
                    let field_ty = self.ty_from_type_ref(&field.ty);
                    self.validate_runtime_type_ref(&field.ty, &field_ty, construct);
                }
            }
            TypeRef::Function { params, ret, .. } => {
                for param in params {
                    let param_ty = self.ty_from_type_ref(param);
                    self.validate_runtime_type_ref(param, &param_ty, construct);
                }
                let ret_ty = self.ty_from_type_ref(ret);
                self.validate_runtime_type_ref(ret, &ret_ty, construct);
            }
            TypeRef::Wildcard { .. } => {}
        }

        if let Ty::TypeParam(name) = ty {
            self.add_error(
                "unavailable_runtime_type_parameter",
                format!(
                    "generic type '{}' is not available to {construct}; use a concrete erased outer type",
                    name
                ),
                reference.span(),
            );
        }
    }

    fn bind_pattern(&mut self, pattern: &Pattern, scrutinee: &Ty) {
        match pattern {
            Pattern::Wildcard { .. } => {}
            Pattern::Extract { inner, span } => {
                let inner_ty = self.extract_pattern_inner_type(scrutinee, *span);
                self.bind_pattern(inner, &inner_ty);
            }
            Pattern::Binding { name, .. } => self.define_local(name, scrutinee.clone(), false),
            Pattern::Type { name, target, .. } => {
                let target_ty = self.ty_from_type_ref(target);
                self.validate_runtime_type_ref(target, &target_ty, "runtime type patterns");
                if let Some(name) = name {
                    self.define_local(name, target_ty, false);
                }
            }
            Pattern::Literal { value, .. } => {
                let literal_ty = self.check_expr(value);
                if !self.is_assignable(&literal_ty, scrutinee) {
                    self.add_error(
                        "pattern_type_mismatch",
                        format!(
                            "pattern value has type '{}', but the matched field has type '{}'",
                            literal_ty.describe(),
                            scrutinee.describe()
                        ),
                        pattern.span(),
                    );
                }
            }
            Pattern::Tuple { elements, .. } => {
                if let Ty::Tuple(items) = scrutinee {
                    for (pattern, item) in elements.iter().zip(items.iter()) {
                        self.bind_pattern(pattern, item);
                    }
                } else {
                    if !matches!(scrutinee, Ty::Unknown) {
                        self.add_error(
                            "invalid_destructure",
                            format!(
                                "tuple pattern requires a tuple value, got '{}'",
                                scrutinee.describe()
                            ),
                            pattern.span(),
                        );
                    }
                    for pattern in elements {
                        self.bind_pattern(pattern, &Ty::Unknown);
                    }
                }
            }
            Pattern::List { elements, rest, .. } => {
                let element_ty = match self.list_element_type(scrutinee) {
                    Some(element_ty) => element_ty,
                    None if matches!(scrutinee, Ty::Unknown) => Ty::Unknown,
                    None => {
                        self.add_error(
                            "invalid_destructure",
                            format!(
                                "vector pattern requires a Vector value, got '{}'",
                                scrutinee.describe()
                            ),
                            pattern.span(),
                        );
                        Ty::Unknown
                    }
                };
                for element in elements {
                    self.bind_pattern(element, &element_ty);
                }
                if let Some(rest) = rest {
                    if rest.name != "_" {
                        self.define_local(
                            &rest.name,
                            Ty::Named("Vector".to_string(), vec![element_ty]),
                            false,
                        );
                    }
                }
            }
            Pattern::Record { path, fields, .. } => {
                let target_name = if path.is_empty() {
                    scrutinee.describe()
                } else {
                    path.join(".")
                };
                let Some((is_enum_case, _, target_fields)) =
                    self.lookup_record_pattern_target(path, scrutinee)
                else {
                    for field in fields {
                        self.bind_pattern(&field.pattern, &Ty::Unknown);
                    }
                    self.add_error(
                        "unknown_match_case",
                        if path.is_empty() {
                            format!(
                                "headless record pattern requires a concrete class, shape, anonymous shape, or enum value; got '{}'",
                                scrutinee.describe()
                            )
                        } else {
                            format!("unknown record pattern '{target_name}'")
                        },
                        pattern.span(),
                    );
                    return;
                };
                if is_enum_case && target_fields.is_empty() {
                    let case_name = path.last().map(String::as_str).unwrap_or("case");
                    self.add_error(
                        "zero_payload_record_pattern",
                        format!(
                            "zero-payload enum case '{case_name}' is a bare pattern; write '{case_name}' without braces"
                        ),
                        pattern.span(),
                    );
                }
                for field in fields {
                    if let Some((_, field_ty)) =
                        target_fields.iter().find(|(name, _)| name == &field.name)
                    {
                        self.bind_pattern(&field.pattern, field_ty);
                    } else {
                        self.bind_pattern(&field.pattern, &Ty::Unknown);
                        self.add_error(
                            "unknown_pattern_field",
                            format!(
                                "record pattern '{}' has no visible field '{}'",
                                target_name, field.name
                            ),
                            field.span,
                        );
                    }
                }
            }
            Pattern::Constructor { path, args, .. } => {
                let case_name = path.last().cloned().unwrap_or_default();
                if let Some(case) = self.lookup_case_by_pattern(path, scrutinee) {
                    if !args.is_empty() {
                        let fields = case
                            .params
                            .iter()
                            .take(args.len())
                            .map(|field| field.name.as_str())
                            .collect::<Vec<_>>()
                            .join(", ");
                        self.add_error(
                            "positional_record_pattern",
                            format!(
                                "positional enum payload patterns are not supported; write '{} {{ {} }}' with named fields",
                                case_name, fields
                            ),
                            pattern.span(),
                        );
                    }
                    if !enum_case_pattern_accepts_arity(&case.params, args.len()) {
                        self.add_error(
                            "invalid_destructure",
                            format!(
                                "constructor pattern '{}' expects {} fields, got {}",
                                case_name,
                                enum_case_pattern_required_count(&case.params),
                                args.len()
                            ),
                            pattern.span(),
                        );
                    }
                    let mut subst = HashMap::new();
                    infer_type_subst(&case.result, scrutinee, &mut subst);
                    for (pattern, param) in args.iter().zip(case.params.iter()) {
                        self.bind_pattern(
                            pattern,
                            &materialize_type(&substitute_type(&param.ty, &subst)),
                        );
                    }
                } else if let Some(fields) = self.lookup_destructured_type_fields(path) {
                    self.add_error(
                        "positional_record_pattern",
                        format!(
                            "positional class and shape patterns are not supported; write '{} {{ field }}' with named fields",
                            case_name
                        ),
                        pattern.span(),
                    );
                    if args.len() != fields.len() {
                        self.add_error(
                            "invalid_destructure",
                            format!(
                                "constructor pattern '{}' expects {} fields, got {}",
                                case_name,
                                fields.len(),
                                args.len()
                            ),
                            pattern.span(),
                        );
                    }
                    for (pattern, field_ty) in args.iter().zip(fields.iter()) {
                        self.bind_pattern(pattern, field_ty);
                    }
                } else {
                    for pattern in args {
                        self.bind_pattern(pattern, &Ty::Unknown);
                    }
                    if !case_name.is_empty() {
                        self.add_error(
                            "unknown_match_case",
                            format!("unknown constructor pattern '{}'", case_name),
                            pattern.span(),
                        );
                    }
                }
            }
        }
    }

    fn pattern_is_irrefutable(&self, pattern: &Pattern, scrutinee: &Ty) -> bool {
        match pattern {
            Pattern::Wildcard { .. } | Pattern::Binding { .. } => true,
            Pattern::Extract { .. } => false,
            Pattern::Type { target, .. } => {
                !matches!(scrutinee, Ty::Unknown)
                    && self.is_assignable(scrutinee, &self.ty_from_type_ref(target))
            }
            Pattern::Literal { .. } => false,
            Pattern::Tuple { elements, .. } => match scrutinee {
                Ty::Tuple(items) if items.len() == elements.len() => elements
                    .iter()
                    .zip(items.iter())
                    .all(|(pattern, item)| self.pattern_is_irrefutable(pattern, item)),
                _ => false,
            },
            Pattern::List { elements, rest, .. } => {
                elements.is_empty() && rest.is_some() && self.list_element_type(scrutinee).is_some()
            }
            Pattern::Record { path, fields, .. } => {
                let Some((is_enum_case, target_ty, target_fields)) =
                    self.lookup_record_pattern_target(path, scrutinee)
                else {
                    return false;
                };
                !is_enum_case
                    && self.is_assignable(scrutinee, &target_ty)
                    && fields.iter().all(|field| {
                        target_fields
                            .iter()
                            .find(|(name, _)| name == &field.name)
                            .is_some_and(|(_, ty)| self.pattern_is_irrefutable(&field.pattern, ty))
                    })
            }
            Pattern::Constructor { path, args, .. } => {
                if self.lookup_case_by_pattern(path, scrutinee).is_some() {
                    return false;
                }
                let Some((destructured_ty, field_tys)) =
                    self.lookup_destructured_type_pattern(path)
                else {
                    return false;
                };
                self.is_assignable(scrutinee, &destructured_ty)
                    && args.len() == field_tys.len()
                    && args
                        .iter()
                        .zip(field_tys.iter())
                        .all(|(pattern, field_ty)| self.pattern_is_irrefutable(pattern, field_ty))
            }
        }
    }

    fn check_match_exhaustiveness(
        &mut self,
        value_ty: &Ty,
        cases: &[MatchCase],
        span: crate::source::Span,
    ) {
        let Ty::Named(name, _) = value_ty else {
            return;
        };
        let Some(sig) = self.lookup_any_type(name) else {
            return;
        };
        if sig.kind != TypeKind::Enum || sig.enum_cases.is_empty() {
            return;
        }

        let mut covered = HashSet::new();
        let mut wildcard = false;
        for case in cases {
            if case.guard.is_some() {
                continue;
            }
            if self.pattern_is_irrefutable(&case.pattern, value_ty) {
                wildcard = true;
                break;
            }
            match &case.pattern {
                Pattern::Wildcard { .. } | Pattern::Binding { .. } => {
                    wildcard = true;
                    break;
                }
                Pattern::Constructor { path, .. } => {
                    if let Some(case_name) = self.match_case_name_for_enum(path, &sig.name) {
                        covered.insert(case_name);
                    }
                }
                Pattern::Record { path, .. } => {
                    if let Some(case_name) = self.match_case_name_for_enum(path, &sig.name) {
                        covered.insert(case_name);
                    }
                }
                _ => {}
            }
        }
        if wildcard {
            return;
        }
        let missing = sig
            .enum_cases
            .keys()
            .filter(|case_name| !covered.contains(case_name.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        if missing.is_empty() {
            return;
        }
        self.add_error(
            "non_exhaustive_match",
            format!("match does not cover enum cases: {}", missing.join(", ")),
            span,
        );
    }

    fn match_case_name_for_enum<'b>(&self, path: &'b [String], enum_name: &str) -> Option<&'b str> {
        match path {
            [case_name] => self
                .lookup_any_type(enum_name)
                .and_then(|sig| {
                    sig.enum_cases
                        .contains_key(case_name)
                        .then_some(case_name.as_str())
                })
                .or_else(|| {
                    self.lookup_case_by_path(path)
                        .filter(
                            |case| matches!(&case.result, Ty::Named(name, _) if name == enum_name),
                        )
                        .map(|_| case_name.as_str())
                }),
            [type_name, case_name] if type_name == enum_name => Some(case_name.as_str()),
            [module_alias, type_name, case_name] => self
                .world
                .lookup_module_alias(self.module, module_alias)
                .and_then(|module| module.types.get(type_name))
                .filter(|sig| sig.name == enum_name)
                .map(|_| case_name.as_str()),
            _ => None,
        }
    }

    fn lookup_case_by_path(&self, path: &[String]) -> Option<EnumCaseSig> {
        match path {
            [case_name] => self.world.lookup_enum_case(self.module, case_name),
            [type_name, case_name] => self
                .lookup_any_non_object_type(type_name)
                .and_then(|sig| sig.enum_cases.get(case_name).cloned()),
            [module_alias, type_name, case_name] => self
                .world
                .lookup_module_alias(self.module, module_alias)
                .and_then(|module| module.types.get(type_name).cloned())
                .and_then(|sig| sig.enum_cases.get(case_name).cloned()),
            _ => None,
        }
    }

    fn lookup_case_by_pattern(&self, path: &[String], scrutinee: &Ty) -> Option<EnumCaseSig> {
        if let Ty::Named(enum_name, _) = scrutinee {
            if let Some(case_name) = self.match_case_name_for_enum(path, enum_name) {
                if let Some(sig) = self.lookup_any_type(enum_name) {
                    if let Some(case) = sig.enum_cases.get(case_name) {
                        return Some(case.clone());
                    }
                }
            }
        }
        self.lookup_case_by_path(path)
    }

    fn lookup_destructured_type_pattern(&self, path: &[String]) -> Option<(Ty, Vec<Ty>)> {
        let sig = match path {
            [name] => self.lookup_any_type(name),
            [module_alias, name] => self
                .world
                .lookup_module_alias(self.module, module_alias)
                .and_then(|module| {
                    module
                        .types
                        .get(name)
                        .cloned()
                        .or_else(|| module.objects.get(name).cloned())
                }),
            _ => None,
        }?;
        let ty = Ty::Named(
            sig.name.clone(),
            sig.type_params.iter().map(|_| Ty::Unknown).collect(),
        );
        let fields = sig
            .fields
            .iter()
            .filter(|field| !field.hidden)
            .map(|field| field.ty.clone())
            .collect();
        Some((ty, fields))
    }

    fn lookup_destructured_type_fields(&self, path: &[String]) -> Option<Vec<Ty>> {
        self.lookup_destructured_type_pattern(path)
            .map(|(_, fields)| fields)
    }

    fn lookup_record_pattern_target(
        &self,
        path: &[String],
        scrutinee: &Ty,
    ) -> Option<(bool, Ty, Vec<(String, Ty)>)> {
        if path.is_empty() {
            return match scrutinee {
                Ty::Record(fields) => Some((false, scrutinee.clone(), fields.clone())),
                Ty::Named(name, args) => {
                    let sig = self.lookup_any_type(name)?;
                    if !matches!(
                        sig.kind,
                        TypeKind::Class | TypeKind::Record | TypeKind::Enum
                    ) {
                        return None;
                    }
                    let subst = sig
                        .type_params
                        .iter()
                        .cloned()
                        .zip(args.iter().cloned())
                        .collect::<HashMap<_, _>>();
                    Some((
                        false,
                        scrutinee.clone(),
                        sig.fields
                            .iter()
                            .filter(|field| !field.hidden)
                            .map(|field| {
                                (
                                    field.name.clone(),
                                    materialize_type(&substitute_type(&field.ty, &subst)),
                                )
                            })
                            .collect(),
                    ))
                }
                _ => None,
            };
        }
        if let Some(case) = self.lookup_case_by_pattern(path, scrutinee) {
            let mut subst = HashMap::new();
            infer_type_subst(&case.result, scrutinee, &mut subst);
            let fields = case
                .params
                .iter()
                .filter(|field| !field.hidden)
                .map(|field| {
                    (
                        field.name.clone(),
                        materialize_type(&substitute_type(&field.ty, &subst)),
                    )
                })
                .collect();
            return Some((true, case.result, fields));
        }

        let sig = match path {
            [name] => self.lookup_any_type(name),
            [module_alias, name] => self
                .world
                .lookup_module_alias(self.module, module_alias)
                .and_then(|module| {
                    module
                        .types
                        .get(name)
                        .cloned()
                        .or_else(|| module.objects.get(name).cloned())
                }),
            _ => None,
        }?;
        if !matches!(sig.kind, TypeKind::Class | TypeKind::Record) {
            return None;
        }
        let args = match scrutinee {
            Ty::Named(name, args) if name == &sig.name => args.clone(),
            _ => sig.type_params.iter().map(|_| Ty::Unknown).collect(),
        };
        let subst = sig
            .type_params
            .iter()
            .cloned()
            .zip(args.iter().cloned())
            .collect::<HashMap<_, _>>();
        let target = Ty::Named(sig.name.clone(), args);
        let fields = sig
            .fields
            .iter()
            .filter(|field| !field.hidden)
            .map(|field| {
                (
                    field.name.clone(),
                    materialize_type(&substitute_type(&field.ty, &subst)),
                )
            })
            .collect();
        Some((false, target, fields))
    }

    fn unwrap_inner_type(&self, ty: &Ty) -> Ty {
        match ty {
            Ty::Named(name, args) if name == "Option" && args.len() == 1 => args[0].clone(),
            Ty::Named(name, args) if name == "Result" && args.len() >= 1 => args[0].clone(),
            Ty::Named(name, args) if name == "Either" && args.len() == 2 => args[1].clone(),
            _ => Ty::Unknown,
        }
    }

    fn list_element_type(&self, ty: &Ty) -> Option<Ty> {
        match ty {
            Ty::Named(name, args) if name == "Vector" && args.len() == 1 => args.first().cloned(),
            _ => None,
        }
    }

    fn check_try_expr(&mut self, value: &Expr, span: crate::source::Span) -> Ty {
        if self.current_return == Ty::Unknown {
            self.add_error("invalid_try", "try used outside callable body", span);
            return Ty::Unknown;
        }

        let value_ty = self.check_expr(value);
        let inner = self.unwrap_inner_type(&value_ty);
        if inner == Ty::Unknown && !matches!(value_ty, Ty::Unknown) {
            self.add_error(
                "invalid_try",
                format!(
                    "try requires Option[T], Result[T, E], or Either[L, R], got '{}'",
                    value_ty.describe()
                ),
                span,
            );
            return inner;
        }

        if !self.try_propagates_from(&value_ty, &self.current_return) {
            self.add_error(
                "invalid_try",
                format!(
                    "try on '{}' cannot propagate from enclosing return type '{}'",
                    value_ty.describe(),
                    self.current_return.describe()
                ),
                span,
            );
        }
        inner
    }

    fn check_extract_or_expr(
        &mut self,
        value: &Expr,
        fallback: &Expr,
        span: crate::source::Span,
    ) -> Ty {
        let value_ty = self.check_expr(value);
        let inner = self.unwrap_inner_type(&value_ty);
        if inner == Ty::Unknown && !matches!(value_ty, Ty::Unknown) {
            self.add_error(
                "invalid_extract_or",
                format!(
                    "'??' requires Option[T], Result[T, E], or Either[L, R], got '{}'",
                    value_ty.describe()
                ),
                span,
            );
            self.check_expr(fallback);
            return Ty::Unknown;
        }

        let fallback_ty = self.check_expr_against(fallback, &inner);
        self.require_assignable(
            &fallback_ty,
            &inner,
            fallback.span(),
            "invalid_extract_or_fallback",
            format!(
                "'??' fallback has type {} but extracted success value has type {}",
                self.diagnostic_type_phrase(&fallback_ty),
                self.diagnostic_type_phrase(&inner)
            ),
        );
        inner
    }

    fn check_return_control_expr(&mut self, value: Option<&Expr>, span: crate::source::Span) -> Ty {
        if self.defer_depth > 0 {
            self.add_error(
                "invalid_defer_control_flow",
                "defer block cannot contain 'return'",
                span,
            );
        }
        if self.callable_depth == 0 {
            self.add_error(
                "invalid_return",
                "return used outside of a callable body",
                span,
            );
            value.map(|expr| self.check_expr(expr));
            return Ty::never();
        }
        let expected = self.current_return.clone();
        let actual = value
            .map(|expr| self.check_expr_against(expr, &expected))
            .unwrap_or_else(Ty::unit);
        self.require_assignable(
            &actual,
            &expected,
            span,
            "invalid_return_type",
            format!(
                "return has type {} but enclosing callable expects {}",
                self.diagnostic_type_phrase(&actual),
                self.diagnostic_type_phrase(&expected)
            ),
        );
        Ty::never()
    }

    fn check_break_control_expr(&mut self, span: crate::source::Span) -> Ty {
        if self.defer_depth > 0 {
            self.add_error(
                "invalid_defer_control_flow",
                "defer block cannot contain 'break'",
                span,
            );
        }
        if self.loop_depth == 0 {
            self.add_error("invalid_break", "break used outside of a loop", span);
        }
        Ty::never()
    }

    fn check_continue_control_expr(&mut self, span: crate::source::Span) -> Ty {
        if self.defer_depth > 0 {
            self.add_error(
                "invalid_defer_control_flow",
                "defer block cannot contain 'continue'",
                span,
            );
        }
        if self.loop_depth == 0 {
            self.add_error("invalid_continue", "continue used outside of a loop", span);
        }
        Ty::never()
    }

    fn extract_pattern_inner_type(&mut self, scrutinee: &Ty, span: crate::source::Span) -> Ty {
        let inner = self.unwrap_inner_type(scrutinee);
        if inner == Ty::Unknown {
            let message = if matches!(scrutinee, Ty::Unknown) {
                "'<-' pattern requires a known source type; add a type annotation or use an explicit pattern like 'Some(x)', 'Ok(x)', or 'Right(x)'".to_string()
            } else {
                format!(
                    "'<-' pattern requires Option[T], Result[T, E], or Either[L, R], got '{}'",
                    scrutinee.describe()
                )
            };
            self.add_error("invalid_extract_pattern", message, span);
        }
        inner
    }

    fn try_propagates_from(&self, source: &Ty, target: &Ty) -> bool {
        match (source, target) {
            (Ty::Unknown, _) | (_, Ty::Unknown) => true,
            (Ty::Named(source_name, source_args), Ty::Named(target_name, target_args))
                if source_name == "Option"
                    && target_name == "Option"
                    && source_args.len() == 1
                    && target_args.len() == 1 =>
            {
                true
            }
            (Ty::Named(source_name, source_args), Ty::Named(target_name, target_args))
                if source_name == "Result"
                    && target_name == "Result"
                    && !source_args.is_empty()
                    && !target_args.is_empty() =>
            {
                match (source_args.get(1), target_args.get(1)) {
                    (Some(source_error), Some(target_error)) => {
                        self.is_assignable(source_error, target_error)
                    }
                    _ => true,
                }
            }
            (Ty::Named(source_name, source_args), Ty::Named(target_name, target_args))
                if source_name == "Either"
                    && target_name == "Either"
                    && source_args.len() == 2
                    && target_args.len() == 2 =>
            {
                self.is_assignable(&source_args[0], &target_args[0])
            }
            _ => false,
        }
    }

    fn known_iterable_item_type(&self, ty: &Ty) -> Option<Ty> {
        match ty {
            Ty::Named(name, args)
                if (name == "Vector"
                    || name == "LinkedList"
                    || name == "Set"
                    || name == "Array"
                    || name == "Iterable"
                    || name == "Iterator")
                    && args.len() == 1 =>
            {
                Some(args[0].clone())
            }
            Ty::Named(name, args) if name == "Map" && args.len() == 2 => {
                Some(Ty::Tuple(vec![args[0].clone(), args[1].clone()]))
            }
            Ty::Named(name, args) if name == "IntRange" && args.is_empty() => Some(Ty::int()),
            _ => None,
        }
    }

    fn check_spread_only_collection_literal(
        &mut self,
        items: &[Expr],
        expected: &Ty,
    ) -> Option<Ty> {
        if items.is_empty() || !items.iter().all(|item| matches!(item, Expr::Spread { .. })) {
            return None;
        }

        let spread_types = items
            .iter()
            .filter_map(|item| match item {
                Expr::Spread { value, span, .. } => Some((self.check_expr(value), *span)),
                _ => None,
            })
            .collect::<Vec<_>>();
        let expected_is_map = matches!(
            expected,
            Ty::Named(name, args) if name == "Map" && args.len() == 2
        );
        let expected_is_list = matches!(
            expected,
            Ty::Named(name, args) if name == "Vector" && args.len() == 1
        );
        let has_map = spread_types.iter().any(|(ty, _)| is_map_ty(ty));
        let has_known_non_map = spread_types
            .iter()
            .any(|(ty, _)| !matches!(ty, Ty::Unknown) && !is_map_ty(ty));

        if has_map && has_known_non_map {
            let span = spread_types
                .iter()
                .find(|(ty, _)| !matches!(ty, Ty::Unknown) && !is_map_ty(ty))
                .map(|(_, span)| *span)
                .unwrap_or_else(|| items[0].span());
            self.add_error(
                "mixed_collection_spreads",
                "cannot mix map spreads with vector or iterable spreads in the same bracket literal",
                span,
            );
            return Some(Ty::Unknown);
        }

        if has_map {
            if expected_is_list {
                self.add_error(
                    "invalid_vector_spread",
                    "cannot spread a map into a vector literal",
                    items[0].span(),
                );
                return Some(Ty::Unknown);
            }
            return Some(join_spread_map_types(
                expected,
                spread_types.into_iter().map(|part| part.0),
            ));
        }

        if expected_is_map && has_known_non_map {
            let (other, span) = spread_types
                .iter()
                .find(|(ty, _)| !matches!(ty, Ty::Unknown))
                .cloned()
                .unwrap_or((Ty::Unknown, items[0].span()));
            self.add_error(
                "invalid_map_spread",
                format!(
                    "cannot spread '{}' into a map literal; map spread requires a map value",
                    other.describe()
                ),
                span,
            );
            return Some(Ty::Unknown);
        }

        let mut item_ty = Ty::Unknown;
        for (item, (spread_ty, _)) in items.iter().zip(spread_types) {
            if let Some(current) = self.known_iterable_item_type(&spread_ty) {
                item_ty = join_types(&item_ty, &current);
            } else if !matches!(spread_ty, Ty::Unknown) {
                self.add_error(
                    "invalid_vector_spread",
                    format!(
                        "vector spread requires an iterable value, got '{}'",
                        spread_ty.describe()
                    ),
                    item.span(),
                );
            }
        }
        Some(Ty::list(item_ty))
    }

    fn iterable_item_type(&self, ty: &Ty) -> Ty {
        self.known_iterable_item_type(ty).unwrap_or(Ty::Unknown)
    }

    fn index_result_type(&self, ty: &Ty) -> Ty {
        match ty {
            Ty::Named(name, args) if (name == "Vector" || name == "Array") && args.len() == 1 => {
                args[0].clone()
            }
            Ty::Named(name, args) if name == "Map" && args.len() == 2 => {
                Ty::option(args[1].clone())
            }
            Ty::Tuple(items) => join_many_types(items),
            Ty::Unknown => Ty::Unknown,
            _ => Ty::Unknown,
        }
    }

    fn field_sig_for_member(&self, receiver: &Ty, name: &str) -> Option<FieldSig> {
        let Ty::Named(type_name, _) = receiver else {
            return None;
        };
        let sig = self.lookup_any_type(type_name)?;
        sig.fields.iter().find(|field| field.name == name).cloned()
    }

    fn extension_this_hidden_field(&self, receiver: &Expr, receiver_ty: &Ty, name: &str) -> bool {
        if !matches!(receiver, Expr::Identifier { name: receiver_name, .. } if receiver_name == "this")
        {
            return false;
        }
        let Some(target_name) = &self.current_extension_target else {
            return false;
        };
        let Ty::Named(receiver_name, _) = receiver_ty else {
            return false;
        };
        if receiver_name != target_name {
            return false;
        }
        self.field_sig_for_member(receiver_ty, name)
            .is_some_and(|field| field.hidden)
    }

    fn extension_this_hidden_method(&self, receiver: &Expr, receiver_ty: &Ty, name: &str) -> bool {
        if !matches!(receiver, Expr::Identifier { name: receiver_name, .. } if receiver_name == "this")
        {
            return false;
        }
        let Some(target_name) = &self.current_extension_target else {
            return false;
        };
        let Ty::Named(receiver_name, _) = receiver_ty else {
            return false;
        };
        if receiver_name != target_name {
            return false;
        }
        self.lookup_any_type(receiver_name).is_some_and(|sig| {
            sig.methods.get(name).is_some_and(|methods| {
                methods
                    .iter()
                    .any(|method| method.visibility == Visibility::Hidden)
            })
        })
    }

    fn check_extension_hidden_method_call(
        &mut self,
        callee: &Expr,
        args: &[crate::ast::CallArg],
    ) -> bool {
        let Expr::Member {
            receiver,
            name,
            span,
        } = callee
        else {
            return false;
        };
        if self.current_extension_target.is_none() {
            return false;
        }
        let receiver_ty = self.check_expr(receiver);
        if !self.extension_this_hidden_method(receiver, &receiver_ty, name) {
            return false;
        }
        self.add_extension_hidden_access_error("method", name, *span);
        for arg in args {
            self.check_expr(&arg.value);
        }
        true
    }

    fn add_extension_hidden_access_error(
        &mut self,
        member_kind: &str,
        member_name: &str,
        span: crate::source::Span,
    ) {
        let target_name = self
            .current_extension_target
            .as_deref()
            .unwrap_or("extended type");
        self.add_error(
            "invalid_extension_access",
            format!(
                "extension method cannot access hidden {member_kind} '{member_name}'; extension methods can access only visible members of '{target_name}'"
            ),
            span,
        );
    }

    fn can_initialize_field_in_constructor(
        &self,
        receiver: &Expr,
        receiver_ty: &Ty,
        field_name: &str,
    ) -> bool {
        if self.current_method.as_deref() != Some("new") {
            return false;
        }
        let Some(owner) = &self.current_owner else {
            return false;
        };
        let Expr::Identifier { name, .. } = receiver else {
            return false;
        };
        if name != "this" {
            return false;
        }
        let Ty::Named(receiver_name, _) = receiver_ty else {
            return false;
        };
        if receiver_name != &owner.name {
            return false;
        }
        owner.fields.iter().any(|field| field.name == field_name)
    }

    fn member_type(&self, receiver: &Ty, name: &str) -> Option<Ty> {
        if name == "hash" && self.is_hashable_type(receiver, &mut HashSet::new()) {
            return Some(Ty::Function(Vec::new(), Box::new(Ty::int())));
        }
        match receiver {
            Ty::Named(type_name, args) => {
                let Some(sig) = self.lookup_any_type(type_name) else {
                    let extension_methods =
                        self.extension_method_sigs_for_named_type(type_name, args, name);
                    if let Some(first) = extension_methods.first() {
                        return Some(Ty::Function(
                            first.params.iter().map(|param| param.ty.clone()).collect(),
                            Box::new(first.ret.clone()),
                        ));
                    }
                    return universal_member_type(name);
                };
                let subst = sig
                    .type_params
                    .iter()
                    .cloned()
                    .zip(args.iter().cloned())
                    .collect::<HashMap<_, _>>();
                if let Some(case) = sig.enum_cases.get(name) {
                    return Some(substitute_type(&case.result, &subst));
                }
                if let Some(field) = sig.fields.iter().find(|field| field.name == name) {
                    return Some(substitute_type(&field.ty, &subst));
                }
                if let Some(methods) = self.method_sigs_for_type(&sig, name) {
                    let first = methods.first()?;
                    return Some(Ty::Function(
                        first
                            .params
                            .iter()
                            .map(|param| substitute_type(&param.ty, &subst))
                            .collect(),
                        Box::new(substitute_type(&first.ret, &subst)),
                    ));
                }
                let extension_methods =
                    self.extension_method_sigs_for_named_type(type_name, args, name);
                if let Some(first) = extension_methods.first() {
                    return Some(Ty::Function(
                        first.params.iter().map(|param| param.ty.clone()).collect(),
                        Box::new(first.ret.clone()),
                    ));
                }
                universal_member_type(name)
            }
            Ty::Record(fields) => fields
                .iter()
                .find(|(field_name, _)| field_name == name)
                .map(|(_, ty)| ty.clone())
                .or_else(|| universal_member_type(name)),
            Ty::TypeParam(param) => self
                .type_param_method_sigs(param, name)
                .first()
                .map(|method| {
                    Ty::Function(
                        method.params.iter().map(|param| param.ty.clone()).collect(),
                        Box::new(method.ret.clone()),
                    )
                })
                .or_else(|| universal_member_type(name)),
            Ty::Unknown => Some(Ty::Unknown),
            _ => universal_member_type(name),
        }
    }

    fn can_access_hidden_constructor(&self, owner: &TypeSig) -> bool {
        self.current_owner
            .as_ref()
            .is_some_and(|current| current.name == owner.name)
    }

    fn member_method_sigs(&self, receiver: &Ty, name: &str) -> Option<Vec<FunctionSig>> {
        if name == "hash" && self.is_hashable_type(receiver, &mut HashSet::new()) {
            return Some(vec![hash_method_sig()]);
        }
        match receiver {
            Ty::Named(type_name, args) => {
                let Some(sig) = self.lookup_any_type(type_name) else {
                    let extension_methods =
                        self.extension_method_sigs_for_named_type(type_name, args, name);
                    if extension_methods.is_empty() {
                        return universal_method_sigs(name);
                    }
                    return Some(extension_methods);
                };
                let Some(methods) = self.method_sigs_for_type(&sig, name) else {
                    let extension_methods =
                        self.extension_method_sigs_for_named_type(type_name, args, name);
                    if extension_methods.is_empty() {
                        return universal_method_sigs(name);
                    }
                    return Some(extension_methods);
                };
                let subst = sig
                    .type_params
                    .iter()
                    .cloned()
                    .zip(args.iter().cloned())
                    .collect::<HashMap<_, _>>();
                Some(
                    methods
                        .into_iter()
                        .map(|method| FunctionSig {
                            type_params: method.type_params,
                            reified_type_params: method.reified_type_params,
                            generic_conditions: method
                                .generic_conditions
                                .into_iter()
                                .map(|condition| substitute_generic_condition(&condition, &subst))
                                .collect(),
                            params: method
                                .params
                                .into_iter()
                                .map(|param| ParamSig {
                                    name: param.name,
                                    ty: substitute_type(&param.ty, &subst),
                                    variadic: param.variadic,
                                    lazy: param.lazy,
                                    has_initializer: param.has_initializer,
                                })
                                .collect(),
                            ret: substitute_type(&method.ret, &subst),
                            visibility: method.visibility,
                            has_body: method.has_body,
                        })
                        .collect(),
                )
            }
            Ty::Unknown => universal_method_sigs(name),
            Ty::TypeParam(param) => {
                let methods = self.type_param_method_sigs(param, name);
                if methods.is_empty() {
                    universal_method_sigs(name)
                } else {
                    Some(methods)
                }
            }
            _ => universal_method_sigs(name),
        }
    }

    fn type_param_method_sigs(&self, param: &str, name: &str) -> Vec<FunctionSig> {
        self.type_param_bounds(param)
            .into_iter()
            .flat_map(|bound| self.method_sigs_for_generic_bound(&bound, name))
            .collect()
    }

    fn method_sigs_for_generic_bound(&self, bound: &Ty, name: &str) -> Vec<FunctionSig> {
        let Ty::Named(bound_name, args) = bound else {
            return Vec::new();
        };
        let Some(sig) = self.lookup_any_type(bound_name) else {
            return Vec::new();
        };
        let Some(methods) = self.method_sigs_for_type(&sig, name) else {
            return Vec::new();
        };
        let subst = sig
            .type_params
            .iter()
            .cloned()
            .zip(args.iter().cloned())
            .collect::<HashMap<_, _>>();
        methods
            .into_iter()
            .map(|method| instantiate_function_sig(method, &subst))
            .collect()
    }

    fn extension_method_sigs_for_named_type(
        &self,
        type_name: &str,
        args: &[Ty],
        name: &str,
    ) -> Vec<FunctionSig> {
        let subst = self
            .lookup_any_type(type_name)
            .map(|sig| {
                sig.type_params
                    .iter()
                    .cloned()
                    .zip(args.iter().cloned())
                    .collect::<HashMap<_, _>>()
            })
            .unwrap_or_default();
        self.world
            .extension_method_sigs(self.module, type_name, name)
            .into_iter()
            .map(|method| FunctionSig {
                type_params: method.type_params,
                reified_type_params: method.reified_type_params,
                generic_conditions: method
                    .generic_conditions
                    .into_iter()
                    .map(|condition| substitute_generic_condition(&condition, &subst))
                    .collect(),
                params: method
                    .params
                    .into_iter()
                    .map(|param| ParamSig {
                        name: param.name,
                        ty: substitute_type(&param.ty, &subst),
                        variadic: param.variadic,
                        lazy: param.lazy,
                        has_initializer: param.has_initializer,
                    })
                    .collect(),
                ret: substitute_type(&method.ret, &subst),
                visibility: method.visibility,
                has_body: method.has_body,
            })
            .collect()
    }

    fn method_sigs_for_type(&self, sig: &TypeSig, name: &str) -> Option<Vec<FunctionSig>> {
        let mut seen = HashSet::new();
        self.method_sigs_for_type_inner(sig, name, &mut seen)
    }

    fn method_sigs_for_type_inner(
        &self,
        sig: &TypeSig,
        name: &str,
        seen: &mut HashSet<String>,
    ) -> Option<Vec<FunctionSig>> {
        if !seen.insert(sig.name.clone()) {
            return None;
        }
        if let Some(methods) = sig.methods.get(name) {
            return Some(methods.clone());
        }
        for bound in &sig.with_bounds {
            let Ty::Named(bound_name, _) = bound else {
                continue;
            };
            let Some(bound_sig) = self.lookup_any_type(bound_name) else {
                continue;
            };
            if let Some(methods) = self.method_sigs_for_type_inner(&bound_sig, name, seen) {
                return Some(methods);
            }
        }
        None
    }

    fn module_member_value_type(&self, expr: &Expr) -> Option<Ty> {
        let (alias, member) = module_alias_and_member(expr)?;
        let module = self.world.lookup_module_alias(self.module, &alias)?;
        if let Some(functions) = module.functions.get(&member) {
            let sig = functions.first()?;
            return Some(Ty::Function(
                sig.params.iter().map(|param| param.ty.clone()).collect(),
                Box::new(sig.ret.clone()),
            ));
        }
        if let Some(globals) = self.world.checked_globals.get(&module.path) {
            if let Some(value) = globals.get(&member) {
                return Some(value.ty.clone());
            }
        }
        if let Some(sig) = module.types.get(&member) {
            return Some(Ty::Named(
                sig.name.clone(),
                sig.type_params
                    .iter()
                    .map(|param| Ty::TypeParam(param.clone()))
                    .collect(),
            ));
        }
        if let Some(sig) = module.objects.get(&member) {
            return Some(Ty::Named(
                sig.name.clone(),
                sig.type_params
                    .iter()
                    .map(|param| Ty::TypeParam(param.clone()))
                    .collect(),
            ));
        }
        None
    }

    fn lookup_value(&self, name: &str) -> Option<ValueInfo> {
        self.lookup_scoped_value(name)
            .or_else(|| self.lookup_global_value(name))
    }

    fn lookup_scoped_value(&self, name: &str) -> Option<ValueInfo> {
        for scope in self.scopes.iter().rev() {
            if let Some(value) = scope.get(name) {
                return Some(value.clone());
            }
        }
        None
    }

    fn lookup_global_value(&self, name: &str) -> Option<ValueInfo> {
        self.globals
            .get(name)
            .cloned()
            .or_else(|| self.world.lookup_imported_global(self.module, name))
    }

    fn lookup_implicit_field(&self, name: &str) -> Option<FieldSig> {
        self.current_owner
            .as_ref()?
            .fields
            .iter()
            .find(|field| field.name == name)
            .cloned()
    }

    fn undefined_value_message(&self, name: &str) -> String {
        if self
            .current_owner
            .as_ref()
            .is_some_and(|owner| owner.fields.iter().any(|field| field.name == name))
        {
            format!(
                "undefined name '{}'; if you meant the field, write 'this.{}'",
                name, name
            )
        } else {
            format!("undefined name '{}'", name)
        }
    }

    fn unknown_member_message(&self, receiver: &Expr, receiver_ty: &Ty, name: &str) -> String {
        if name == "orPanic"
            && !matches!(receiver_ty, Ty::Unknown)
            && self.unwrap_known_lifted_type(receiver_ty).is_some()
        {
            return format!(
                "method 'orPanic' was removed from '{}'; use postfix '!!' for unsafe extraction",
                receiver_ty.describe(),
            );
        }

        if let Ty::Function(_, _) = receiver_ty {
            if let Expr::Member {
                receiver: method_receiver,
                name: method_name,
                ..
            } = receiver
            {
                let access = self
                    .describe_member_path(receiver)
                    .unwrap_or_else(|| format!("<expr>.{method_name}"));
                let base_ty = self.probe_expr_type(method_receiver);
                if !matches!(base_ty, Ty::Unknown) {
                    return format!(
                        "cannot access member '{}' on method '{}'; '{}' on type '{}' is a method value of type '{}', not an instance",
                        name,
                        method_name,
                        access,
                        base_ty.describe(),
                        receiver_ty.describe(),
                    );
                }
                return format!(
                    "cannot access member '{}' on method '{}'; '{}' is a method value of type '{}', not an instance",
                    name,
                    method_name,
                    access,
                    receiver_ty.describe(),
                );
            }
            return format!(
                "cannot access member '{}' on function value of type '{}'",
                name,
                receiver_ty.describe(),
            );
        }

        if !matches!(receiver_ty, Ty::Unknown)
            && self.unwrap_known_lifted_type(receiver_ty).is_some()
        {
            return format!(
                "cannot access member '{}' directly on lifted value '{}'; use map or flatMap explicitly",
                name,
                receiver_ty.describe()
            );
        }

        format!(
            "type '{}' has no field or method '{}'",
            receiver_ty.describe(),
            name
        )
    }

    fn describe_member_path(&self, expr: &Expr) -> Option<String> {
        match expr {
            Expr::Identifier { name, .. } => Some(name.clone()),
            Expr::Member { receiver, name, .. } => {
                Some(format!("{}.{}", self.describe_member_path(receiver)?, name))
            }
            _ => None,
        }
    }

    fn is_builtin_print_call(&self, callee: &Expr) -> bool {
        match callee {
            Expr::Identifier { name, .. } => {
                matches!(name.as_str(), "print" | "println" | "printf")
            }
            Expr::Member { receiver, name, .. } => {
                matches!(name.as_str(), "print" | "println" | "printf")
                    && path_starts_with_os(receiver)
            }
            _ => false,
        }
    }

    fn is_builtin_panic_call(&self, callee: &Expr) -> bool {
        match callee {
            Expr::Identifier { name, .. } => name == "panic",
            _ => false,
        }
    }

    fn is_builtin_assert_call(&self, callee: &Expr) -> bool {
        match callee {
            Expr::Identifier { name, .. } => name == "assert",
            _ => false,
        }
    }

    fn check_builtin_assert_call(
        &mut self,
        args: &[crate::ast::CallArg],
        span: crate::source::Span,
    ) -> Ty {
        if !matches!(args.len(), 1 | 2) {
            self.add_error(
                "invalid_argument_count",
                format!("assert expects 1 or 2 arguments, got {}", args.len()),
                span,
            );
        }

        if let Some(condition) = args.first() {
            let actual = self.check_expr(&condition.value);
            self.require_assignable(
                &actual,
                &Ty::bool(),
                condition.span,
                "invalid_argument_type",
                format!(
                    "assert condition has type '{}' but expects 'Bool'",
                    actual.describe()
                ),
            );
        }
        if let Some(message) = args.get(1) {
            let actual = self.check_expr(&message.value);
            self.require_assignable(
                &actual,
                &Ty::str(),
                message.span,
                "invalid_argument_type",
                format!(
                    "assert message has type '{}' but expects 'Str'",
                    actual.describe()
                ),
            );
        }
        for extra in args.iter().skip(2) {
            self.check_expr(&extra.value);
        }

        Ty::unit()
    }

    fn check_typeof_type_ref(&mut self, ty_ref: &TypeRef, represented: &Ty) {
        match represented {
            Ty::TypeParam(name) if self.is_reified_type_param(name) => {}
            Ty::TypeParam(name) => {
                self.add_error(
                    "generic_type_not_reified",
                    format!(
                        "generic type '{}' is not available at runtime; pass a Type[{}] parameter or mark '{}' as reified",
                        name, name, name
                    ),
                    ty_ref.span(),
                );
            }
            other if type_contains_type_param(other) => {
                self.add_error(
                    "generic_type_not_reified",
                    "typeOf currently supports reified type parameters only as direct type arguments, for example typeOf[A]",
                    ty_ref.span(),
                );
            }
            _ => {}
        }
    }

    fn choose_overload<'b>(
        &self,
        overloads: &'b [FunctionSig],
        args: &[crate::ast::CallArg],
    ) -> Option<&'b FunctionSig> {
        let arg_types = args
            .iter()
            .map(|arg| self.probe_expr_type(call_arg_value_expr(arg)))
            .collect::<Vec<_>>();
        overloads
            .iter()
            .filter_map(|sig| {
                let arrangement = arrange_param_args(&sig.params, args);
                if arrangement.overflow > 0 || arrangement.missing_required > 0 {
                    return None;
                }
                let mut score = 0usize;
                for (index, param) in sig.params.iter().enumerate() {
                    for arg in arrangement
                        .slots
                        .get(index)
                        .map(Vec::as_slice)
                        .unwrap_or(&[])
                    {
                        let arg_index = args
                            .iter()
                            .position(|candidate| std::ptr::eq(candidate, *arg))
                            .unwrap_or(0);
                        let actual = &arg_types[arg_index];
                        let expected = call_arg_expected_ty_for_arg(param.variadic, &param.ty, arg);
                        if empty_collection_literal_span(&arg.value).is_some() {
                            if is_list_or_map_ty(&expected) {
                                score += 2;
                                continue;
                            }
                            return None;
                        }
                        if !matches!(actual, Ty::Unknown) {
                            if self.arg_matches_expected(arg, actual, &expected) {
                                score += 2;
                            } else if type_contains_type_param(&expected) {
                                score += 1;
                            } else {
                                return None;
                            }
                        }
                        if arg.name.as_deref() == Some(param.name.as_str()) {
                            score += 1;
                        }
                    }
                }
                if !sig.params.iter().any(|param| param.variadic) {
                    score += 1;
                }
                Some((score, sig))
            })
            .max_by_key(|(score, _)| *score)
            .map(|(_, sig)| sig)
    }

    fn arg_matches_expected(&self, arg: &crate::ast::CallArg, actual: &Ty, expected: &Ty) -> bool {
        let _ = arg;
        self.is_assignable(actual, expected)
    }

    fn probe_expr_type(&self, expr: &Expr) -> Ty {
        match expr {
            Expr::Identifier { name, .. } => self
                .lookup_scoped_value(name)
                .map(|value| value.ty)
                .or_else(|| self.lookup_implicit_field(name).map(|field| field.ty))
                .or_else(|| self.lookup_global_value(name).map(|value| value.ty))
                .or_else(|| self.lookup_function_type(name))
                .or_else(|| self.lookup_named_constructor_type(name))
                .unwrap_or(Ty::Unknown),
            Expr::Integer { .. } => Ty::int(),
            Expr::Float { .. } => Ty::float(),
            Expr::String { .. } => Ty::str(),
            Expr::Bool { .. } => Ty::bool(),
            Expr::Unit { .. } => Ty::unit(),
            Expr::RecordLiteral { fields, .. } => {
                let mut out = Vec::new();
                for field in fields {
                    if let Some(name) = &field.name {
                        let ty = field
                            .ty
                            .as_ref()
                            .map(|ty| self.ty_from_type_ref(ty))
                            .unwrap_or_else(|| self.probe_expr_type(&field.value));
                        upsert_shape_field(&mut out, name.clone(), ty);
                    } else if let Ty::Record(spread_fields) = self.probe_expr_type(&field.value) {
                        for (name, ty) in spread_fields {
                            upsert_shape_field(&mut out, name, ty);
                        }
                    }
                }
                Ty::Record(out)
            }
            Expr::Member { receiver, name, .. } => {
                if let Some(ty) = self.module_member_value_type(expr) {
                    return ty;
                }
                if let Some(ty) = self.static_member_value_type(receiver, name, &Ty::Unknown) {
                    return ty;
                }
                let receiver_ty = self.probe_expr_type(receiver);
                if name == "runtimeType" {
                    return Ty::value_runtime_type(receiver_ty);
                }
                self.member_type(&receiver_ty, name).unwrap_or(Ty::Unknown)
            }
            Expr::Call {
                callee,
                args,
                uses_brace_syntax,
                ..
            } => {
                let normalized_args =
                    self.normalize_trailing_brace_call_args(callee, args, *uses_brace_syntax);
                self.callable_signature_for_args_probe(callee, &normalized_args, *uses_brace_syntax)
                    .map(|(_, ret)| ret)
                    .unwrap_or(Ty::Unknown)
            }
            _ => Ty::Unknown,
        }
    }

    fn check_record_constructor_conversion(
        &mut self,
        sig: &TypeSig,
        ret: &Ty,
        expr: &Expr,
        span: crate::source::Span,
    ) -> Ty {
        let Expr::RecordLiteral { fields, values, .. } = expr else {
            return materialize_type(ret);
        };
        if !values.is_empty() {
            self.add_error(
                "positional_brace_construction",
                format!(
                    "{} '{}' uses brace field construction; write '{}(...)' for positional construction",
                    type_kind_label(sig.kind),
                    sig.name,
                    sig.name
                ),
                span,
            );
            return materialize_type(ret);
        }
        if let Some(field) = sig
            .fields
            .iter()
            .find(|field| field.hidden && !field.has_initializer)
        {
            self.add_error(
                "no_matching_overload",
                format!(
                    "{} '{}' has no implicit field constructor because hidden field '{}' has no initializer; define 'new' to initialize it",
                    type_kind_label(sig.kind),
                    sig.name,
                    field.name
                ),
                span,
            );
            return materialize_type(ret);
        }

        let visible_fields = sig
            .fields
            .iter()
            .filter(|field| !field.hidden)
            .collect::<Vec<_>>();

        let required_visible = visible_fields
            .iter()
            .filter(|field| !field.has_initializer)
            .count();

        if fields.len() < required_visible || fields.len() > visible_fields.len() {
            self.add_error(
                "no_matching_overload",
                format!(
                    "{} '{}' brace field construction expects {}..{} visible fields, got {}",
                    type_kind_label(sig.kind),
                    sig.name,
                    required_visible,
                    visible_fields.len(),
                    fields.len()
                ),
                span,
            );
            return materialize_type(ret);
        }

        for arg in fields {
            let Some(name) = arg.name.as_deref() else {
                self.add_error(
                    "no_matching_overload",
                    format!(
                        "{} '{}' requires construction fields that match the visible shape",
                        type_kind_label(sig.kind),
                        sig.name
                    ),
                    span,
                );
                return materialize_type(ret);
            };
            let Some(field) = visible_fields.iter().find(|field| field.name == name) else {
                self.add_error(
                    "no_matching_overload",
                    format!(
                        "{} '{}' has no visible field '{}' for brace field construction",
                        type_kind_label(sig.kind),
                        sig.name,
                        name
                    ),
                    arg.span,
                );
                return materialize_type(ret);
            };
            let actual = self.check_expr_against(&arg.value, &field.ty);
            if !self.is_assignable(&actual, &field.ty) {
                self.add_error(
                    "invalid_argument_type",
                    format!(
                        "field '{}' in {} '{}' expects '{}' but got '{}'",
                        field.name,
                        type_kind_label(sig.kind),
                        sig.name,
                        field.ty.describe(),
                        actual.describe()
                    ),
                    arg.span,
                );
                return materialize_type(ret);
            }
        }

        for field in &visible_fields {
            if field.has_initializer {
                continue;
            }
            if !fields
                .iter()
                .any(|arg| arg.name.as_deref() == Some(field.name.as_str()))
            {
                self.add_error(
                    "no_matching_overload",
                    format!(
                        "{} '{}' brace field construction is missing required field '{}'",
                        type_kind_label(sig.kind),
                        sig.name,
                        field.name
                    ),
                    span,
                );
                return materialize_type(ret);
            }
        }

        materialize_type(ret)
    }

    fn check_positional_record_constructor_conversion(
        &mut self,
        sig: &TypeSig,
        ret: &Ty,
        args: &[crate::ast::CallArg],
        span: crate::source::Span,
    ) -> Ty {
        if let Some(field) = sig
            .fields
            .iter()
            .find(|field| field.hidden && !field.has_initializer)
        {
            self.add_error(
                "no_matching_overload",
                format!(
                    "{} '{}' has no implicit positional constructor because hidden field '{}' has no initializer; define 'new' to initialize it",
                    type_kind_label(sig.kind),
                    sig.name,
                    field.name
                ),
                span,
            );
            return materialize_type(ret);
        }

        if sig.fields.iter().enumerate().any(|(index, field)| {
            field.hidden
                && field.has_initializer
                && sig.fields[index + 1..].iter().any(|later| !later.hidden)
        }) {
            self.add_error(
                "no_matching_overload",
                format!(
                    "{} '{}' cannot use positional construction because hidden defaulted fields must come after all visible fields",
                    type_kind_label(sig.kind),
                    sig.name
                ),
                span,
            );
            return materialize_type(ret);
        }

        let params = sig
            .fields
            .iter()
            .filter(|field| !field.hidden)
            .cloned()
            .collect::<Vec<_>>();
        self.check_constructor_signature(&params, ret, args, span)
    }

    fn lookup_functions(&self, name: &str) -> Option<Vec<FunctionSig>> {
        self.module
            .functions
            .get(name)
            .cloned()
            .or_else(|| self.world.lookup_imported_function(self.module, name))
            .or_else(|| self.world.ambient.functions.get(name).cloned())
    }

    fn lookup_implicit_method_functions(&self, name: &str) -> Option<Vec<FunctionSig>> {
        if self.lookup_value(name).is_some()
            || self.lookup_functions(name).is_some()
            || self
                .lookup_bare_enum_case_value_type(name, &Ty::Unknown)
                .is_some()
            || self.lookup_named_constructor_type(name).is_some()
        {
            return None;
        }
        let owner = self.current_owner.as_ref()?;
        self.method_sigs_for_type(owner, name)
    }

    fn lookup_function_type(&self, name: &str) -> Option<Ty> {
        let functions = self.lookup_functions(name)?;
        let sig = functions.first()?;
        Some(Ty::Function(
            sig.params.iter().map(|param| param.ty.clone()).collect(),
            Box::new(sig.ret.clone()),
        ))
    }

    fn lookup_named_constructor_type(&self, name: &str) -> Option<Ty> {
        if let Some(sig) = self.lookup_type_local(name) {
            return Some(Ty::Named(
                sig.name.clone(),
                sig.type_params
                    .iter()
                    .map(|param| Ty::TypeParam(param.clone()))
                    .collect(),
            ));
        }
        if let Some(sig) = self.world.lookup_imported_type(self.module, name) {
            return Some(Ty::Named(
                sig.name.clone(),
                sig.type_params
                    .iter()
                    .map(|param| Ty::TypeParam(param.clone()))
                    .collect(),
            ));
        }
        if let Some(sig) = self.world.ambient.types.get(name) {
            return Some(Ty::Named(
                sig.name.clone(),
                sig.type_params
                    .iter()
                    .map(|param| Ty::TypeParam(param.clone()))
                    .collect(),
            ));
        }
        if let Some(sig) = self.world.ambient.objects.get(name) {
            return Some(Ty::Named(
                sig.name.clone(),
                sig.type_params
                    .iter()
                    .map(|param| Ty::TypeParam(param.clone()))
                    .collect(),
            ));
        }
        None
    }

    fn lookup_bare_enum_case_value_type(&self, name: &str, expected: &Ty) -> Option<Ty> {
        let case = self.world.lookup_enum_case(self.module, name)?;
        case.params
            .iter()
            .all(|param| param.has_initializer)
            .then(|| self.materialize_enum_case_result_against(&case.result, expected))
    }

    fn materialize_enum_case_result_against(&self, result: &Ty, expected: &Ty) -> Ty {
        match (result, expected) {
            (Ty::Named(result_name, result_args), Ty::Named(expected_name, expected_args))
                if result_name == expected_name && result_args.len() == expected_args.len() =>
            {
                expected.clone()
            }
            _ => result.clone(),
        }
    }

    fn lookup_type_local(&self, name: &str) -> Option<TypeSig> {
        self.module
            .types
            .get(name)
            .cloned()
            .or_else(|| self.module.objects.get(name).cloned())
    }

    fn lookup_object_local(&self, name: &str) -> Option<TypeSig> {
        self.module.objects.get(name).cloned()
    }

    fn lookup_unique_module_type(&self, name: &str) -> Option<TypeSig> {
        let mut matches = self
            .world
            .modules
            .values()
            .filter_map(|module| {
                module
                    .types
                    .get(name)
                    .cloned()
                    .or_else(|| module.objects.get(name).cloned())
            })
            .collect::<Vec<_>>();
        if matches.len() == 1 {
            matches.pop()
        } else {
            None
        }
    }

    fn lookup_unique_module_object(&self, name: &str) -> Option<TypeSig> {
        let mut matches = self
            .world
            .modules
            .values()
            .filter_map(|module| module.objects.get(name).cloned())
            .collect::<Vec<_>>();
        if matches.len() == 1 {
            matches.pop()
        } else {
            None
        }
    }

    fn lookup_any_object(&self, name: &str) -> Option<TypeSig> {
        self.lookup_object_local(name)
            .or_else(|| {
                self.module.symbol_imports.get(name).and_then(|imported| {
                    (imported.kind == ImportedKind::Object)
                        .then(|| self.world.modules.get(&imported.module_path))
                        .flatten()
                        .and_then(|module| module.objects.get(&imported.original_name).cloned())
                })
            })
            .or_else(|| self.lookup_unique_module_object(name))
            .or_else(|| self.world.ambient.objects.get(name).cloned())
    }

    fn lookup_any_non_object_type(&self, name: &str) -> Option<TypeSig> {
        self.module
            .types
            .get(name)
            .cloned()
            .or_else(|| {
                self.module.symbol_imports.get(name).and_then(|imported| {
                    matches!(imported.kind, ImportedKind::Type | ImportedKind::Interface)
                        .then(|| self.world.modules.get(&imported.module_path))
                        .flatten()
                        .and_then(|module| module.types.get(&imported.original_name).cloned())
                })
            })
            .or_else(|| {
                let mut matches = self
                    .world
                    .modules
                    .values()
                    .filter_map(|module| module.types.get(name).cloned())
                    .collect::<Vec<_>>();
                if matches.len() == 1 {
                    matches.pop()
                } else {
                    None
                }
            })
            .or_else(|| self.world.ambient.types.get(name).cloned())
    }

    fn lookup_any_type(&self, name: &str) -> Option<TypeSig> {
        self.anonymous_types
            .get(name)
            .cloned()
            .or_else(|| self.lookup_type_local(name))
            .or_else(|| self.world.lookup_imported_type(self.module, name))
            .or_else(|| self.lookup_unique_module_type(name))
            .or_else(|| self.world.ambient.types.get(name).cloned())
            .or_else(|| self.world.ambient.objects.get(name).cloned())
    }

    fn resolve_named_type(&self, name: &str, args: Vec<Ty>) -> Ty {
        if self.is_type_param(name) {
            return Ty::TypeParam(name.to_string());
        }
        if name == "Never" && args.is_empty() {
            return Ty::Never;
        }
        if let Some(sig) = self.lookup_any_type(name) {
            return Ty::Named(sig.name.clone(), args);
        }
        Ty::Named(name.to_string(), args)
    }

    fn ty_from_type_ref(&self, reference: &TypeRef) -> Ty {
        match reference {
            TypeRef::Wildcard { .. } => Ty::Wildcard,
            TypeRef::Named { name, args, .. } => self.resolve_named_type(
                name,
                args.iter().map(|arg| self.ty_from_type_ref(arg)).collect(),
            ),
            TypeRef::Tuple { fields, .. } => Ty::Tuple(
                fields
                    .iter()
                    .map(|field| self.ty_from_type_ref(&field.ty))
                    .collect(),
            ),
            TypeRef::Record { fields, .. } => Ty::Record(
                fields
                    .iter()
                    .map(|field| (field.name.clone(), self.ty_from_type_ref(&field.ty)))
                    .collect(),
            ),
            TypeRef::Function { params, ret, .. } => Ty::Function(
                params
                    .iter()
                    .map(|param| self.ty_from_type_ref(param))
                    .collect(),
                Box::new(self.ty_from_type_ref(ret)),
            ),
        }
    }

    fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    fn define_local(&mut self, name: &str, ty: Ty, mutable: bool) {
        if name == "_" {
            return;
        }
        let ty = self.capture_wildcards(ty);
        if self.scopes.is_empty() {
            self.push_scope();
        }
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(
                name.to_string(),
                ValueInfo {
                    ty,
                    mutable,
                    stable: !mutable,
                },
            );
        }
    }

    fn mark_current_local_unstable(&mut self, name: &str) {
        if let Some(value) = self.scopes.last_mut().and_then(|scope| scope.get_mut(name)) {
            value.stable = false;
        }
    }

    fn push_type_params_with_conditions<'b>(
        &mut self,
        params: impl Iterator<Item = &'b str>,
        conditions: Vec<GenericConditionSig>,
    ) {
        self.type_params.push(TypeParamScope {
            names: params.map(|name| name.to_string()).collect::<HashSet<_>>(),
            reified: HashSet::new(),
            conditions,
        });
    }

    fn push_ast_type_params(&mut self, params: &[TypeParam], conditions: &[GenericCondition]) {
        let mut known = self
            .type_params
            .iter()
            .flat_map(|scope| scope.names.iter().cloned())
            .collect::<HashSet<_>>();
        known.extend(params.iter().map(|param| param.name.clone()));
        self.type_params.push(TypeParamScope {
            names: params
                .iter()
                .map(|param| param.name.clone())
                .collect::<HashSet<_>>(),
            reified: params
                .iter()
                .filter(|param| param.reified)
                .map(|param| param.name.clone())
                .collect::<HashSet<_>>(),
            conditions: generic_condition_sigs(params, conditions, &known),
        });
    }

    fn validate_generic_clause(&mut self, params: &[TypeParam], conditions: &[GenericCondition]) {
        for param in params {
            for bound in &param.bounds {
                self.validate_interface_generic_bound(bound, bound.span());
            }
        }
        for condition in conditions {
            if let GenericCondition::Bound {
                subject,
                bound,
                span,
            } = condition
            {
                if !matches!(self.ty_from_type_ref(subject), Ty::TypeParam(_)) {
                    self.add_error(
                        "invalid_generic_condition_subject",
                        "the left side of a generic bound condition must be a local or enclosing type parameter",
                        subject.span(),
                    );
                }
                self.validate_interface_generic_bound(bound, *span);
            }
        }
    }

    fn validate_interface_generic_bound(&mut self, bound: &TypeRef, span: crate::source::Span) {
        let Ty::Named(name, _) = self.ty_from_type_ref(bound) else {
            self.add_error(
                "invalid_generic_bound",
                "generic bounds must name an interface",
                span,
            );
            return;
        };
        let Some(sig) = self.lookup_any_type(&name) else {
            return;
        };
        if sig.kind != TypeKind::Interface {
            self.add_error(
                "invalid_generic_bound",
                format!(
                    "generic bound '{}' is a {}; bounds must name interfaces",
                    name,
                    type_kind_label(sig.kind)
                ),
                span,
            );
        }
    }

    fn validate_type_ref_generic_applications(&mut self, reference: &TypeRef) {
        match reference {
            TypeRef::Wildcard { .. } => {}
            TypeRef::Named { name, args, span } => {
                for arg in args {
                    self.validate_type_ref_generic_applications(arg);
                }
                let Some(sig) = self.lookup_any_type(name) else {
                    return;
                };
                if sig.generic_conditions.is_empty() || sig.type_params.len() != args.len() {
                    return;
                }
                let subst = sig
                    .type_params
                    .iter()
                    .cloned()
                    .zip(args.iter().map(|arg| self.ty_from_type_ref(arg)))
                    .collect::<HashMap<_, _>>();
                let concrete_conditions = sig
                    .generic_conditions
                    .iter()
                    .filter(|condition| {
                        !generic_condition_contains_wildcard(&substitute_generic_condition(
                            condition, &subst,
                        ))
                    })
                    .cloned()
                    .collect::<Vec<_>>();
                self.check_call_generic_conditions(&concrete_conditions, &subst, *span);
            }
            TypeRef::Tuple { fields, .. } => {
                for field in fields {
                    self.validate_type_ref_generic_applications(&field.ty);
                }
            }
            TypeRef::Record { fields, .. } => {
                for field in fields {
                    self.validate_type_ref_generic_applications(&field.ty);
                }
            }
            TypeRef::Function { params, ret, .. } => {
                for param in params {
                    self.validate_type_ref_generic_applications(param);
                }
                self.validate_type_ref_generic_applications(ret);
            }
        }
    }

    fn pop_type_params(&mut self) {
        self.type_params.pop();
    }

    fn is_type_param(&self, name: &str) -> bool {
        self.type_params
            .iter()
            .rev()
            .any(|scope| scope.names.contains(name))
    }

    fn is_reified_type_param(&self, name: &str) -> bool {
        self.type_params
            .iter()
            .rev()
            .any(|scope| scope.reified.contains(name))
    }

    fn type_param_bounds(&self, name: &str) -> Vec<Ty> {
        let equivalent = self.equivalent_type_params(name);
        self.type_params
            .iter()
            .flat_map(|scope| scope.conditions.iter())
            .filter_map(|condition| match condition {
                GenericConditionSig::Bound {
                    subject: Ty::TypeParam(subject),
                    bound,
                } if equivalent.contains(subject) => Some(bound.clone()),
                _ => None,
            })
            .collect()
    }

    fn equivalent_type_params(&self, name: &str) -> HashSet<String> {
        let mut equivalent = HashSet::from([name.to_string()]);
        loop {
            let mut changed = false;
            for condition in self
                .type_params
                .iter()
                .flat_map(|scope| scope.conditions.iter())
            {
                let GenericConditionSig::Equal {
                    left: Ty::TypeParam(left),
                    right: Ty::TypeParam(right),
                } = condition
                else {
                    continue;
                };
                if equivalent.contains(left) {
                    changed |= equivalent.insert(right.clone());
                }
                if equivalent.contains(right) {
                    changed |= equivalent.insert(left.clone());
                }
            }
            if !changed {
                return equivalent;
            }
        }
    }

    fn type_params_are_equal(&self, left: &str, right: &str) -> bool {
        self.equivalent_type_params(left).contains(right)
    }

    fn require_assignable(
        &mut self,
        actual: &Ty,
        expected: &Ty,
        span: crate::source::Span,
        code: &'static str,
        message: impl Into<String>,
    ) {
        if !self.is_assignable(actual, expected) {
            self.add_error(code, message, span);
        }
    }

    fn diagnostic_type_phrase(&self, ty: &Ty) -> String {
        match ty {
            Ty::Capture(id) => self
                .capture_labels
                .get(id)
                .cloned()
                .unwrap_or_else(|| "captured unknown type".to_string()),
            _ => format!("'{}'", ty.describe()),
        }
    }

    fn diagnostic_type_mismatch_message(
        &self,
        subject: &str,
        actual: &Ty,
        expected_subject: &str,
        expected: &Ty,
    ) -> String {
        if let (Some(actual_id), Some(expected_id)) = (capture_id(actual), capture_id(expected)) {
            if actual_id != expected_id {
                return format!(
                    "{subject} has {}, but {expected_subject} expects a different {}",
                    self.diagnostic_type_phrase(actual),
                    self.diagnostic_type_phrase(expected)
                );
            }
        }
        format!(
            "{subject} has type {} but {expected_subject} expects {}",
            self.diagnostic_type_phrase(actual),
            self.diagnostic_type_phrase(expected)
        )
    }

    fn require_bool(&mut self, ty: &Ty, span: crate::source::Span, message: &str) {
        if !ty.is_bool() && !matches!(ty, Ty::Unknown) {
            self.add_error("invalid_condition_type", message, span);
        }
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

    fn is_assignable(&self, actual: &Ty, expected: &Ty) -> bool {
        let mut seen = HashSet::new();
        self.is_assignable_inner(actual, expected, &mut seen)
    }

    fn structural_fields_for_type(&self, ty: &Ty) -> Option<Vec<(String, Ty)>> {
        match ty {
            Ty::Record(fields) => Some(fields.clone()),
            Ty::Named(name, args) => {
                let sig = self.lookup_any_type(name)?;
                if !matches!(sig.kind, TypeKind::Class | TypeKind::Record) {
                    return None;
                }
                let subst = sig
                    .type_params
                    .iter()
                    .cloned()
                    .zip(args.iter().cloned())
                    .collect::<HashMap<_, _>>();
                Some(
                    sig.fields
                        .iter()
                        .filter(|field| !field.hidden)
                        .map(|field| (field.name.clone(), substitute_type(&field.ty, &subst)))
                        .collect(),
                )
            }
            _ => None,
        }
    }

    fn shape_target_fields(&self, expected: &Ty) -> Option<Vec<FieldSig>> {
        match expected {
            Ty::Record(fields) => Some(
                fields
                    .iter()
                    .map(|(name, ty)| FieldSig {
                        name: name.clone(),
                        ty: ty.clone(),
                        mutable: false,
                        hidden: false,
                        has_initializer: false,
                        variadic: false,
                    })
                    .collect(),
            ),
            Ty::Named(name, args) => {
                let sig = self.lookup_any_type(name)?;
                if sig.kind != TypeKind::Record {
                    return None;
                }
                let subst = sig
                    .type_params
                    .iter()
                    .cloned()
                    .zip(args.iter().cloned())
                    .collect::<HashMap<_, _>>();
                Some(
                    sig.fields
                        .iter()
                        .filter(|field| !field.hidden)
                        .map(|field| FieldSig {
                            name: field.name.clone(),
                            ty: substitute_type(&field.ty, &subst),
                            mutable: field.mutable,
                            hidden: field.hidden,
                            has_initializer: field.has_initializer,
                            variadic: field.variadic,
                        })
                        .collect(),
                )
            }
            _ => None,
        }
    }

    fn structurally_assignable_to_shape(&self, actual: &Ty, expected: &Ty) -> bool {
        let Some(expected_fields) = self.shape_target_fields(expected) else {
            return false;
        };

        let Some(actual_fields) = self.structural_fields_for_type(actual) else {
            return false;
        };

        expected_fields.iter().all(|expected| {
            actual_fields
                .iter()
                .find(|(actual_name, _)| actual_name == &expected.name)
                .map(|(_, actual_ty)| self.is_assignable(actual_ty, &expected.ty))
                .unwrap_or(expected.has_initializer)
        })
    }

    fn shape_fields_match_exactly(&self, left: &Ty, right: &Ty) -> bool {
        let Some(left_fields) = self.shape_target_fields(left) else {
            return false;
        };
        let Some(right_fields) = self.shape_target_fields(right) else {
            return false;
        };
        left_fields.len() == right_fields.len()
            && left_fields.iter().all(|left_field| {
                right_fields
                    .iter()
                    .find(|right_field| right_field.name == left_field.name)
                    .is_some_and(|right_field| {
                        self.is_assignable(&left_field.ty, &right_field.ty)
                            && self.is_assignable(&right_field.ty, &left_field.ty)
                    })
            })
    }

    fn implicitly_satisfies_value_bound(&self, actual: &Ty, expected: &Ty) -> bool {
        match expected {
            Ty::Named(name, args) if name == "Eq" && args.len() == 1 => {
                self.shape_fields_match_exactly(actual, &args[0])
            }
            Ty::Named(name, args) if name == "Hashed" && args.len() == 1 => {
                (self.generic_types_are_equal(actual, &args[0])
                    || self.shape_fields_match_exactly(actual, &args[0]))
                    && self.is_hashable_type(actual, &mut HashSet::new())
            }
            _ => false,
        }
    }

    fn is_hashable_type(&self, ty: &Ty, seen: &mut HashSet<String>) -> bool {
        match ty {
            Ty::Named(name, args)
                if args.is_empty()
                    && matches!(
                        name.as_str(),
                        "Bool" | "Float" | "Int" | "Rune" | "Str" | "Unit"
                    ) =>
            {
                true
            }
            Ty::Record(fields) => fields
                .iter()
                .all(|(_, field_ty)| self.is_hashable_type(field_ty, seen)),
            Ty::Named(name, args) => {
                let Some(sig) = self.lookup_any_type(name) else {
                    return false;
                };
                match sig.kind {
                    TypeKind::Enum | TypeKind::Object => true,
                    TypeKind::Record => {
                        let key = ty.describe();
                        if !seen.insert(key.clone()) {
                            return true;
                        }
                        let subst = sig
                            .type_params
                            .iter()
                            .cloned()
                            .zip(args.iter().cloned())
                            .collect::<HashMap<_, _>>();
                        let hashable = sig.fields.iter().all(|field| {
                            self.is_hashable_type(&substitute_type(&field.ty, &subst), seen)
                        });
                        seen.remove(&key);
                        hashable
                    }
                    TypeKind::Class => self.type_sig_has_hashed_bound(&sig, ty, seen),
                    TypeKind::Annotation | TypeKind::Interface => false,
                }
            }
            Ty::TypeParam(name) => self.type_param_bounds(name).iter().any(|bound| {
                matches!(
                    bound,
                    Ty::Named(bound_name, args)
                        if bound_name == "Hashed"
                            && args.len() == 1
                            && matches!(&args[0], Ty::TypeParam(bound_param) if bound_param == name)
                )
            }),
            Ty::Unknown
            | Ty::Wildcard
            | Ty::Capture(_)
            | Ty::Never
            | Ty::Tuple(_)
            | Ty::Function(_, _) => false,
        }
    }

    fn type_sig_has_hashed_bound(
        &self,
        sig: &TypeSig,
        actual: &Ty,
        seen: &mut HashSet<String>,
    ) -> bool {
        let key = format!("hashed:{}:{}", sig.name, actual.describe());
        if !seen.insert(key.clone()) {
            return false;
        }
        let subst = match actual {
            Ty::Named(_, args) => sig
                .type_params
                .iter()
                .cloned()
                .zip(args.iter().cloned())
                .collect::<HashMap<_, _>>(),
            _ => HashMap::new(),
        };
        let found = sig.with_bounds.iter().any(|bound| {
            let bound = substitute_type(bound, &subst);
            let Ty::Named(name, args) = &bound else {
                return false;
            };
            if name == "Hashed" {
                return args.len() == 1 && self.generic_types_are_equal(&args[0], actual);
            }
            self.lookup_any_type(name)
                .is_some_and(|parent| self.type_sig_has_hashed_bound(&parent, &bound, seen))
        });
        seen.remove(&key);
        found
    }

    fn is_assignable_inner(
        &self,
        actual: &Ty,
        expected: &Ty,
        seen: &mut HashSet<(String, String)>,
    ) -> bool {
        if is_assignable(actual, expected) {
            return true;
        }
        if let (Ty::TypeParam(left), Ty::TypeParam(right)) = (actual, expected) {
            if self.type_params_are_equal(left, right) {
                return true;
            }
        }
        if let Ty::TypeParam(name) = actual {
            if self
                .type_param_bounds(name)
                .into_iter()
                .any(|bound| self.is_assignable_inner(&bound, expected, seen))
            {
                return true;
            }
        }
        if self.implicitly_satisfies_value_bound(actual, expected) {
            return true;
        }
        if self.structurally_assignable_to_shape(actual, expected) {
            return true;
        }

        let (Ty::Named(actual_name, actual_args), Ty::Named(expected_name, _)) = (actual, expected)
        else {
            return false;
        };

        let key = (actual_name.clone(), expected_name.clone());
        if !seen.insert(key) {
            return false;
        }

        let Some(sig) = self.lookup_any_type(actual_name) else {
            return false;
        };
        let subst = sig
            .type_params
            .iter()
            .cloned()
            .zip(actual_args.iter().cloned())
            .collect::<HashMap<_, _>>();
        sig.with_bounds.iter().any(|bound| {
            let bound_ty = substitute_type(bound, &subst);
            self.is_assignable_inner(&bound_ty, expected, seen)
        })
    }
}

fn enum_case_pattern_accepts_arity(params: &[FieldSig], arity: usize) -> bool {
    arity <= params.len() && params[arity..].iter().all(|param| param.has_initializer)
}

fn constructor_field_sigs_from_params(params: &[ParamSig]) -> Vec<FieldSig> {
    params
        .iter()
        .map(|param| FieldSig {
            name: param.name.clone(),
            ty: param.ty.clone(),
            mutable: false,
            hidden: false,
            has_initializer: param.has_initializer,
            variadic: param.variadic,
        })
        .collect()
}

fn constructor_target_name(ret: &Ty) -> Option<&str> {
    match ret {
        Ty::Named(name, _) => Some(name.as_str()),
        _ => None,
    }
}

fn positional_constructor_prefix_message(
    target_name: &str,
    params: &[FieldSig],
    arg_count: usize,
) -> Option<String> {
    if arg_count > params.len() {
        return None;
    }
    let (required_index, required) = params
        .iter()
        .enumerate()
        .skip(arg_count)
        .find(|(_, param)| !param.variadic && !param.has_initializer)?;
    let defaulted_before = params[..required_index]
        .iter()
        .filter(|param| param.has_initializer)
        .map(|param| param.name.clone())
        .collect::<Vec<_>>();
    let defaulted = defaulted_before.first()?;
    Some(format!(
        "positional construction for {target_name} with {arg_count} {} leaves required field '{}' unset; positional arguments fill fields in declaration order and do not skip defaulted fields. Use {target_name} {{ {}: ... }} to omit defaulted field '{}', pass all fields positionally in declaration order, or move defaulted fields after required fields.",
        if arg_count == 1 {
            "argument"
        } else {
            "arguments"
        },
        required.name,
        required.name,
        defaulted
    ))
}

fn enum_case_pattern_required_count(params: &[FieldSig]) -> usize {
    params.iter().filter(|param| !param.has_initializer).count()
}

fn first_shape_literal_span(expr: &Expr) -> Option<crate::source::Span> {
    match expr {
        Expr::ShapeLiteral { span, .. } => Some(*span),
        Expr::Group { inner, .. } => first_shape_literal_span(inner),
        _ => None,
    }
}

fn empty_collection_literal_span(expr: &Expr) -> Option<crate::source::Span> {
    match expr {
        Expr::ListLiteral { items, span } if items.is_empty() => Some(*span),
        Expr::Group { inner, .. } => empty_collection_literal_span(inner),
        _ => None,
    }
}

fn constructor_body_delegates(body: &CallableBody) -> bool {
    match body {
        CallableBody::Expr(expr) => is_constructor_delegation_expr(expr),
        CallableBody::Block(_) => false,
    }
}

fn constructor_delegation_call(
    body: &CallableBody,
) -> Option<(&[crate::ast::CallArg], bool, crate::source::Span)> {
    let CallableBody::Expr(expr) = body else {
        return None;
    };
    constructor_delegation_call_expr(expr)
}

fn constructor_delegation_call_expr(
    expr: &Expr,
) -> Option<(&[crate::ast::CallArg], bool, crate::source::Span)> {
    match expr {
        Expr::Call {
            callee,
            args,
            uses_brace_syntax,
            span,
        } if matches!(callee.as_ref(), Expr::Identifier { name, .. } if name == "this") => {
            Some((args.as_slice(), *uses_brace_syntax, *span))
        }
        Expr::Group { inner, .. } => constructor_delegation_call_expr(inner),
        _ => None,
    }
}

fn constructor_body_contains_this_delegation_call(body: &CallableBody) -> bool {
    match body {
        CallableBody::Expr(expr) => is_constructor_delegation_expr(expr),
        CallableBody::Block(block) => block.statements.iter().any(constructor_stmt_delegates),
    }
}

fn constructor_stmt_delegates(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::Expr(stmt) => is_constructor_delegation_expr(&stmt.expr),
        Stmt::Return(stmt) => stmt
            .value
            .as_ref()
            .is_some_and(is_constructor_delegation_expr),
        _ => false,
    }
}

fn constructor_body_contains_delegation_attempt(body: &CallableBody) -> bool {
    match body {
        CallableBody::Expr(expr) => is_constructor_delegation_attempt_expr(expr),
        CallableBody::Block(block) => block
            .statements
            .iter()
            .any(constructor_stmt_contains_delegation_attempt),
    }
}

fn constructor_stmt_contains_delegation_attempt(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::Expr(stmt) => is_constructor_delegation_attempt_expr(&stmt.expr),
        Stmt::Return(stmt) => stmt
            .value
            .as_ref()
            .is_some_and(is_constructor_delegation_attempt_expr),
        _ => false,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LoopControlKind {
    Break,
    Continue,
}

impl LoopControlKind {
    fn diagnostic_code(self) -> &'static str {
        match self {
            LoopControlKind::Break => "invalid_for_yield_break",
            LoopControlKind::Continue => "invalid_for_yield_continue",
        }
    }

    fn keyword(self) -> &'static str {
        match self {
            LoopControlKind::Break => "break",
            LoopControlKind::Continue => "continue",
        }
    }

    fn state_name(self) -> &'static str {
        match self {
            LoopControlKind::Break => "early-exit",
            LoopControlKind::Continue => "skip",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LoopControlSpan {
    kind: LoopControlKind,
    span: crate::source::Span,
}

fn loop_control_targeting_current_loop_in_block(block: &Block) -> Option<LoopControlSpan> {
    block
        .statements
        .iter()
        .find_map(loop_control_targeting_current_loop_in_stmt)
}

fn condition_clause_expr(clause: &IfConditionClause) -> &Expr {
    match clause {
        IfConditionClause::Let(clause) => &clause.value,
        IfConditionClause::Expr(expr) => expr,
    }
}

fn loop_control_targeting_current_loop_in_stmt(stmt: &Stmt) -> Option<LoopControlSpan> {
    match stmt {
        Stmt::Break(stmt) => Some(LoopControlSpan {
            kind: LoopControlKind::Break,
            span: stmt.span,
        }),
        Stmt::Continue(stmt) => Some(LoopControlSpan {
            kind: LoopControlKind::Continue,
            span: stmt.span,
        }),
        Stmt::Binding(stmt) => stmt
            .values
            .iter()
            .find_map(loop_control_targeting_current_loop_in_expr),
        Stmt::PatternBinding(stmt) => stmt
            .clauses
            .iter()
            .find_map(|clause| loop_control_targeting_current_loop_in_expr(&clause.value))
            .or_else(|| loop_control_targeting_current_loop_in_expr(&stmt.value)),
        Stmt::Assignment(stmt) => stmt
            .targets
            .iter()
            .chain(stmt.values.iter())
            .find_map(loop_control_targeting_current_loop_in_expr),
        Stmt::Defer(stmt) => match &stmt.action {
            crate::ast::DeferAction::Call(expr) => {
                loop_control_targeting_current_loop_in_expr(expr)
            }
            // Defer blocks have separate control-flow rules; loop control there
            // never targets this comprehension item.
            crate::ast::DeferAction::Block(_) => None,
        },
        Stmt::If(stmt) => loop_control_targeting_current_loop_in_if_stmt(stmt),
        Stmt::Match(stmt) => {
            loop_control_targeting_current_loop_in_expr(&stmt.value).or_else(|| {
                stmt.cases.iter().find_map(|case| {
                    case.guard
                        .as_ref()
                        .and_then(loop_control_targeting_current_loop_in_expr)
                        .or_else(|| match &case.body {
                            MatchCaseBody::Block(block) => {
                                loop_control_targeting_current_loop_in_block(block)
                            }
                            MatchCaseBody::Expr(expr) => {
                                loop_control_targeting_current_loop_in_expr(expr)
                            }
                        })
                })
            })
        }
        Stmt::While(stmt) => stmt.condition_clauses.iter().find_map(|condition| {
            loop_control_targeting_current_loop_in_expr(condition_clause_expr(condition))
        }),
        Stmt::For(stmt) => stmt.bindings.iter().find_map(|binding| {
            binding
                .iterable
                .as_ref()
                .and_then(loop_control_targeting_current_loop_in_expr)
                .or_else(|| {
                    binding
                        .values
                        .iter()
                        .find_map(loop_control_targeting_current_loop_in_expr)
                })
        }),
        Stmt::LetElse(stmt) => stmt
            .clauses
            .iter()
            .find_map(|clause| loop_control_targeting_current_loop_in_expr(&clause.value))
            .or_else(|| loop_control_targeting_current_loop_in_expr(&stmt.value))
            .or_else(|| loop_control_targeting_current_loop_in_block(&stmt.else_block)),
        Stmt::Expr(stmt) => loop_control_targeting_current_loop_in_expr(&stmt.expr),
        Stmt::Return(_) | Stmt::LocalFunction(_) => None,
    }
}

fn loop_control_targeting_current_loop_in_expr(expr: &Expr) -> Option<LoopControlSpan> {
    match expr {
        Expr::ListLiteral { items, .. }
        | Expr::TupleLiteral { items, .. }
        | Expr::ShapeLiteral { items, .. } => items
            .iter()
            .find_map(loop_control_targeting_current_loop_in_expr),
        Expr::Call { callee, args, .. } => loop_control_targeting_current_loop_in_expr(callee)
            .or_else(|| {
                args.iter()
                    .find_map(|arg| loop_control_targeting_current_loop_in_expr(&arg.value))
            }),
        Expr::Member { receiver, .. } => loop_control_targeting_current_loop_in_expr(receiver),
        Expr::Index {
            receiver, index, ..
        } => loop_control_targeting_current_loop_in_expr(receiver)
            .or_else(|| loop_control_targeting_current_loop_in_expr(index)),
        Expr::RecordUpdate {
            receiver, patch, ..
        } => loop_control_targeting_current_loop_in_expr(receiver)
            .or_else(|| loop_control_targeting_current_loop_in_expr(patch)),
        Expr::RecordLiteral { fields, values, .. } => fields
            .iter()
            .find_map(|field| loop_control_targeting_current_loop_in_expr(&field.value))
            .or_else(|| {
                values
                    .iter()
                    .find_map(loop_control_targeting_current_loop_in_expr)
            }),
        Expr::Try { value, .. }
        | Expr::Unary { expr: value, .. }
        | Expr::Group { inner: value, .. }
        | Expr::Spread { value, .. } => loop_control_targeting_current_loop_in_expr(value),
        Expr::ExtractOr {
            value, fallback, ..
        } => loop_control_targeting_current_loop_in_expr(value)
            .or_else(|| loop_control_targeting_current_loop_in_expr(fallback)),
        Expr::Break { span } => Some(LoopControlSpan {
            kind: LoopControlKind::Break,
            span: *span,
        }),
        Expr::Continue { span } => Some(LoopControlSpan {
            kind: LoopControlKind::Continue,
            span: *span,
        }),
        Expr::Return { .. } => None,
        Expr::Binary { left, right, .. } => loop_control_targeting_current_loop_in_expr(left)
            .or_else(|| loop_control_targeting_current_loop_in_expr(right)),
        Expr::Is { left, .. } => loop_control_targeting_current_loop_in_expr(left),
        Expr::If {
            condition,
            then_block,
            else_branch,
            ..
        } => loop_control_targeting_current_loop_in_expr(condition)
            .or_else(|| loop_control_targeting_current_loop_in_block(then_block))
            .or_else(|| match else_branch.as_ref() {
                ElseExprBranch::If(expr) => loop_control_targeting_current_loop_in_expr(expr),
                ElseExprBranch::Block(block) => loop_control_targeting_current_loop_in_block(block),
            }),
        Expr::Block { body, .. } => loop_control_targeting_current_loop_in_block(body),
        Expr::Match { value, cases, .. } => loop_control_targeting_current_loop_in_expr(value)
            .or_else(|| {
                cases.iter().find_map(|case| {
                    case.guard
                        .as_ref()
                        .and_then(loop_control_targeting_current_loop_in_expr)
                        .or_else(|| match &case.body {
                            MatchCaseBody::Block(block) => {
                                loop_control_targeting_current_loop_in_block(block)
                            }
                            MatchCaseBody::Expr(expr) => {
                                loop_control_targeting_current_loop_in_expr(expr)
                            }
                        })
                })
            }),
        Expr::ForYield { bindings, .. } => bindings.iter().find_map(|binding| {
            binding
                .iterable
                .as_ref()
                .and_then(loop_control_targeting_current_loop_in_expr)
                .or_else(|| {
                    binding
                        .values
                        .iter()
                        .find_map(loop_control_targeting_current_loop_in_expr)
                })
        }),
        // Nested callables and anonymous interface methods are separate
        // control-flow boundaries.
        Expr::Lambda { .. } | Expr::AnonymousInterface { .. } | Expr::AnonymousObject { .. } => {
            None
        }
        Expr::Identifier { .. }
        | Expr::Placeholder { .. }
        | Expr::Integer { .. }
        | Expr::Float { .. }
        | Expr::String { .. }
        | Expr::Bool { .. }
        | Expr::Unit { .. }
        | Expr::TypeOf { .. } => None,
    }
}

fn loop_control_targeting_current_loop_in_if_stmt(stmt: &IfStmt) -> Option<LoopControlSpan> {
    stmt.condition
        .as_ref()
        .and_then(loop_control_targeting_current_loop_in_expr)
        .or_else(|| {
            stmt.condition_clauses
                .iter()
                .find_map(|clause| match clause {
                    IfConditionClause::Let(clause) => {
                        loop_control_targeting_current_loop_in_expr(&clause.value)
                    }
                    IfConditionClause::Expr(expr) => {
                        loop_control_targeting_current_loop_in_expr(expr)
                    }
                })
        })
        .or_else(|| {
            stmt.pattern_value
                .as_ref()
                .and_then(loop_control_targeting_current_loop_in_expr)
        })
        .or_else(|| {
            stmt.pattern_clauses
                .iter()
                .find_map(|clause| loop_control_targeting_current_loop_in_expr(&clause.value))
        })
        .or_else(|| {
            stmt.binding_value
                .as_ref()
                .and_then(loop_control_targeting_current_loop_in_expr)
        })
        .or_else(|| loop_control_targeting_current_loop_in_block(&stmt.then_block))
        .or_else(|| match &stmt.else_branch {
            Some(ElseBranch::If(stmt)) => loop_control_targeting_current_loop_in_if_stmt(stmt),
            Some(ElseBranch::Block(block)) => loop_control_targeting_current_loop_in_block(block),
            None => None,
        })
}

fn lazy_arg_forbidden_control_flow_span(expr: &Expr) -> Option<crate::source::Span> {
    match expr {
        Expr::Try { span, .. } => Some(*span),
        Expr::Return { span, .. } | Expr::Break { span } | Expr::Continue { span } => Some(*span),
        Expr::Spread { value, .. } => lazy_arg_forbidden_control_flow_span(value),
        Expr::ListLiteral { items, .. }
        | Expr::TupleLiteral { items, .. }
        | Expr::ShapeLiteral { items, .. } => {
            items.iter().find_map(lazy_arg_forbidden_control_flow_span)
        }
        Expr::Call { callee, args, .. } => {
            lazy_arg_forbidden_control_flow_span(callee).or_else(|| {
                args.iter()
                    .find_map(|arg| lazy_arg_forbidden_control_flow_span(&arg.value))
            })
        }
        Expr::Member { receiver, .. } => lazy_arg_forbidden_control_flow_span(receiver),
        Expr::Index {
            receiver, index, ..
        } => lazy_arg_forbidden_control_flow_span(receiver)
            .or_else(|| lazy_arg_forbidden_control_flow_span(index)),
        Expr::RecordUpdate {
            receiver, patch, ..
        } => lazy_arg_forbidden_control_flow_span(receiver)
            .or_else(|| lazy_arg_forbidden_control_flow_span(patch)),
        Expr::RecordLiteral { fields, values, .. } => fields
            .iter()
            .find_map(|field| lazy_arg_forbidden_control_flow_span(&field.value))
            .or_else(|| values.iter().find_map(lazy_arg_forbidden_control_flow_span)),
        Expr::Unary { expr, .. } => lazy_arg_forbidden_control_flow_span(expr),
        Expr::ExtractOr {
            value, fallback, ..
        } => lazy_arg_forbidden_control_flow_span(value)
            .or_else(|| lazy_arg_forbidden_control_flow_span(fallback)),
        Expr::Binary { left, right, .. } => lazy_arg_forbidden_control_flow_span(left)
            .or_else(|| lazy_arg_forbidden_control_flow_span(right)),
        Expr::Is { left, .. } => lazy_arg_forbidden_control_flow_span(left),
        Expr::If {
            condition,
            then_block,
            else_branch,
            ..
        } => lazy_arg_forbidden_control_flow_span(condition)
            .or_else(|| lazy_arg_forbidden_control_flow_span_in_block(then_block))
            .or_else(|| match else_branch.as_ref() {
                ElseExprBranch::If(expr) => lazy_arg_forbidden_control_flow_span(expr),
                ElseExprBranch::Block(block) => {
                    lazy_arg_forbidden_control_flow_span_in_block(block)
                }
            }),
        Expr::Block { body, .. } => lazy_arg_forbidden_control_flow_span_in_block(body),
        Expr::Match { value, cases, .. } => {
            lazy_arg_forbidden_control_flow_span(value).or_else(|| {
                cases.iter().find_map(|case| {
                    case.guard
                        .as_ref()
                        .and_then(lazy_arg_forbidden_control_flow_span)
                        .or_else(|| match &case.body {
                            MatchCaseBody::Block(block) => {
                                lazy_arg_forbidden_control_flow_span_in_block(block)
                            }
                            MatchCaseBody::Expr(expr) => lazy_arg_forbidden_control_flow_span(expr),
                        })
                })
            })
        }
        Expr::ForYield {
            bindings,
            yield_body,
            ..
        } => bindings
            .iter()
            .find_map(|binding| {
                binding
                    .iterable
                    .as_ref()
                    .and_then(lazy_arg_forbidden_control_flow_span)
                    .or_else(|| {
                        binding
                            .values
                            .iter()
                            .find_map(lazy_arg_forbidden_control_flow_span)
                    })
            })
            .or_else(|| lazy_arg_forbidden_control_flow_span_in_block(yield_body)),
        // Nested callables have their own control-flow boundary.
        Expr::Lambda { .. } | Expr::AnonymousInterface { .. } | Expr::AnonymousObject { .. } => {
            None
        }
        Expr::Group { inner, .. } => lazy_arg_forbidden_control_flow_span(inner),
        Expr::Identifier { .. }
        | Expr::Placeholder { .. }
        | Expr::Integer { .. }
        | Expr::Float { .. }
        | Expr::String { .. }
        | Expr::Bool { .. }
        | Expr::Unit { .. }
        | Expr::TypeOf { .. } => None,
    }
}

fn lazy_arg_forbidden_control_flow_span_in_block(block: &Block) -> Option<crate::source::Span> {
    block
        .statements
        .iter()
        .find_map(lazy_arg_forbidden_control_flow_span_in_stmt)
}

fn lazy_arg_forbidden_control_flow_span_in_stmt(stmt: &Stmt) -> Option<crate::source::Span> {
    match stmt {
        Stmt::Return(stmt) => Some(stmt.span),
        Stmt::Break(stmt) => Some(stmt.span),
        Stmt::Continue(stmt) => Some(stmt.span),
        Stmt::Binding(stmt) => stmt
            .values
            .iter()
            .find_map(lazy_arg_forbidden_control_flow_span),
        Stmt::PatternBinding(stmt) => stmt
            .clauses
            .iter()
            .find_map(|clause| lazy_arg_forbidden_control_flow_span(&clause.value))
            .or_else(|| lazy_arg_forbidden_control_flow_span(&stmt.value)),
        Stmt::Assignment(stmt) => stmt
            .targets
            .iter()
            .chain(stmt.values.iter())
            .find_map(lazy_arg_forbidden_control_flow_span),
        Stmt::Defer(stmt) => match &stmt.action {
            crate::ast::DeferAction::Call(expr) => lazy_arg_forbidden_control_flow_span(expr),
            crate::ast::DeferAction::Block(block) => {
                lazy_arg_forbidden_control_flow_span_in_block(block)
            }
        },
        Stmt::If(stmt) => lazy_arg_forbidden_control_flow_span_in_if_stmt(stmt),
        Stmt::Match(stmt) => lazy_arg_forbidden_control_flow_span(&stmt.value).or_else(|| {
            stmt.cases.iter().find_map(|case| {
                case.guard
                    .as_ref()
                    .and_then(lazy_arg_forbidden_control_flow_span)
                    .or_else(|| match &case.body {
                        MatchCaseBody::Block(block) => {
                            lazy_arg_forbidden_control_flow_span_in_block(block)
                        }
                        MatchCaseBody::Expr(expr) => lazy_arg_forbidden_control_flow_span(expr),
                    })
            })
        }),
        Stmt::While(stmt) => stmt
            .condition_clauses
            .iter()
            .find_map(|condition| {
                lazy_arg_forbidden_control_flow_span(condition_clause_expr(condition))
            })
            .or_else(|| lazy_arg_forbidden_control_flow_span_in_block(&stmt.body)),
        Stmt::For(stmt) => stmt
            .bindings
            .iter()
            .find_map(|binding| {
                binding
                    .iterable
                    .as_ref()
                    .and_then(lazy_arg_forbidden_control_flow_span)
                    .or_else(|| {
                        binding
                            .values
                            .iter()
                            .find_map(lazy_arg_forbidden_control_flow_span)
                    })
            })
            .or_else(|| lazy_arg_forbidden_control_flow_span_in_block(&stmt.body)),
        Stmt::LetElse(stmt) => stmt
            .clauses
            .iter()
            .find_map(|clause| lazy_arg_forbidden_control_flow_span(&clause.value))
            .or_else(|| lazy_arg_forbidden_control_flow_span(&stmt.value))
            .or_else(|| lazy_arg_forbidden_control_flow_span_in_block(&stmt.else_block)),
        Stmt::Expr(stmt) => lazy_arg_forbidden_control_flow_span(&stmt.expr),
        Stmt::LocalFunction(_) => None,
    }
}

fn lazy_arg_forbidden_control_flow_span_in_if_stmt(stmt: &IfStmt) -> Option<crate::source::Span> {
    stmt.condition
        .as_ref()
        .and_then(lazy_arg_forbidden_control_flow_span)
        .or_else(|| {
            stmt.condition_clauses
                .iter()
                .find_map(|clause| match clause {
                    IfConditionClause::Let(clause) => {
                        lazy_arg_forbidden_control_flow_span(&clause.value)
                    }
                    IfConditionClause::Expr(expr) => lazy_arg_forbidden_control_flow_span(expr),
                })
        })
        .or_else(|| {
            stmt.pattern_value
                .as_ref()
                .and_then(lazy_arg_forbidden_control_flow_span)
        })
        .or_else(|| {
            stmt.pattern_clauses
                .iter()
                .find_map(|clause| lazy_arg_forbidden_control_flow_span(&clause.value))
        })
        .or_else(|| {
            stmt.binding_value
                .as_ref()
                .and_then(lazy_arg_forbidden_control_flow_span)
        })
        .or_else(|| lazy_arg_forbidden_control_flow_span_in_block(&stmt.then_block))
        .or_else(|| match &stmt.else_branch {
            Some(ElseBranch::If(stmt)) => lazy_arg_forbidden_control_flow_span_in_if_stmt(stmt),
            Some(ElseBranch::Block(block)) => lazy_arg_forbidden_control_flow_span_in_block(block),
            None => None,
        })
}

fn is_constructor_delegation_expr(expr: &Expr) -> bool {
    match expr {
        Expr::Call { callee, .. } => {
            matches!(callee.as_ref(), Expr::Identifier { name, .. } if name == "this")
        }
        Expr::Group { inner, .. } => is_constructor_delegation_expr(inner),
        _ => false,
    }
}

fn is_constructor_delegation_attempt_expr(expr: &Expr) -> bool {
    match expr {
        Expr::Call { callee, .. } => {
            matches!(callee.as_ref(), Expr::Identifier { name, .. } if name == "this" || name == "new")
        }
        Expr::Group { inner, .. } => is_constructor_delegation_attempt_expr(inner),
        _ => false,
    }
}

fn constructor_cycle_node_label(node: &ConstructorCycleNode) -> String {
    let params = node
        .sig
        .params
        .iter()
        .map(|param| format!("{} {}", param.name, param.ty.describe()))
        .collect::<Vec<_>>()
        .join(", ");
    format!("new({params})")
}

fn constructor_assigned_fields(
    body: &CallableBody,
    owner: &TypeSig,
    method: &MethodDecl,
) -> HashSet<String> {
    let CallableBody::Block(block) = body else {
        return HashSet::new();
    };
    let param_names = method
        .params
        .iter()
        .map(|param| param.name.as_str())
        .collect::<HashSet<_>>();
    let owner_fields = owner
        .fields
        .iter()
        .map(|field| field.name.as_str())
        .collect::<HashSet<_>>();
    let mut assigned = HashSet::new();
    for statement in &block.statements {
        let Stmt::Assignment(statement) = statement else {
            continue;
        };
        if statement.operator != AssignOp::Assign {
            continue;
        }
        for target in &statement.targets {
            if let Some(name) = constructor_assigned_field(target, &owner_fields, &param_names) {
                assigned.insert(name.to_string());
            }
        }
    }
    assigned
}

fn constructor_assigned_field<'a>(
    target: &'a Expr,
    owner_fields: &HashSet<&str>,
    param_names: &HashSet<&str>,
) -> Option<&'a str> {
    match target {
        Expr::Member { receiver, name, .. }
            if matches!(receiver.as_ref(), Expr::Identifier { name, .. } if name == "this")
                && owner_fields.contains(name.as_str()) =>
        {
            Some(name.as_str())
        }
        Expr::Identifier { name, .. }
            if owner_fields.contains(name.as_str()) && !param_names.contains(name.as_str()) =>
        {
            Some(name.as_str())
        }
        Expr::Group { inner, .. } => constructor_assigned_field(inner, owner_fields, param_names),
        _ => None,
    }
}

fn function_sig_from_function(
    function: &FunctionDecl,
    owner_type_params: &[String],
) -> FunctionSig {
    let type_params = function
        .type_params
        .iter()
        .map(|param| param.name.clone())
        .chain(owner_type_params.iter().cloned())
        .collect::<HashSet<_>>();
    FunctionSig {
        type_params: function
            .type_params
            .iter()
            .map(|param| param.name.clone())
            .collect(),
        reified_type_params: function
            .type_params
            .iter()
            .filter(|param| param.reified)
            .map(|param| param.name.clone())
            .collect(),
        generic_conditions: generic_condition_sigs(
            &function.type_params,
            &function.type_conditions,
            &type_params,
        ),
        params: function
            .params
            .iter()
            .map(|param| ParamSig {
                name: param.name.clone(),
                ty: param
                    .ty
                    .as_ref()
                    .map(|ty| convert_type_ref(ty, &type_params))
                    .unwrap_or(Ty::Unknown),
                variadic: param.variadic,
                lazy: param.lazy,
                has_initializer: param.initializer.is_some(),
            })
            .collect(),
        ret: function
            .return_type
            .as_ref()
            .map(|ty| convert_type_ref(ty, &type_params))
            .unwrap_or(Ty::Unknown),
        visibility: function.visibility,
        has_body: true,
    }
}

fn generic_condition_sigs(
    params: &[TypeParam],
    conditions: &[GenericCondition],
    type_params: &HashSet<String>,
) -> Vec<GenericConditionSig> {
    let mut result = Vec::new();
    for param in params {
        for bound in &param.bounds {
            result.push(GenericConditionSig::Bound {
                subject: Ty::TypeParam(param.name.clone()),
                bound: convert_type_ref(bound, type_params),
            });
        }
    }
    result.extend(conditions.iter().map(|condition| match condition {
        GenericCondition::Bound { subject, bound, .. } => GenericConditionSig::Bound {
            subject: convert_type_ref(subject, type_params),
            bound: convert_type_ref(bound, type_params),
        },
        GenericCondition::Equal { left, right, .. } => GenericConditionSig::Equal {
            left: convert_type_ref(left, type_params),
            right: convert_type_ref(right, type_params),
        },
    }));
    result
}

fn function_sig_from_method(method: &MethodDecl, owner_type_params: &[String]) -> FunctionSig {
    let type_params = method
        .type_params
        .iter()
        .map(|param| param.name.clone())
        .chain(owner_type_params.iter().cloned())
        .collect::<HashSet<_>>();
    FunctionSig {
        type_params: method
            .type_params
            .iter()
            .map(|param| param.name.clone())
            .collect(),
        reified_type_params: method
            .type_params
            .iter()
            .filter(|param| param.reified)
            .map(|param| param.name.clone())
            .collect(),
        generic_conditions: generic_condition_sigs(
            &method.type_params,
            &method.type_conditions,
            &type_params,
        ),
        params: method
            .params
            .iter()
            .map(|param| ParamSig {
                name: param.name.clone(),
                ty: param
                    .ty
                    .as_ref()
                    .map(|ty| convert_type_ref(ty, &type_params))
                    .unwrap_or(Ty::Unknown),
                variadic: param.variadic,
                lazy: param.lazy,
                has_initializer: param.initializer.is_some(),
            })
            .collect(),
        ret: method
            .return_type
            .as_ref()
            .map(|ty| convert_type_ref(ty, &type_params))
            .unwrap_or(Ty::Unknown),
        visibility: method.visibility,
        has_body: method.body.is_some(),
    }
}

fn enum_case_field_sig_ty(
    field: &FieldDecl,
    owner_fields: &[FieldSig],
    owner_type_params: &HashSet<String>,
) -> Ty {
    field
        .ty
        .as_ref()
        .map(|ty| convert_type_ref(ty, owner_type_params))
        .or_else(|| {
            owner_fields
                .iter()
                .find(|owner_field| owner_field.name == field.name)
                .map(|owner_field| owner_field.ty.clone())
        })
        .or_else(|| field.initializer.as_ref().and_then(infer_literal_type))
        .unwrap_or(Ty::Unknown)
}

fn type_sig_from_decl(decl: &TypeDecl) -> TypeSig {
    let owner_params = decl
        .type_params
        .iter()
        .map(|param| param.name.clone())
        .collect::<HashSet<_>>();

    let mut fields = Vec::new();
    let mut methods = HashMap::new();
    let mut enum_cases = HashMap::new();

    for member in &decl.members {
        match member {
            TypeMember::Field(field) => fields.push(FieldSig {
                name: field.name.clone(),
                ty: field
                    .ty
                    .as_ref()
                    .map(|ty| convert_type_ref(ty, &owner_params))
                    .or_else(|| field.initializer.as_ref().and_then(infer_literal_type))
                    .unwrap_or(Ty::Unknown),
                mutable: field.mutable,
                hidden: field.visibility == Visibility::Hidden,
                has_initializer: field.initializer.is_some(),
                variadic: false,
            }),
            TypeMember::Method(method) => {
                methods
                    .entry(method.name.clone())
                    .or_insert_with(Vec::new)
                    .push(function_sig_from_method(
                        method,
                        &decl
                            .type_params
                            .iter()
                            .map(|param| param.name.clone())
                            .collect::<Vec<_>>(),
                    ));
            }
            TypeMember::Case(case) => {
                let ctor_params = case
                    .fields
                    .iter()
                    .map(|field| FieldSig {
                        name: field.name.clone(),
                        ty: enum_case_field_sig_ty(field, &fields, &owner_params),
                        mutable: field.mutable,
                        hidden: field.visibility == Visibility::Hidden,
                        has_initializer: field.initializer.is_some(),
                        variadic: false,
                    })
                    .collect::<Vec<_>>();
                enum_cases.insert(
                    case.name.clone(),
                    EnumCaseSig {
                        params: ctor_params,
                        field_count: case.fields.len(),
                        result: Ty::Named(
                            decl.name.clone(),
                            decl.type_params
                                .iter()
                                .map(|param| Ty::TypeParam(param.name.clone()))
                                .collect(),
                        ),
                    },
                );
            }
        }
    }

    TypeSig {
        kind: decl.kind,
        name: decl.name.clone(),
        type_params: decl
            .type_params
            .iter()
            .map(|param| param.name.clone())
            .collect(),
        generic_conditions: generic_condition_sigs(
            &decl.type_params,
            &decl.type_conditions,
            &owner_params,
        ),
        with_bounds: decl
            .with_bounds
            .iter()
            .map(|bound| convert_type_ref(bound, &owner_params))
            .collect(),
        fields,
        methods,
        enum_cases,
    }
}

fn convert_type_ref(reference: &TypeRef, type_params: &HashSet<String>) -> Ty {
    match reference {
        TypeRef::Wildcard { .. } => Ty::Wildcard,
        TypeRef::Named { name, args, .. } => {
            if type_params.contains(name) {
                Ty::TypeParam(name.clone())
            } else if name == "Never" && args.is_empty() {
                Ty::Never
            } else {
                Ty::Named(
                    name.clone(),
                    args.iter()
                        .map(|arg| convert_type_ref(arg, type_params))
                        .collect(),
                )
            }
        }
        TypeRef::Tuple { fields, .. } => Ty::Tuple(
            fields
                .iter()
                .map(|field| convert_type_ref(&field.ty, type_params))
                .collect(),
        ),
        TypeRef::Record { fields, .. } => Ty::Record(
            fields
                .iter()
                .map(|field| (field.name.clone(), convert_type_ref(&field.ty, type_params)))
                .collect(),
        ),
        TypeRef::Function { params, ret, .. } => Ty::Function(
            params
                .iter()
                .map(|param| convert_type_ref(param, type_params))
                .collect(),
            Box::new(convert_type_ref(ret, type_params)),
        ),
    }
}

fn infer_literal_type(expr: &Expr) -> Option<Ty> {
    match expr {
        Expr::Group { inner, .. } => infer_literal_type(inner),
        Expr::Integer { .. } => Some(Ty::int()),
        Expr::Float { .. } => Some(Ty::float()),
        Expr::String { .. } => Some(Ty::str()),
        Expr::Bool { .. } => Some(Ty::bool()),
        Expr::Unit { .. } => Some(Ty::unit()),
        Expr::ListLiteral { items, .. } => {
            let item = items.iter().fold(Ty::Unknown, |acc, item| {
                join_types(&acc, &infer_literal_type(item).unwrap_or(Ty::Unknown))
            });
            Some(Ty::Named("Vector".to_string(), vec![item]))
        }
        _ => None,
    }
}

fn type_ref_named_name(reference: &TypeRef) -> Option<&str> {
    match reference {
        TypeRef::Named { name, .. } => Some(name.as_str()),
        _ => None,
    }
}

fn is_list_type_ref(reference: &TypeRef) -> bool {
    matches!(reference, TypeRef::Named { name, args, .. } if name == "Vector" && args.len() == 1)
}

fn is_map_ty(ty: &Ty) -> bool {
    matches!(ty, Ty::Named(name, args) if name == "Map" && args.len() == 2)
}

fn is_list_or_map_ty(ty: &Ty) -> bool {
    matches!(
        ty,
        Ty::Named(name, args)
            if (name == "Vector" && args.len() == 1) || (name == "Map" && args.len() == 2)
    )
}

fn join_spread_map_types(expected: &Ty, spread_types: impl IntoIterator<Item = Ty>) -> Ty {
    let (mut key, mut value) = match expected {
        Ty::Named(name, args) if name == "Map" && args.len() == 2 => {
            (args[0].clone(), args[1].clone())
        }
        _ => (Ty::Unknown, Ty::Unknown),
    };
    for ty in spread_types {
        if let Ty::Named(name, args) = ty {
            if name == "Map" && args.len() == 2 {
                key = join_types(&key, &args[0]);
                value = join_types(&value, &args[1]);
            }
        }
    }
    Ty::Named("Map".to_string(), vec![key, value])
}

fn is_unit_type_ref(reference: &TypeRef) -> bool {
    matches!(reference, TypeRef::Named { name, args, .. } if name == "Unit" && args.is_empty())
}

fn unit_function_param_span(reference: &TypeRef) -> Option<crate::source::Span> {
    match reference {
        TypeRef::Function { params, .. } => params.iter().find_map(|param| {
            if is_unit_type_ref(param) {
                Some(param.span())
            } else {
                unit_function_param_span(param)
            }
        }),
        TypeRef::Named { args, .. } => args.iter().find_map(unit_function_param_span),
        TypeRef::Tuple { fields, .. } => fields
            .iter()
            .find_map(|field| unit_function_param_span(&field.ty)),
        TypeRef::Record { fields, .. } => fields
            .iter()
            .find_map(|field| unit_function_param_span(&field.ty)),
        TypeRef::Wildcard { .. } => None,
    }
}

fn variadic_arg_ty(ty: &Ty) -> Option<Ty> {
    match ty {
        Ty::Named(name, args) if name == "Vector" && args.len() == 1 => args.first().cloned(),
        Ty::Unknown => Some(Ty::Unknown),
        _ => None,
    }
}

fn call_arg_expected_ty(variadic: bool, param_ty: &Ty, is_named_arg: bool) -> Ty {
    if variadic && !is_named_arg {
        variadic_arg_ty(param_ty).unwrap_or(Ty::Unknown)
    } else {
        param_ty.clone()
    }
}

fn call_arg_expected_ty_for_arg(variadic: bool, param_ty: &Ty, arg: &crate::ast::CallArg) -> Ty {
    if variadic && arg.name.is_none() && matches!(arg.value, Expr::Spread { .. }) {
        param_ty.clone()
    } else {
        call_arg_expected_ty(variadic, param_ty, arg.name.is_some())
    }
}

fn call_arg_value_expr(arg: &crate::ast::CallArg) -> &Expr {
    match &arg.value {
        Expr::Spread { value, .. } => value,
        _ => &arg.value,
    }
}

fn impl_target_type_params(reference: &TypeRef) -> Vec<String> {
    match reference {
        TypeRef::Named { args, .. } => args
            .iter()
            .filter_map(|arg| match arg {
                TypeRef::Named { name, args, .. } if args.is_empty() => Some(name.clone()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

struct ArgArrangement<'a> {
    slots: Vec<Vec<&'a crate::ast::CallArg>>,
    overflow: usize,
    missing_required: usize,
}

fn arrange_param_args<'a>(
    params: &[ParamSig],
    args: &'a [crate::ast::CallArg],
) -> ArgArrangement<'a> {
    let mut slots = vec![Vec::new(); params.len()];
    let mut positional_index = 0usize;
    let mut overflow = 0usize;
    for arg in args {
        if let Some(name) = &arg.name {
            if let Some(index) = params.iter().position(|param| param.name == *name) {
                if slots[index].is_empty() {
                    slots[index].push(arg);
                } else {
                    overflow += 1;
                }
            } else {
                overflow += 1;
            }
            continue;
        }

        while positional_index < params.len()
            && !params[positional_index].variadic
            && !slots[positional_index].is_empty()
        {
            positional_index += 1;
        }
        if params.last().is_some_and(|param| param.variadic)
            && positional_index >= params.len().saturating_sub(1)
        {
            if let Some(slot) = slots.last_mut() {
                if slot.first().is_some_and(|arg| arg.name.is_some()) {
                    overflow += 1;
                } else {
                    slot.push(arg);
                }
            } else {
                overflow += 1;
            }
        } else if positional_index < params.len() {
            slots[positional_index].push(arg);
            if !params[positional_index].variadic {
                positional_index += 1;
            }
        } else {
            overflow += 1;
        }
    }

    let missing_required = params
        .iter()
        .enumerate()
        .filter(|(index, param)| {
            !param.variadic && !param.has_initializer && slots[*index].is_empty()
        })
        .count();

    ArgArrangement {
        slots,
        overflow,
        missing_required,
    }
}

fn arrange_constructor_args<'a>(
    params: &[FieldSig],
    args: &'a [crate::ast::CallArg],
) -> ArgArrangement<'a> {
    let mut slots = vec![Vec::new(); params.len()];
    let mut positional_index = 0usize;
    let mut overflow = 0usize;
    for arg in args {
        if let Some(name) = &arg.name {
            if let Some(index) = params.iter().position(|param| param.name == *name) {
                if slots[index].is_empty() {
                    slots[index].push(arg);
                } else {
                    overflow += 1;
                }
            } else {
                overflow += 1;
            }
            continue;
        }

        while positional_index < params.len()
            && !params[positional_index].variadic
            && !slots[positional_index].is_empty()
        {
            positional_index += 1;
        }
        if params.last().is_some_and(|param| param.variadic)
            && positional_index >= params.len().saturating_sub(1)
        {
            if let Some(slot) = slots.last_mut() {
                if slot.first().is_some_and(|arg| arg.name.is_some()) {
                    overflow += 1;
                } else {
                    slot.push(arg);
                }
            } else {
                overflow += 1;
            }
        } else if positional_index < params.len() {
            slots[positional_index].push(arg);
            if !params[positional_index].variadic {
                positional_index += 1;
            }
        } else {
            overflow += 1;
        }
    }

    let missing_required = params
        .iter()
        .enumerate()
        .filter(|(index, param)| {
            !param.variadic && !param.has_initializer && slots[*index].is_empty()
        })
        .count();

    ArgArrangement {
        slots,
        overflow,
        missing_required,
    }
}

fn call_uses_structural_record_arg(args: &[crate::ast::CallArg], uses_brace_syntax: bool) -> bool {
    uses_brace_syntax && has_single_record_literal_arg(args)
}

fn has_single_record_literal_arg(args: &[crate::ast::CallArg]) -> bool {
    matches!(
        args,
        [crate::ast::CallArg {
            name: None,
            value: Expr::RecordLiteral { .. },
            ..
        }]
    )
}

fn brace_record_constructor_args(args: &[crate::ast::CallArg]) -> Option<Vec<crate::ast::CallArg>> {
    let [
        crate::ast::CallArg {
            name: None,
            value: Expr::RecordLiteral { fields, values, .. },
            ..
        },
    ] = args
    else {
        return None;
    };

    if values.is_empty() && fields.iter().all(|field| field.name.is_some()) {
        return Some(fields.clone());
    }

    None
}

fn trailing_brace_call_has_lambda_arg(args: &[crate::ast::CallArg]) -> bool {
    let [
        crate::ast::CallArg {
            name: None, value, ..
        },
    ] = args
    else {
        return false;
    };

    match value {
        Expr::Lambda { .. } => true,
        Expr::Block { body, .. } => matches!(
            body.statements.as_slice(),
            [Stmt::Expr(crate::ast::ExprStmt {
                expr: Expr::Lambda { .. },
                ..
            })]
        ),
        _ => false,
    }
}

fn constructor_uses_parenthesized_record_arg(
    checker: &Checker<'_>,
    args: &[crate::ast::CallArg],
    uses_brace_syntax: bool,
) -> bool {
    !uses_brace_syntax
        && matches!(
            args,
            [crate::ast::CallArg { name: None, value, .. }]
                if matches!(checker.probe_expr_type(value), Ty::Record(_))
        )
}

fn format_factory_help_args(args: &[crate::ast::CallArg]) -> Option<String> {
    if let [
        crate::ast::CallArg {
            name: None,
            value: Expr::RecordLiteral { fields, values, .. },
            ..
        },
    ] = args
    {
        if !fields.is_empty() {
            return fields
                .iter()
                .map(|field| {
                    Some(format!(
                        "{} = {}",
                        field.name.as_ref()?,
                        format_help_expr(&field.value)
                    ))
                })
                .collect::<Option<Vec<_>>>()
                .map(|items| items.join(", "));
        }
        return Some(
            values
                .iter()
                .map(format_help_expr)
                .collect::<Vec<_>>()
                .join(", "),
        );
    }

    args.iter()
        .map(|arg| {
            Some(match &arg.name {
                Some(name) => format!("{name} = {}", format_help_expr(&arg.value)),
                None => format_help_expr(&arg.value),
            })
        })
        .collect::<Option<Vec<_>>>()
        .map(|items| items.join(", "))
}

fn format_help_expr(expr: &Expr) -> String {
    match expr {
        Expr::Identifier { name, .. } => name.clone(),
        Expr::Integer { raw, .. } | Expr::Float { raw, .. } => raw.clone(),
        Expr::String { raw, .. } if raw.starts_with('"') || raw.starts_with("raw\"") => raw.clone(),
        Expr::String { raw, .. } => format!("{raw:?}"),
        Expr::Bool { value, .. } => value.to_string(),
        Expr::Unit { .. } => "()".to_string(),
        _ => "...".to_string(),
    }
}

fn path_starts_with_os(expr: &Expr) -> bool {
    match expr {
        Expr::Identifier { name, .. } => name == "OS",
        Expr::Member { receiver, .. } => path_starts_with_os(receiver),
        Expr::Group { inner, .. } => path_starts_with_os(inner),
        _ => false,
    }
}

fn expr_path_for_known_value(expr: &Expr) -> Option<Vec<String>> {
    match expr {
        Expr::Identifier { name, .. } => Some(vec![name.clone()]),
        Expr::Member { receiver, name, .. } => {
            let mut path = expr_path_for_known_value(receiver)?;
            path.push(name.clone());
            Some(path)
        }
        Expr::Group { inner, .. } => expr_path_for_known_value(inner),
        _ => None,
    }
}

fn infer_type_subst(expected: &Ty, actual: &Ty, subst: &mut HashMap<String, Ty>) {
    match expected {
        Ty::Wildcard => {}
        Ty::Capture(_) => {}
        Ty::TypeParam(name) => {
            let actual = materialize_type(actual);
            if matches!(actual, Ty::Unknown) {
                return;
            }
            subst
                .entry(name.clone())
                .and_modify(|existing| *existing = join_types(existing, &actual))
                .or_insert(actual);
        }
        Ty::Named(expected_name, expected_args) => {
            if let Ty::Named(actual_name, actual_args) = actual {
                if expected_name == actual_name && expected_args.len() == actual_args.len() {
                    for (expected_arg, actual_arg) in expected_args.iter().zip(actual_args.iter()) {
                        infer_type_subst(expected_arg, actual_arg, subst);
                    }
                }
            }
        }
        Ty::Tuple(expected_items) => {
            if let Ty::Tuple(actual_items) = actual {
                for (expected_item, actual_item) in expected_items.iter().zip(actual_items.iter()) {
                    infer_type_subst(expected_item, actual_item, subst);
                }
            }
        }
        Ty::Record(expected_fields) => {
            if let Ty::Record(actual_fields) = actual {
                for (expected_name, expected_ty) in expected_fields {
                    if let Some((_, actual_ty)) = actual_fields
                        .iter()
                        .find(|(actual_name, _)| actual_name == expected_name)
                    {
                        infer_type_subst(expected_ty, actual_ty, subst);
                    }
                }
            }
        }
        Ty::Function(expected_params, expected_ret) => {
            if let Ty::Function(actual_params, actual_ret) = actual {
                if expected_params.len() == actual_params.len() {
                    for (expected_param, actual_param) in
                        expected_params.iter().zip(actual_params.iter())
                    {
                        infer_type_subst(expected_param, actual_param, subst);
                    }
                }
                infer_type_subst(expected_ret, actual_ret, subst);
            }
        }
        Ty::Never => {}
        Ty::Unknown => {}
    }
}

fn substitute_type(ty: &Ty, subst: &HashMap<String, Ty>) -> Ty {
    match ty {
        Ty::Wildcard => Ty::Wildcard,
        Ty::Capture(id) => Ty::Capture(*id),
        Ty::TypeParam(name) => subst.get(name).cloned().unwrap_or_else(|| ty.clone()),
        Ty::Never => Ty::Never,
        Ty::Named(name, args) => Ty::Named(
            name.clone(),
            args.iter().map(|arg| substitute_type(arg, subst)).collect(),
        ),
        Ty::Tuple(items) => Ty::Tuple(
            items
                .iter()
                .map(|item| substitute_type(item, subst))
                .collect(),
        ),
        Ty::Record(fields) => Ty::Record(
            fields
                .iter()
                .map(|(name, ty)| (name.clone(), substitute_type(ty, subst)))
                .collect(),
        ),
        Ty::Function(params, ret) => Ty::Function(
            params
                .iter()
                .map(|param| substitute_type(param, subst))
                .collect(),
            Box::new(substitute_type(ret, subst)),
        ),
        Ty::Unknown => Ty::Unknown,
    }
}

fn substitute_generic_condition(
    condition: &GenericConditionSig,
    subst: &HashMap<String, Ty>,
) -> GenericConditionSig {
    match condition {
        GenericConditionSig::Bound { subject, bound } => GenericConditionSig::Bound {
            subject: substitute_type(subject, subst),
            bound: substitute_type(bound, subst),
        },
        GenericConditionSig::Equal { left, right } => GenericConditionSig::Equal {
            left: substitute_type(left, subst),
            right: substitute_type(right, subst),
        },
    }
}

fn generic_condition_contains_wildcard(condition: &GenericConditionSig) -> bool {
    match condition {
        GenericConditionSig::Bound { subject, bound } => {
            type_contains_wildcard(subject) || type_contains_wildcard(bound)
        }
        GenericConditionSig::Equal { left, right } => {
            type_contains_wildcard(left) || type_contains_wildcard(right)
        }
    }
}

fn type_contains_wildcard(ty: &Ty) -> bool {
    match ty {
        Ty::Wildcard | Ty::Capture(_) => true,
        Ty::Named(_, args) | Ty::Tuple(args) => args.iter().any(type_contains_wildcard),
        Ty::Record(fields) => fields.iter().any(|(_, ty)| type_contains_wildcard(ty)),
        Ty::Function(params, ret) => {
            params.iter().any(type_contains_wildcard) || type_contains_wildcard(ret)
        }
        Ty::TypeParam(_) | Ty::Never | Ty::Unknown => false,
    }
}

fn instantiate_function_sig(method: FunctionSig, subst: &HashMap<String, Ty>) -> FunctionSig {
    FunctionSig {
        type_params: method.type_params,
        reified_type_params: method.reified_type_params,
        generic_conditions: method
            .generic_conditions
            .into_iter()
            .map(|condition| substitute_generic_condition(&condition, subst))
            .collect(),
        params: method
            .params
            .into_iter()
            .map(|param| ParamSig {
                name: param.name,
                ty: substitute_type(&param.ty, subst),
                variadic: param.variadic,
                lazy: param.lazy,
                has_initializer: param.has_initializer,
            })
            .collect(),
        ret: substitute_type(&method.ret, subst),
        visibility: method.visibility,
        has_body: method.has_body,
    }
}

fn runtime_type_arg(ty: Ty) -> Ty {
    match ty {
        Ty::Unknown | Ty::Never => Ty::Wildcard,
        Ty::Capture(_) => Ty::Wildcard,
        other => other,
    }
}

fn runtime_value_type_arg(ty: Ty) -> Ty {
    match ty {
        Ty::Unknown | Ty::Never => Ty::Wildcard,
        Ty::Capture(_) => Ty::Wildcard,
        ty if ty.is_any() => Ty::Wildcard,
        other => other,
    }
}

fn type_contains_type_param(ty: &Ty) -> bool {
    match ty {
        Ty::Wildcard => false,
        Ty::Capture(_) => false,
        Ty::TypeParam(_) => true,
        Ty::Named(_, args) | Ty::Tuple(args) => args.iter().any(type_contains_type_param),
        Ty::Record(fields) => fields.iter().any(|(_, ty)| type_contains_type_param(ty)),
        Ty::Function(params, ret) => {
            params.iter().any(type_contains_type_param) || type_contains_type_param(ret)
        }
        Ty::Never | Ty::Unknown => false,
    }
}

fn materialize_type(ty: &Ty) -> Ty {
    match ty {
        Ty::Wildcard => Ty::Wildcard,
        Ty::Capture(id) => Ty::Capture(*id),
        Ty::TypeParam(_) => Ty::Unknown,
        Ty::Never => Ty::Never,
        Ty::Named(name, args) => {
            Ty::Named(name.clone(), args.iter().map(materialize_type).collect())
        }
        Ty::Tuple(items) => Ty::Tuple(items.iter().map(materialize_type).collect()),
        Ty::Record(fields) => Ty::Record(
            fields
                .iter()
                .map(|(name, ty)| (name.clone(), materialize_type(ty)))
                .collect(),
        ),
        Ty::Function(params, ret) => Ty::Function(
            params.iter().map(materialize_type).collect(),
            Box::new(materialize_type(ret)),
        ),
        Ty::Unknown => Ty::Unknown,
    }
}

fn function_sig_parts_for_probe(
    sig: FunctionSig,
    explicit_type_args: &[Ty],
) -> (Vec<ParamSig>, Ty) {
    let mut subst = HashMap::new();
    if explicit_type_args.len() == sig.type_params.len() {
        for (name, ty) in sig.type_params.iter().zip(explicit_type_args.iter()) {
            subst.insert(name.clone(), ty.clone());
        }
    }
    let params = sig
        .params
        .into_iter()
        .map(|mut param| {
            param.ty = substitute_type(&param.ty, &subst);
            param
        })
        .collect();
    let ret = substitute_type(&sig.ret, &subst);
    (params, ret)
}

fn type_arg_refs_from_expr(expr: &Expr) -> Option<Vec<TypeRef>> {
    match expr {
        Expr::TupleLiteral { items, .. } => items.iter().map(type_ref_from_expr).collect(),
        Expr::Group { inner, .. } => type_arg_refs_from_expr(inner),
        _ => Some(vec![type_ref_from_expr(expr)?]),
    }
}

fn type_ref_from_expr(expr: &Expr) -> Option<TypeRef> {
    match expr {
        Expr::Identifier { name, span } => Some(TypeRef::Named {
            name: name.clone(),
            args: Vec::new(),
            span: *span,
        }),
        Expr::Index {
            receiver,
            index,
            span,
        } => {
            let TypeRef::Named { name, .. } = type_ref_from_expr(receiver)? else {
                return None;
            };
            Some(TypeRef::Named {
                name,
                args: type_arg_refs_from_expr(index)?,
                span: *span,
            })
        }
        Expr::Group { inner, .. } => type_ref_from_expr(inner),
        _ => None,
    }
}

fn callable_name_for_diagnostic(expr: &Expr) -> String {
    match expr {
        Expr::Identifier { name, .. } => name.clone(),
        Expr::Member { name, .. } => name.clone(),
        Expr::Index { receiver, .. } => callable_name_for_diagnostic(receiver),
        Expr::Group { inner, .. } => callable_name_for_diagnostic(inner),
        _ => "callee".to_string(),
    }
}

fn is_assignable(actual: &Ty, expected: &Ty) -> bool {
    if matches!(expected, Ty::Wildcard) {
        return true;
    }
    if matches!(actual, Ty::Unknown) || matches!(expected, Ty::Unknown) {
        return true;
    }
    if expected.is_any() {
        return true;
    }
    if matches!(actual, Ty::Never) {
        return true;
    }
    if actual == expected {
        return true;
    }
    match (actual, expected) {
        (Ty::Never, Ty::Never) => true,
        (Ty::Wildcard, Ty::Wildcard) => true,
        (Ty::Capture(left), Ty::Capture(right)) => left == right,
        (Ty::TypeParam(left), Ty::TypeParam(right)) => left == right,
        (Ty::Named(left, left_args), Ty::Named(right, right_args)) => {
            left == right
                && left_args.len() == right_args.len()
                && type_args_assignable(left, left_args, right_args)
        }
        (Ty::Tuple(left), Ty::Tuple(right)) => {
            left.len() == right.len()
                && left
                    .iter()
                    .zip(right.iter())
                    .all(|(left, right)| is_assignable(left, right))
        }
        (Ty::Record(left), Ty::Record(right)) => right.iter().all(|(name, right_ty)| {
            left.iter()
                .find(|(left_name, _)| left_name == name)
                .is_some_and(|(_, left_ty)| is_assignable(left_ty, right_ty))
        }),
        (Ty::Function(left_params, left_ret), Ty::Function(right_params, right_ret)) => {
            left_params.len() == right_params.len()
                && left_params
                    .iter()
                    .zip(right_params.iter())
                    .all(|(left, right)| is_assignable(left, right))
                && is_assignable(left_ret, right_ret)
        }
        _ => false,
    }
}

fn capture_id(ty: &Ty) -> Option<usize> {
    match ty {
        Ty::Capture(id) => Some(*id),
        _ => None,
    }
}

fn wildcard_capture_labels(type_name: &str, args: &[Ty]) -> Vec<Option<String>> {
    let rendered_args = args.iter().map(Ty::describe).collect::<Vec<_>>();
    let rendered = |index: usize| -> String {
        let mut parts = rendered_args.clone();
        if let Some(slot) = parts.get_mut(index) {
            *slot = "_".to_string();
        }
        format!("{type_name}[{}]", parts.join(", "))
    };

    args.iter()
        .enumerate()
        .map(|(index, arg)| {
            if !matches!(arg, Ty::Wildcard) {
                return None;
            }
            match (type_name, index, args.len()) {
                ("Vector" | "LinkedList" | "Array" | "Set", 0, 1) => {
                    Some(format!("captured element type of {}", rendered(index)))
                }
                ("Map", 0, 2) => Some(format!("captured key type of {}", rendered(index))),
                ("Map", 1, 2) => Some(format!("captured value type of {}", rendered(index))),
                _ => Some(format!(
                    "captured type argument {} of {}",
                    index + 1,
                    rendered(index)
                )),
            }
        })
        .collect()
}

fn type_args_assignable(type_name: &str, actual_args: &[Ty], expected_args: &[Ty]) -> bool {
    if is_reflection_metadata_type(type_name) {
        return actual_args
            .iter()
            .zip(expected_args.iter())
            .all(|(actual, expected)| matches!(expected, Ty::Wildcard) || actual == expected);
    }

    actual_args
        .iter()
        .zip(expected_args.iter())
        .all(|(actual, expected)| is_assignable(actual, expected))
}

fn upsert_shape_field(fields: &mut Vec<(String, Ty)>, name: String, ty: Ty) {
    if let Some((_, existing_ty)) = fields.iter_mut().find(|(field, _)| field == &name) {
        *existing_ty = ty;
    } else {
        fields.push((name, ty));
    }
}

fn is_reflection_metadata_type(name: &str) -> bool {
    matches!(
        name,
        "Type"
            | "ClassType"
            | "ShapeType"
            | "EnumType"
            | "InterfaceType"
            | "ObjectType"
            | "AnnotationType"
    )
}

fn join_types(left: &Ty, right: &Ty) -> Ty {
    if matches!(left, Ty::Unknown) {
        return right.clone();
    }
    if matches!(right, Ty::Unknown) {
        return left.clone();
    }
    if is_assignable(left, right) {
        return right.clone();
    }
    if is_assignable(right, left) {
        return left.clone();
    }
    Ty::Unknown
}

fn join_many_types(items: &[Ty]) -> Ty {
    let mut out = Ty::Unknown;
    for item in items {
        out = join_types(&out, item);
    }
    out
}

fn runtime_type_ref_has_arguments(reference: &TypeRef) -> bool {
    match reference {
        TypeRef::Named { args, .. } => !args.is_empty(),
        TypeRef::Tuple { fields, .. } => fields
            .iter()
            .any(|field| runtime_type_ref_has_arguments(&field.ty)),
        TypeRef::Record { fields, .. } => fields
            .iter()
            .any(|field| runtime_type_ref_has_arguments(&field.ty)),
        TypeRef::Function { params, ret, .. } => {
            params.iter().any(runtime_type_ref_has_arguments) || runtime_type_ref_has_arguments(ret)
        }
        TypeRef::Wildcard { .. } => false,
    }
}

fn module_alias_and_member(expr: &Expr) -> Option<(String, String)> {
    match expr {
        Expr::Member { receiver, name, .. } => {
            let Expr::Identifier { name: alias, .. } = receiver.as_ref() else {
                return None;
            };
            Some((alias.clone(), name.clone()))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{SourceFile, lex, parse_program};

    fn parse_inline(src: &str) -> Program {
        let file = SourceFile::new("test.lum", src);
        let lexed = lex(&file);
        assert!(lexed.diagnostics.is_empty(), "{:#?}", lexed.diagnostics);
        let parsed = parse_program(&lexed.tokens);
        assert!(parsed.diagnostics.is_empty(), "{:#?}", parsed.diagnostics);
        parsed.program.expect("program")
    }

    fn workspace_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../..")
            .canonicalize()
            .expect("workspace root")
    }

    #[test]
    fn checks_simple_function_program() {
        let program = parse_inline(
            r#"
def add(left Int, right Int) Int = left + right

def main() Int {
    value Int = add(1, 2)
    return value
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn allows_identity_comparison_for_reference_types() {
        let program = parse_inline(
            r#"
class Box {
    value Int
}

object Shared {
}

def main() Unit {
    first = Box(1)
    alias = first
    same = first === alias
    different = first !== Box(1)

    values = [1, 2]
    valuesAlias = values
    sameVector = values === valuesAlias
    sameObject = Shared === Shared

    anonymous = object { label Str = "value" }
    anonymousAlias = anonymous
    sameAnonymous = anonymous === anonymousAlias
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn rejects_identity_comparison_for_value_and_interface_types() {
        let program = parse_inline(
            r#"
shape Point {
    x Int
}

interface Named {
    def name() Str
}

class Person with Named {
    label Str
    def name() Str = this.label
}

def main() Unit {
    intIdentity = 1 === 1
    shapeIdentity = Point { x: 1 } === Point { x: 1 }
    option Option[Person] = Some(Person("Ada"))
    optionIdentity = option === option
    left Named = Person("Ada")
    right Named = left
    interfaceIdentity = left === right
}
"#,
        );
        let result = check_program(&program);
        assert_eq!(
            result
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.code == "invalid_identity_operand")
                .count(),
            4,
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn rejects_identity_comparison_between_unrelated_classes() {
        let program = parse_inline(
            r#"
class FirstBox {
}

class SecondBox {
}

def main() Unit {
    same = FirstBox {} === SecondBox {}
}
"#,
        );
        let result = check_program(&program);
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "incompatible_identity_operands"),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn allows_structural_equality_between_distinct_matching_shapes() {
        let program = parse_inline(
            r#"
shape Point {
    x Int
    y Str
}

shape ReorderedPoint {
    y Str
    x Int
}

def main() Unit {
    left = Point(1, "one")
    right = ReorderedPoint("one", 1)
    same = left == right
    different = left != right
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn rejects_shape_equality_with_different_fields_or_types() {
        let program = parse_inline(
            r#"
shape Point {
    x Int
    y Str
}

shape MissingField {
    x Int
}

shape DifferentType {
    x Int
    y Int
}

def main() Unit {
    missing = Point(1, "one") == MissingField(1)
    wrongType = Point(1, "one") == DifferentType(1, 1)
    nonShape = Point(1, "one") == 1
}
"#,
        );
        let result = check_program(&program);
        assert_eq!(
            result
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.code == "incompatible_shape_equality")
                .count(),
            3,
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn rejects_any_and_uncontracted_class_equality() {
        let program = parse_inline(
            r#"
class Account {
    id Int
}

def main() Unit {
    unknown Any = 1
    dynamic = unknown == 1
    classValue = Account(1) == Account(1)
    explicitDynamic = unknown.sameValue(1)
}
"#,
        );
        let result = check_program(&program);
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "dynamic_equality_requires_same_value"),
            "{:#?}",
            result.diagnostics
        );
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "missing_equality_contract"),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn accepts_explicit_and_implicit_widening_to_any() {
        let program = parse_inline(
            r#"
shape Point {
    x Int
    y Int
}

def consume(value Any) Unit = ()

def main() Unit {
    number Any = Any(42)
    text Any = Any("hello")
    point Any = Any(Point(1, 2))
    again Any = Any(point)
    implicit Any = "hello"
    consume("implicit")
    consume(Any("explicit"))
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn rejects_invalid_explicit_any_widening_forms() {
        let program = parse_inline(
            r#"
def main() Unit {
    missing = Any()
    multiple = Any(1, 2)
    braced = Any { value: 1 }
}
"#,
        );
        let result = check_program(&program);
        assert!(
            result.diagnostics.iter().any(|diagnostic| {
                diagnostic.code == "invalid_any_widening_arity"
                    && diagnostic.message.contains("got 0")
            }),
            "{:#?}",
            result.diagnostics
        );
        assert!(
            result.diagnostics.iter().any(|diagnostic| {
                diagnostic.code == "invalid_any_widening_arity"
                    && diagnostic.message.contains("got 2")
            }),
            "{:#?}",
            result.diagnostics
        );
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "invalid_any_widening_syntax"),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn accepts_class_equality_with_explicit_eq_contract() {
        let program = parse_inline(
            r#"
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

def main() Unit {
    same = Account(1) == Account(1)
    different = Account(1) != Account(2)
    left Identified = Entry(1)
    right Identified = AlternateEntry(1)
    interfaceEqual = left == right
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn derives_eq_and_hashed_bounds_for_eligible_shapes() {
        let program = parse_inline(
            r#"
enum State {
    case Ready
}

object Marker {
}

class StableReference with Hashed[StableReference] {
    id Int

    def equals(other StableReference) Bool = this.id == other.id
    def hash() Int = this.id
}

shape Coordinate {
    x Int
    y Int
}

shape CacheKey {
    coordinate Coordinate
    state State
    marker Marker
    reference StableReference
}

def requireEq[T with Eq[T]](value T) Unit = ()
def requireHash[T with Hashed[T]](value T) Unit = ()

def main() Unit {
    key = CacheKey(Coordinate(1, 2), State.Ready, Marker, StableReference(3))
    requireEq(key)
    requireHash(key)
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn rejects_hashed_bound_for_shape_with_non_hashed_class_field() {
        let program = parse_inline(
            r#"
class MutableReference {
    id Int
}

shape Snapshot {
    reference MutableReference
}

def requireEq[T with Eq[T]](value T) Unit = ()
def requireHash[T with Hashed[T]](value T) Unit = ()

def main() Unit {
    snapshot = Snapshot(MutableReference(1))
    requireEq(snapshot)
    requireHash(snapshot)
}
"#,
        );
        let result = check_program(&program);
        assert!(
            result.diagnostics.iter().any(|diagnostic| diagnostic.code
                == "generic_bound_not_satisfied"
                && diagnostic.message.contains("Hashed")),
            "{:#?}",
            result.diagnostics
        );
        assert_eq!(
            result
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.code == "generic_bound_not_satisfied")
                .count(),
            1,
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn rejects_unhashable_map_keys() {
        let program = parse_inline(
            r#"
class MutableKey {
    id Int
}

def main() Unit {
    values = [MutableKey(1): "one"]
}
"#,
        );
        let result = check_program(&program);
        assert!(
            result.diagnostics.iter().any(|diagnostic| {
                diagnostic.code == "map_key_not_hashable"
                    && diagnostic.message.contains("Hashed[MutableKey]")
            }),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn rejects_unhashable_explicit_map_key_types() {
        let program = parse_inline(
            r#"
class MutableKey {
    id Int
}

def main() Unit {
    values [MutableKey: Str] = []
}
"#,
        );
        let result = check_program(&program);
        assert!(
            result.diagnostics.iter().any(|diagnostic| {
                diagnostic.code == "generic_bound_not_satisfied"
                    && diagnostic.message.contains("Hashed[MutableKey]")
            }),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn hashed_classes_must_implement_equality_and_hashing() {
        let program = parse_inline(
            r#"
class IncompleteKey with Hashed[IncompleteKey] {
    id Int

    def equals(other IncompleteKey) Bool = this.id == other.id
}
"#,
        );
        let result = check_program(&program);
        assert!(
            result.diagnostics.iter().any(|diagnostic| {
                diagnostic.code == "missing_interface_member" && diagnostic.message.contains("hash")
            }),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn reports_argument_mismatch() {
        let program = parse_inline(
            r#"
def add(left Int, right Int) Int = left + right

def main() Int {
    return add(1)
}
"#,
        );
        let result = check_program(&program);
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diag| diag.code == "invalid_argument_count"),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn checks_import_forms_example() {
        let result =
            check_path(workspace_root().join("examples/import_forms.lum")).expect("typecheck");
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn checks_bumper_example() {
        let result = check_path(workspace_root().join("examples/random_code/bumper.lum"))
            .expect("typecheck");
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn checks_extension_methods_example() {
        let result =
            check_path(workspace_root().join("examples/extension_methods.lum")).expect("typecheck");
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn rejects_extension_methods_on_objects() {
        let program = parse_inline(
            r#"
object Tools {
}

ext Tools {
    def label() Str = "tools"
}
"#,
        );
        let result = check_program(&program);
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diag| diag.code == "invalid_extension_target"),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn rejects_extension_methods_on_annotations() {
        let program = parse_inline(
            r#"
annotation Route {
    path Str
}

ext Route {
    def label() Str = this.path
}
"#,
        );
        let result = check_program(&program);
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diag| diag.code == "invalid_extension_target"),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn rejects_extension_methods_reading_hidden_fields() {
        let program = parse_inline(
            r#"
class User {
    hidden token Str = "secret"
    name Str
}

ext User {
    def reveal() Str = this.token
}
"#,
        );
        let result = check_program(&program);
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diag| diag.code == "invalid_extension_access"),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn rejects_extension_methods_calling_hidden_methods() {
        let program = parse_inline(
            r#"
class User {
    name Str

    hidden def secret() Str = this.name
}

ext User {
    def reveal() Str = this.secret()
}
"#,
        );
        let result = check_program(&program);
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diag| diag.code == "invalid_extension_access"),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn allows_parenthesized_class_constructor_call() {
        let program = parse_inline(
            r#"
class User {
    name Str
}

def main() Unit {
    _ User = User("Ada")
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn allows_implicit_empty_brace_constructor_when_all_fields_initialized() {
        let program = parse_inline(
            r#"
class OrderManager {
    hidden map Map[Int, Str] = Map()
    hidden var currentTick Int = 0
    hidden queue [Str] = []
}

def main() Unit {
    _ OrderManager = OrderManager {}
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn rejects_implicit_constructors_when_hidden_field_lacks_initializer() {
        let program = parse_inline(
            r#"
class SecretUser {
    name Str
    hidden token Str
}

def main() Unit {
    _ SecretUser = SecretUser { name: "Ada" }
    _ SecretUser = SecretUser("Ada")
}
"#,
        );
        let result = check_program(&program);
        assert!(
            result.diagnostics.iter().any(|diag| {
                diag.message.contains(
                    "no implicit field constructor because hidden field 'token' has no initializer",
                )
            }),
            "{:#?}",
            result.diagnostics
        );
        assert!(
            result.diagnostics.iter().any(|diag| {
                diag.message.contains(
                    "no implicit positional constructor because hidden field 'token' has no initializer",
                )
            }),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn allows_explicit_constructor_when_hidden_field_lacks_initializer() {
        let program = parse_inline(
            r#"
class SecretUser {
    name Str
    hidden token Str

    new(name Str) {
        this.name = name
        this.token = "secret"
    }
}


def main() Unit {
    _ SecretUser = SecretUser { name: "Ada" }
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn rejects_explicit_constructor_that_leaves_required_field_uninitialized() {
        let program = parse_inline(
            r#"
class SecretUser {
    name Str
    hidden token Str

    new(name Str) {
        this.name = name
    }
}

"#,
        );
        let result = check_program(&program);
        assert!(
            result.diagnostics.iter().any(|diag| {
                diag.code == "uninitialized_field"
                    && diag.message.contains("must initialize field 'token'")
            }),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn allows_positional_class_construction_with_trailing_defaults() {
        let program = parse_inline(
            r#"
class User {
    name Str
    age Int
    city Str = "NYC"
    hidden score Int = 5

    def scoreValue() Int = this.score
}


def main() Int {
    user User = User("Ada", 10)
    return user.scoreValue()
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn allows_constructor_shapes_with_default_before_required_for_named_or_full_positional_calls() {
        let program = parse_inline(
            r#"
class Page {
    body Str = "body"
    title Str
}

class Article {
    body Str
    title Str

    new(body Str = "body", title Str) {
        this.body = body
        this.title = title
    }
}


def main() Unit {
    _ Page = Page("custom body", "Intro")
    _ Page = Page { title: "Intro" }
    _ Article = Article("custom body", "Intro")
    _ Article = Article { title: "Intro" }
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn rejects_short_positional_construction_that_skips_default_before_required_field() {
        let program = parse_inline(
            r#"
class Page {
    body Str = "body"
    title Str
}

class Article {
    body Str
    title Str

    new(body Str = "body", title Str) {
        this.body = body
        this.title = title
    }
}


def main() Unit {
    _ Page = Page("Intro")
    _ Article = Article("Intro")
}
"#,
        );
        let result = check_program(&program);
        assert_eq!(result.diagnostics.len(), 2, "{:#?}", result.diagnostics);
        assert!(
            result.diagnostics.iter().all(|diag| {
                diag.code == "invalid_argument_count"
                    && diag.message.contains("leaves required field 'title' unset")
                    && diag.message.contains("do not skip defaulted fields")
                    && diag.message.contains("{ title: ... }")
            }),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn reports_named_brace_field_type_mismatch() {
        let program = parse_inline(
            r#"
class Order {
    quantity Int
}

def main() Unit {
    _ Order = Order { quantity: "oops" }
}
"#,
        );
        let result = check_program(&program);
        assert!(
            result.diagnostics.iter().any(|diag| {
                diag.code == "invalid_argument_type"
                    && diag
                        .message
                        .contains("field 'quantity' in class 'Order' expects 'Int'")
                    && diag.message.contains("got 'Str'")
            }),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn allows_parenthesized_enum_constructor_with_brace_class_payload() {
        let program = parse_inline(
            r#"
class Order {
    quantity Int
}

def main() Option[Order] {
    Some(
        Order {
            quantity: 7
        }
    )
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn allows_named_brace_enum_constructor_payload() {
        let program = parse_inline(
            r#"
class Order {
    quantity Int
}

def main() Option[Order] {
    Some {
        value: Order {
            quantity: 7
        }
    }
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn rejects_trailing_brace_call_for_non_lambda_argument() {
        let program = parse_inline(
            r#"
def wrap(value Int) Int = value

def main() Int {
    wrap { value: 7 }
}
"#,
        );
        let result = check_program(&program);
        assert!(
            result.diagnostics.iter().any(|diag| {
                diag.code == "invalid_trailing_brace_call"
                    && diag.message.contains("only accepts lambda arguments")
            }),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn allows_trailing_brace_call_for_lambda_argument() {
        let program = parse_inline(
            r#"
def main() Unit {
    items = Vector(1, 2, 3)
    mapped = items.map { item => item + 1 }
    OS.println(mapped.size())
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn allows_trailing_block_call_for_explicit_zero_arg_lambda_argument() {
        let program = parse_inline(
            r#"
def process(f fn() Unit) Unit = f()
def compute(f fn() Int) Int = f()

def main() Unit {
    process { () => println("hehe") }

    value Int = compute { () => 42 }
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn infers_generic_result_type_from_trailing_lambda_body() {
        let program = parse_inline(
            r#"
def transactionally[T](work fn() Result[T, Str]) Result[T, Str] = work()

def run() Result[Unit, Str] {
    try transactionally { () => Ok(()) }
    Ok(())
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn allows_bare_method_calls_inside_declaration_bodies() {
        let program = parse_inline(
            r#"
class Counter {
    value Int

    def add(delta Int) Int = this.value + delta
    def twice(delta Int) Int = add(delta) + add(delta)
}


def main() Int {
    counter Counter = Counter(5)
    return counter.twice(2)
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn checks_inline_constructors_receiver_calls_and_contextual_map_assignments() {
        let program = parse_inline(
            r#"
class Cache {
    hidden var values [Str : Int] = []

    new() {}

    def currentValue() Int = 7

    def store(key Str) Unit {
        values[key] := currentValue()
    }

    def reset() Unit {
        this.values := []
    }
}

def main() Unit {
    cache = Cache()
    cache.store("answer")
    cache.reset()
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn allows_bare_field_access_inside_impls_when_not_shadowed() {
        let program = parse_inline(
            r#"
class Counter {
    var count Int

    new(initial Int) {
        this.count = initial
    }

    def value() Int = count

    def bump() Unit {
        count := count + 1
    }

    def shadow(count Int) Int = this.count + count
}


def main() Unit {
    counter = Counter(5)
    counter.bump()
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn allows_option_when_static_helper() {
        let program = parse_inline(
            r#"
def main() Option[Int] {
    return Option.when(true, 7)
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn allows_int_and_float_parse_static_helpers() {
        let program = parse_inline(
            r#"
def main() Unit {
    whole Option[Int] = Int.parse("7")
    decimal Option[Float] = Float.parse("1.2")
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn allows_array_primitive_static_helpers() {
        let program = parse_inline(
            r#"
def main() Unit {
    ints Array[Int] = Array.ofInt(3)
    floats Array[Float] = Array.ofFloat(3)
    bools Array[Bool] = Array.ofBool(3)
    texts Array[Str] = Array.ofStr(3)
    runes Array[Rune] = Array.ofRune(3)
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn allows_array_fill_static_helper() {
        let program = parse_inline(
            r#"
def main() Unit {
    values Array[Int] = Array.fill(3, 7)
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn allows_array_generate_static_helper() {
        let program = parse_inline(
            r#"
def main() Unit {
    values Array[Int] = Array.generate(3, idx => idx + 1)
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn allows_assert_runtime_calls() {
        let program = parse_inline(
            r#"
def main() Unit {
    assert(1 + 2 == 3)
    assert(true, "still true")
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn allows_annotation_decl_with_default_field_values() {
        let program = parse_inline(
            r#"
annotation Route {
    path Str
    method Str = "GET"
}

@Route { path: "/health" }
def health() Str = "ok"
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn rejects_annotation_mutable_and_hidden_fields() {
        let program = parse_inline(
            r#"
annotation Route {
    hidden path Str
    var method Str = "GET"
}
"#,
        );
        let result = check_program(&program);
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diag| diag.code == "invalid_annotation_field"
                    && diag.message.contains("hidden field 'path'")),
            "{:#?}",
            result.diagnostics
        );
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diag| diag.code == "invalid_annotation_field"
                    && diag.message.contains("mutable field 'method'")),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn allows_single_statement_match_arms_without_braces() {
        let program = parse_inline(
            r#"
def main(flag Bool) Unit {
    var total Int = 0
    match flag {
        case true => total += 1
        case false => total += 2
    }
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn allows_list_remove_first_and_seeded_reduce() {
        let program = parse_inline(
            r#"
def main() Unit {
    values = Vector(1, 2, 3)
    removed Option[Int] = values.removeFirst()
    total Int = values.reduce(0, (acc, value) => acc + value)
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn rejects_non_bool_assert_condition() {
        let program = parse_inline(
            r#"
def main() Unit {
    assert(1)
}
"#,
        );
        let result = check_program(&program);
        assert!(
            result.diagnostics.iter().any(|diag| {
                diag.code == "invalid_argument_type"
                    && diag
                        .message
                        .contains("assert condition has type 'Int' but expects 'Bool'")
            }),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn allows_try_with_same_option_family_and_different_success_type() {
        let program = parse_inline(
            r#"
def main(source Option[Int]) Option[Str] {
    value = try source
    panic("done")
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn allows_try_with_same_result_family_and_different_success_type() {
        let program = parse_inline(
            r#"
def main(source Result[Int, Str]) Result[Str, Str] {
    value = try source
    panic("done")
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn rejects_try_with_mismatched_container_family() {
        let program = parse_inline(
            r#"
def main(source Result[Int, Str]) Option[Int] {
    value = try source
    panic("done")
}
"#,
        );
        let result = check_program(&program);
        assert!(
            result.diagnostics.iter().any(|diag| {
                diag.code == "invalid_try"
                    && diag
                        .message
                        .contains("cannot propagate from enclosing return type 'Option[Int]'")
            }),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn rejects_try_with_mismatched_result_error_type() {
        let program = parse_inline(
            r#"
def main(source Result[Int, Int]) Result[Str, Str] {
    value = try source
    panic("done")
}
"#,
        );
        let result = check_program(&program);
        assert!(
            result.diagnostics.iter().any(|diag| {
                diag.code == "invalid_try"
                    && diag
                        .message
                        .contains("cannot propagate from enclosing return type 'Result[Str, Str]'")
            }),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn rejects_try_with_mismatched_either_left_type() {
        let program = parse_inline(
            r#"
def main(source Either[Int, Int]) Either[Str, Str] {
    value = try source
    panic("done")
}
"#,
        );
        let result = check_program(&program);
        assert!(
            result.diagnostics.iter().any(|diag| {
                diag.code == "invalid_try"
                    && diag
                        .message
                        .contains("cannot propagate from enclosing return type 'Either[Str, Str]'")
            }),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn allows_brace_call_when_explicit_new_exists() {
        let program = parse_inline(
            r#"
class User {
    name Str

    new(name Str) {
        this.name = name
    }
}


def main() Unit {
    _ User = User("Ada")
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn allows_defaulted_constructor_parameters() {
        let program = parse_inline(
            r#"
class User {
    name Str
    age Int

    new(name Str, age Int = 0) {
        this.name = name
        this.age = age
    }
}


def main() Unit {
    _ User = User("Ada")
    _ User = User { age: 12, name: "Ben" }
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn allows_variadic_constructor_parameters() {
        let program = parse_inline(
            r#"
class Path {
    segments [Str]

    new(segments [Str] vararg) {
        this.segments = segments
    }
}


def main() Unit {
    _ Path = Path()
    _ Path = Path("usr", "local", "bin")
    _ Path = Path { segments: ["etc", "hosts"] }
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn allows_defaulted_variadic_constructor_parameters() {
        let program = parse_inline(
            r#"
class Path {
    segments [Str]

    new(segments [Str] vararg = ["tmp"]) {
        this.segments = segments
    }
}


def main() Unit {
    _ Path = Path()
    _ Path = Path("usr")
    _ Path = Path { segments: ["etc"] }
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn rejects_variadic_constructor_parameter_not_last() {
        let program = parse_inline(
            r#"
class Bad {
    items [Int]
    suffix Int

    new(items [Int] vararg, suffix Int) {
        this.items = items
        this.suffix = suffix
    }
}

"#,
        );
        let result = check_program(&program);
        assert!(
            result.diagnostics.iter().any(|diag| {
                diag.code == "invalid_variadic_param"
                    && diag
                        .message
                        .contains("variadic constructor parameter must be last")
            }),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn rejects_multiple_variadic_constructor_parameters() {
        let program = parse_inline(
            r#"
class Bad {
    left [Int]
    right [Int]

    new(left [Int] vararg, right [Int] vararg) {
        this.left = left
        this.right = right
    }
}

"#,
        );
        let result = check_program(&program);
        assert!(
            result.diagnostics.iter().any(|diag| {
                diag.code == "invalid_variadic_param"
                    && diag
                        .message
                        .contains("only one variadic constructor parameter is allowed")
            }),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn rejects_multiple_variadic_function_parameters() {
        let program = parse_inline(
            r#"
def bad(left [Int] vararg, right [Int] vararg) Unit = ()
"#,
        );
        let result = check_program(&program);
        assert!(
            result.diagnostics.iter().any(|diag| {
                diag.code == "invalid_variadic_param"
                    && diag
                        .message
                        .contains("only one variadic parameter is allowed")
            }),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn allows_variadic_constructor_parameter_after_default() {
        let program = parse_inline(
            r#"
class Bucket {
    items [Int]

    new(prefix Int = 0, items [Int] vararg) {
        this.items = items
    }
}


def main() Unit {
    _ Bucket = Bucket(1, 2, 3)
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn rejects_variadic_constructor_parameter_without_list_type() {
        let program = parse_inline(
            r#"
class Bad {
    items [Int]

    new(items Int vararg) {
        this.items = items
    }
}

"#,
        );
        let result = check_program(&program);
        assert!(
            result.diagnostics.iter().any(|diag| {
                diag.code == "invalid_variadic_param"
                    && diag
                        .message
                        .contains("variadic constructor parameter must use a vector type")
            }),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn allows_named_argument_for_variadic_constructor_parameter() {
        let program = parse_inline(
            r#"
class Path {
    segments [Str]

    new(segments [Str] vararg) {
        this.segments = segments
    }
}


def main() Unit {
    _ Path = Path { segments: ["usr"] }
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn reports_hidden_field_constructor_with_factory_help() {
        let program = parse_inline(
            r#"
class User {
    name Str

    hidden new(name Str) {
        this.name = name
    }
}

object User {
    def create(name Str) User = User { name: name }
}


def main() Unit {
    user = User { name: "Ada" }
}
"#,
        );
        let result = check_program(&program);
        let diagnostic = result
            .diagnostics
            .iter()
            .find(|diag| diag.code == "constructor_unavailable")
            .expect("constructor diagnostic");
        assert_eq!(diagnostic.message, "constructor is not available");
        assert_eq!(
            diagnostic.label.as_deref(),
            Some("field construction is hidden")
        );
        assert_eq!(
            diagnostic.notes,
            vec!["User declares a private primary constructor"]
        );
        assert_eq!(diagnostic.helps, vec!["use User.create(name = \"Ada\")"]);
    }

    #[test]
    fn allows_constructor_equals_for_field_initialization() {
        let program = parse_inline(
            r#"
class Counter {
    hidden var count Int = 0
    name Str = "unknown"

    new(count Int, name Str) {
        this.count = count
        this.name = name
    }
}


def main() Unit {
    _ Counter = Counter(1, "Ada")
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn rejects_reassign_operator_on_field_inside_constructor() {
        let program = parse_inline(
            r#"
class Counter {
    hidden var count Int = 0

    new(count Int) {
        this.count := count
    }
}

"#,
        );
        let result = check_program(&program);
        assert!(
            result.diagnostics.iter().any(|diag| {
                diag.code == "unexpected_token"
                    && diag
                        .message
                        .contains("use '=' for field initialization in constructors")
            }),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn rejects_parenthesized_anonymous_shape_type_construction() {
        let program = parse_inline(
            r#"
class User {
    name Str
}

def main() Unit {
    _ User = User({ name: "Ada" })
}
"#,
        );
        let result = check_program(&program);
        assert!(
            result.diagnostics.iter().any(|diag| diag
                .message
                .contains("does not accept anonymous shape arguments")),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn allows_typed_anonymous_shape_fields() {
        let program = parse_inline(
            r#"
def main() Unit {
    value = "Ada"
    user = {
        name Str: value
        age Int: 42
    }
    typed { name Str, age Int } = user
    OS.println(typed.name)
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn allows_anonymous_shape_spread_copy_and_extend() {
        let program = parse_inline(
            r#"
def main() Unit {
    base = { name: "Ada", age: 10 }
    aged = base with { age: 42 }
    updated = { ...aged, city: "Tampa" }
    name Str = updated.name
    age Int = updated.age
    city Str = updated.city
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn explicit_shape_field_resolves_spread_overlap() {
        let program = parse_inline(
            r#"
def main() Unit {
    point = { x: 1, y: 2 }
    dot = { x: 3, time: 4 }
    merged = {
        ...point
        ...dot
        x: point.x
    }
    x Int = merged.x
    y Int = merged.y
    time Int = merged.time
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn rejects_unresolved_shape_spread_overlap() {
        let program = parse_inline(
            r#"
def main() Unit {
    point = { x: 1, y: 2 }
    dot = { x: 3, time: 4 }
    merged = { ...point, ...dot }
}
"#,
        );
        let result = check_program(&program);
        assert!(
            result.diagnostics.iter().any(|diag| {
                diag.code == "ambiguous_shape_field"
                    && diag.message.contains("field 'x'")
                    && diag.message.contains("point")
                    && diag.message.contains("dot")
                    && diag.message.contains("override")
            }),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn override_shape_spread_resolves_earlier_overlap() {
        let program = parse_inline(
            r#"
def main() Unit {
    point = { x: 1, y: 2 }
    dot = { x: 3, time: 4 }
    merged = { ...point, override ...dot }
    x Int = merged.x
    y Int = merged.y
    time Int = merged.time
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn later_protected_spread_reopens_overlap_after_override() {
        let program = parse_inline(
            r#"
def main() Unit {
    first = { x: 1 }
    second = { x: 2 }
    third = { x: 3 }
    merged = { ...first, override ...second, ...third }
}
"#,
        );
        let result = check_program(&program);
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diag| diag.code == "ambiguous_shape_field"),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn rejects_non_shape_spread() {
        let program = parse_inline(
            r#"
def main() Unit {
    value = { ...10, age: 42 }
}
"#,
        );
        let result = check_program(&program);
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diag| diag.code == "invalid_shape_spread"),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn rejects_duplicate_explicit_shape_fields() {
        let program = parse_inline(
            r#"
def main() Unit {
    value = { age: 10, age: 42 }
}
"#,
        );
        let result = check_program(&program);
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diag| diag.code == "duplicate_shape_field"),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn rejects_typed_anonymous_shape_field_initializer_mismatch() {
        let program = parse_inline(
            r#"
def main() Unit {
    user = {
        name Str: 42
    }
}
"#,
        );
        let result = check_program(&program);
        assert!(
            result.diagnostics.iter().any(|diag| {
                diag.code == "invalid_field_initializer_type"
                    && diag.message.contains(
                        "field 'name' is annotated as 'Str' but initializer has type 'Int'",
                    )
            }),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn rejects_positional_construction_when_hidden_default_breaks_visible_order() {
        let program = parse_inline(
            r#"
class Broken {
    name Str
    hidden score Int = 5
    age Int
}

def main() Unit {
    _ Broken = Broken("Ada", 10)
}
"#,
        );
        let result = check_program(&program);
        assert!(
            result.diagnostics.iter().any(|diag| diag
                .message
                .contains("hidden defaulted fields must come after all visible fields")),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn rejects_field_initializer_calling_instance_method() {
        let program = parse_inline(
            r#"
class AssetPrices {
    assets [Str] = this.makeAssets()

    def makeAssets() [Str] = []
}

"#,
        );
        let result = check_program(&program);
        assert!(
            result.diagnostics.iter().any(|diag| {
                diag.code == "invalid_field_initializer"
                    && diag
                        .message
                        .contains("field initializer cannot call instance method 'makeAssets'")
            }),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn rejects_field_initializer_reading_later_field() {
        let program = parse_inline(
            r#"
class Pair {
    right Int = this.left
    left Int = 1
}
"#,
        );
        let result = check_program(&program);
        assert!(
            result.diagnostics.iter().any(|diag| {
                diag.code == "invalid_field_initializer"
                    && diag.message.contains(
                        "field initializer can read only fields declared earlier; 'left' is not available yet",
                    )
            }),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn rejects_class_to_tuple_destructuring() {
        let program = parse_inline(
            r#"
class Box {
    value Int
    label Str
}

def main() Unit {
    let (a, b) = Box(1, "x")
}
"#,
        );
        let result = check_program(&program);
        assert!(
            result.diagnostics.iter().any(|diag| {
                diag.code == "invalid_destructure"
                    && diag
                        .message
                        .contains("tuple destructuring requires a tuple value")
            }),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn allows_named_class_destructuring_by_field() {
        let program = parse_inline(
            r#"
class User {
    name Str
    location Str
    age Int
}

def main() Str {
    user User = User { name: "Sergey", location: "Tampa", age: 37 }
    let { location Str as loc, name } = user
    return name + " from " + loc
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn allows_plain_class_destructuring_by_field_name_in_any_order() {
        let program = parse_inline(
            r#"
class User {
    name Str
    location Str
}

def main() Str {
    user User = User { name: "Sergey", location: "Tampa" }
    let { location, name } = user
    return name + " from " + location
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn named_class_destructuring_skips_hidden_fields() {
        let program = parse_inline(
            r#"
class SecretUser {
    name Str
    hidden token Str
    location Str

    new(name Str, token Str, location Str) {
        this.name = name
        this.token = token
        this.location = location
    }
}


def main() Str {
    let { location, name } = SecretUser("Sergey", "secret", "Tampa")
    return name + " from " + location
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn rejects_unknown_named_destructure_field() {
        let program = parse_inline(
            r#"
class User {
    name Str
    location Str
}

def main() Unit {
    user User = User { name: "Sergey", location: "Tampa" }
    let { missing } = user
}
"#,
        );
        let result = check_program(&program);
        assert!(
            result.diagnostics.iter().any(|diag| {
                diag.code == "invalid_destructure"
                    && diag
                        .message
                        .contains("does not have a field named 'missing'")
            }),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn allows_partial_brace_destructuring_by_omission() {
        let program = parse_inline(
            r#"
class User {
    name Str
    location Str
}

def main() Str {
    user User = User { name: "Sergey", location: "Tampa" }
    let { location } = user
    return location
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn rejects_class_to_tuple_assignment() {
        let program = parse_inline(
            r#"
class Box {
    value Int
    label Str
}

def main() Unit {
    pair (Int, Str) = Box(1, "x")
}
"#,
        );
        let result = check_program(&program);
        assert!(
            result.diagnostics.iter().any(|diag| {
                diag.code == "invalid_binding_type"
                    && diag.message.contains(
                        "cannot assign value of type 'Box' to binding 'pair' of type '(Int, Str)'",
                    )
            }),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn allows_shape_positional_anonymous_shape_assignment() {
        let program = parse_inline(
            r#"
def main() Int {
    point { x Int, y Int } = shape(4, 5)
    point.x + point.y
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn allows_named_shape_positional_construction() {
        let program = parse_inline(
            r#"
shape Point {
    x Int
    y Int
}

def main() Int {
    point Point = Point(4, 5)
    point.x + point.y
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn rejects_tuple_to_anonymous_shape_assignment() {
        let program = parse_inline(
            r#"
def main() Unit {
    point { x Int, y Int } = (4, 5)
}
"#,
        );
        let result = check_program(&program);
        assert!(
            result.diagnostics.iter().any(|diag| {
                diag.code == "invalid_tuple_shape_conversion"
                    && diag
                        .message
                        .contains("tuple values cannot construct anonymous shape")
            }),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn rejects_shape_positional_named_shape_assignment() {
        let program = parse_inline(
            r#"
shape Point {
    x Int
    y Int
}

def main() Unit {
    point Point = shape(4, 5)
}
"#,
        );
        let result = check_program(&program);
        assert!(
            result.diagnostics.iter().any(|diag| {
                diag.code == "missing_shape_context" && diag.message.contains("use 'Point(...)'")
            }),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn rejects_shape_positional_without_expected_anonymous_shape() {
        let program = parse_inline(
            r#"
def main() Unit {
    point = shape(4, 5)
}
"#,
        );
        let result = check_program(&program);
        assert!(
            result.diagnostics.iter().any(|diag| {
                diag.code == "missing_shape_context"
                    && diag
                        .message
                        .contains("requires an expected anonymous shape type")
            }),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn allows_anonymous_shape_as_named_shape_enum_payload() {
        let program = parse_inline(
            r#"
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

def route() Result[HttpResponse, HttpError] {
    Err({ status: 400, body: "bad request" })
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn allows_class_to_named_shape_assignment() {
        let program = parse_inline(
            r#"
class Pixel {
    x Int
    y Int
}

shape Point {
    x Int
    y Int
}

def main() Int {
    pixel Pixel = Pixel(4, 5)
    point Point = pixel
    point.x + point.y
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn allows_shape_with_interface_after_explicit_shape_view() {
        let program = parse_inline(
            r#"
interface Named {
    def label() Str
}

shape PointView with Named {
    x Int
    y Int

    def label() Str = this.x + "," + this.y
}


class Pixel {
    x Int
    y Int
}

def main() Unit {
    pixel Pixel = Pixel(4, 5)
    view PointView = pixel
    named Named = view
    _ Str = named.label()
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn rejects_class_to_interface_through_shape_without_explicit_shape_view() {
        let program = parse_inline(
            r#"
interface Named {
    def label() Str
}

shape PointView with Named {
    x Int
    y Int

    def label() Str = this.x + "," + this.y
}


class Pixel {
    x Int
    y Int
}

def main() Unit {
    pixel Pixel = Pixel(4, 5)
    named Named = pixel
}
"#,
        );
        let result = check_program(&program);
        assert!(
            result.diagnostics.iter().any(|diag| {
                diag.code == "invalid_binding_type"
                    && diag.message.contains("cannot assign value of type 'Pixel'")
            }),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn rejects_tuple_to_class_assignment() {
        let program = parse_inline(
            r#"
class User {
    name Str
    age Int
}

def main() Unit {
    user User = ("Ada", 42)
}
"#,
        );
        let result = check_program(&program);
        assert!(
            result.diagnostics.iter().any(|diag| {
                diag.code == "invalid_tuple_shape_conversion"
                    && diag
                        .message
                        .contains("tuple values cannot construct class 'User'")
            }),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn rejects_shape_positional_with_wrong_field_type() {
        let program = parse_inline(
            r#"
def main() Unit {
    point { x Int, y Str } = shape(4, 5)
}
"#,
        );
        let result = check_program(&program);
        assert!(
            result.diagnostics.iter().any(|diag| {
                diag.code == "invalid_argument_type"
                    && diag
                        .message
                        .contains("shape field 'y' expects 'Str' but got 'Int'")
            }),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn rejects_tuple_pattern_against_class_value() {
        let program = parse_inline(
            r#"
class Box {
    value Int
    label Str
}

def main() Unit {
    box Box = Box(1, "x")
    match box {
        case (a, b) => OS.println(a, b)
    }
}
"#,
        );
        let result = check_program(&program);
        assert!(
            result.diagnostics.iter().any(|diag| {
                diag.code == "invalid_destructure"
                    && diag
                        .message
                        .contains("tuple pattern requires a tuple value")
            }),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn rejects_irrefutable_if_let_tuple_pattern() {
        let program = parse_inline(
            r#"
def main() Unit {
    pair (Int, Str) = (1, "x")
    if let (left, right) = pair {
        OS.println(left, right)
    }
}
"#,
        );
        let result = check_program(&program);
        assert!(
            result.diagnostics.iter().any(|diag| {
                diag.code == "irrefutable_if_let"
                    && diag
                        .message
                        .contains("if let pattern is irrefutable for value of type '(Int, Str)'")
            }),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn rejects_irrefutable_if_let_type_pattern() {
        let program = parse_inline(
            r#"
class Worker {
}

def main() Unit {
    worker Worker = Worker {}
    if let item Worker = worker {
        OS.println(item)
    }
}
"#,
        );
        let result = check_program(&program);
        assert!(
            result.diagnostics.iter().any(|diag| {
                diag.code == "irrefutable_if_let"
                    && diag
                        .message
                        .contains("if let pattern is irrefutable for value of type 'Worker'")
            }),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn rejects_non_diverging_let_else() {
        let program = parse_inline(
            r#"
def main() Unit {
    value Option[Int] = Some(1)
    let Some { value as item } = value else {
        ()
    }
    OS.println(item)
}
"#,
        );
        let result = check_program(&program);
        assert!(
            result.diagnostics.iter().any(|diag| {
                diag.code == "non_diverging_let_else"
                    && diag
                        .message
                        .contains("must exit control flow with 'return', 'break', 'continue', or a call returning Never")
            }),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn rejects_placeholder_lambda_shorthand() {
        let program = parse_inline(
            r#"
def main() Unit {
    items = Vector(1, 2, 3)
    mapped = items.map(_ + 1)
}
"#,
        );
        let result = check_program(&program);
        assert!(
            result.diagnostics.iter().any(|diag| {
                diag.code == "invalid_placeholder_expr"
                    && diag.message.contains("'_' is not a value")
            }),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn allows_ignored_lambda_parameter_slots() {
        let program = parse_inline(
            r#"
def consumeOne(f fn(Int) Int) Int = f(10)

def consumeTwo(f fn(Int, Int) Int) Int = f(20, 3)

def main() Unit {
    one = consumeOne((_) => 1)
    bare = consumeOne(_ => 1)
    left = consumeTwo((x, _) => x)
    right = consumeTwo((_, value) => value + 1)
    typed = consumeTwo((_ Int, value Int) => value + 2)
    both = consumeTwo((_, _) => 1)
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn rejects_reading_ignored_lambda_parameter_slot() {
        let program = parse_inline(
            r#"
def consumeTwo(f fn(Int, Int) Int) Int = f(20, 3)

def main() Unit {
    value = consumeTwo((_, item) => _ + item)
}
"#,
        );
        let result = check_program(&program);
        assert!(
            result.diagnostics.iter().any(|diag| {
                diag.code == "invalid_placeholder_expr"
                    && diag.message.contains("'_' is not a value")
            }),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn accepts_by_name_function_parameters_and_forwarding() {
        let program = parse_inline(
            r#"
def inner(value => Int) Int = value

def outer(value => Int) Int =
    inner(value)

def main() Unit {
    result Int = outer(5)
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn rejects_by_name_constructor_and_vararg_parameters() {
        let program = parse_inline(
            r#"
class Box {
    value Int

    new(value => Int) {
        this.value = value
    }
}


def bad(values => [Int] vararg) Unit = ()
"#,
        );
        let result = check_program(&program);
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diag| diag.code == "invalid_by_name_param"
                    && diag
                        .message
                        .contains("constructor parameters cannot be by-name")),
            "{:#?}",
            result.diagnostics
        );
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diag| diag.code == "invalid_by_name_param"
                    && diag.message.contains("by-name parameters cannot be vararg")),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn rejects_try_inside_by_name_argument_expression() {
        let program = parse_inline(
            r#"
def delayed(value => Int) Int = value

def load() Option[Int] = Some(1)

def main() Unit {
    result Int = delayed(try load())
}
"#,
        );
        let result = check_program(&program);
        assert!(
            result.diagnostics.iter().any(|diag| {
                diag.code == "invalid_by_name_argument"
                    && diag
                        .message
                        .contains("by-name argument expressions cannot contain return")
            }),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn allows_diverging_let_else_with_continue() {
        let program = parse_inline(
            r#"
enum MaybeInt {
    case NoneX
    case SomeX {
        value Int
    }
}

def main() Unit {
    values Vector[MaybeInt] = [MaybeInt.SomeX(1), MaybeInt.NoneX]
    for value <- values {
        let SomeX { value as item } = value else continue
        OS.println(item)
    }
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn allows_let_else_with_never_call_fallback() {
        let program = parse_inline(
            r#"
enum MaybeInt {
    case NoneX
    case SomeX {
        value Int
    }
}

def fail() Never = panic("boom")

def main(value MaybeInt) Unit {
    let SomeX { value as item } = value else fail()
    OS.println(item)
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn allows_extract_shorthand_forms() {
        let program = parse_inline(
            r#"
def main(
    optionValue Option[Int],
    resultValue Result[Int, Str],
    eitherValue Either[Str, Int]
) Int {
    let optionItem <- optionValue else return 0
    let resultItem <- resultValue else return 1
    let eitherItem <- eitherValue else return 2
    let {
        left <- optionValue
        middle <- resultValue
        right <- eitherValue
    } else return 3
    if let branch <- optionValue {
        return optionItem + resultItem + eitherItem + left + middle + right + branch
    }
    return optionItem + resultItem + eitherItem
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn allows_refutable_while_let_conditions() {
        let program = parse_inline(
            r#"
def sum(start Option[Int]) Int {
    var current = start
    var total = 0
    while let value <- current && value > 0 {
        total += value
        current := None
    }
    total
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn rejects_irrefutable_while_let_conditions() {
        let program = parse_inline(
            r#"
def main() Unit {
    while let value = 1 {
        println(value)
    }
}
"#,
        );
        let result = check_program(&program);
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "irrefutable_while_let"),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn allows_bare_zero_payload_enum_cases() {
        let program = parse_inline(
            r#"
enum MaybeInt {
    case Missing
    case Present {
        value Int
    }
}

def main() Unit {
    first MaybeInt = Missing
    second MaybeInt = MaybeInt.Missing
    third MaybeInt = Present(1)
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn materializes_bare_zero_payload_enum_case_patterns() {
        let program = parse_inline(
            r#"
def mapOption[X](value Option[Int], f fn(Int) X) Option[X] {
    match value {
        case Some { value as item } => Some(f(item))
        case None => None
    }
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn rejects_bare_payload_enum_case_patterns() {
        let program = parse_inline(
            r#"
def main(value Option[Int]) Int {
    match value {
        case Some => 1
        case None => 0
    }
}
"#,
        );
        let result = check_program(&program);
        assert!(
            result.diagnostics.iter().any(|diag| {
                diag.code == "invalid_destructure"
                    && diag
                        .message
                        .contains("constructor pattern 'Some' expects 1 fields, got 0")
            }),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn allows_enum_case_default_constructor_shapes() {
        let program = parse_inline(
            r#"
enum Outcome {
    tag Str

    case Left {
        value Str
        tag = "left"
    }
}

def main() Unit {
    omitted Outcome = Outcome.Left("bad")
    explicit Outcome = Outcome.Left("bad", "custom")
    namedOmitted Outcome = Outcome.Left { value: "named" }
    namedExplicit Outcome = Outcome.Left { value: "named", tag: "custom" }
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn materializes_bare_enum_case_in_expected_shape_field() {
        let program = parse_inline(
            r#"
class Node {
    value Int
}

def missing() { node Option[Node] } {
    return { node: None }
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn rejects_zero_payload_enum_case_call_syntax() {
        let program = parse_inline(
            r#"
def main() Unit {
    value Option[Int] = None()
}
"#,
        );
        let result = check_program(&program);
        assert!(
            result.diagnostics.iter().any(|diag| {
                diag.code == "invalid_enum_case_call"
                    && diag
                        .message
                        .contains("enum case 'None' does not accept call syntax")
            }),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn allows_plain_let_only_for_irrefutable_patterns() {
        let program = parse_inline(
            r#"
def main() Int {
    pair (Int, Int) = (4, 5)
    let (left, right) = pair
    return left + right
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn rejects_refutable_plain_let_patterns_without_else() {
        let program = parse_inline(
            r#"
def main(optionValue Option[Int]) Int {
    knownOption = Some(4)
    let Some { value as optionItem } = optionValue
    let {
        Some { value as knownItem } = knownOption
    }
    return 0
}
"#,
        );
        let result = check_program(&program);
        let matches = result
            .diagnostics
            .iter()
            .filter(|diag| {
                diag.code == "refutable_let_pattern"
                    && diag.message.contains("add an 'else' fallback")
            })
            .count();
        assert_eq!(matches, 2, "{:#?}", result.diagnostics);
    }

    #[test]
    fn rejects_extract_shorthand_for_unknown_source_type() {
        let program = parse_inline(
            r#"
def main(value) Int {
    let item <- value else return 0
    return 0
}
"#,
        );
        let result = check_program(&program);
        assert!(
            result.diagnostics.iter().any(|diag| {
                diag.code == "invalid_extract_pattern"
                    && diag.message.contains("requires a known source type")
            }),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn rejects_refutable_for_pattern() {
        let program = parse_inline(
            r#"
	def main() Unit {
	    values Vector[Option[Int]] = [Some(1), None]
	    mapped = for {
	        maybe <- values
	        let Some { value } = maybe
	    } yield value
	    OS.println(mapped.size())
	}
	"#,
        );
        let result = check_program(&program);
        assert!(
            result.diagnostics.iter().any(|diag| {
                diag.code == "refutable_for_pattern"
                    && diag
                        .message
                        .contains("for pattern must be irrefutable for value of type 'Option[Int]'")
            }),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn rejects_continue_in_lifted_for_yield() {
        let cases = [
            (
                "Option",
                r#"
def main() Option[Int] =
    for item <- Some(1) yield {
        if item > 0 {
            continue
        }
        item + 1
    }
"#,
            ),
            (
                "Result",
                r#"
def main() Result[Int, Str] =
    for item <- Ok(1) yield {
        if item > 0 {
            continue
        }
        item + 1
    }
"#,
            ),
            (
                "Either",
                r#"
def main() Either[Str, Int] =
    for item <- Right(1) yield {
        if item > 0 {
            continue
        }
        item + 1
    }
"#,
            ),
        ];

        for (family, source) in cases {
            let program = parse_inline(source);
            let result = check_program(&program);
            assert!(
                result.diagnostics.iter().any(|diag| {
                    diag.code == "invalid_for_yield_continue"
                        && diag.message.contains(family)
                        && diag.message.contains("no skip state")
                }),
                "{family}: {:#?}",
                result.diagnostics
            );
        }
    }

    #[test]
    fn rejects_break_in_lifted_for_yield() {
        let cases = [
            (
                "Option",
                r#"
def main() Option[Int] =
    for item <- Some(1) yield {
        if item > 0 {
            break
        }
        item + 1
    }
"#,
            ),
            (
                "Result",
                r#"
def main() Result[Int, Str] =
    for item <- Ok(1) yield {
        if item > 0 {
            break
        }
        item + 1
    }
"#,
            ),
            (
                "Either",
                r#"
def main() Either[Str, Int] =
    for item <- Right(1) yield {
        if item > 0 {
            break
        }
        item + 1
    }
"#,
            ),
        ];

        for (family, source) in cases {
            let program = parse_inline(source);
            let result = check_program(&program);
            assert!(
                result.diagnostics.iter().any(|diag| {
                    diag.code == "invalid_for_yield_break"
                        && diag.message.contains(family)
                        && diag.message.contains("no early-exit state")
                }),
                "{family}: {:#?}",
                result.diagnostics
            );
        }
    }

    #[test]
    fn rejects_discarded_pure_trailing_expression_in_unit_callable() {
        let program = parse_inline(
            r#"
def main() Unit {
    value Int = 10
    value + 5
}
"#,
        );
        let result = check_program(&program);
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diag| diag.code == "discarded_expression"),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn rejects_discarded_identifier_statement() {
        let program = parse_inline(
            r#"
def main() Int {
    value Int = 10
    value
    return 0
}
"#,
        );
        let result = check_program(&program);
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diag| diag.code == "discarded_expression"),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn allows_discarded_call_expression_in_unit_callable() {
        let program = parse_inline(
            r#"
def sideEffect() Int = 5

def main() Unit {
    sideEffect()
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn rejects_discarded_pure_expression_in_if_branch() {
        let program = parse_inline(
            r#"
def main() Unit {
    if true {
        1 + 2
    } else {
        3 + 4
    }
}
"#,
        );
        let result = check_program(&program);
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diag| diag.code == "discarded_expression"),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn allows_defer_call_and_block() {
        let program = parse_inline(
            r#"
def cleanup() Unit {}

def main() Unit {
    defer cleanup()
    defer {
        OS.println("later")
    }
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn rejects_control_flow_inside_defer_block() {
        let program = parse_inline(
            r#"
def main() Unit {
    defer {
        return ()
    }
}
"#,
        );
        let result = check_program(&program);
        assert!(
            result.diagnostics.iter().any(|diag| {
                diag.code == "invalid_defer_control_flow"
                    && diag.message.contains("defer block cannot contain 'return'")
            }),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn allows_generic_reflection_type_annotations() {
        let program = parse_inline(
            r#"
class User {
    name Str
}

enum Status {
    case Pending
}

def main() Unit {
    user User = User("Ada")
    declared Type[User] = typeOf[User]
    actual Type[User] = user.runtimeType
    unknown Type[_] = declared
    anyMetadata Type[Any] = typeOf[Any]
    classType ClassType[User] = declared.asClass() !!
    unknownClass ClassType[_] = classType
    enumType EnumType[Status] = typeOf[Status].asEnum() !!
    fieldType Type[_] = (classType.fields().at(0) !!).fieldType()
    OS.println(actual.name() !!, unknown.name() !!, anyMetadata.kind(), unknownClass.name() !!, enumType.name() !!, fieldType.name() !!)
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn allows_existential_type_erasure_and_capture_reads() {
        let program = parse_inline(
            r#"
def speak(values Vector[_]) Unit {
    OS.println(values.size())
}

def intStrMap() Map[Int, Str] = Map()
def strIntMap() Map[Str, Int] = Map()

def main() Unit {
    a Vector[_] = Vector(1, 2, 3)
    b Map[_, Str] = intStrMap()
    c Map[_, _] = strIntMap()

    first Any = a[0]
    captured = a[0]
    sameCapture = captured

    speak(a)
    OS.println(a.size(), b.size(), c.size(), first, sameCapture)
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn rejects_writes_through_existential_list_capture() {
        let program = parse_inline(
            r#"
def main() Unit {
    values Vector[_] = Vector(1, 2, 3)
    values.add(7)
}
"#,
        );
        let result = check_program(&program);
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diag| diag.code == "invalid_argument_type"),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn rejects_writes_through_existential_map_captures() {
        let program = parse_inline(
            r#"
def intStrMap() Map[Int, Str] = Map()
def strIntMap() Map[Str, Int] = Map()

def main() Unit {
    keyErased Map[_, Str] = intStrMap()
    valueErased Map[Str, _] = strIntMap()

    keyErased.put(1, "one")
    valueErased.put("one", 1)
}
"#,
        );
        let result = check_program(&program);
        let count = result
            .diagnostics
            .iter()
            .filter(|diag| diag.code == "invalid_argument_type")
            .count();
        assert!(count >= 2, "{:#?}", result.diagnostics);
    }

    #[test]
    fn rejects_assigning_existential_capture_to_concrete_type() {
        let program = parse_inline(
            r#"
class SomeType {
    value Int
}

def main() Unit {
    values Vector[_] = Vector(1, 2, 3)
    concrete SomeType = values[0]
}
"#,
        );
        let result = check_program(&program);
        assert!(
            result.diagnostics.iter().any(|diag| {
                diag.code == "invalid_binding_type"
                    && diag
                        .message
                        .contains("binding 'concrete' of type 'SomeType'")
            }),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn keeps_existential_captures_equal_only_within_same_source() {
        let accepted = parse_inline(
            r#"
def same[T](left T, right T) Unit {}

def main() Unit {
    values Vector[_] = Vector(1, 2, 3)
    same(values[0], values[1])
}
"#,
        );
        let accepted_result = check_program(&accepted);
        assert!(
            accepted_result.diagnostics.is_empty(),
            "{:#?}",
            accepted_result.diagnostics
        );

        let rejected = parse_inline(
            r#"
def same[T](left T, right T) Unit {}

def main() Unit {
    left Vector[_] = Vector(1)
    right Vector[_] = Vector(2)
    same(left[0], right[0])
}
"#,
        );
        let rejected_result = check_program(&rejected);
        assert!(
            rejected_result
                .diagnostics
                .iter()
                .any(|diag| diag.code == "invalid_argument_type"),
            "{:#?}",
            rejected_result.diagnostics
        );
    }

    #[test]
    fn rejects_exact_any_reflection_metadata_for_non_any_type() {
        let program = parse_inline(
            r#"
class User {
    name Str
}

def main() Unit {
    metadata Type[Any] = typeOf[User]
}
"#,
        );
        let result = check_program(&program);
        assert!(
            result.diagnostics.iter().any(|diag| {
                diag.code == "invalid_binding_type"
                    && diag
                        .message
                        .contains("cannot assign value of type 'Type[User]'")
                    && diag
                        .message
                        .contains("binding 'metadata' of type 'Type[Any]'")
            }),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn treats_runtime_type_of_any_as_unknown_metadata() {
        let program = parse_inline(
            r#"
def inspect(value Any) Unit {
    metadata Type[_] = value.runtimeType
    exactAny Type[Any] = value.runtimeType
}
"#,
        );
        let result = check_program(&program);
        assert!(
            result.diagnostics.iter().any(|diag| {
                diag.code == "invalid_binding_type"
                    && diag
                        .message
                        .contains("cannot assign value of type 'Type[_]'")
                    && diag
                        .message
                        .contains("binding 'exactAny' of type 'Type[Any]'")
            }),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn rejects_raw_reflection_type_annotations() {
        let program = parse_inline(
            r#"
class User {
    name Str
}

def main() Unit {
    declared Type = typeOf[User]
}
"#,
        );
        let result = check_program(&program);
        assert!(
            result.diagnostics.iter().any(|diag| {
                diag.code == "invalid_binding_type"
                    && diag
                        .message
                        .contains("cannot assign value of type 'Type[User]'")
            }),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn checks_empty_collection_literal_from_expected_type() {
        let program = parse_inline(
            r#"
def countMap(values [Str : Int]) Int = values.size()
def countArray(values [Str]) Int = values.size()
def emptyMap() [Str : Int] = []
def emptyArray() [Str] = []

def main() Int {
    directMap [Str : Int] = []
    directArray [Str] = []
    return directMap.size() + directArray.size() + countMap([]) + countArray([]) + emptyMap().size() + emptyArray().size()
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn checks_list_and_map_spread_literals() {
        let program = parse_inline(
            r#"
def main() Unit {
    first = [1, 2]
    second = [3, 4]
    values [Int] = [0, ...first, ...second, 5]

    defaults [Str : Int] = ["one": 1, "shared": 2]
    overrides [Str : Int] = ["shared": 20, "three": 3]
    copy [Str : Int] = [...defaults]
    merged [Str : Int] = [...defaults, "two": 2, ...overrides]
    entries [(Str, Int)] = [...merged.entries()]

    println(values.size(), copy.size(), merged.size(), entries.size())
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn rejects_cross_family_collection_spreads() {
        let program = parse_inline(
            r#"
def main() Unit {
    values = [1, 2]
    entries [Str : Int] = ["one": 1]
    mixed = [...values, ...entries]
    invalidList = [0, ...entries]
    invalidMap [Str : Int] = [...values]
}
"#,
        );
        let result = check_program(&program);
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diag| diag.code == "mixed_collection_spreads"),
            "{:#?}",
            result.diagnostics
        );
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diag| diag.code == "invalid_vector_spread"),
            "{:#?}",
            result.diagnostics
        );
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diag| diag.code == "invalid_map_spread"),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn rejects_empty_collection_literal_without_expected_type() {
        let program = parse_inline(
            r#"
def main() Unit {
    values = []
}
"#,
        );
        let result = check_program(&program);
        assert!(
            result.diagnostics.iter().any(|diag| {
                diag.code == "cannot_infer_empty_collection_type"
                    && diag
                        .message
                        .contains("cannot infer the type of empty collection '[]'")
            }),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn rejects_empty_collection_literal_for_non_bracket_collection_types() {
        let program = parse_inline(
            r#"
def main() Unit {
    values Set[Int] = []
    array Array[Int] = []
}
"#,
        );
        let result = check_program(&program);
        assert_eq!(
            result
                .diagnostics
                .iter()
                .filter(|diag| diag.code == "invalid_empty_collection_context")
                .count(),
            2,
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn rejects_ambiguous_empty_collection_overload() {
        let program = parse_inline(
            r#"
class Consumer {

    new(values [Str]) {}
    new(values [Str : Int]) {}
}


def main() Unit {
    consumer = Consumer([])
}
"#,
        );
        let result = check_program(&program);
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diag| diag.code == "ambiguous_empty_collection"),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn checks_unsafe_extract_for_lifted_values_and_linked_list_access() {
        let program = parse_inline(
            r#"
shape User {
    name Str
}

def fromOption(value Option[User]) Str = value!!.name
def fromResult(value Result[Int, Str]) Int = value !!
def fromEither(value Either[Str, Int]) Int = value !!
def invoke(value Option[fn() Str]) Str = value!!()
def indexed(value Option[[Str: Int]]) Int = value!!["first"]!!
def nested(value Option[Option[Int]]) Int = value!!!!

def main() Unit {
    values LinkedList[Int] = LinkedList()
    values.add(5)
    first Int = values.at(0) !!
    println(first)
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn narrows_stable_identifier_inside_successful_is_branch() {
        let program = parse_inline(
            r#"
def textSize(value Any) Int {
    if value is Str {
        return value.size()
    }
    0
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn preserves_positive_is_narrowing_after_early_exit() {
        let program = parse_inline(
            r#"
def textSize(value Any) Int {
    if !(value is Str) {
        return 0
    }
    value.size()
}

def textSizeWithElse(value Any) Int {
    if value is Str {
    } else {
        return 0
    }
    value.size()
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn does_not_narrow_mutable_identifiers() {
        let program = parse_inline(
            r#"
def textSize(source Any) Int {
    var value Any = source
    if value is Str {
        return value.size()
    }
    0
}
"#,
        );
        let result = check_program(&program);
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "unknown_member"),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn models_runtime_collections_as_concrete_classes() {
        let ambient = default_inline_ambient();
        for name in ["Vector", "Map", "Array", "Set", "LinkedList"] {
            let ty = ambient
                .types
                .get(name)
                .unwrap_or_else(|| panic!("missing stdlib type {name}"));
            assert_eq!(ty.kind, TypeKind::Class, "{name} should be a class");
            assert!(
                ty.methods.contains_key("new"),
                "{name} should declare construction"
            );
        }
    }

    #[test]
    fn constructs_runtime_collection_classes_with_normal_class_syntax() {
        let program = parse_inline(
            r#"
def main() Unit {
    vector Vector[Int] = Vector {}
    map Map[Str, Int] = Map {}
    array Array[Int] = Array(1, 2)
    set Set[Int] = Set(1, 2)
    linked LinkedList[Int] = LinkedList {}
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn checks_direct_generic_bounds_and_cross_parameter_equality() {
        let program = parse_inline(
            r#"
interface Callable {
    def call() Unit
}

class Action with Callable {
    def call() Unit = ()
}

def invoke[T with Callable](value T) Unit = value.call()

def same[L, R when L = R](left L, right R) L = right

def main() Unit {
    invoke(Action {})
    value Int = same(1, 2)
    println(value)
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn checks_conditions_on_generic_type_applications() {
        let program = parse_inline(
            r#"
interface Callable {
    def call() Unit
}

class Action with Callable {
    def call() Unit = ()
}

class Plain {}

class Box[T with Callable] {
    value T
}

class Pair[L, R when L = R] {
    left L
    right R
}

def valid(value Box[Action]) Unit = value.value.call()
def invalid(value Box[Plain]) Unit = ()
def validPair(value Pair[Int, Int]) Unit = ()
def invalidPair(value Pair[Int, Str]) Unit = ()
"#,
        );
        let result = check_program(&program);
        assert_eq!(
            result
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.code == "generic_bound_not_satisfied")
                .count(),
            1,
            "{:#?}",
            result.diagnostics
        );
        assert_eq!(
            result
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.code == "generic_equality_not_satisfied")
                .count(),
            1,
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn rejects_unsatisfied_generic_bounds_and_equalities() {
        let program = parse_inline(
            r#"
interface Callable {
    def call() Unit
}

class Plain {}

def invoke[T with Callable](value T) Unit = value.call()
def same[L, R when L = R](left L, right R) L = right

def main() Unit {
    invoke(Plain {})
    _ = same(1, "two")
}
"#,
        );
        let result = check_program(&program);
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "generic_bound_not_satisfied"),
            "{:#?}",
            result.diagnostics
        );
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "generic_equality_not_satisfied"),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn checks_owner_type_conditions_on_generic_methods() {
        let program = parse_inline(
            r#"
interface Callable {
    def call() Unit
}

class Action with Callable {
    def call() Unit = ()
}

class Context[L, R] {
    def invoke[when L with Callable](value L) Unit = value.call()
    def merge[when L = R](value R) L = value
}

def useContext(context Context[Action, Action], action Action) Action {
    context.invoke(action)
    return context.merge(action)
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn rejects_unsatisfied_owner_type_conditions() {
        let program = parse_inline(
            r#"
interface Callable {
    def call() Unit
}

class Plain {}

class Context[L, R] {
    def invoke[when L with Callable](value L) Unit = value.call()
    def merge[when L = R](value R) L = value
}

def invalid(context Context[Plain, Str], plain Plain) Unit {
    context.invoke(plain)
    _ = context.merge("value")
}
"#,
        );
        let result = check_program(&program);
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "generic_bound_not_satisfied"),
            "{:#?}",
            result.diagnostics
        );
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "generic_equality_not_satisfied"),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn rejects_unsafe_extract_from_plain_values() {
        let program = parse_inline(
            r#"
def invalid(value Int) Int = value !!
"#,
        );
        let result = check_program(&program);
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diag| diag.code == "invalid_unsafe_extract"),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn rejects_removed_or_panic_methods_on_lifted_values() {
        let program = parse_inline(
            r#"
def fromOption(value Option[Int]) Int = value.orPanic()
def fromResult(value Result[Int, Str]) Int = value.orPanic()
def fromEither(value Either[Str, Int]) Int = value.orPanic()
"#,
        );
        let result = check_program(&program);
        let removed = result
            .diagnostics
            .iter()
            .filter(|diag| {
                diag.code == "unknown_member"
                    && diag.message.contains("method 'orPanic' was removed")
                    && diag.message.contains("postfix '!!'")
            })
            .count();
        assert_eq!(removed, 3, "{:#?}", result.diagnostics);
    }

    #[test]
    fn checks_parity_examples() {
        let root = workspace_root();
        let paths = [
            "examples/classes.lum",
            "examples/tuple_destructuring.lum",
            "examples/shape_destructuring.lum",
            "examples/class_destructuring.lum",
            "examples/enums.lum",
            "examples/enum_object_same_name.lum",
            "examples/imports.lum",
            "examples/interface_default_methods.lum",
            "examples/vector_hof.lum",
            "examples/set_map_hof.lum",
            "examples/placeholder_lambda.lum",
            "examples/zip.lum",
        ];

        for path in paths {
            let result = check_path(root.join(path)).unwrap_or_else(|err| panic!("{path}: {err}"));
            assert!(
                result.diagnostics.is_empty(),
                "{path}: {:#?}",
                result.diagnostics
            );
        }
    }
}
