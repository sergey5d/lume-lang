use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
};

use crate::{
    Diagnostic,
    ast::{
        AssignOp, AssignmentStmt, BinaryOp, Block, CallableBody, ElseBranch,
        ElseExprBranch, Expr, ForBinding, FunctionDecl, IfStmt, ImplBlock, Item, LambdaBody,
        MatchCase, MatchCaseBody, MethodDecl, Pattern, Program, Stmt, TypeDecl, TypeKind,
        TypeMember, TypeRef,
    },
    resolver::{
        ImportedKind, ImportedSymbol, LoadedModule, ModuleGraph, collect_module_order,
        find_stdlib_dir, load_module_graph, parse_program_from_path, read_directives,
        resolve_path,
    },
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
        world.checked_globals.insert(module_path.clone(), checked_globals);
        diagnostics.extend(module_diagnostics.into_iter().map(|diagnostic| {
            crate::resolver::LocatedDiagnostic {
                path: display_path.clone(),
                diagnostic,
            }
        }));
    }

    Ok(PathCheckResult { diagnostics })
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Ty {
    Unknown,
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

    fn unit() -> Self {
        Self::named("Unit")
    }

    fn describe(&self) -> String {
        match self {
            Ty::Unknown => "<unknown>".to_string(),
            Ty::Named(name, args) if args.is_empty() => name.clone(),
            Ty::Named(name, args) => format!(
                "{}[{}]",
                name,
                args.iter().map(Ty::describe).collect::<Vec<_>>().join(", ")
            ),
            Ty::Tuple(items) => format!(
                "({})",
                items.iter().map(Ty::describe).collect::<Vec<_>>().join(", ")
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
                params.iter().map(Ty::describe).collect::<Vec<_>>().join(", "),
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
}

#[derive(Debug, Clone)]
struct ParamSig {
    name: String,
    ty: Ty,
    variadic: bool,
}

#[derive(Debug, Clone)]
struct FunctionSig {
    params: Vec<ParamSig>,
    ret: Ty,
}

#[derive(Debug, Clone)]
struct FieldSig {
    name: String,
    ty: Ty,
    mutable: bool,
    has_initializer: bool,
}

#[derive(Debug, Clone)]
struct EnumCaseSig {
    params: Vec<FieldSig>,
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
    objects: HashMap<String, TypeSig>,
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
            objects: HashMap::new(),
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
            objects: HashMap::new(),
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
                Item::Impl(block) => self.merge_impl(block),
                Item::Statement(Stmt::Binding(binding)) => {
                    self.global_binding_stmts.push(binding.clone());
                }
                _ => {}
            }
        }
    }

    fn merge_impl(&mut self, block: &ImplBlock) {
        let Some(target_name) = type_ref_named_name(&block.target) else {
            return;
        };
        let target_type_params = impl_target_type_params(&block.target);
        if let Some(sig) = self.types.get_mut(target_name) {
            for method in &block.methods {
                sig.methods
                    .entry(method.name.clone())
                    .or_default()
                    .push(function_sig_from_method(method, &target_type_params));
            }
            return;
        }
        if let Some(sig) = self.objects.get_mut(target_name) {
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
    objects: HashMap<String, TypeSig>,
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
                    ambient.enum_cases.insert(case_name.clone(), case_sig.clone());
                }
                ambient.types.insert(name, sig);
            }
            for (name, sig) in module.objects {
                ambient.objects.insert(name, sig);
            }
        }

        if let Some(os) = ambient.objects.get("OS") {
            for builtin in ["print", "println", "printf", "panic"] {
                if let Some(sigs) = os.methods.get(builtin) {
                    ambient.functions.insert(builtin.to_string(), sigs.clone());
                }
            }
        }

        Ok(ambient)
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

    fn lookup_module_alias<'a>(&'a self, module: &ModuleInfo, alias: &str) -> Option<&'a ModuleInfo> {
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
    placeholder_hints: Vec<Ty>,
    current_return: Ty,
    current_owner: Option<TypeSig>,
    current_method: Option<String>,
    loop_depth: usize,
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
            placeholder_hints: Vec::new(),
            current_return: Ty::Unknown,
            current_owner: None,
            current_method: None,
            loop_depth: 0,
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
                .map(|expr| self.check_expr(expr))
                .collect::<Vec<_>>();
            let slot_types = self.binding_slot_types(&binding_stmt.bindings, &value_types);
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
                    },
                );
            }
        }
        self.pop_scope();
    }

    fn check_function(&mut self, function: &FunctionDecl) {
        let previous_return = self.current_return.clone();
        self.push_type_params(function.type_params.iter().map(|param| param.name.as_str()));
        let expected_return = function
            .return_type
            .as_ref()
            .map(|ty| self.ty_from_type_ref(ty))
            .unwrap_or(Ty::Unknown);
        self.current_return = expected_return.clone();
        self.push_scope();
        for param in &function.params {
            let ty = param
                .ty
                .as_ref()
                .map(|value| self.ty_from_type_ref(value))
                .unwrap_or(Ty::Unknown);
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
    }

    fn check_type_decl(&mut self, decl: &TypeDecl) {
        let Some(type_sig) = self.lookup_type_local(&decl.name) else {
            return;
        };
        self.push_type_params(type_sig.type_params.iter().map(String::as_str));

        for member in &decl.members {
            match member {
                TypeMember::Field(field) => {
                    if let Some(initializer) = &field.initializer {
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
                }
                TypeMember::Method(method) => self.check_method(method, &type_sig),
                TypeMember::Case(_) => {}
            }
        }

        self.pop_type_params();
    }

    fn check_impl(&mut self, block: &ImplBlock) {
        let Some(target_name) = type_ref_named_name(&block.target) else {
            return;
        };
        let Some(type_sig) = self.lookup_type_local(target_name) else {
            return;
        };
        self.push_type_params(type_sig.type_params.iter().map(String::as_str));
        for method in &block.methods {
            self.check_method(method, &type_sig);
        }
        self.pop_type_params();
    }

    fn check_method(&mut self, method: &MethodDecl, owner: &TypeSig) {
        let previous_return = self.current_return.clone();
        let previous_owner = self.current_owner.clone();
        let previous_method = self.current_method.clone();
        self.push_type_params(method.type_params.iter().map(|param| param.name.as_str()));
        let expected_return = method
            .return_type
            .as_ref()
            .map(|ty| self.ty_from_type_ref(ty))
            .unwrap_or(Ty::Unknown);
        self.current_return = expected_return.clone();
        self.current_owner = Some(owner.clone());
        self.current_method = Some(method.name.clone());
        self.push_scope();

        let self_ty = Ty::Named(
            owner.name.clone(),
            owner
                .type_params
                .iter()
                .map(|name| Ty::TypeParam(name.clone()))
                .collect(),
        );
        self.define_local("this", self_ty, false);
        for field in &owner.fields {
            self.define_local(&field.name, field.ty.clone(), field.mutable);
        }
        for param in &method.params {
            let ty = param
                .ty
                .as_ref()
                .map(|value| self.ty_from_type_ref(value))
                .unwrap_or(Ty::Unknown);
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

        self.pop_scope();
        self.pop_type_params();
        self.current_return = previous_return;
        self.current_owner = previous_owner;
        self.current_method = previous_method;
    }

    fn check_callable_body(&mut self, body: &CallableBody) -> Ty {
        match body {
            CallableBody::Expr(expr) => self.check_expr_against(expr, &self.current_return.clone()),
            CallableBody::Block(block) => self.check_block_against(block, &self.current_return.clone()),
        }
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
                last = ty;
            }
        }
        self.pop_scope();
        last
    }

    fn check_if_stmt_value(&mut self, stmt: &IfStmt, expected: &Ty) -> Ty {
        if let Some(value) = &stmt.binding_value {
            let value_ty = self.check_expr(value);
            let inner = self.unwrap_inner_type(&value_ty);
            let slot_types = self.destructure_slots(&inner, stmt.bindings.len());
            self.push_scope();
            for (index, binding) in stmt.bindings.iter().enumerate() {
                let inferred = slot_types
                    .get(index)
                    .cloned()
                    .unwrap_or(Ty::Unknown);
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
                let slot_types = self.binding_slot_types(&binding_stmt.bindings, &value_types);
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
                    self.define_local(&binding.name, ty, binding.mutable);
                }
                Ty::unit()
            }
            Stmt::Assignment(assignment) => {
                self.check_assignment(assignment);
                Ty::unit()
            }
            Stmt::If(stmt) => {
                self.check_if_stmt(stmt);
                Ty::unit()
            }
            Stmt::Match(stmt) => {
                let value_ty = self.check_expr(&stmt.value);
                for case in &stmt.cases {
                    self.check_match_case(case, &value_ty);
                }
                Ty::unit()
            }
            Stmt::While(stmt) => {
                let condition = self.check_expr(&stmt.condition);
                self.require_bool(&condition, stmt.condition.span(), "while condition must be Bool");
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
            Stmt::Return(return_stmt) => {
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
                if self.loop_depth == 0 {
                    self.add_error("invalid_break", "break used outside of a loop", break_stmt.span);
                }
                Ty::unit()
            }
            Stmt::Expr(expr_stmt) => self.check_expr(&expr_stmt.expr),
            Stmt::Unwrap(stmt) => {
                let value_ty = self.check_expr(&stmt.value);
                let inner = self.unwrap_inner_type(&value_ty);
                let slot_types = self.destructure_slots(&inner, stmt.bindings.len());
                for (index, binding) in stmt.bindings.iter().enumerate() {
                    let inferred = slot_types
                        .get(index)
                        .cloned()
                        .unwrap_or(Ty::Unknown);
                    let explicit = binding.ty.as_ref().map(|ty| self.ty_from_type_ref(ty));
                    let ty = explicit.clone().unwrap_or_else(|| inferred.clone());
                    if let Some(expected) = explicit {
                        self.require_assignable(
                            &inferred,
                            &expected,
                            binding.span,
                            "invalid_binding_type",
                            format!(
                                "cannot unwrap '{}' into binding '{}' of type '{}'",
                                inferred.describe(),
                                binding.name,
                                expected.describe()
                            ),
                        );
                    }
                    self.define_local(&binding.name, ty, false);
                }
                if let Some(else_block) = &stmt.else_block {
                    self.check_block(else_block);
                }
                Ty::unit()
            }
            Stmt::UnwrapBlock(block) => {
                for clause in &block.clauses {
                    self.check_stmt(&Stmt::Unwrap(clause.clone()));
                }
                if let Some(else_block) = &block.else_block {
                    self.check_block(else_block);
                }
                Ty::unit()
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

    fn check_if_stmt(&mut self, stmt: &IfStmt) {
        if let Some(value) = &stmt.binding_value {
            let value_ty = self.check_expr(value);
            let inner = self.unwrap_inner_type(&value_ty);
            let slot_types = self.destructure_slots(&inner, stmt.bindings.len());
            self.push_scope();
            for (index, binding) in stmt.bindings.iter().enumerate() {
                let inferred = slot_types
                    .get(index)
                    .cloned()
                    .unwrap_or(Ty::Unknown);
                let explicit = binding.ty.as_ref().map(|ty| self.ty_from_type_ref(ty));
                let ty = explicit.clone().unwrap_or_else(|| inferred.clone());
                self.define_local(&binding.name, ty, false);
            }
            self.check_block(&stmt.then_block);
            self.pop_scope();
        } else if let Some(condition) = &stmt.condition {
            let condition_ty = self.check_expr(condition);
            self.require_bool(&condition_ty, condition.span(), "if condition must be Bool");
            self.check_block(&stmt.then_block);
        }
        if let Some(else_branch) = &stmt.else_branch {
            self.check_else_branch(else_branch);
        }
    }

    fn check_else_branch(&mut self, branch: &ElseBranch) {
        match branch {
            ElseBranch::If(stmt) => self.check_if_stmt(stmt),
            ElseBranch::Block(block) => {
                self.check_block(block);
            }
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
        let item_ty = if let Some(iterable) = &binding.iterable {
            let iterable_ty = self.check_expr(iterable);
            self.iterable_item_type(&iterable_ty)
        } else {
            Ty::Unknown
        };
        for value in &binding.values {
            self.check_expr(value);
        }
        let slot_types = self.destructure_slots(&item_ty, binding.bindings.len());
        for (index, local) in binding.bindings.iter().enumerate() {
            let inferred = slot_types
                .get(index)
                .cloned()
                .unwrap_or(Ty::Unknown);
            let explicit = local.ty.as_ref().map(|ty| self.ty_from_type_ref(ty));
            let ty = explicit.clone().unwrap_or_else(|| inferred.clone());
            self.define_local(&local.name, ty, local.mutable);
        }
    }

    fn binding_slot_types(&self, bindings: &[crate::ast::Binding], value_types: &[Ty]) -> Vec<Ty> {
        if bindings.len() > 1 && value_types.len() == 1 {
            return self.destructure_slots(&value_types[0], bindings.len());
        }
        (0..bindings.len())
            .map(|index| value_types.get(index).cloned().unwrap_or(Ty::Unknown))
            .collect()
    }

    fn destructure_slots(&self, ty: &Ty, count: usize) -> Vec<Ty> {
        if count <= 1 {
            return vec![ty.clone()];
        }
        let mut slots = match ty {
            Ty::Tuple(items) => items.clone(),
            Ty::Record(fields) => fields.iter().map(|(_, ty)| ty.clone()).collect(),
            Ty::Named(name, _) => self
                .lookup_any_type(name)
                .map(|sig| sig.fields.iter().map(|field| field.ty.clone()).collect())
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
            let expected = self.assignment_target_type(target);
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
            if assignment.operator != AssignOp::Reassign
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

    fn assignment_target_type(&mut self, target: &Expr) -> Ty {
        match target {
            Expr::Identifier { name, span } => {
                if let Some(value) = self.lookup_value(name) {
                    if !value.mutable {
                        self.add_error(
                            "assign_immutable",
                            format!("cannot assign to immutable binding '{}'", name),
                            *span,
                        );
                    }
                    value.ty
                } else {
                    self.add_error("undefined_name", format!("undefined name '{}'", name), *span);
                    Ty::Unknown
                }
            }
            Expr::Member { receiver, name, span } => {
                let receiver_ty = self.check_expr(receiver);
                self.member_type(&receiver_ty, name).unwrap_or_else(|| {
                    self.add_error(
                        "unknown_member",
                        format!(
                            "type '{}' has no field or method '{}'",
                            receiver_ty.describe(),
                            name
                        ),
                        *span,
                    );
                    Ty::Unknown
                })
            }
            Expr::Index {
                receiver, index, ..
            } => {
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

    fn check_expr_against(&mut self, expr: &Expr, expected: &Ty) -> Ty {
        if let Ty::Function(params, ret) = expected {
            if !matches!(expr, Expr::Lambda { .. }) && contains_placeholder_expr(expr) {
                let placeholder_ty = params.first().cloned().unwrap_or(Ty::Unknown);
                self.placeholder_hints.push(placeholder_ty);
                let body_ty = self.check_expr_against(expr, ret);
                self.placeholder_hints.pop();
                return Ty::Function(params.clone(), Box::new(body_ty));
            }
        }
        match expr {
            Expr::Identifier { name, span } => self
                .lookup_value(name)
                .map(|value| value.ty)
                .or_else(|| self.lookup_function_type(name))
                .or_else(|| self.lookup_named_constructor_type(name))
                .unwrap_or_else(|| {
                    self.add_error("undefined_name", format!("undefined name '{}'", name), *span);
                    Ty::Unknown
                }),
            Expr::Placeholder { .. } => self.placeholder_hints.last().cloned().unwrap_or(Ty::Unknown),
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
            Expr::TupleLiteral { items, .. } => {
                Ty::Tuple(items.iter().map(|item| self.check_expr(item)).collect())
            }
            Expr::Call { callee, args, span } => self.check_call(callee, args, *span),
            Expr::Member { receiver, name, span } => {
                if let Some(ty) = self.module_member_value_type(expr) {
                    return ty;
                }
                if let Some(ty) = self.static_member_value_type(receiver, name) {
                    return ty;
                }
                let receiver_ty = self.check_expr(receiver);
                self.member_type(&receiver_ty, name).unwrap_or_else(|| {
                    self.add_error(
                        "unknown_member",
                        format!(
                            "type '{}' has no field or method '{}'",
                            receiver_ty.describe(),
                            name
                        ),
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
                    Ty::Named(name, args) if (name == "Array" || name == "List") && args.len() == 1 => {
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
                            _ => format!("index expression expects Int, got '{}'", index_ty.describe()),
                        },
                        index.span(),
                    );
                }
                self.index_result_type(&receiver_ty)
            }
            Expr::RecordUpdate { receiver, updates, .. } => {
                let base = self.check_expr(receiver);
                for update in updates {
                    self.check_expr(&update.value);
                }
                base
            }
            Expr::RecordLiteral { fields, values, .. } => {
                if !fields.is_empty() {
                    return Ty::Record(
                        fields
                            .iter()
                            .filter_map(|field| {
                                field
                                    .name
                                    .as_ref()
                                    .map(|name| (name.clone(), self.check_expr(&field.value)))
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
                    Ty::Record(Vec::new())
                }
            }
            Expr::AnonymousInterface { .. } => Ty::Unknown,
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
                                format!("unary '-' expects numeric operand, got '{}'", inner.describe()),
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
                ..
            } => {
                let value_ty = self.check_expr(value);
                let mut result = Ty::Unknown;
                for case in cases {
                    let current = self.check_match_case(case, &value_ty);
                    result = join_types(&result, &current);
                }
                if *partial {
                    Ty::option(result)
                } else {
                    result
                }
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
        let external_params = expected_params.clone().unwrap_or_else(|| vec![Ty::Unknown; params.len()]);
        let destructured_external = matches!(&expected_params, Some(params_hint) if params_hint.len() == 1 && params.len() != 1);
        let hinted_params = match expected_params {
            Some(ref params_hint) if params_hint.len() == params.len() => params_hint.clone(),
            Some(ref params_hint) if destructured_external => {
                self.destructure_slots(&params_hint[0], params.len())
            }
            _ => vec![Ty::Unknown; params.len()],
        };

        self.push_scope();
        let mut param_types = Vec::new();
        for (index, param) in params.iter().enumerate() {
            let ty = param
                .ty
                .as_ref()
                .map(|ty| self.ty_from_type_ref(ty))
                .unwrap_or_else(|| hinted_params.get(index).cloned().unwrap_or(Ty::Unknown));
            param_types.push(ty.clone());
            self.define_local(&param.name, ty, false);
        }
        let ret = match body {
            LambdaBody::Expr(expr) => expected_ret
                .as_ref()
                .map(|ret| self.check_expr_against(expr, ret))
                .unwrap_or_else(|| self.check_expr(expr)),
            LambdaBody::Block(block) => self.check_block_against(
                block,
                expected_ret.as_ref().unwrap_or(&Ty::Unknown),
            ),
        };
        self.pop_scope();
        let ret = if expected_ret.as_ref().is_some_and(|expected| *expected == Ty::unit()) {
            Ty::unit()
        } else {
            ret
        };
        if destructured_external {
            Ty::Function(external_params, Box::new(ret))
        } else {
            Ty::Function(param_types, Box::new(ret))
        }
    }

    fn check_call(&mut self, callee: &Expr, args: &[crate::ast::CallArg], span: crate::source::Span) -> Ty {
        if self.is_builtin_print_call(callee) {
            for arg in args {
                self.check_expr(&arg.value);
            }
            return Ty::unit();
        }
        if let Some(ty) = self.try_check_constructor_call(callee, args, span) {
            return ty;
        }
        if let Some((params, ret)) = self.callable_signature_for_args(callee, args) {
            return self.check_signature_call(&params, &ret, args, span);
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
                    })
                    .collect::<Vec<_>>();
                self.check_signature_call(&sig_params, &ret, args, span)
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

    fn check_signature_call(
        &mut self,
        params: &[ParamSig],
        ret: &Ty,
        args: &[crate::ast::CallArg],
        span: crate::source::Span,
    ) -> Ty {
        let arrangement = arrange_param_args(params, args);
        let min_required = params.iter().filter(|param| !param.variadic).count();
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
                    if max_allowed == usize::MAX { "many".to_string() } else { max_allowed.to_string() },
                    args.len()
                ),
                span,
            );
        }

        let mut subst = HashMap::new();
        let mut checked_args = Vec::new();
        for (index, param) in params.iter().enumerate() {
            let slot = arrangement.slots.get(index).map(Vec::as_slice).unwrap_or(&[]);
            let raw_expected = param.ty.clone();
            for arg in slot {
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
                format!(
                    "argument has type '{}' but parameter expects '{}'",
                    actual.describe(),
                    expected.describe()
                ),
            );
        }

        materialize_type(&substitute_type(ret, &subst))
    }

    fn try_check_constructor_call(
        &mut self,
        callee: &Expr,
        args: &[crate::ast::CallArg],
        span: crate::source::Span,
    ) -> Option<Ty> {
        match callee {
            Expr::Identifier { name, .. } => {
                if let Some(ty) = self.check_builtin_constructor(name, args, span) {
                    return Some(ty);
                }
                if name == "init"
                    && self.current_method.as_deref() == Some("init")
                    && self.current_owner.is_some()
                {
                    return self
                        .current_owner
                        .clone()
                        .map(|owner| self.check_named_type_constructor(&owner, args, span));
                }
                if let Some(case) = self.world.lookup_enum_case(self.module, name) {
                    return Some(self.check_constructor_signature(&case.params, &case.result, args, span));
                }
                if let Some(sig) = self.lookup_type_local(name) {
                    return Some(self.check_named_type_constructor(&sig, args, span));
                }
                if let Some(sig) = self.world.lookup_imported_type(self.module, name) {
                    return Some(self.check_named_type_constructor(&sig, args, span));
                }
                if let Some(sig) = self.world.ambient.types.get(name).cloned() {
                    return Some(self.check_named_type_constructor(&sig, args, span));
                }
                None
            }
            Expr::Member { receiver, name, .. } => {
                if let Some(module) = module_alias_and_member(callee).and_then(|(alias, member)| {
                    self.world.lookup_module_alias(self.module, &alias).map(|module| (module, member))
                }) {
                    let (module_info, member) = module;
                    if let Some(sig) = module_info.types.get(&member).cloned() {
                        return Some(self.check_named_type_constructor(&sig, args, span));
                    }
                    if let Some(sig) = module_info.objects.get(&member).cloned() {
                        return Some(self.check_named_type_constructor(&sig, args, span));
                    }
                }
                if let Expr::Identifier { name: type_name, .. } = receiver.as_ref() {
                    if type_name == "Array" && name == "ofLength" {
                        if args.len() != 1 {
                            self.add_error(
                                "invalid_argument_count",
                                format!("Array.ofLength expects 1 argument, got {}", args.len()),
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
                                format!("Array.ofLength expects Int length, got '{}'", ty.describe()),
                            );
                        }
                        return Some(Ty::Named("Array".to_string(), vec![Ty::Unknown]));
                    }
                    if let Some(sig) = self.lookup_type_local(type_name) {
                        if let Some(case) = sig.enum_cases.get(name).cloned() {
                            return Some(self.check_constructor_signature(&case.params, &case.result, args, span));
                        }
                        if sig.kind == TypeKind::Object {
                            return None;
                        }
                    }
                    if let Some(sig) = self.world.lookup_imported_type(self.module, type_name) {
                        if let Some(case) = sig.enum_cases.get(name).cloned() {
                            return Some(self.check_constructor_signature(&case.params, &case.result, args, span));
                        }
                    }
                    if let Some(sig) = self.world.ambient.types.get(type_name) {
                        if let Some(case) = sig.enum_cases.get(name).cloned() {
                            return Some(self.check_constructor_signature(&case.params, &case.result, args, span));
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
    ) -> Option<Ty> {
        match name {
            "Range" => {
                if !(args.len() == 2 || args.len() == 3) {
                    self.add_error(
                        "invalid_argument_count",
                        format!("Range constructor expects 2 or 3 arguments, got {}", args.len()),
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
                        format!("Range constructor arguments must be Int, got '{}'", ty.describe()),
                    );
                }
                Some(Ty::Named("IntRange".to_string(), Vec::new()))
            }
            "List" => {
                let mut item = Ty::Unknown;
                for arg in args {
                    item = join_types(&item, &self.check_expr(&arg.value));
                }
                Some(Ty::Named("List".to_string(), vec![item]))
            }
            "Set" => {
                let mut item = Ty::Unknown;
                for arg in args {
                    item = join_types(&item, &self.check_expr(&arg.value));
                }
                Some(Ty::Named("Set".to_string(), vec![item]))
            }
            "Array" => {
                let mut item = Ty::Unknown;
                for arg in args {
                    item = join_types(&item, &self.check_expr(&arg.value));
                }
                Some(Ty::Named("Array".to_string(), vec![item]))
            }
            "Map" => {
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
    ) -> Ty {
        if let Some(overloads) = sig.methods.get("init") {
            if let Some(ctor) = self.choose_overload(overloads, args) {
                let ret = Ty::Named(
                    sig.name.clone(),
                    sig.type_params
                        .iter()
                        .map(|name| Ty::TypeParam(name.clone()))
                        .collect(),
                );
                let params = ctor
                    .params
                    .iter()
                    .map(|param| FieldSig {
                        name: param.name.clone(),
                        ty: param.ty.clone(),
                        mutable: false,
                        has_initializer: false,
                    })
                    .collect::<Vec<_>>();
                return self.check_constructor_signature(&params, &ret, args, span);
            }
        }

        let fields = sig
            .fields
            .iter()
            .filter(|field| !field.mutable || sig.kind == TypeKind::Record || sig.kind == TypeKind::Class)
            .cloned()
            .collect::<Vec<_>>();
        let ret = Ty::Named(
            sig.name.clone(),
            sig.type_params
                .iter()
                .map(|name| Ty::TypeParam(name.clone()))
                .collect(),
        );
        self.check_constructor_signature(&fields, &ret, args, span)
    }

    fn check_constructor_signature(
        &mut self,
        params: &[FieldSig],
        ret: &Ty,
        args: &[crate::ast::CallArg],
        span: crate::source::Span,
    ) -> Ty {
        if args.iter().all(|arg| arg.name.is_none()) {
            if args.len() == 1 {
                if let Some(record_ty) = self.extract_constructor_record_arg(&args[0].value, params) {
                    return self.check_record_constructor_conversion(params, ret, &record_ty, span);
                }
            }
            let arrangement = arrange_constructor_args(params, args);
            let min_required = params.iter().filter(|param| !param.has_initializer).count();
            if arrangement.overflow > 0 || arrangement.missing_required > 0 || args.len() < min_required || args.len() > params.len() {
                self.add_error(
                    "invalid_argument_count",
                    format!("call expects {}..{} arguments, got {}", min_required, params.len(), args.len()),
                    span,
                );
            }
            let mut subst = HashMap::new();
            let mut checked_args = Vec::new();
            for (index, param) in params.iter().enumerate() {
                for arg in arrangement.slots.get(index).map(Vec::as_slice).unwrap_or(&[]) {
                    let expected = substitute_type(&param.ty, &subst);
                    let actual = self.check_expr_against(&arg.value, &expected);
                    infer_type_subst(&expected, &actual, &mut subst);
                    checked_args.push((arg.span, actual, param.ty.clone(), String::new()));
                }
            }
            for (arg_span, actual, raw_expected, _) in checked_args {
                let expected = materialize_type(&substitute_type(&raw_expected, &subst));
                self.require_assignable(
                    &actual,
                    &expected,
                    arg_span,
                    "invalid_argument_type",
                    format!("constructor argument has type '{}' but expects '{}'", actual.describe(), expected.describe()),
                );
            }
            return materialize_type(&substitute_type(ret, &subst));
        }

        let arrangement = arrange_constructor_args(params, args);
        if arrangement.overflow > 0 || arrangement.missing_required > 0 {
            let min_required = params.iter().filter(|param| !param.has_initializer).count();
            self.add_error(
                "invalid_argument_count",
                format!("call expects {}..{} arguments, got {}", min_required, params.len(), args.len()),
                span,
            );
        }
        let mut subst = HashMap::new();
        let mut checked_args = Vec::new();
        for (index, param) in params.iter().enumerate() {
            for arg in arrangement.slots.get(index).map(Vec::as_slice).unwrap_or(&[]) {
                let expected = substitute_type(&param.ty, &subst);
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
                    format!("constructor argument has type '{}' but expects '{}'", actual.describe(), expected.describe())
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

    fn callable_signature_for_args(
        &mut self,
        callee: &Expr,
        args: &[crate::ast::CallArg],
    ) -> Option<(Vec<ParamSig>, Ty)> {
        match callee {
            Expr::Identifier { name, .. } => {
                if let Some(functions) = self.lookup_functions(name) {
                    let sig = self
                        .choose_overload(&functions, args)
                        .or_else(|| functions.first())
                        ?.clone();
                    return Some((sig.params, sig.ret));
                }
                None
            }
            Expr::Member { receiver, name, .. } => {
                if let Some((module, member)) = module_alias_and_member(callee)
                    .and_then(|(alias, member)| self.world.lookup_module_alias(self.module, &alias).map(|module| (module, member)))
                {
                    if let Some(functions) = module.functions.get(&member) {
                        let sig = self
                            .choose_overload(functions, args)
                            .or_else(|| functions.first())
                            ?.clone();
                        return Some((sig.params, sig.ret));
                    }
                }
                if let Some(sigs) = self.static_method_sigs(receiver, name) {
                    let sig = self
                        .choose_overload(&sigs, args)
                        .or_else(|| sigs.first())
                        ?.clone();
                    return Some((sig.params, sig.ret));
                }
                let receiver_ty = self.check_expr(receiver);
                let methods = self.member_method_sigs(&receiver_ty, name)?;
                let method = self
                    .choose_overload(&methods, args)
                    .or_else(|| methods.first())
                    ?.clone();
                Some((method.params, method.ret))
            }
            _ => None,
        }
    }

    fn static_member_value_type(&self, receiver: &Expr, name: &str) -> Option<Ty> {
        let Expr::Identifier { name: type_name, .. } = receiver else {
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
            return Some(case.result.clone());
        }
        None
    }

    fn static_method_sigs(&self, receiver: &Expr, name: &str) -> Option<Vec<FunctionSig>> {
        let Expr::Identifier { name: type_name, .. } = receiver else {
            return None;
        };
        if let Some(sig) = self.lookup_any_object(type_name) {
            return self.method_sigs_for_type(&sig, name);
        }
        let sig = self.lookup_any_non_object_type(type_name)?;
        self.method_sigs_for_type(&sig, name)
    }

    fn check_binary_expr(&mut self, left: &Ty, op: BinaryOp, right: &Ty, span: crate::source::Span) -> Ty {
        match op {
            BinaryOp::Add => {
                if left.is_str() || right.is_str() {
                    Ty::str()
                } else if left.is_float_like() || right.is_float_like() {
                    if !left.is_numeric() && !matches!(left, Ty::Unknown) {
                        self.add_error("invalid_binary_operand", "left operand must be numeric", span);
                    }
                    if !right.is_numeric() && !matches!(right, Ty::Unknown) {
                        self.add_error("invalid_binary_operand", "right operand must be numeric", span);
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
                    self.add_error("invalid_binary_operand", "left operand must be numeric", span);
                }
                if !right.is_numeric() && !matches!(right, Ty::Unknown) {
                    self.add_error("invalid_binary_operand", "right operand must be numeric", span);
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
            BinaryOp::Concat => left.clone(),
            BinaryOp::Remove | BinaryOp::Append | BinaryOp::Prepend | BinaryOp::Compose | BinaryOp::Colon => Ty::Unknown,
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
                    for pattern in elements {
                        self.bind_pattern(pattern, &Ty::Unknown);
                    }
                }
            }
            Pattern::Constructor { path, args, .. } => {
                let case_name = path.last().cloned().unwrap_or_default();
                if let Some(case) = self.lookup_case_by_path(path) {
                    let mut subst = HashMap::new();
                    infer_type_subst(&case.result, scrutinee, &mut subst);
                    for (pattern, param) in args.iter().zip(case.params.iter()) {
                        self.bind_pattern(pattern, &materialize_type(&substitute_type(&param.ty, &subst)));
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

    fn lookup_destructured_type_fields(&self, path: &[String]) -> Option<Vec<Ty>> {
        let sig = match path {
            [name] => self.lookup_any_type(name),
            [module_alias, name] => self
                .world
                .lookup_module_alias(self.module, module_alias)
                .and_then(|module| module.types.get(name).cloned().or_else(|| module.objects.get(name).cloned())),
            _ => None,
        }?;
        Some(sig.fields.iter().map(|field| field.ty.clone()).collect())
    }

    fn unwrap_inner_type(&self, ty: &Ty) -> Ty {
        match ty {
            Ty::Named(name, args) if name == "Option" && args.len() == 1 => args[0].clone(),
            Ty::Named(name, args) if name == "Result" && args.len() >= 1 => args[0].clone(),
            Ty::Named(name, args) if name == "Either" && args.len() == 2 => args[1].clone(),
            _ => Ty::Unknown,
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
            Ty::Named(name, args)
                if (name == "Array" || name == "List") && args.len() == 1 =>
            {
                args[0].clone()
            }
            Ty::Named(name, args) if name == "Map" && args.len() == 2 => Ty::option(args[1].clone()),
            Ty::Tuple(items) => join_many_types(items),
            Ty::Unknown => Ty::Unknown,
            _ => Ty::Unknown,
        }
    }

    fn member_type(&self, receiver: &Ty, name: &str) -> Option<Ty> {
        match receiver {
            Ty::Named(type_name, args) => {
                let sig = self
                    .lookup_any_type(type_name)?;
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
                                })
                                .collect(),
                            ret: substitute_type(&method.ret, &subst),
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
        for scope in self.scopes.iter().rev() {
            if let Some(value) = scope.get(name) {
                return Some(value.clone());
            }
        }
        self.globals
            .get(name)
            .cloned()
            .or_else(|| self.world.lookup_imported_global(self.module, name))
    }

    fn is_builtin_print_call(&self, callee: &Expr) -> bool {
        match callee {
            Expr::Identifier { name, .. } => {
                matches!(name.as_str(), "print" | "println" | "printf" | "panic")
            }
            Expr::Member { receiver, name, .. } => {
                matches!(name.as_str(), "print" | "println" | "printf" | "panic")
                    && path_starts_with_os(receiver)
            }
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
                    for arg in arrangement.slots.get(index).map(Vec::as_slice).unwrap_or(&[]) {
                        let arg_index = args
                            .iter()
                            .position(|candidate| std::ptr::eq(candidate, *arg))
                            .unwrap_or(0);
                        let actual = &arg_types[arg_index];
                        if !matches!(actual, Ty::Unknown) {
                            if self.is_assignable(actual, &param.ty) {
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
                .lookup_value(name)
                .map(|value| value.ty)
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
                    .filter_map(|field| field.name.as_ref().map(|name| (name.clone(), self.probe_expr_type(&field.value))))
                    .collect(),
            ),
            _ => Ty::Unknown,
        }
    }

    fn extract_constructor_record_arg(&mut self, expr: &Expr, params: &[FieldSig]) -> Option<Ty> {
        match expr {
            Expr::RecordLiteral { fields, values, .. } if !fields.is_empty() => Some(Ty::Record(
                fields
                    .iter()
                    .filter_map(|field| field.name.as_ref().map(|name| (name.clone(), self.check_expr(&field.value))))
                    .collect(),
            )),
            Expr::RecordLiteral { fields, values, .. } if fields.is_empty() && !values.is_empty() => Some(Ty::Record(
                values
                    .iter()
                    .enumerate()
                    .filter_map(|(index, value)| params.get(index).map(|param| (param.name.clone(), self.check_expr(value))))
                    .collect(),
            )),
            _ => {
                let actual = self.check_expr(expr);
                matches!(actual, Ty::Record(_)).then_some(actual)
            }
        }
    }

    fn check_record_constructor_conversion(
        &mut self,
        params: &[FieldSig],
        ret: &Ty,
        record_ty: &Ty,
        span: crate::source::Span,
    ) -> Ty {
        let Ty::Record(fields) = record_ty else {
            return materialize_type(ret);
        };
        let min_required = params.iter().filter(|param| !param.has_initializer).count();
        let present = params
            .iter()
            .filter(|param| fields.iter().any(|(name, _)| name == &param.name))
            .count();
        if present < min_required {
            self.add_error(
                "invalid_argument_count",
                format!("call expects {}..{} arguments, got 1", min_required, params.len()),
                span,
            );
        }
        for param in params {
            let Some((_, actual)) = fields.iter().find(|(name, _)| name == &param.name) else {
                continue;
            };
            self.require_assignable(
                actual,
                &param.ty,
                span,
                "invalid_argument_type",
                format!(
                    "argument for '{}' has type '{}' but expects '{}'",
                    param.name,
                    actual.describe(),
                    param.ty.describe()
                ),
            );
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
        if let Some(case) = self.world.lookup_enum_case(self.module, name) {
            return Some(case.result);
        }
        None
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
        self.lookup_type_local(name)
            .or_else(|| self.world.lookup_imported_type(self.module, name))
            .or_else(|| self.lookup_unique_module_type(name))
            .or_else(|| self.world.ambient.types.get(name).cloned())
            .or_else(|| self.world.ambient.objects.get(name).cloned())
    }

    fn resolve_named_type(&self, name: &str, args: Vec<Ty>) -> Ty {
        if self.is_type_param(name) {
            return Ty::TypeParam(name.to_string());
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
                params.iter().map(|param| self.ty_from_type_ref(param)).collect(),
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
        if self.scopes.is_empty() {
            self.push_scope();
        }
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name.to_string(), ValueInfo { ty, mutable });
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
        self.diagnostics.push(Diagnostic::error(code, message, span));
    }

    fn is_assignable(&self, actual: &Ty, expected: &Ty) -> bool {
        let mut seen = HashSet::new();
        self.is_assignable_inner(actual, expected, &mut seen)
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

        let (Ty::Named(actual_name, actual_args), Ty::Named(expected_name, _)) = (actual, expected) else {
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

fn function_sig_from_function(function: &FunctionDecl, owner_type_params: &[String]) -> FunctionSig {
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
            })
            .collect(),
        ret: function
            .return_type
            .as_ref()
            .map(|ty| convert_type_ref(ty, &type_params))
            .unwrap_or(Ty::Unknown),
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
            })
            .collect(),
        ret: method
            .return_type
            .as_ref()
            .map(|ty| convert_type_ref(ty, &type_params))
            .unwrap_or(Ty::Unknown),
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
                has_initializer: field.initializer.is_some(),
            }),
            TypeMember::Method(method) => {
                methods
                    .entry(method.name.clone())
                    .or_insert_with(Vec::new)
                    .push(function_sig_from_method(
                        method,
                        &decl.type_params.iter().map(|param| param.name.clone()).collect::<Vec<_>>(),
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
                        has_initializer: false,
                    })
                    .collect::<Vec<_>>();
                enum_cases.insert(
                    case.name.clone(),
                    EnumCaseSig {
                        params: ctor_params,
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
        type_params: decl.type_params.iter().map(|param| param.name.clone()).collect(),
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
            } else {
                Ty::Named(
                    name.clone(),
                    args.iter().map(|arg| convert_type_ref(arg, type_params)).collect(),
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
        Expr::Integer { .. } => Some(Ty::int()),
        Expr::Float { .. } => Some(Ty::float()),
        Expr::String { .. } => Some(Ty::str()),
        Expr::Bool { .. } => Some(Ty::bool()),
        Expr::Unit { .. } => Some(Ty::unit()),
        _ => None,
    }
}

fn type_ref_named_name(reference: &TypeRef) -> Option<&str> {
    match reference {
        TypeRef::Named { name, .. } => Some(name.as_str()),
        _ => None,
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
                if params[index].variadic || slots[index].is_empty() {
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
        if params.last().is_some_and(|param| param.variadic) && positional_index >= params.len().saturating_sub(1) {
            if let Some(slot) = slots.last_mut() {
                slot.push(arg);
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
        .filter(|(index, param)| !param.variadic && slots[*index].is_empty())
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

        while positional_index < params.len() && !slots[positional_index].is_empty() {
            positional_index += 1;
        }
        if positional_index < params.len() {
            slots[positional_index].push(arg);
            positional_index += 1;
        } else {
            overflow += 1;
        }
    }

    let missing_required = params
        .iter()
        .enumerate()
        .filter(|(index, param)| !param.has_initializer && slots[*index].is_empty())
        .count();

    ArgArrangement {
        slots,
        overflow,
        missing_required,
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
                    for (expected_param, actual_param) in expected_params.iter().zip(actual_params.iter()) {
                        infer_type_subst(expected_param, actual_param, subst);
                    }
                } else if expected_params.len() == 1 && actual_params.len() != 1 {
                    let expected_slots = match &expected_params[0] {
                        Ty::Tuple(items) => items.clone(),
                        Ty::Record(fields) => fields.iter().map(|(_, ty)| ty.clone()).collect(),
                        _ => Vec::new(),
                    };
                    if expected_slots.len() == actual_params.len() {
                        for (expected_param, actual_param) in expected_slots.iter().zip(actual_params.iter()) {
                            infer_type_subst(expected_param, actual_param, subst);
                        }
                    }
                }
                infer_type_subst(expected_ret, actual_ret, subst);
            }
        }
        Ty::Unknown => {}
    }
}

fn substitute_type(ty: &Ty, subst: &HashMap<String, Ty>) -> Ty {
    match ty {
        Ty::TypeParam(name) => subst.get(name).cloned().unwrap_or_else(|| ty.clone()),
        Ty::Named(name, args) => Ty::Named(
            name.clone(),
            args.iter().map(|arg| substitute_type(arg, subst)).collect(),
        ),
        Ty::Tuple(items) => Ty::Tuple(items.iter().map(|item| substitute_type(item, subst)).collect()),
        Ty::Record(fields) => Ty::Record(
            fields
                .iter()
                .map(|(name, ty)| (name.clone(), substitute_type(ty, subst)))
                .collect(),
        ),
        Ty::Function(params, ret) => Ty::Function(
            params.iter().map(|param| substitute_type(param, subst)).collect(),
            Box::new(substitute_type(ret, subst)),
        ),
        Ty::Unknown => Ty::Unknown,
    }
}

fn materialize_type(ty: &Ty) -> Ty {
    match ty {
        Ty::TypeParam(_) => Ty::Unknown,
        Ty::Named(name, args) => Ty::Named(name.clone(), args.iter().map(materialize_type).collect()),
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
    if actual == expected {
        return true;
    }
    match (actual, expected) {
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

fn contains_placeholder_expr(expr: &Expr) -> bool {
    match expr {
        Expr::Placeholder { .. } => true,
        Expr::ListLiteral { items, .. } | Expr::TupleLiteral { items, .. } => {
            items.iter().any(contains_placeholder_expr)
        }
        Expr::Call { callee, args, .. } => {
            contains_placeholder_expr(callee)
                || args.iter().any(|arg| contains_placeholder_expr(&arg.value))
        }
        Expr::Member { receiver, .. } => contains_placeholder_expr(receiver),
        Expr::Index { receiver, index, .. } => {
            contains_placeholder_expr(receiver) || contains_placeholder_expr(index)
        }
        Expr::RecordUpdate { receiver, updates, .. } => {
            contains_placeholder_expr(receiver)
                || updates.iter().any(|arg| contains_placeholder_expr(&arg.value))
        }
        Expr::RecordLiteral { fields, .. } => fields.iter().any(|field| contains_placeholder_expr(&field.value)),
        Expr::AnonymousInterface { methods, .. } => methods.iter().any(|method| {
            method
                .body
                .as_ref()
                .is_some_and(|body| match body {
                    CallableBody::Expr(expr) => contains_placeholder_expr(expr),
                    CallableBody::Block(block) => block.statements.iter().any(stmt_contains_placeholder),
                })
        }),
        Expr::Unary { expr, .. } => contains_placeholder_expr(expr),
        Expr::Binary { left, right, .. } => {
            contains_placeholder_expr(left) || contains_placeholder_expr(right)
        }
        Expr::Is { left, .. } => contains_placeholder_expr(left),
        Expr::If {
            condition,
            then_block,
            else_branch,
            ..
        } => {
            contains_placeholder_expr(condition)
                || then_block.statements.iter().any(stmt_contains_placeholder)
                || match else_branch.as_ref() {
                    ElseExprBranch::If(expr) => contains_placeholder_expr(expr),
                    ElseExprBranch::Block(block) => block.statements.iter().any(stmt_contains_placeholder),
                }
        }
        Expr::Block { body, .. } => body.statements.iter().any(stmt_contains_placeholder),
        Expr::Match { value, cases, .. } => {
            contains_placeholder_expr(value)
                || cases.iter().any(|case| match &case.body {
                    MatchCaseBody::Expr(expr) => contains_placeholder_expr(expr),
                    MatchCaseBody::Block(block) => block.statements.iter().any(stmt_contains_placeholder),
                })
        }
        Expr::ForYield {
            yield_body, bindings, ..
        } => {
            bindings.iter().any(|binding| {
                binding
                    .iterable
                    .as_ref()
                    .is_some_and(contains_placeholder_expr)
                    || binding.values.iter().any(contains_placeholder_expr)
            })
                || yield_body.statements.iter().any(stmt_contains_placeholder)
        }
        Expr::Lambda { body, .. } => match body {
            LambdaBody::Expr(expr) => contains_placeholder_expr(expr),
            LambdaBody::Block(block) => block.statements.iter().any(stmt_contains_placeholder),
        },
        Expr::Group { inner, .. } => contains_placeholder_expr(inner),
        Expr::Identifier { .. }
        | Expr::Integer { .. }
        | Expr::Float { .. }
        | Expr::String { .. }
        | Expr::Bool { .. }
        | Expr::Unit { .. } => false,
    }
}

fn stmt_contains_placeholder(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::Binding(binding) => binding.values.iter().any(contains_placeholder_expr),
        Stmt::Assignment(assignment) => assignment.values.iter().any(contains_placeholder_expr),
        Stmt::Expr(expr) => contains_placeholder_expr(&expr.expr),
        Stmt::If(stmt) => {
            stmt.condition
                .as_ref()
                .is_some_and(contains_placeholder_expr)
                || stmt
                    .binding_value
                    .as_ref()
                    .is_some_and(contains_placeholder_expr)
                || stmt.then_block.statements.iter().any(stmt_contains_placeholder)
                || stmt
                    .else_branch
                    .as_ref()
                    .is_some_and(|branch| match branch {
                        ElseBranch::If(stmt) => stmt_contains_placeholder(&Stmt::If((**stmt).clone())),
                        ElseBranch::Block(block) => block.statements.iter().any(stmt_contains_placeholder),
                    })
        }
        Stmt::While(stmt) => {
            contains_placeholder_expr(&stmt.condition)
                || stmt.body.statements.iter().any(stmt_contains_placeholder)
        }
        Stmt::For(stmt) => {
            stmt.bindings.iter().any(|binding| {
                binding
                    .iterable
                    .as_ref()
                    .is_some_and(contains_placeholder_expr)
                    || binding.values.iter().any(contains_placeholder_expr)
            })
                || stmt.body.statements.iter().any(stmt_contains_placeholder)
        }
        Stmt::Unwrap(stmt) => {
            contains_placeholder_expr(&stmt.value)
                || stmt
                    .else_block
                    .as_ref()
                    .is_some_and(|block| block.statements.iter().any(stmt_contains_placeholder))
        }
        Stmt::UnwrapBlock(stmt) => {
            stmt.clauses.iter().any(|clause| contains_placeholder_expr(&clause.value))
                || stmt
                    .else_block
                    .as_ref()
                    .is_some_and(|block| block.statements.iter().any(stmt_contains_placeholder))
        }
        Stmt::Match(stmt) => {
            contains_placeholder_expr(&stmt.value)
                || stmt.cases.iter().any(|case| match &case.body {
                    MatchCaseBody::Expr(expr) => contains_placeholder_expr(expr),
                    MatchCaseBody::Block(block) => block.statements.iter().any(stmt_contains_placeholder),
                })
        }
        Stmt::LocalFunction(function) => match &function.body {
            CallableBody::Expr(expr) => contains_placeholder_expr(expr),
            CallableBody::Block(block) => block.statements.iter().any(stmt_contains_placeholder),
        },
        Stmt::Return(stmt) => stmt
            .value
            .as_ref()
            .is_some_and(|expr| contains_placeholder_expr(expr)),
        Stmt::Break(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{lex, parse_program, SourceFile};

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
        let result = check_path(workspace_root().join("examples/import_forms.lum")).expect("typecheck");
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn checks_bumper_example() {
        let result = check_path(workspace_root().join("examples/random_code/bumper.lum")).expect("typecheck");
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn checks_parity_examples() {
        let root = workspace_root();
        let paths = [
            "examples/tuple_destructuring.lum",
            "examples/record_destructuring.lum",
            "examples/class_destructuring.lum",
            "examples/enums.lum",
            "examples/enum_object_same_name.lum",
            "examples/imports.lum",
            "examples/interface_default_methods.lum",
            "examples/list_hof.lum",
            "examples/set_map_hof.lum",
            "examples/placeholder_lambda.lum",
            "examples/zip.lum",
        ];

        for path in paths {
            let result = check_path(root.join(path)).unwrap_or_else(|err| panic!("{path}: {err}"));
            assert!(result.diagnostics.is_empty(), "{path}: {:#?}", result.diagnostics);
        }
    }
}
