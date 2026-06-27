use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
};

use crate::{
    Diagnostic,
    ast::{
        AssignOp, AssignmentStmt, BinaryOp, BindingStmt, Block, CallableBody, DestructureKind,
        ElseBranch, ElseExprBranch, Expr, ForBinding, FunctionDecl, IfConditionClause, IfStmt,
        ImplBlock, ImplTargetKind, Item, LambdaBody, MatchCase, MatchCaseBody, MatchStmt,
        MethodDecl, Param, Pattern, PatternBindingKind, PatternBindingStmt, Program, Stmt,
        TypeDecl, TypeKind, TypeMember, TypeRef, Visibility,
    },
    resolver::{
        ImportedKind, ImportedSymbol, LoadedModule, ModuleGraph, collect_module_order,
        find_stdlib_dir, load_module_graph, parse_program_from_path, read_directives, resolve_path,
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
    let resolved = resolve_path(path.as_ref())?;
    if !resolved.diagnostics.is_empty() {
        return Ok(PathCheckResult {
            diagnostics: resolved.diagnostics,
        });
    }

    let (graph, root_path) = load_module_graph(path.as_ref())?;
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
    Never,
    Named(String, Vec<Ty>),
    Tuple(Vec<Ty>),
    Record(Vec<(String, Ty)>),
    Function(Vec<Ty>, Box<Ty>),
    TypeParam(String),
}

impl Ty {
    fn named(name: impl Into<String>) -> Self {
        Self::Named(name.into(), Vec::new())
    }

    fn never() -> Self {
        Self::Never
    }

    fn list(item: Ty) -> Self {
        Self::Named("List".to_string(), vec![item])
    }

    fn option(item: Ty) -> Self {
        Self::Named("Option".to_string(), vec![item])
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
                "({}) -> {}",
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

    fn is_str(&self) -> bool {
        matches!(self, Ty::Named(name, args) if name == "Str" && args.is_empty())
    }

    fn is_int_like(&self) -> bool {
        matches!(self, Ty::Named(name, args) if args.is_empty() && (name == "Int" || name == "Int64"))
    }

    fn is_float_like(&self) -> bool {
        matches!(self, Ty::Named(name, args) if args.is_empty() && (name == "Float" || name == "Float64"))
    }

    fn is_numeric(&self) -> bool {
        self.is_int_like() || self.is_float_like()
    }
}

#[derive(Debug, Clone)]
struct ValueInfo {
    ty: Ty,
    mutable: bool,
    known: Option<KnownValue>,
}

#[derive(Debug, Clone)]
enum KnownValue {
    Unit,
    Bool(bool),
    Int(i64),
    Float(String),
    String(String),
    List(Vec<KnownValue>),
    Tuple(Vec<KnownValue>),
    Constructor {
        path: Vec<String>,
        args: Vec<KnownValue>,
    },
}

#[derive(Debug, Clone)]
struct ParamSig {
    name: String,
    ty: Ty,
    variadic: bool,
    has_initializer: bool,
}

#[derive(Debug, Clone)]
struct FunctionSig {
    params: Vec<ParamSig>,
    ret: Ty,
    visibility: Visibility,
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
    functions: HashMap<String, Vec<FunctionSig>>,
    types: HashMap<String, TypeSig>,
    singles: HashMap<String, TypeSig>,
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
            functions: HashMap::new(),
            types: HashMap::new(),
            singles: HashMap::new(),
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
            functions: HashMap::new(),
            types: HashMap::new(),
            singles: HashMap::new(),
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
                    if decl.kind == TypeKind::Single {
                        self.singles.insert(decl.name.clone(), sig);
                    } else {
                        self.types.insert(decl.name.clone(), sig);
                    }
                }
                Item::Statement(Stmt::Binding(binding)) => {
                    self.global_binding_stmts.push(binding.clone());
                }
                _ => {}
            }
        }
        for item in &items {
            if let Item::Impl(block) = item {
                self.merge_impl(block);
            }
        }
    }

    fn merge_impl(&mut self, block: &ImplBlock) {
        let Some(target_name) = type_ref_named_name(&block.target) else {
            return;
        };
        let target_type_params = impl_target_type_params(&block.target);
        let target = match block.target_kind {
            ImplTargetKind::Instance => self.types.get_mut(target_name),
            ImplTargetKind::Single => {
                if !self.singles.contains_key(target_name) {
                    if matches!(&block.target, TypeRef::Named { args, .. } if !args.is_empty()) {
                        return;
                    }
                    let sig = self
                        .types
                        .get(target_name)
                        .cloned()
                        .map(|base| synthetic_single_sig_from_type(&base))
                        .unwrap_or_else(|| standalone_single_sig(target_name));
                    self.singles.insert(target_name.to_string(), sig);
                }
                self.singles.get_mut(target_name)
            }
        };
        if let Some(sig) = target {
            for method in &block.methods {
                sig.methods
                    .entry(method.name.clone())
                    .or_default()
                    .push(function_sig_from_method(method, &target_type_params));
            }
        }
    }
}

#[derive(Debug, Clone, Default)]
struct AmbientInfo {
    functions: HashMap<String, Vec<FunctionSig>>,
    types: HashMap<String, TypeSig>,
    singles: HashMap<String, TypeSig>,
    enum_cases: HashMap<String, EnumCaseSig>,
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
            for (name, sig) in module.types {
                for (case_name, case_sig) in &sig.enum_cases {
                    ambient
                        .enum_cases
                        .insert(case_name.clone(), case_sig.clone());
                }
                ambient.types.insert(name, sig);
            }
            for (name, sig) in module.singles {
                ambient.singles.insert(name, sig);
            }
        }

        if let Some(os) = ambient.singles.get("OS") {
            for builtin in ["print", "println", "printf", "panic"] {
                if let Some(sigs) = os.methods.get(builtin) {
                    ambient.functions.insert(builtin.to_string(), sigs.clone());
                }
            }
        }

        Ok(ambient)
    }
}

fn synthetic_single_sig_from_type(base: &TypeSig) -> TypeSig {
    TypeSig {
        kind: TypeKind::Single,
        name: base.name.clone(),
        type_params: Vec::new(),
        with_bounds: Vec::new(),
        fields: Vec::new(),
        methods: HashMap::new(),
        enum_cases: HashMap::new(),
    }
}

fn standalone_single_sig(name: &str) -> TypeSig {
    TypeSig {
        kind: TypeKind::Single,
        name: name.to_string(),
        type_params: Vec::new(),
        with_bounds: Vec::new(),
        fields: Vec::new(),
        methods: HashMap::new(),
        enum_cases: HashMap::new(),
    }
}

fn type_kind_label(kind: TypeKind) -> &'static str {
    match kind {
        TypeKind::Class => "class",
        TypeKind::Record => "shape",
        TypeKind::Single => "single",
        TypeKind::Interface => "interface",
        TypeKind::Enum => "enum",
    }
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
                                imports: module.imports.clone(),
                                symbol_imports: module.symbol_imports.clone(),
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
        if let Some(single_name) = imported.single_name.as_deref() {
            let single = source.singles.get(single_name)?;
            return single.methods.get(&imported.original_name).cloned();
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
            ImportedKind::Single => self
                .modules
                .get(&imported.module_path)?
                .singles
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
    type_params: Vec<HashSet<String>>,
    current_return: Ty,
    current_owner: Option<TypeSig>,
    current_method: Option<String>,
    loop_depth: usize,
    defer_depth: usize,
    globals: HashMap<String, ValueInfo>,
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
            loop_depth: 0,
            defer_depth: 0,
            globals: HashMap::new(),
        }
    }

    fn check_module(&mut self) {
        self.check_global_bindings();
        for item in &self.module.program.items {
            match item {
                Item::Function(function) => self.check_function(function),
                Item::Type(decl) => self.check_type_decl(decl),
                Item::Impl(block) => self.check_impl(block),
                _ => {}
            }
        }
    }

    fn check_global_bindings(&mut self) {
        self.push_scope();
        for binding_stmt in &self.module.global_binding_stmts {
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
                            "cannot assign value of type '{}' to binding '{}' of type '{}'",
                            inferred.describe(),
                            binding.name,
                            expected.describe()
                        ),
                    );
                }
                self.globals.insert(
                    binding.name.clone(),
                    ValueInfo {
                        ty,
                        mutable: binding.mutable,
                        known: (!binding.mutable
                            && binding_stmt.bindings.len() == 1
                            && binding_stmt.values.len() == 1
                            && binding_stmt.destructure.is_none())
                        .then(|| self.known_value_from_expr(&binding_stmt.values[0]))
                        .flatten(),
                    },
                );
            }
        }
        self.pop_scope();
    }

    fn check_function(&mut self, function: &FunctionDecl) {
        let previous_return = self.current_return.clone();
        let previous_defer_depth = self.defer_depth;
        self.push_type_params(function.type_params.iter().map(|param| param.name.as_str()));
        let expected_return = function
            .return_type
            .as_ref()
            .map(|ty| self.ty_from_type_ref(ty))
            .unwrap_or(Ty::Unknown);
        self.current_return = expected_return.clone();
        self.defer_depth = 0;
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
    }

    fn check_type_decl(&mut self, decl: &TypeDecl) {
        let Some(type_sig) = self.lookup_type_local(&decl.name) else {
            return;
        };
        self.push_type_params(type_sig.type_params.iter().map(String::as_str));

        for member in &decl.members {
            match member {
                TypeMember::Field(field) => {
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
                    if decl.kind == TypeKind::Interface && method.name == "new" {
                        self.add_error(
                            "invalid_interface_method",
                            format!(
                                "interface '{}': interfaces cannot declare constructors",
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

        self.check_type_field_initializers(decl, &type_sig);

        self.pop_type_params();
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
                let actual = self.check_expr(initializer);
                let expected = field
                    .ty
                    .as_ref()
                    .map(|ty| self.ty_from_type_ref(ty))
                    .unwrap_or_else(|| actual.clone());
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

    fn check_impl(&mut self, block: &ImplBlock) {
        let Some(target_name) = type_ref_named_name(&block.target) else {
            return;
        };
        let type_sig = match block.target_kind {
            ImplTargetKind::Instance => {
                let Some(type_sig) = self.lookup_type_local(target_name) else {
                    self.add_error(
                        "unknown_impl_target",
                        format!("unknown impl target '{}'", target_name),
                        block.span,
                    );
                    return;
                };
                if type_sig.kind == TypeKind::Interface || type_sig.kind == TypeKind::Single {
                    self.add_error(
                        "unknown_impl_target",
                        format!("unknown impl target '{}'", target_name),
                        block.span,
                    );
                    return;
                }
                type_sig
            }
            ImplTargetKind::Single => {
                if let Some(single_sig) = self.lookup_single_local(target_name) {
                    single_sig
                } else if let Some(base_sig) = self.lookup_type_local(target_name) {
                    if matches!(&block.target, TypeRef::Named { args, .. } if !args.is_empty()) {
                        self.add_error(
                            "invalid_type_arity",
                            format!(
                                "single '{}' expects no type arguments; write 'impl single {}'",
                                target_name, target_name
                            ),
                            block.span,
                        );
                        return;
                    }
                    synthetic_single_sig_from_type(&base_sig)
                } else {
                    if matches!(&block.target, TypeRef::Named { args, .. } if !args.is_empty()) {
                        self.add_error(
                            "invalid_type_arity",
                            format!(
                                "single '{}' expects no type arguments; write 'impl single {}'",
                                target_name, target_name
                            ),
                            block.span,
                        );
                        return;
                    }
                    standalone_single_sig(target_name)
                }
            }
        };
        self.push_type_params(type_sig.type_params.iter().map(String::as_str));
        for method in &block.methods {
            if type_sig.kind == TypeKind::Record && method.name == "new" {
                self.add_error(
                    "invalid_shape_constructor",
                    format!(
                        "shape '{}' cannot declare custom constructors; use brace construction",
                        type_sig.name
                    ),
                    method.span,
                );
                continue;
            }
            self.check_method(method, &type_sig);
        }
        self.pop_type_params();
    }

    fn check_method(&mut self, method: &MethodDecl, owner: &TypeSig) {
        let previous_return = self.current_return.clone();
        let previous_owner = self.current_owner.clone();
        let previous_method = self.current_method.clone();
        let previous_defer_depth = self.defer_depth;
        self.push_type_params(method.type_params.iter().map(|param| param.name.as_str()));
        let expected_return = method
            .return_type
            .as_ref()
            .map(|ty| self.ty_from_type_ref(ty))
            .unwrap_or(Ty::Unknown);
        self.current_return = expected_return.clone();
        self.defer_depth = 0;
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
    }

    fn check_constructor_initializes_required_fields(
        &mut self,
        method: &MethodDecl,
        owner: &TypeSig,
    ) {
        if method.name != "new" || !matches!(owner.kind, TypeKind::Class | TypeKind::Single) {
            return;
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

    fn check_param_list_rules(&mut self, params: &[Param], is_constructor: bool) {
        let mut seen_default = false;
        let mut seen_variadic = false;
        for (index, param) in params.iter().enumerate() {
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
            if param.variadic && seen_default {
                self.add_error(
                    "invalid_variadic_param",
                    if is_constructor {
                        "variadic constructor parameter cannot follow defaulted parameters"
                    } else {
                        "variadic parameter cannot follow defaulted parameters"
                    },
                    param.span,
                );
            }
            if param.variadic {
                let is_list_type = param.ty.as_ref().is_some_and(is_list_type_ref);
                if !is_list_type {
                    self.add_error(
                        "invalid_variadic_param",
                        if is_constructor {
                            "variadic constructor parameter must use a list type like '[T] vararg'"
                        } else {
                            "variadic parameter must use a list type like '[T] vararg'"
                        },
                        param.span,
                    );
                }
                seen_variadic = true;
            }
            if param.initializer.is_some() {
                seen_default = true;
            } else if seen_default && !param.variadic {
                self.add_error(
                    "invalid_constructor_default",
                    if is_constructor {
                        "constructor parameters without defaults cannot follow defaulted parameters"
                    } else {
                        "parameters without defaults cannot follow defaulted parameters"
                    },
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
                "constructor parameter defaults must be literal constants for now",
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
            Expr::ListLiteral { items, .. } | Expr::TupleLiteral { items, .. } => {
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
                receiver, updates, ..
            } => {
                self.check_field_initializer_expr(receiver, owner, initialized_fields);
                for update in updates {
                    self.check_field_initializer_expr(&update.value, owner, initialized_fields);
                }
            }
            Expr::RecordLiteral { fields, values, .. } => {
                for field in fields {
                    self.check_field_initializer_expr(&field.value, owner, initialized_fields);
                }
                for value in values {
                    self.check_field_initializer_expr(value, owner, initialized_fields);
                }
            }
            Expr::AnonymousInterface { .. } | Expr::Lambda { .. } => {}
            Expr::Try { value, .. }
            | Expr::Unary { expr: value, .. }
            | Expr::Group { inner: value, .. } => {
                self.check_field_initializer_expr(value, owner, initialized_fields);
            }
            Expr::Binary { left, right, .. } => {
                self.check_field_initializer_expr(left, owner, initialized_fields);
                self.check_field_initializer_expr(right, owner, initialized_fields);
            }
            Expr::Is { left, .. } => {
                self.check_field_initializer_expr(left, owner, initialized_fields);
            }
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
            Stmt::ExpectCondition(stmt) => {
                self.check_field_initializer_expr(&stmt.condition, owner, initialized_fields);
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
                self.check_field_initializer_expr(&stmt.condition, owner, initialized_fields);
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

    fn check_record_update(
        &mut self,
        base: &Ty,
        updates: &[crate::ast::CallArg],
        span: crate::source::Span,
    ) {
        match base {
            Ty::Named(name, args) => {
                let Some(sig) = self.lookup_any_type(name) else {
                    for update in updates {
                        self.check_expr(&update.value);
                    }
                    return;
                };
                if !matches!(sig.kind, TypeKind::Class | TypeKind::Record) {
                    self.add_error(
                        "invalid_record_update",
                        "update requires a class, record, or anonymous record value",
                        span,
                    );
                    for update in updates {
                        self.check_expr(&update.value);
                    }
                    return;
                }
                if sig.kind == TypeKind::Class && sig.fields.iter().any(|field| field.hidden) {
                    self.add_error(
                        "invalid_record_update",
                        "class update requires a class without private fields",
                        span,
                    );
                }
                let subst = sig
                    .type_params
                    .iter()
                    .cloned()
                    .zip(args.iter().cloned())
                    .collect::<HashMap<_, _>>();
                let fields = sig
                    .fields
                    .iter()
                    .filter(|field| !field.hidden)
                    .map(|field| (field.name.clone(), substitute_type(&field.ty, &subst)))
                    .collect::<Vec<_>>();
                self.check_record_update_fields(&fields, updates);
            }
            Ty::Record(fields) => self.check_record_update_fields(fields, updates),
            Ty::Unknown => {
                for update in updates {
                    self.check_expr(&update.value);
                }
            }
            _ => {
                self.add_error(
                    "invalid_record_update",
                    "update requires a class, record, or anonymous record value",
                    span,
                );
                for update in updates {
                    self.check_expr(&update.value);
                }
            }
        }
    }

    fn check_record_update_fields(
        &mut self,
        fields: &[(String, Ty)],
        updates: &[crate::ast::CallArg],
    ) {
        for update in updates {
            let Some(name) = update.name.as_deref() else {
                self.check_expr(&update.value);
                continue;
            };
            let Some((_, expected)) = fields.iter().find(|(field, _)| field == name) else {
                self.add_error(
                    "invalid_record_update",
                    format!("update field '{}' does not exist on left-hand shape", name),
                    update.span,
                );
                self.check_expr(&update.value);
                continue;
            };
            let actual = self.check_expr_against(&update.value, expected);
            self.require_assignable(
                &actual,
                expected,
                update.span,
                "invalid_record_update",
                format!(
                    "update field '{}' expects '{}', got '{}'",
                    name,
                    expected.describe(),
                    actual.describe()
                ),
            );
        }
    }

    fn record_merge_shape_fields(
        &mut self,
        ty: &Ty,
        side: &str,
        span: crate::source::Span,
    ) -> Option<Vec<(String, Ty)>> {
        match ty {
            Ty::Record(fields) => Some(fields.clone()),
            Ty::Named(name, args) => {
                let Some(sig) = self.lookup_any_type(name) else {
                    self.add_error(
                        "invalid_record_merge",
                        format!(
                            "record merge operands must be record-shaped values; {side} operand has type '{}'",
                            ty.describe()
                        ),
                        span,
                    );
                    return None;
                };
                if !matches!(sig.kind, TypeKind::Class | TypeKind::Record) || sig.fields.is_empty()
                {
                    self.add_error(
                        "invalid_record_merge",
                        format!(
                            "record merge operands must be record-shaped values; {side} operand has type '{}'",
                            ty.describe()
                        ),
                        span,
                    );
                    return None;
                }
                if sig.fields.iter().any(|field| field.hidden) {
                    self.add_error(
                        "invalid_record_merge",
                        format!(
                            "record merge cannot use type '{}' because it has hidden fields",
                            sig.name
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
                        .map(|field| (field.name.clone(), substitute_type(&field.ty, &subst)))
                        .collect(),
                )
            }
            Ty::Unknown => None,
            _ => {
                self.add_error(
                    "invalid_record_merge",
                    format!(
                        "record merge operands must be record-shaped values; {side} operand has type '{}'",
                        ty.describe()
                    ),
                    span,
                );
                None
            }
        }
    }

    fn check_record_merge_expr(&mut self, left: &Ty, right: &Ty, span: crate::source::Span) -> Ty {
        let Some(left_fields) = self.record_merge_shape_fields(left, "left", span) else {
            return Ty::Unknown;
        };
        let Some(right_fields) = self.record_merge_shape_fields(right, "right", span) else {
            return Ty::Unknown;
        };

        let left_names = left_fields
            .iter()
            .map(|(name, _)| name.as_str())
            .collect::<HashSet<_>>();
        let mut has_overlap = false;
        for (name, _) in &right_fields {
            if left_names.contains(name.as_str()) {
                has_overlap = true;
                self.add_error(
                    "invalid_record_merge",
                    format!("record merge field '{}' exists on both operands", name),
                    span,
                );
            }
        }
        if has_overlap {
            return Ty::Unknown;
        }

        Ty::Record(left_fields.into_iter().chain(right_fields).collect())
    }

    fn check_block(&mut self, block: &Block) -> Ty {
        self.check_block_against(block, &Ty::Unknown)
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
        let then_ty = self.check_block_against(&stmt.then_block, expected);
        let else_ty = stmt
            .else_branch
            .as_ref()
            .map(|branch| self.check_else_branch_value(branch, expected))
            .unwrap_or_else(Ty::unit);
        join_types(&then_ty, &else_ty)
    }

    fn check_else_branch_value(&mut self, branch: &ElseBranch, expected: &Ty) -> Ty {
        match branch {
            ElseBranch::If(stmt) => self.check_if_stmt_value(stmt, expected),
            ElseBranch::Block(block) => self.check_block_against(block, expected),
        }
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
                let known_value = (!binding_stmt.bindings[0].mutable
                    && binding_stmt.bindings.len() == 1
                    && binding_stmt.values.len() == 1
                    && binding_stmt.destructure.is_none())
                .then(|| self.known_value_from_expr(&binding_stmt.values[0]))
                .flatten();
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
                                "cannot assign value of type '{}' to binding '{}' of type '{}'",
                                inferred.describe(),
                                binding.name,
                                expected.describe()
                            ),
                        );
                    }
                    self.define_local_known(
                        &binding.name,
                        ty,
                        binding.mutable,
                        (index == 0).then(|| known_value.clone()).flatten(),
                    );
                }
                Ty::unit()
            }
            Stmt::PatternBinding(stmt) => {
                self.check_pattern_binding_stmt(stmt);
                Ty::unit()
            }
            Stmt::ExpectCondition(stmt) => {
                self.check_expect_condition_stmt(stmt);
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
                let condition = self.check_expr(&stmt.condition);
                self.require_bool(
                    &condition,
                    stmt.condition.span(),
                    "while condition must be Bool",
                );
                self.loop_depth += 1;
                self.check_block(&stmt.body);
                self.loop_depth -= 1;
                Ty::unit()
            }
            Stmt::For(stmt) => {
                self.push_scope();
                self.loop_depth += 1;
                for binding in &stmt.bindings {
                    self.check_for_binding(binding);
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
                if self.defer_depth > 0 {
                    self.add_error(
                        "invalid_defer_control_flow",
                        "defer block cannot contain 'return'",
                        return_stmt.span,
                    );
                }
                let expected = self.current_return.clone();
                let actual = return_stmt
                    .value
                    .as_ref()
                    .map(|expr| self.check_expr_against(expr, &expected))
                    .unwrap_or_else(Ty::unit);
                self.require_assignable(
                    &actual,
                    &expected,
                    return_stmt.span,
                    "invalid_return_type",
                    format!(
                        "return has type '{}' but enclosing callable expects '{}'",
                        actual.describe(),
                        expected.describe()
                    ),
                );
                actual
            }
            Stmt::Break(break_stmt) => {
                if self.defer_depth > 0 {
                    self.add_error(
                        "invalid_defer_control_flow",
                        "defer block cannot contain 'break'",
                        break_stmt.span,
                    );
                }
                if self.loop_depth == 0 {
                    self.add_error(
                        "invalid_break",
                        "break used outside of a loop",
                        break_stmt.span,
                    );
                }
                Ty::unit()
            }
            Stmt::Continue(continue_stmt) => {
                if self.defer_depth > 0 {
                    self.add_error(
                        "invalid_defer_control_flow",
                        "defer block cannot contain 'continue'",
                        continue_stmt.span,
                    );
                }
                if self.loop_depth == 0 {
                    self.add_error(
                        "invalid_continue",
                        "continue used outside of a loop",
                        continue_stmt.span,
                    );
                }
                Ty::unit()
            }
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
        if self.current_return == Ty::Unknown {
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
                "let-else used outside callable body",
                stmt.span,
            );
            return;
        }
        self.check_block(&stmt.else_block);
        if !self.block_guarantees_control_exit(&stmt.else_block) {
            self.add_error(
                "non_diverging_let_else",
                "let-else fallback must exit control flow with 'return', 'break', 'continue', or a call returning Never",
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
                if matches!(stmt.kind, PatternBindingKind::Let) {
                    self.require_safe_let_pattern(
                        &clause.pattern,
                        &value_ty,
                        &clause.value,
                        clause.pattern.span(),
                    );
                }
                self.bind_pattern(&clause.pattern, &value_ty);
            }
            return;
        }
        let value_ty = self.check_expr(&stmt.value);
        if matches!(stmt.kind, PatternBindingKind::Let) {
            self.require_safe_let_pattern(
                &stmt.pattern,
                &value_ty,
                &stmt.value,
                stmt.pattern.span(),
            );
        }
        self.bind_pattern(&stmt.pattern, &value_ty);
    }

    fn check_expect_condition_stmt(&mut self, stmt: &crate::ast::ExpectConditionStmt) {
        let condition_ty = self.check_expr(&stmt.condition);
        self.require_bool(
            &condition_ty,
            stmt.condition.span(),
            "expect condition must be Bool",
        );
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
            Stmt::ExpectCondition(_) => false,
            _ => false,
        }
    }

    fn expr_guarantees_control_exit(&self, expr: &Expr) -> bool {
        match expr {
            Expr::Group { inner, .. } => self.expr_guarantees_control_exit(inner),
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

    fn require_irrefutable_for_pattern(
        &mut self,
        pattern: &Pattern,
        scrutinee: &Ty,
        source: &Expr,
        span: crate::source::Span,
    ) {
        if !self.pattern_is_irrefutable(pattern, scrutinee)
            && !self.known_expr_matches_pattern(pattern, source)
        {
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
                    "plain 'let' pattern may fail for value of type '{}'; use 'let ... else ...' instead",
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
            (Pattern::Extract { inner, .. }, source) => {
                let Some((path, args)) = self.constructor_expr_parts(source) else {
                    return false;
                };
                path.last()
                    .is_some_and(|name| matches!(name.as_str(), "Some" | "Ok" | "Right"))
                    && args.len() == 1
                    && self.source_expr_proves_pattern_match(inner, args[0])
            }
            (Pattern::Tuple { elements, .. }, Expr::TupleLiteral { items, .. })
                if elements.len() == items.len() =>
            {
                elements
                    .iter()
                    .zip(items.iter())
                    .all(|(pattern, item)| self.source_expr_proves_pattern_match(pattern, item))
            }
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

    fn known_expr_matches_pattern(&self, pattern: &Pattern, source: &Expr) -> bool {
        match self.known_value_from_expr(source) {
            Some(KnownValue::List(items)) => items
                .iter()
                .all(|item| self.known_value_matches_pattern(item, pattern)),
            Some(value) => self.known_value_matches_pattern(&value, pattern),
            None => false,
        }
    }

    fn known_value_from_expr(&self, expr: &Expr) -> Option<KnownValue> {
        match expr {
            Expr::Identifier { name, .. } => self.lookup_value(name)?.known.clone(),
            Expr::Group { inner, .. } => self.known_value_from_expr(inner),
            Expr::Unit { .. } => Some(KnownValue::Unit),
            Expr::Bool { value, .. } => Some(KnownValue::Bool(*value)),
            Expr::Integer { raw, .. } => raw.parse().ok().map(KnownValue::Int),
            Expr::Float { raw, .. } => Some(KnownValue::Float(raw.clone())),
            Expr::String { raw, .. } => Some(KnownValue::String(raw.clone())),
            Expr::ListLiteral { items, .. } => items
                .iter()
                .map(|item| self.known_value_from_expr(item))
                .collect::<Option<Vec<_>>>()
                .map(KnownValue::List),
            Expr::TupleLiteral { items, .. } => items
                .iter()
                .map(|item| self.known_value_from_expr(item))
                .collect::<Option<Vec<_>>>()
                .map(KnownValue::Tuple),
            Expr::Call { callee, args, .. } => {
                let path = expr_path_for_known_value(callee)?;
                if path == ["List".to_string()] {
                    if args.iter().any(|arg| arg.name.is_some()) {
                        return None;
                    }
                    return args
                        .iter()
                        .map(|arg| self.known_value_from_expr(&arg.value))
                        .collect::<Option<Vec<_>>>()
                        .map(KnownValue::List);
                }
                if args.iter().any(|arg| arg.name.is_some()) {
                    return None;
                }
                if self.lookup_case_by_path(&path).is_none()
                    && self.lookup_destructured_type_pattern(&path).is_none()
                {
                    return None;
                }
                let values = args
                    .iter()
                    .map(|arg| self.known_value_from_expr(&arg.value))
                    .collect::<Option<Vec<_>>>()?;
                Some(KnownValue::Constructor { path, args: values })
            }
            other => {
                let path = expr_path_for_known_value(other)?;
                let case = self.lookup_case_by_path(&path)?;
                case.params.is_empty().then_some(KnownValue::Constructor {
                    path,
                    args: Vec::new(),
                })
            }
        }
    }

    fn known_value_matches_pattern(&self, value: &KnownValue, pattern: &Pattern) -> bool {
        match pattern {
            Pattern::Wildcard { .. } | Pattern::Binding { .. } => true,
            Pattern::Extract { inner, .. } => match value {
                KnownValue::Constructor { path, args }
                    if path
                        .last()
                        .is_some_and(|name| matches!(name.as_str(), "Some" | "Ok" | "Right"))
                        && args.len() == 1 =>
                {
                    self.known_value_matches_pattern(&args[0], inner)
                }
                _ => false,
            },
            Pattern::Type { .. } => false,
            Pattern::Literal {
                value: pattern_value,
                ..
            } => {
                matches!(
                    (value, pattern_value),
                    (KnownValue::Unit, Expr::Unit { .. })
                        | (KnownValue::Bool(true), Expr::Bool { value: true, .. })
                        | (KnownValue::Bool(false), Expr::Bool { value: false, .. })
                ) || match (value, pattern_value) {
                    (KnownValue::Int(left), Expr::Integer { raw, .. }) => {
                        raw.parse::<i64>().ok().is_some_and(|right| *left == right)
                    }
                    (KnownValue::Float(left), Expr::Float { raw, .. }) => left == raw,
                    (KnownValue::String(left), Expr::String { raw, .. }) => left == raw,
                    _ => false,
                }
            }
            Pattern::Tuple { elements, .. } => match value {
                KnownValue::Tuple(items) if items.len() == elements.len() => items
                    .iter()
                    .zip(elements.iter())
                    .all(|(item, pattern)| self.known_value_matches_pattern(item, pattern)),
                _ => false,
            },
            Pattern::Constructor { path, args, .. } => match value {
                KnownValue::Constructor {
                    path: value_path,
                    args: value_args,
                } if value_path == path && value_args.len() == args.len() => value_args
                    .iter()
                    .zip(args.iter())
                    .all(|(item, pattern)| self.known_value_matches_pattern(item, pattern)),
                _ => false,
            },
        }
    }

    fn check_match_case(&mut self, case: &MatchCase, value_ty: &Ty) -> Ty {
        self.push_scope();
        self.bind_pattern(&case.pattern, value_ty);
        if let Some(guard) = &case.guard {
            let guard_ty = self.check_expr(guard);
            self.require_bool(&guard_ty, guard.span(), "match guard must be Bool");
        }
        let ty = match &case.body {
            MatchCaseBody::Block(block) => self.check_block(block),
            MatchCaseBody::Expr(expr) => self.check_expr(expr),
        };
        self.pop_scope();
        ty
    }

    fn check_for_binding(&mut self, binding: &ForBinding) {
        if let Some(pattern) = &binding.pattern {
            let value_ty = if let Some(iterable) = &binding.iterable {
                let iterable_ty = self.check_expr(iterable);
                self.iterable_item_type(&iterable_ty)
            } else {
                binding
                    .values
                    .first()
                    .map(|expr| self.check_expr(expr))
                    .unwrap_or(Ty::Unknown)
            };
            let source = binding
                .iterable
                .as_ref()
                .or_else(|| binding.values.first())
                .expect("pattern-based for binding should have a source");
            self.require_irrefutable_for_pattern(pattern, &value_ty, source, pattern.span());
            self.bind_pattern(pattern, &value_ty);
            return;
        }
        let slot_types = if let Some(iterable) = &binding.iterable {
            let iterable_ty = self.check_expr(iterable);
            let item_ty = self.iterable_item_type(&iterable_ty);
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
                "brace destructuring requires a class or anonymous record value, got '{}'",
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
        let value_types = assignment
            .values
            .iter()
            .map(|expr| self.check_expr(expr))
            .collect::<Vec<_>>();
        for (index, target) in assignment.targets.iter().enumerate() {
            let actual = value_types.get(index).cloned().unwrap_or(Ty::Unknown);
            let expected = self.assignment_target_type(target, assignment.operator);
            self.require_assignable(
                &actual,
                &expected,
                target.span(),
                "invalid_assignment_type",
                format!(
                    "cannot assign value of type '{}' to target of type '{}'",
                    actual.describe(),
                    expected.describe()
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
                self.check_expr(index);
                self.index_result_type(&receiver_ty)
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

    fn expected_shape_fields(&self, expected: &Ty) -> Option<(String, Vec<FieldSig>)> {
        match expected {
            Ty::Record(fields) => Some((
                "anonymous shape".to_string(),
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
            )),
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
                Some((
                    format!("shape '{}'", sig.name),
                    sig.fields
                        .iter()
                        .filter(|field| !field.hidden)
                        .map(|field| FieldSig {
                            name: field.name.clone(),
                            ty: substitute_type(&field.ty, &subst),
                            mutable: field.mutable,
                            hidden: field.hidden,
                            has_initializer: field.has_initializer,
                            variadic: false,
                        })
                        .collect(),
                ))
            }
            _ => None,
        }
    }

    fn check_tuple_literal_against_shape(
        &mut self,
        items: &[Expr],
        expected: &Ty,
        span: crate::source::Span,
    ) -> Option<Ty> {
        let Some((label, fields)) = self.expected_shape_fields(expected) else {
            if let Ty::Named(name, _) = expected {
                if let Some(sig) = self.lookup_any_type(name) {
                    if sig.kind == TypeKind::Class {
                        for item in items {
                            self.check_expr(item);
                        }
                        self.add_error(
                            "invalid_tuple_shape_conversion",
                            format!(
                                "tuple values cannot construct class '{}'; use '{}(...)' or '{} {{ ... }}'",
                                sig.name, sig.name, sig.name
                            ),
                            span,
                        );
                        return Some(materialize_type(expected));
                    }
                }
            }
            return None;
        };

        if items.len() != fields.len() {
            self.add_error(
                "invalid_argument_count",
                format!(
                    "tuple construction for {} expects {} values, got {}",
                    label,
                    fields.len(),
                    items.len()
                ),
                span,
            );
        }

        for (item, field) in items.iter().zip(fields.iter()) {
            let actual = self.check_expr_against(item, &field.ty);
            self.require_assignable(
                &actual,
                &field.ty,
                item.span(),
                "invalid_argument_type",
                format!(
                    "tuple field '{}' for {} expects '{}' but got '{}'",
                    field.name,
                    label,
                    field.ty.describe(),
                    actual.describe()
                ),
            );
        }

        Some(materialize_type(expected))
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
                    "'_' is not a valid expression here; use an explicit lambda like 'x -> ...'",
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
            Expr::ListLiteral { items, .. } => {
                let mut item_ty = Ty::Unknown;
                for item in items {
                    let current = self.check_expr(item);
                    item_ty = join_types(&item_ty, &current);
                }
                Ty::list(item_ty)
            }
            Expr::TupleLiteral { items, span } => {
                if let Some(ty) = self.check_tuple_literal_against_shape(items, expected, *span) {
                    ty
                } else {
                    Ty::Tuple(items.iter().map(|item| self.check_expr(item)).collect())
                }
            }
            Expr::Call {
                callee,
                args,
                uses_brace_syntax,
                span,
            } => self.check_call(callee, args, *uses_brace_syntax, *span),
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
                receiver, index, ..
            } => {
                let receiver_ty = self.check_expr(receiver);
                let index_ty = self.check_expr(index);
                let valid_index = match &receiver_ty {
                    Ty::Named(name, args)
                        if (name == "Array" || name == "List") && args.len() == 1 =>
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
                updates,
                span,
            } => {
                let base = self.check_expr(receiver);
                self.check_record_update(&base, updates, *span);
                base
            }
            Expr::RecordLiteral { fields, values, .. } => {
                if !fields.is_empty() {
                    let expected_fields = match expected {
                        Ty::Record(fields) => fields.as_slice(),
                        _ => &[],
                    };
                    return Ty::Record(
                        fields
                            .iter()
                            .filter_map(|field| {
                                field.name.as_ref().map(|name| {
                                    let annotated_ty =
                                        field.ty.as_ref().map(|ty| self.ty_from_type_ref(ty));
                                    let expected_ty =
                                        annotated_ty.clone().unwrap_or_else(|| {
                                            expected_fields
                                                .iter()
                                                .find(|(expected_name, _)| expected_name == name)
                                                .map(|(_, ty)| ty.clone())
                                                .unwrap_or(Ty::Unknown)
                                        });
                                    let actual = self.check_expr_against(&field.value, &expected_ty);
                                    if let Some(annotated_ty) = annotated_ty {
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
                                        (name.clone(), annotated_ty)
                                    } else {
                                        (name.clone(), actual)
                                    }
                                })
                            })
                            .collect(),
                    );
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
                            "positional anonymous shape construction requires an expected shape type; assign a tuple to an explicitly typed shape",
                            expr.span(),
                        );
                    }
                    Ty::Record(Vec::new())
                }
            }
            Expr::AnonymousInterface { .. } => Ty::Unknown,
            Expr::Try { value, span } => {
                if self.current_return == Ty::Unknown {
                    self.add_error("invalid_try", "try used outside callable body", *span);
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
                        *span,
                    );
                } else if !self.try_propagates_from(&value_ty, &self.current_return) {
                    self.add_error(
                        "invalid_try",
                        format!(
                            "try on '{}' cannot propagate from enclosing return type '{}'",
                            value_ty.describe(),
                            self.current_return.describe()
                        ),
                        *span,
                    );
                }
                inner
            }
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
                self.ty_from_type_ref(target);
                Ty::bool()
            }
            Expr::If {
                condition,
                then_block,
                else_branch,
                ..
            } => {
                let cond_ty = self.check_expr(condition);
                self.require_bool(&cond_ty, condition.span(), "if condition must be Bool");
                let then_ty = self.check_block(then_block);
                let else_ty = self.check_else_expr_branch(else_branch);
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
                for case in cases {
                    let current = self.check_match_case(case, &value_ty);
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
                for binding in bindings {
                    self.check_for_binding(binding);
                }
                let yield_ty = self.check_block(yield_body);
                self.loop_depth -= 1;
                self.pop_scope();
                Ty::list(yield_ty)
            }
            Expr::Lambda { params, body, .. } => self.check_lambda_expr(params, body, expected),
            Expr::Group { inner, .. } => self.check_expr(inner),
        }
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
    ) -> Ty {
        if uses_brace_syntax
            && !self.brace_call_uses_structural_construction(callee)
            && !self.brace_call_targets_current_constructor(callee)
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
        if let Some(ty) = self.check_builtin_static_method_call(callee, &normalized_args, span) {
            return ty;
        }
        if let Some(ty) = self.check_builtin_static_factory_call(callee, &normalized_args, span) {
            return ty;
        }
        if let Some(ty) =
            self.try_check_constructor_call(callee, &normalized_args, uses_brace_syntax, span)
        {
            return ty;
        }
        if let Some((params, ret)) =
            self.callable_signature_for_args(callee, &normalized_args, uses_brace_syntax)
        {
            return self.check_signature_call(&params, &ret, &normalized_args, span);
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

    fn reject_parenthesized_named_constructor_args(
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
            "constructor parentheses accept positional arguments only; use braces for named constructor arguments",
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
            "List" => "List",
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
                                            "Array.generate expects (Int) -> T generator, got '{}'",
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
                                        "Array.generate expects (Int) -> T generator, got '{}'",
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
            Expr::Call { .. } | Expr::Try { .. } | Expr::Unit { .. } | Expr::ForYield { .. } => {}
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
            Expr::Call { .. } | Expr::Try { .. } | Expr::Unit { .. } | Expr::ForYield { .. } => {}
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
            Stmt::ExpectCondition(_) => {}
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
            let slot = arrangement
                .slots
                .get(index)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            for arg in slot {
                let raw_expected =
                    call_arg_expected_ty(param.variadic, &param.ty, arg.name.is_some());
                let expected = substitute_type(&raw_expected, &subst);
                let actual = self.check_expr_against(&arg.value, &expected);
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
                    format!(
                        "argument has type '{}' but parameter expects '{}'",
                        actual.describe(),
                        expected.describe()
                    )
                },
            );
        }

        materialize_type(&substitute_type(ret, &subst))
    }

    fn try_check_constructor_call(
        &mut self,
        callee: &Expr,
        args: &[crate::ast::CallArg],
        uses_brace_syntax: bool,
        span: crate::source::Span,
    ) -> Option<Ty> {
        let structural_record_arg = call_uses_structural_record_arg(args, uses_brace_syntax);
        let parenthesized_record_arg =
            constructor_uses_parenthesized_record_arg(self, args, uses_brace_syntax);
        match callee {
            Expr::Identifier { name, .. } => {
                if !structural_record_arg {
                    if let Some(ty) =
                        self.check_builtin_constructor(name, args, span, uses_brace_syntax)
                    {
                        return Some(ty);
                    }
                }
                if name == "new"
                    && self.current_method.as_deref() == Some("new")
                    && self.current_owner.is_some()
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
                if let Some(case) = self.world.lookup_enum_case(self.module, name) {
                    return Some(self.check_enum_case_constructor_signature(
                        name,
                        &case,
                        args,
                        span,
                        uses_brace_syntax,
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
                    if let Some(sig) = module_info.singles.get(&member).cloned() {
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
                            ));
                        }
                        if sig.kind == TypeKind::Single {
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
                            ));
                        }
                    }
                }
                None
            }
            _ => None,
        }
    }

    fn check_builtin_constructor(
        &mut self,
        name: &str,
        args: &[crate::ast::CallArg],
        span: crate::source::Span,
        uses_brace_syntax: bool,
    ) -> Option<Ty> {
        match name {
            "Range" => {
                self.reject_parenthesized_named_constructor_args(args, uses_brace_syntax, span);
                if !(args.len() == 2 || args.len() == 3) {
                    self.add_error(
                        "invalid_argument_count",
                        format!(
                            "Range constructor expects 2 or 3 arguments, got {}",
                            args.len()
                        ),
                        span,
                    );
                }
                for arg in args {
                    let ty = self.check_expr(&arg.value);
                    self.require_assignable(
                        &ty,
                        &Ty::int(),
                        arg.span,
                        "invalid_argument_type",
                        format!(
                            "Range constructor arguments must be Int, got '{}'",
                            ty.describe()
                        ),
                    );
                }
                Some(Ty::Named("IntRange".to_string(), Vec::new()))
            }
            "List" => {
                self.reject_parenthesized_named_constructor_args(args, uses_brace_syntax, span);
                let mut item = Ty::Unknown;
                for arg in args {
                    item = join_types(&item, &self.check_expr(&arg.value));
                }
                Some(Ty::Named("List".to_string(), vec![item]))
            }
            "Set" => {
                self.reject_parenthesized_named_constructor_args(args, uses_brace_syntax, span);
                let mut item = Ty::Unknown;
                for arg in args {
                    item = join_types(&item, &self.check_expr(&arg.value));
                }
                Some(Ty::Named("Set".to_string(), vec![item]))
            }
            "Array" => {
                self.reject_parenthesized_named_constructor_args(args, uses_brace_syntax, span);
                let mut item = Ty::Unknown;
                for arg in args {
                    item = join_types(&item, &self.check_expr(&arg.value));
                }
                Some(Ty::Named("Array".to_string(), vec![item]))
            }
            "Map" => {
                self.reject_parenthesized_named_constructor_args(args, uses_brace_syntax, span);
                let mut key = Ty::Unknown;
                let mut value = Ty::Unknown;
                for arg in args {
                    match &arg.value {
                        Expr::Binary {
                            left,
                            op: BinaryOp::Colon,
                            right,
                            ..
                        } => {
                            key = join_types(&key, &self.check_expr(left));
                            value = join_types(&value, &self.check_expr(right));
                        }
                        _ => match self.check_expr(&arg.value) {
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
                        },
                    }
                }
                Some(Ty::Named("Map".to_string(), vec![key, value]))
            }
            _ => None,
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
                    "constructor syntax for '{}' does not accept anonymous shape arguments in '(...)'; use named brace arguments or positional values directly",
                    sig.name
                ),
                span,
            );
            return ret;
        }

        self.reject_parenthesized_named_constructor_args(args, uses_brace_syntax, span);

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
            if let Some(ctor) = self.choose_overload(&visible, args) {
                let params = ctor
                    .params
                    .iter()
                    .map(|param| FieldSig {
                        name: param.name.clone(),
                        ty: param.ty.clone(),
                        mutable: false,
                        hidden: false,
                        has_initializer: param.has_initializer,
                        variadic: param.variadic,
                    })
                    .collect::<Vec<_>>();
                return self.check_constructor_signature(&params, &ret, args, span);
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
                        "{} '{}' brace construction requires named fields",
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
                    let raw_expected =
                        call_arg_expected_ty(param.variadic, &param.ty, arg.name.is_some());
                    let expected = substitute_type(&raw_expected, &subst);
                    let actual = self.check_expr_against(&arg.value, &expected);
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
                    format!(
                        "constructor argument has type '{}' but expects '{}'",
                        actual.describe(),
                        expected.describe()
                    ),
                );
            }
            return materialize_type(&substitute_type(ret, &subst));
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
                let raw_expected =
                    call_arg_expected_ty(param.variadic, &param.ty, arg.name.is_some());
                let expected = substitute_type(&raw_expected, &subst);
                let actual = self.check_expr_against(&arg.value, &expected);
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
                    format!(
                        "constructor argument has type '{}' but expects '{}'",
                        actual.describe(),
                        expected.describe()
                    )
                } else {
                    format!(
                        "argument for '{}' has type '{}' but expects '{}'",
                        field_name,
                        actual.describe(),
                        expected.describe()
                    )
                },
            );
        }

        materialize_type(&substitute_type(ret, &subst))
    }

    fn check_enum_case_constructor_signature(
        &mut self,
        case_name: &str,
        case: &EnumCaseSig,
        args: &[crate::ast::CallArg],
        span: crate::source::Span,
        uses_brace_syntax: bool,
    ) -> Ty {
        if case.field_count == 0 && args.is_empty() {
            self.add_error(
                "invalid_enum_case_call",
                format!("enum case '{case_name}' does not accept call syntax; use '{case_name}'"),
                span,
            );
            return case.result.clone();
        }
        self.reject_parenthesized_named_constructor_args(args, uses_brace_syntax, span);
        self.check_constructor_signature(&case.params, &case.result, args, span)
    }

    fn callable_signature_for_args(
        &mut self,
        callee: &Expr,
        args: &[crate::ast::CallArg],
        _uses_brace_syntax: bool,
    ) -> Option<(Vec<ParamSig>, Ty)> {
        match callee {
            Expr::Identifier { name, .. } => {
                if let Some(functions) = self.lookup_functions(name) {
                    let sig = self
                        .choose_overload(&functions, args)
                        .or_else(|| functions.first())?
                        .clone();
                    return Some((sig.params, sig.ret));
                }
                if let Some(methods) = self.lookup_implicit_method_functions(name) {
                    let sig = self
                        .choose_overload(&methods, args)
                        .or_else(|| methods.first())?
                        .clone();
                    return Some((sig.params, sig.ret));
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
                        let sig = self
                            .choose_overload(functions, args)
                            .or_else(|| functions.first())?
                            .clone();
                        return Some((sig.params, sig.ret));
                    }
                }
                if let Some(sigs) = self.static_method_sigs(receiver, name) {
                    let sig = self
                        .choose_overload(&sigs, args)
                        .or_else(|| sigs.first())?
                        .clone();
                    return Some((sig.params, sig.ret));
                }
                let receiver_ty = self.check_expr(receiver);
                let methods = self.member_method_sigs(&receiver_ty, name)?;
                let method = self
                    .choose_overload(&methods, args)
                    .or_else(|| methods.first())?
                    .clone();
                Some((method.params, method.ret))
            }
            _ => None,
        }
    }

    fn callable_signature_for_args_probe(
        &self,
        callee: &Expr,
        args: &[crate::ast::CallArg],
        _uses_brace_syntax: bool,
    ) -> Option<(Vec<ParamSig>, Ty)> {
        match callee {
            Expr::Identifier { name, .. } => {
                if let Some(functions) = self.lookup_functions(name) {
                    let sig = self
                        .choose_overload(&functions, args)
                        .or_else(|| functions.first())?
                        .clone();
                    return Some((sig.params, sig.ret));
                }
                let methods = self.lookup_implicit_method_functions(name)?;
                let sig = self
                    .choose_overload(&methods, args)
                    .or_else(|| methods.first())?
                    .clone();
                Some((sig.params, sig.ret))
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
                        return Some((sig.params, sig.ret));
                    }
                }
                if let Some(sigs) = self.static_method_sigs(receiver, name) {
                    let sig = self
                        .choose_overload(&sigs, args)
                        .or_else(|| sigs.first())?
                        .clone();
                    return Some((sig.params, sig.ret));
                }
                let receiver_ty = self.probe_expr_type(receiver);
                let methods = self.member_method_sigs(&receiver_ty, name)?;
                let method = self
                    .choose_overload(&methods, args)
                    .or_else(|| methods.first())?
                    .clone();
                Some((method.params, method.ret))
            }
            _ => None,
        }
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
                || self.brace_call_targets_enum_case(callee))
        {
            if let Some(args) = brace_record_constructor_args(args) {
                return args;
            }
        }
        args.to_vec()
    }

    fn brace_call_uses_structural_construction(&self, callee: &Expr) -> bool {
        self.brace_call_type_sig(callee)
            .is_some_and(|sig| matches!(sig.kind, TypeKind::Class | TypeKind::Record))
    }

    fn brace_call_targets_explicit_constructor(&self, callee: &Expr) -> bool {
        self.brace_call_type_sig(callee)
            .is_some_and(|sig| sig.kind == TypeKind::Class && sig.methods.contains_key("new"))
    }

    fn brace_call_targets_current_constructor(&self, callee: &Expr) -> bool {
        matches!(callee, Expr::Identifier { name, .. } if name == "new")
            && self.current_method.as_deref() == Some("new")
            && self.current_owner.is_some()
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
                .or_else(|| self.world.ambient.types.get(name).cloned()),
            Expr::Member { .. } => module_alias_and_member(callee).and_then(|(alias, member)| {
                self.world
                    .lookup_module_alias(self.module, &alias)
                    .and_then(|module| module.types.get(&member).cloned())
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
        if let Some(sig) = self.lookup_any_single(type_name) {
            if let Some(methods) = self.method_sigs_for_type(&sig, name) {
                let first = methods.first()?;
                return Some(Ty::Function(
                    first.params.iter().map(|param| param.ty.clone()).collect(),
                    Box::new(first.ret.clone()),
                ));
            }
        }
        let sig = self.lookup_any_non_single_type(type_name)?;
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
        if let Some(sig) = self.lookup_any_single(type_name) {
            return self.method_sigs_for_type(&sig, name);
        }
        let sig = self.lookup_any_non_single_type(type_name)?;
        self.method_sigs_for_type(&sig, name)
    }

    fn hidden_constructor_factory_help(
        &self,
        class_name: &str,
        args: &[crate::ast::CallArg],
    ) -> Option<String> {
        let single = self.lookup_any_single(class_name)?;
        self.method_sigs_for_type(&single, "create")?;
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
            BinaryOp::RecordMerge => self.check_record_merge_expr(left, right, span),
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
            BinaryOp::Eq
            | BinaryOp::NotEq
            | BinaryOp::Less
            | BinaryOp::LessEq
            | BinaryOp::Greater
            | BinaryOp::GreaterEq
            | BinaryOp::And
            | BinaryOp::Or
            | BinaryOp::BitAnd
            | BinaryOp::BitOr => {
                if matches!(op, BinaryOp::And | BinaryOp::Or) {
                    self.require_bool(left, span, "logical operator expects Bool operands");
                    self.require_bool(right, span, "logical operator expects Bool operands");
                }
                Ty::bool()
            }
            BinaryOp::Colon => Ty::Unknown,
        }
    }

    fn check_else_expr_branch(&mut self, branch: &ElseExprBranch) -> Ty {
        match branch {
            ElseExprBranch::If(expr) => self.check_expr(expr),
            ElseExprBranch::Block(block) => self.check_block(block),
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
                if let Some(name) = name {
                    self.define_local(name, target_ty, false);
                }
            }
            Pattern::Literal { value, .. } => {
                self.check_expr(value);
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
            Pattern::Constructor { path, args, .. } => {
                let case_name = path.last().cloned().unwrap_or_default();
                if let Some(case) = self.lookup_case_by_pattern(path, scrutinee) {
                    let mut subst = HashMap::new();
                    infer_type_subst(&case.result, scrutinee, &mut subst);
                    for (pattern, param) in args.iter().zip(case.params.iter()) {
                        self.bind_pattern(
                            pattern,
                            &materialize_type(&substitute_type(&param.ty, &subst)),
                        );
                    }
                } else if let Some(fields) = self.lookup_destructured_type_fields(path) {
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
                .lookup_any_non_single_type(type_name)
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
                        .or_else(|| module.singles.get(name).cloned())
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

    fn unwrap_inner_type(&self, ty: &Ty) -> Ty {
        match ty {
            Ty::Named(name, args) if name == "Option" && args.len() == 1 => args[0].clone(),
            Ty::Named(name, args) if name == "Result" && args.len() >= 1 => args[0].clone(),
            Ty::Named(name, args) if name == "Either" && args.len() == 2 => args[1].clone(),
            _ => Ty::Unknown,
        }
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

    fn iterable_item_type(&self, ty: &Ty) -> Ty {
        match ty {
            Ty::Named(name, args)
                if (name == "List"
                    || name == "Set"
                    || name == "Array"
                    || name == "Iterable"
                    || name == "Iterator")
                    && args.len() == 1 =>
            {
                args[0].clone()
            }
            Ty::Named(name, args) if name == "IntRange" && args.is_empty() => Ty::int(),
            Ty::Unknown => Ty::Unknown,
            _ => Ty::Unknown,
        }
    }

    fn index_result_type(&self, ty: &Ty) -> Ty {
        match ty {
            Ty::Named(name, args) if (name == "Array" || name == "List") && args.len() == 1 => {
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
        match receiver {
            Ty::Named(type_name, args) => {
                let sig = self.lookup_any_type(type_name)?;
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
                None
            }
            Ty::Record(fields) => fields
                .iter()
                .find(|(field_name, _)| field_name == name)
                .map(|(_, ty)| ty.clone()),
            Ty::Unknown => Some(Ty::Unknown),
            _ => None,
        }
    }

    fn can_access_hidden_constructor(&self, owner: &TypeSig) -> bool {
        self.current_owner
            .as_ref()
            .is_some_and(|current| current.name == owner.name)
    }

    fn member_method_sigs(&self, receiver: &Ty, name: &str) -> Option<Vec<FunctionSig>> {
        match receiver {
            Ty::Named(type_name, args) => {
                let sig = self.lookup_any_type(type_name)?;
                let methods = self.method_sigs_for_type(&sig, name)?;
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
                            params: method
                                .params
                                .into_iter()
                                .map(|param| ParamSig {
                                    name: param.name,
                                    ty: substitute_type(&param.ty, &subst),
                                    variadic: param.variadic,
                                    has_initializer: param.has_initializer,
                                })
                                .collect(),
                            ret: substitute_type(&method.ret, &subst),
                            visibility: method.visibility,
                        })
                        .collect(),
                )
            }
            _ => None,
        }
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
        if let Some(sig) = module.singles.get(&member) {
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
            Expr::Member { receiver, name, .. } => name == "panic" && path_starts_with_os(receiver),
            _ => false,
        }
    }

    fn choose_overload<'b>(
        &self,
        overloads: &'b [FunctionSig],
        args: &[crate::ast::CallArg],
    ) -> Option<&'b FunctionSig> {
        let arg_types = args
            .iter()
            .map(|arg| self.probe_expr_type(&arg.value))
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
                        let expected =
                            call_arg_expected_ty(param.variadic, &param.ty, arg.name.is_some());
                        if !matches!(actual, Ty::Unknown) {
                            if self.is_assignable(actual, &expected) {
                                score += 2;
                            } else {
                                return None;
                            }
                        }
                        if arg.name.as_deref() == Some(param.name.as_str()) {
                            score += 1;
                        }
                    }
                }
                Some((score, sig))
            })
            .max_by_key(|(score, _)| *score)
            .map(|(_, sig)| sig)
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
            Expr::RecordLiteral { fields, .. } => Ty::Record(
                fields
                    .iter()
                    .filter_map(|field| {
                        field.name.as_ref().map(|name| {
                            let ty = field
                                .ty
                                .as_ref()
                                .map(|ty| self.ty_from_type_ref(ty))
                                .unwrap_or_else(|| self.probe_expr_type(&field.value));
                            (name.clone(), ty)
                        })
                    })
                    .collect(),
            ),
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
                    "{} '{}' uses named brace construction; write '{}(...)' for positional construction",
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
                    "{} '{}' has no implicit named-field constructor because hidden field '{}' has no initializer; define 'new' to initialize it",
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
                    "{} '{}' named brace construction expects {}..{} visible fields, got {}",
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
                        "{} '{}' requires named brace fields that match the visible shape",
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
                        "{} '{}' has no visible field '{}' for named brace construction",
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
                        "{} '{}' named brace construction is missing required field '{}'",
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

        let uses_named_args = args.iter().any(|arg| arg.name.is_some());
        if !uses_named_args
            && sig.fields.iter().enumerate().any(|(index, field)| {
                field.hidden
                    && field.has_initializer
                    && sig.fields[index + 1..].iter().any(|later| !later.hidden)
            })
        {
            self.add_error(
                "no_matching_overload",
                format!(
                    "{} '{}' cannot use positional construction because private defaulted fields must come after all public fields",
                    type_kind_label(sig.kind),
                    sig.name
                ),
                span,
            );
            return materialize_type(ret);
        }

        let visible_fields = sig
            .fields
            .iter()
            .filter(|field| !field.hidden)
            .cloned()
            .collect::<Vec<_>>();

        if uses_named_args {
            return self.check_constructor_signature(&visible_fields, ret, args, span);
        }

        if args.len() > visible_fields.len()
            || visible_fields[args.len()..]
                .iter()
                .any(|field| !field.has_initializer)
        {
            let required_visible = visible_fields
                .iter()
                .filter(|field| !field.has_initializer)
                .count();
            self.add_error(
                "invalid_argument_count",
                format!(
                    "{} '{}' positional construction expects {}..{} arguments, got {}",
                    type_kind_label(sig.kind),
                    sig.name,
                    required_visible,
                    visible_fields.len(),
                    args.len()
                ),
                span,
            );
            return materialize_type(ret);
        }

        for (arg, field) in args.iter().zip(visible_fields.iter()) {
            let actual = self.check_expr_against(&arg.value, &field.ty);
            if !self.is_assignable(&actual, &field.ty) {
                self.add_error(
                    "invalid_argument_type",
                    format!(
                        "positional field '{}' in {} '{}' expects '{}' but got '{}'",
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

        materialize_type(ret)
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
        if let Some(sig) = self.world.ambient.singles.get(name) {
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
            .is_empty()
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
            .or_else(|| self.module.singles.get(name).cloned())
    }

    fn lookup_single_local(&self, name: &str) -> Option<TypeSig> {
        self.module.singles.get(name).cloned()
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
                    .or_else(|| module.singles.get(name).cloned())
            })
            .collect::<Vec<_>>();
        if matches.len() == 1 {
            matches.pop()
        } else {
            None
        }
    }

    fn lookup_unique_module_single(&self, name: &str) -> Option<TypeSig> {
        let mut matches = self
            .world
            .modules
            .values()
            .filter_map(|module| module.singles.get(name).cloned())
            .collect::<Vec<_>>();
        if matches.len() == 1 {
            matches.pop()
        } else {
            None
        }
    }

    fn lookup_any_single(&self, name: &str) -> Option<TypeSig> {
        self.lookup_single_local(name)
            .or_else(|| {
                self.module.symbol_imports.get(name).and_then(|imported| {
                    (imported.kind == ImportedKind::Single)
                        .then(|| self.world.modules.get(&imported.module_path))
                        .flatten()
                        .and_then(|module| module.singles.get(&imported.original_name).cloned())
                })
            })
            .or_else(|| self.lookup_unique_module_single(name))
            .or_else(|| self.world.ambient.singles.get(name).cloned())
    }

    fn lookup_any_non_single_type(&self, name: &str) -> Option<TypeSig> {
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
        self.lookup_type_local(name)
            .or_else(|| self.world.lookup_imported_type(self.module, name))
            .or_else(|| self.lookup_unique_module_type(name))
            .or_else(|| self.world.ambient.types.get(name).cloned())
            .or_else(|| self.world.ambient.singles.get(name).cloned())
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
        self.define_local_known(name, ty, mutable, None);
    }

    fn define_local_known(&mut self, name: &str, ty: Ty, mutable: bool, known: Option<KnownValue>) {
        if name == "_" {
            return;
        }
        if self.scopes.is_empty() {
            self.push_scope();
        }
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name.to_string(), ValueInfo { ty, mutable, known });
        }
    }

    fn push_type_params<'b>(&mut self, params: impl Iterator<Item = &'b str>) {
        self.type_params
            .push(params.map(|name| name.to_string()).collect::<HashSet<_>>());
    }

    fn pop_type_params(&mut self) {
        self.type_params.pop();
    }

    fn is_type_param(&self, name: &str) -> bool {
        self.type_params
            .iter()
            .rev()
            .any(|scope| scope.contains(name))
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

    fn shape_target_fields(&self, expected: &Ty) -> Option<Vec<(String, Ty)>> {
        match expected {
            Ty::Record(fields) => Some(fields.clone()),
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
                        .map(|field| (field.name.clone(), substitute_type(&field.ty, &subst)))
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

        if let Ty::Tuple(items) = actual {
            return items.len() == expected_fields.len()
                && items
                    .iter()
                    .zip(expected_fields.iter())
                    .all(|(actual, (_, expected))| self.is_assignable(actual, expected));
        }

        let Some(actual_fields) = self.structural_fields_for_type(actual) else {
            return false;
        };

        expected_fields.iter().all(|(expected_name, expected_ty)| {
            actual_fields
                .iter()
                .find(|(actual_name, _)| actual_name == expected_name)
                .is_some_and(|(_, actual_ty)| self.is_assignable(actual_ty, expected_ty))
        })
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

fn constructor_body_delegates(body: &CallableBody) -> bool {
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

fn is_constructor_delegation_expr(expr: &Expr) -> bool {
    match expr {
        Expr::Call { callee, .. } => {
            matches!(callee.as_ref(), Expr::Identifier { name, .. } if name == "new")
        }
        Expr::Group { inner, .. } => is_constructor_delegation_expr(inner),
        _ => false,
    }
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
                has_initializer: param.initializer.is_some(),
            })
            .collect(),
        ret: function
            .return_type
            .as_ref()
            .map(|ty| convert_type_ref(ty, &type_params))
            .unwrap_or(Ty::Unknown),
        visibility: function.visibility,
    }
}

fn function_sig_from_method(method: &MethodDecl, owner_type_params: &[String]) -> FunctionSig {
    let type_params = method
        .type_params
        .iter()
        .map(|param| param.name.clone())
        .chain(owner_type_params.iter().cloned())
        .collect::<HashSet<_>>();
    FunctionSig {
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
                has_initializer: param.initializer.is_some(),
            })
            .collect(),
        ret: method
            .return_type
            .as_ref()
            .map(|ty| convert_type_ref(ty, &type_params))
            .unwrap_or(Ty::Unknown),
        visibility: method.visibility,
    }
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
                    .filter(|field| field.ty.is_some() && field.initializer.is_none())
                    .map(|field| FieldSig {
                        name: field.name.clone(),
                        ty: field
                            .ty
                            .as_ref()
                            .map(|ty| convert_type_ref(ty, &owner_params))
                            .unwrap_or(Ty::Unknown),
                        mutable: field.mutable,
                        hidden: field.visibility == Visibility::Hidden,
                        has_initializer: false,
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
            Some(Ty::Named("List".to_string(), vec![item]))
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
    matches!(reference, TypeRef::Named { name, args, .. } if name == "List" && args.len() == 1)
}

fn variadic_arg_ty(ty: &Ty) -> Option<Ty> {
    match ty {
        Ty::Named(name, args) if name == "List" && args.len() == 1 => args.first().cloned(),
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

    if values.is_empty() {
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
        Ty::TypeParam(name) => {
            let actual = materialize_type(actual);
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

fn materialize_type(ty: &Ty) -> Ty {
    match ty {
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

fn is_assignable(actual: &Ty, expected: &Ty) -> bool {
    if matches!(actual, Ty::Unknown) || matches!(expected, Ty::Unknown) {
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
        (Ty::TypeParam(left), Ty::TypeParam(right)) => left == right,
        (Ty::Named(left, left_args), Ty::Named(right, right_args)) => {
            left == right
                && left_args.len() == right_args.len()
                && left_args
                    .iter()
                    .zip(right_args.iter())
                    .all(|(left, right)| is_assignable(left, right))
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
                    "no implicit named-field constructor because hidden field 'token' has no initializer",
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
}

impl SecretUser {
    new {
        name Str
    } {
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
}

impl SecretUser {
    new {
        name Str
    } {
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
}

impl User {
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
    items = List(1, 2, 3)
    mapped = items.map { item -> item + 1 }
    OS.println(mapped.size())
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn allows_impl_single_without_explicit_single_decl() {
        let program = parse_inline(
            r#"
class User {
    name Str
}

impl single User {
    def make(name Str) User = User { name: name }
}

def main() User = User.make("Ada")
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn allows_bare_method_calls_inside_impls() {
        let program = parse_inline(
            r#"
class Counter {
    value Int
}

impl Counter {
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
    fn allows_bare_field_access_inside_impls_when_not_shadowed() {
        let program = parse_inline(
            r#"
class Counter {
    var count Int
}

impl Counter {
    new {
        initial Int
    } {
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
    values Array[Int] = Array.generate(3, idx -> idx + 1)
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn allows_expect_condition_statement() {
        let program = parse_inline(
            r#"
def main() Unit {
    expect 1 + 2 == 3
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
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
    values = List(1, 2, 3)
    removed Option[Int] = values.removeFirst()
    total Int = values.reduce(0, (acc, value) -> acc + value)
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn rejects_non_bool_expect_condition() {
        let program = parse_inline(
            r#"
def main() Unit {
    expect 1
}
"#,
        );
        let result = check_program(&program);
        assert!(
            result.diagnostics.iter().any(|diag| {
                diag.code == "invalid_condition_type"
                    && diag.message.contains("expect condition must be Bool")
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
}

impl User {
    new {
        name Str
    } {
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
}

impl User {
    new {
        name Str
        age Int = 0
    } {
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
}

impl Path {
    new {
        segments [Str] vararg
    } {
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
}

impl Path {
    new {
        segments [Str] vararg = ["tmp"]
    } {
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
}

impl Bad {
    new {
        items [Int] vararg
        suffix Int
    } {
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
}

impl Bad {
    new {
        left [Int] vararg
        right [Int] vararg
    } {
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
    fn rejects_variadic_constructor_parameter_after_default() {
        let program = parse_inline(
            r#"
class Bad {
    items [Int]
}

impl Bad {
    new {
        prefix Int = 0
        items [Int] vararg
    } {
        this.items = items
    }
}
"#,
        );
        let result = check_program(&program);
        assert!(
            result.diagnostics.iter().any(|diag| {
                diag.code == "invalid_variadic_param"
                    && diag.message.contains(
                        "variadic constructor parameter cannot follow defaulted parameters",
                    )
            }),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn rejects_variadic_constructor_parameter_without_list_type() {
        let program = parse_inline(
            r#"
class Bad {
    items [Int]
}

impl Bad {
    new {
        items Int vararg
    } {
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
                        .contains("variadic constructor parameter must use a list type")
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
}

impl Path {
    new {
        segments [Str] vararg
    } {
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
}

impl User {
    hidden new {
        name Str
    } {
        this.name = name
    }
}

impl single User {
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
}

impl Counter {
    new {
        count Int
        name Str
    } {
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
}

impl Counter {
    new {
        count Int
    } {
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
    fn rejects_parenthesized_anonymous_record_type_construction() {
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
    fn allows_typed_anonymous_record_fields() {
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
    fn rejects_typed_anonymous_record_field_initializer_mismatch() {
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
    fn rejects_positional_construction_when_private_default_breaks_public_order() {
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
                .contains("private defaulted fields must come after all public fields")),
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
}

impl AssetPrices {
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
}

impl SecretUser {
    new {
        name Str
        token Str
        location Str
    } {
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
    fn allows_tuple_to_anonymous_shape_assignment() {
        let program = parse_inline(
            r#"
def main() Int {
    point { x Int, y Int } = (4, 5)
    point.x + point.y
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn allows_tuple_to_named_shape_assignment() {
        let program = parse_inline(
            r#"
shape Point {
    x Int
    y Int
}

def main() Int {
    point Point = (4, 5)
    point.x + point.y
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
}

impl PointView {
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
}

impl PointView {
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
    fn rejects_tuple_to_shape_with_wrong_field_type() {
        let program = parse_inline(
            r#"
def main() Unit {
    point { x Int, y Str } = (4, 5)
}
"#,
        );
        let result = check_program(&program);
        assert!(
            result.diagnostics.iter().any(|diag| {
                diag.code == "invalid_argument_type"
                    && diag
                        .message
                        .contains("tuple field 'y' for anonymous shape expects 'Str' but got 'Int'")
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
    let Some(item) = value else {
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
    items = List(1, 2, 3)
    mapped = items.map(_ + 1)
}
"#,
        );
        let result = check_program(&program);
        assert!(
            result.diagnostics.iter().any(|diag| {
                diag.code == "invalid_placeholder_expr" && diag.message.contains("explicit lambda")
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
    values List[MaybeInt] = [MaybeInt.SomeX(1), MaybeInt.NoneX]
    for value <- values {
        let SomeX(item) = value else continue
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
    let SomeX(item) = value else fail()
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
    expect eitherItem <- eitherValue
    expect {
        left <- optionValue
        middle <- resultValue
        right <- eitherValue
    }
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
    fn materializes_bare_enum_case_in_expected_record_field() {
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
    fn allows_plain_let_extract_when_source_is_known_success_case() {
        let program = parse_inline(
            r#"
def main(seed Int) Int {
    let item <- Some(seed)
    let resultItem <- Ok(item)
    let eitherItem <- Right(resultItem)
    let {
        grouped <- Some(eitherItem)
    }
    return grouped
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
def main(
    optionValue Option[Int],
    resultValue Result[Int, Str],
    eitherValue Either[Str, Int]
) Int {
    knownOption = Some(4)
    widenedOption Option[Int] = Some(5)
    let Some(optionItem) = optionValue
    let optionExtract <- optionValue
    let {
        Some(knownItem) = knownOption
        resultExtract <- resultValue
        eitherExtract <- eitherValue
    }
    let widenedItem <- widenedOption
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
                    && diag.message.contains("use 'let ... else ...' instead")
            })
            .count();
        assert_eq!(matches, 6, "{:#?}", result.diagnostics);
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
    values List[Option[Int]] = [Some(1), None]
    for Some(value) <- values {
        OS.println(value)
    }
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
    fn checks_parity_examples() {
        let root = workspace_root();
        let paths = [
            "examples/classes.lum",
            "examples/tuple_destructuring.lum",
            "examples/record_destructuring.lum",
            "examples/class_destructuring.lum",
            "examples/enums.lum",
            "examples/enum_single_same_name.lum",
            "examples/imports.lum",
            "examples/interface_default_methods.lum",
            "examples/list_hof.lum",
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
