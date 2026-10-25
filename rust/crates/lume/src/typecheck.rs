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
    decimal::Decimal,
    numeric::{parse_float_literal, parse_integer_literal},
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
    pub warnings: Vec<Diagnostic>,
}

#[derive(Debug, Clone, Default)]
pub struct PathCheckResult {
    pub diagnostics: Vec<crate::resolver::LocatedDiagnostic>,
    pub warnings: Vec<crate::resolver::LocatedDiagnostic>,
}

pub fn check_program(program: &Program) -> CheckResult {
    let world = World::from_program(program);
    let Some(module) = world.root_module.as_ref() else {
        return CheckResult {
            diagnostics: Vec::new(),
            warnings: Vec::new(),
        };
    };
    let mut checker = Checker::new(&world, module);
    checker.check_module();
    CheckResult {
        diagnostics: checker.diagnostics,
        warnings: checker.warnings,
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
    let (resolve_errors, mut warnings): (Vec<_>, Vec<_>) = resolved
        .diagnostics
        .into_iter()
        .partition(|located| located.diagnostic.severity == crate::Severity::Error);
    if !resolve_errors.is_empty() {
        return Ok(PathCheckResult {
            diagnostics: resolve_errors,
            warnings,
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
        let (module_diagnostics, module_warnings, checked_globals) = {
            let mut checker = Checker::new(&world, module);
            checker.check_module();
            (
                checker.diagnostics,
                checker.warnings,
                checker.globals.clone(),
            )
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
        warnings.extend(module_warnings.into_iter().map(|diagnostic| {
            crate::resolver::LocatedDiagnostic {
                path: display_path.clone(),
                diagnostic,
            }
        }));
    }

    Ok(PathCheckResult {
        diagnostics,
        warnings,
    })
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

/// Native calls still use the parameter names declared by the standard library.
pub(crate) fn runtime_callable_parameters(
    owner: &str,
    member: &str,
    static_member: bool,
) -> Option<(Vec<String>, Vec<Vec<crate::ir::SourceCallParamSpec>>)> {
    let world = World::from_program(&Program::default());
    let checker = Checker::new(&world, world.root_module.as_ref()?);
    let sig = if static_member {
        world.ambient.objects.get(owner)?
    } else {
        world.ambient.types.get(owner)?
    };
    let methods = checker.method_sigs_for_type(sig, member)?;
    let signatures = methods
        .into_iter()
        .filter(|method| !method.getter)
        .map(|method| {
            method
                .params
                .into_iter()
                .map(|param| crate::ir::SourceCallParamSpec {
                    name: Some(param.name),
                    ty: runtime_signature_type(&param.ty),
                    lazy: param.lazy,
                    variadic: param.variadic,
                    default: None,
                })
                .collect()
        })
        .collect();
    Some((sig.type_params.clone(), signatures))
}

fn runtime_signature_type(ty: &Ty) -> crate::ir::Type {
    use crate::ir::Type;
    match ty {
        Ty::Unknown | Ty::Wildcard | Ty::Capture(_) => Type::Unknown,
        Ty::Never => Type::Never,
        Ty::Named(name, args) => {
            if args.is_empty() {
                match name.as_str() {
                    "Unit" => return Type::Unit,
                    "Bool" => return Type::Bool,
                    "Int" => return Type::Int,
                    "Float" => return Type::Float,
                    "Str" => return Type::Str,
                    _ => {}
                }
            }
            Type::Named {
                name: name.clone(),
                args: args.iter().map(runtime_signature_type).collect(),
            }
        }
        Ty::Union(members) => Type::Union(members.iter().map(runtime_signature_type).collect()),
        Ty::Tuple(items) => Type::Tuple(items.iter().map(runtime_signature_type).collect()),
        Ty::Record(fields) => Type::Record(
            fields
                .iter()
                .map(|(name, ty)| crate::ir::NamedType {
                    name: name.clone(),
                    ty: runtime_signature_type(ty),
                })
                .collect(),
        ),
        Ty::Function(params, ret) => Type::Function {
            params: params.iter().map(runtime_signature_type).collect(),
            ret: Box::new(runtime_signature_type(ret)),
        },
        Ty::TypeParam(name) => Type::TypeParam(name.clone()),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Ty {
    Unknown,
    Wildcard,
    Capture(usize),
    Never,
    Named(String, Vec<Ty>),
    Union(Vec<Ty>),
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

    fn unit() -> Self {
        Self::named("Unit")
    }

    fn union(members: impl IntoIterator<Item = Ty>) -> Self {
        let mut normalized = Vec::new();
        for member in members {
            match member {
                Ty::Union(nested) => normalized.extend(nested),
                Ty::Never => {}
                Ty::Unknown => return Ty::Unknown,
                member if member.is_any() => return Ty::any(),
                member => normalized.push(member),
            }
        }
        normalized.sort_by_key(|member| format!("{member:?}"));
        normalized.dedup();
        match normalized.len() {
            0 => Ty::Never,
            1 => normalized.pop().expect("one union member"),
            _ => Ty::Union(normalized),
        }
    }

    fn describe(&self) -> String {
        match self {
            Ty::Unknown => "<unknown>".to_string(),
            Ty::Wildcard => "_".to_string(),
            Ty::Capture(_) => "_".to_string(),
            Ty::Never => "Never".to_string(),
            Ty::Named(name, args) if args.is_empty() => enum_case_view_parts(name)
                .map(|(owner, case)| format!("{owner}.{case}"))
                .unwrap_or_else(|| name.clone()),
            Ty::Named(name, args) => format!(
                "{}[{}]",
                enum_case_view_parts(name)
                    .map(|(owner, case)| format!("{owner}.{case}"))
                    .unwrap_or_else(|| name.clone()),
                args.iter().map(Ty::describe).collect::<Vec<_>>().join(", ")
            ),
            Ty::Union(members) => members
                .iter()
                .map(Ty::describe)
                .collect::<Vec<_>>()
                .join(" | "),
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
                if matches!(ret.as_ref(), Ty::Union(_)) {
                    format!("({})", ret.describe())
                } else {
                    ret.describe()
                }
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
        self.is_int_like() || self.is_float_like() || self.is_decimal()
    }

    fn is_decimal(&self) -> bool {
        matches!(self, Ty::Named(name, args) if name == "Decimal" && args.is_empty())
    }
}

#[derive(Debug, Clone)]
struct ValueInfo {
    ty: Ty,
    mutable: bool,
    stable: bool,
}

#[derive(Debug, Clone)]
struct MutableBindingUsage {
    name: String,
    span: crate::source::Span,
    reassigned: bool,
}

#[derive(Debug, Clone)]
struct TypeNarrowing {
    name: String,
    ty: Ty,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MemberNamespaceKind {
    Field,
    Getter,
    Method,
}

impl MemberNamespaceKind {
    fn label(self) -> &'static str {
        match self {
            Self::Field => "stored field",
            Self::Getter => "getter",
            Self::Method => "method",
        }
    }

    fn conflicts_with(self, other: Self) -> bool {
        !matches!(
            (self, other),
            (Self::Field, Self::Field) | (Self::Method, Self::Method)
        )
    }
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
    getter: bool,
}

fn normalized_mixin_signature(sig: &FunctionSig) -> FunctionSig {
    let subst = sig
        .type_params
        .iter()
        .enumerate()
        .map(|(index, name)| (name.clone(), Ty::TypeParam(format!("@{index}"))))
        .collect();
    let mut normalized = instantiate_function_sig(sig.clone(), &subst);
    normalized.reified_type_params = sig
        .reified_type_params
        .iter()
        .map(|name| {
            sig.type_params
                .iter()
                .position(|param| param == name)
                .unwrap()
                .to_string()
        })
        .collect();
    normalized
}

fn mixin_signature_key(name: &str, sig: &FunctionSig) -> String {
    let sig = normalized_mixin_signature(sig);
    format!(
        "{name}:{}:[{}]({})",
        sig.getter,
        sig.type_params.len(),
        sig.params
            .iter()
            .map(|param| format!("{}:{}:{}", param.ty.describe(), param.variadic, param.lazy))
            .collect::<Vec<_>>()
            .join(",")
    )
}

#[derive(Debug, Clone, PartialEq)]
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
        getter: false,
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
            getter: false,
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
            getter: false,
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
    visibility: Visibility,
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
struct NamedEnumCaseSig {
    owner: String,
    case: EnumCaseSig,
}

fn enum_case_result_owner(case: &EnumCaseSig) -> Option<String> {
    match &case.result {
        Ty::Named(owner, _) => Some(owner.clone()),
        _ => None,
    }
}

#[derive(Debug, Clone)]
struct TypeSig {
    kind: TypeKind,
    name: String,
    module_id: String,
    type_params: Vec<String>,
    generic_conditions: Vec<GenericConditionSig>,
    with_bounds: Vec<Ty>,
    fields: Vec<FieldSig>,
    methods: HashMap<String, Vec<FunctionSig>>,
    enum_cases: HashMap<String, EnumCaseSig>,
}

fn expand_aliases_in_function_sig(sig: &mut FunctionSig, aliases: &HashMap<String, TypeRef>) {
    for condition in &mut sig.generic_conditions {
        match condition {
            GenericConditionSig::Bound { subject, bound } => {
                *subject = expand_aliases_in_ty(subject, aliases, &mut HashSet::new());
                *bound = expand_aliases_in_ty(bound, aliases, &mut HashSet::new());
            }
            GenericConditionSig::Equal { left, right } => {
                *left = expand_aliases_in_ty(left, aliases, &mut HashSet::new());
                *right = expand_aliases_in_ty(right, aliases, &mut HashSet::new());
            }
        }
    }
    for param in &mut sig.params {
        param.ty = expand_aliases_in_ty(&param.ty, aliases, &mut HashSet::new());
    }
    sig.ret = expand_aliases_in_ty(&sig.ret, aliases, &mut HashSet::new());
}

fn expand_aliases_in_type_sig(sig: &mut TypeSig, aliases: &HashMap<String, TypeRef>) {
    for condition in &mut sig.generic_conditions {
        match condition {
            GenericConditionSig::Bound { subject, bound } => {
                *subject = expand_aliases_in_ty(subject, aliases, &mut HashSet::new());
                *bound = expand_aliases_in_ty(bound, aliases, &mut HashSet::new());
            }
            GenericConditionSig::Equal { left, right } => {
                *left = expand_aliases_in_ty(left, aliases, &mut HashSet::new());
                *right = expand_aliases_in_ty(right, aliases, &mut HashSet::new());
            }
        }
    }
    for bound in &mut sig.with_bounds {
        *bound = expand_aliases_in_ty(bound, aliases, &mut HashSet::new());
    }
    for field in &mut sig.fields {
        field.ty = expand_aliases_in_ty(&field.ty, aliases, &mut HashSet::new());
    }
    for overloads in sig.methods.values_mut() {
        for method in overloads {
            expand_aliases_in_function_sig(method, aliases);
        }
    }
    for case in sig.enum_cases.values_mut() {
        for param in &mut case.params {
            param.ty = expand_aliases_in_ty(&param.ty, aliases, &mut HashSet::new());
        }
        case.result = expand_aliases_in_ty(&case.result, aliases, &mut HashSet::new());
    }
}

fn canonicalize_nested_type_sig(sig: &mut TypeSig, names: &HashSet<String>) {
    let owner = sig.name.clone();
    for condition in &mut sig.generic_conditions {
        match condition {
            GenericConditionSig::Bound { subject, bound } => {
                canonicalize_nested_ty(subject, &owner, names);
                canonicalize_nested_ty(bound, &owner, names);
            }
            GenericConditionSig::Equal { left, right } => {
                canonicalize_nested_ty(left, &owner, names);
                canonicalize_nested_ty(right, &owner, names);
            }
        }
    }
    for bound in &mut sig.with_bounds {
        canonicalize_nested_ty(bound, &owner, names);
    }
    for field in &mut sig.fields {
        canonicalize_nested_ty(&mut field.ty, &owner, names);
    }
    for overloads in sig.methods.values_mut() {
        for method in overloads {
            for condition in &mut method.generic_conditions {
                match condition {
                    GenericConditionSig::Bound { subject, bound } => {
                        canonicalize_nested_ty(subject, &owner, names);
                        canonicalize_nested_ty(bound, &owner, names);
                    }
                    GenericConditionSig::Equal { left, right } => {
                        canonicalize_nested_ty(left, &owner, names);
                        canonicalize_nested_ty(right, &owner, names);
                    }
                }
            }
            for param in &mut method.params {
                canonicalize_nested_ty(&mut param.ty, &owner, names);
            }
            canonicalize_nested_ty(&mut method.ret, &owner, names);
        }
    }
    for case in sig.enum_cases.values_mut() {
        for param in &mut case.params {
            canonicalize_nested_ty(&mut param.ty, &owner, names);
        }
        canonicalize_nested_ty(&mut case.result, &owner, names);
    }
}

fn canonicalize_nested_ty(ty: &mut Ty, owner: &str, names: &HashSet<String>) {
    match ty {
        Ty::Named(name, args) => {
            for arg in args {
                canonicalize_nested_ty(arg, owner, names);
            }
            if let Some(canonical) =
                lexical_nested_name(owner, name, |candidate| names.contains(candidate))
            {
                *name = canonical;
            }
        }
        Ty::Union(members) | Ty::Tuple(members) => {
            for member in members {
                canonicalize_nested_ty(member, owner, names);
            }
        }
        Ty::Record(fields) => {
            for (_, field) in fields {
                canonicalize_nested_ty(field, owner, names);
            }
        }
        Ty::Function(params, ret) => {
            for param in params {
                canonicalize_nested_ty(param, owner, names);
            }
            canonicalize_nested_ty(ret, owner, names);
        }
        Ty::Unknown | Ty::Wildcard | Ty::Capture(_) | Ty::Never | Ty::TypeParam(_) => {}
    }
}

fn canonicalize_nested_type_ref(reference: &mut TypeRef, owner: &str, names: &HashSet<String>) {
    match reference {
        TypeRef::Named { name, args, .. } => {
            for arg in args {
                canonicalize_nested_type_ref(arg, owner, names);
            }
            if let Some(canonical) =
                lexical_nested_name(owner, name, |candidate| names.contains(candidate))
            {
                *name = canonical;
            }
        }
        TypeRef::Tuple { fields, .. } => {
            for field in fields {
                canonicalize_nested_type_ref(&mut field.ty, owner, names);
            }
        }
        TypeRef::Record { fields, .. } => {
            for field in fields {
                canonicalize_nested_type_ref(&mut field.ty, owner, names);
            }
        }
        TypeRef::Function { params, ret, .. } => {
            for param in params {
                canonicalize_nested_type_ref(param, owner, names);
            }
            canonicalize_nested_type_ref(ret, owner, names);
        }
        TypeRef::Union { members, .. } => {
            for member in members {
                canonicalize_nested_type_ref(member, owner, names);
            }
        }
        TypeRef::Wildcard { .. } => {}
    }
}

fn lexical_nested_name(
    owner: &str,
    name: &str,
    mut exists: impl FnMut(&str) -> bool,
) -> Option<String> {
    if name.contains('.') {
        return exists(name).then(|| name.to_string());
    }
    let mut scope = owner;
    loop {
        let candidate = format!("{scope}.{name}");
        if exists(&candidate) {
            return Some(candidate);
        }
        let Some((parent, _)) = scope.rsplit_once('.') else {
            return None;
        };
        scope = parent;
    }
}

fn canonicalize_qualified_function_sig(
    sig: &mut FunctionSig,
    owner: &ModuleInfo,
    modules: &HashMap<PathBuf, ModuleInfo>,
) {
    for condition in &mut sig.generic_conditions {
        match condition {
            GenericConditionSig::Bound { subject, bound } => {
                canonicalize_qualified_ty(subject, owner, modules);
                canonicalize_qualified_ty(bound, owner, modules);
            }
            GenericConditionSig::Equal { left, right } => {
                canonicalize_qualified_ty(left, owner, modules);
                canonicalize_qualified_ty(right, owner, modules);
            }
        }
    }
    for param in &mut sig.params {
        canonicalize_qualified_ty(&mut param.ty, owner, modules);
    }
    canonicalize_qualified_ty(&mut sig.ret, owner, modules);
}

fn canonicalize_qualified_type_sig(
    sig: &mut TypeSig,
    owner: &ModuleInfo,
    modules: &HashMap<PathBuf, ModuleInfo>,
) {
    for condition in &mut sig.generic_conditions {
        match condition {
            GenericConditionSig::Bound { subject, bound } => {
                canonicalize_qualified_ty(subject, owner, modules);
                canonicalize_qualified_ty(bound, owner, modules);
            }
            GenericConditionSig::Equal { left, right } => {
                canonicalize_qualified_ty(left, owner, modules);
                canonicalize_qualified_ty(right, owner, modules);
            }
        }
    }
    for bound in &mut sig.with_bounds {
        canonicalize_qualified_ty(bound, owner, modules);
    }
    for field in &mut sig.fields {
        canonicalize_qualified_ty(&mut field.ty, owner, modules);
    }
    for overloads in sig.methods.values_mut() {
        for method in overloads {
            canonicalize_qualified_function_sig(method, owner, modules);
        }
    }
    for case in sig.enum_cases.values_mut() {
        for param in &mut case.params {
            canonicalize_qualified_ty(&mut param.ty, owner, modules);
        }
        canonicalize_qualified_ty(&mut case.result, owner, modules);
    }
}

fn canonicalize_qualified_ty(
    ty: &mut Ty,
    owner: &ModuleInfo,
    modules: &HashMap<PathBuf, ModuleInfo>,
) {
    match ty {
        Ty::Named(name, args) => {
            for arg in args.iter_mut() {
                canonicalize_qualified_ty(arg, owner, modules);
            }
            let Some((module_alias, member)) = name.split_once('.') else {
                return;
            };
            let Some(target) = owner
                .imports
                .get(module_alias)
                .and_then(|path| modules.get(path))
            else {
                return;
            };
            if args.is_empty()
                && let Some(alias) = target.aliases.get(member)
            {
                let converted = convert_type_ref(alias, &HashSet::new());
                let mut expanded =
                    expand_aliases_in_ty(&converted, &target.aliases, &mut HashSet::new());
                canonicalize_qualified_ty(&mut expanded, target, modules);
                *ty = expanded;
            } else {
                *name = member.to_string();
            }
        }
        Ty::Union(members) | Ty::Tuple(members) => {
            for member in members {
                canonicalize_qualified_ty(member, owner, modules);
            }
        }
        Ty::Record(fields) => {
            for (_, field) in fields {
                canonicalize_qualified_ty(field, owner, modules);
            }
        }
        Ty::Function(params, ret) => {
            for param in params {
                canonicalize_qualified_ty(param, owner, modules);
            }
            canonicalize_qualified_ty(ret, owner, modules);
        }
        Ty::Unknown | Ty::Wildcard | Ty::Capture(_) | Ty::Never | Ty::TypeParam(_) => {}
    }
}

fn expand_aliases_in_ty(
    ty: &Ty,
    aliases: &HashMap<String, TypeRef>,
    visiting: &mut HashSet<String>,
) -> Ty {
    match ty {
        Ty::Named(name, args) if args.is_empty() && aliases.contains_key(name) => {
            if !visiting.insert(name.clone()) {
                return ty.clone();
            }
            let target = convert_type_ref(&aliases[name], &HashSet::new());
            let expanded = expand_aliases_in_ty(&target, aliases, visiting);
            visiting.remove(name);
            expanded
        }
        Ty::Named(name, args) => Ty::Named(
            name.clone(),
            args.iter()
                .map(|arg| expand_aliases_in_ty(arg, aliases, visiting))
                .collect(),
        ),
        Ty::Union(members) => Ty::union(
            members
                .iter()
                .map(|member| expand_aliases_in_ty(member, aliases, visiting)),
        ),
        Ty::Tuple(fields) => Ty::Tuple(
            fields
                .iter()
                .map(|field| expand_aliases_in_ty(field, aliases, visiting))
                .collect(),
        ),
        Ty::Record(fields) => Ty::Record(
            fields
                .iter()
                .map(|(name, ty)| (name.clone(), expand_aliases_in_ty(ty, aliases, visiting)))
                .collect(),
        ),
        Ty::Function(params, ret) => Ty::Function(
            params
                .iter()
                .map(|param| expand_aliases_in_ty(param, aliases, visiting))
                .collect(),
            Box::new(expand_aliases_in_ty(ret, aliases, visiting)),
        ),
        _ => ty.clone(),
    }
}

fn module_identity(path: &Path, program: &Program) -> String {
    program
        .module
        .as_ref()
        .map(|module| module.name.clone())
        .unwrap_or_else(|| path.display().to_string())
}

#[derive(Debug, Clone, Default)]
struct ModuleInfo {
    path: PathBuf,
    display_path: String,
    module_id: String,
    program: Program,
    imports: HashMap<String, PathBuf>,
    symbol_imports: HashMap<String, ImportedSymbol>,
    extension_imports: Vec<PathBuf>,
    functions: HashMap<String, Vec<FunctionSig>>,
    types: HashMap<String, TypeSig>,
    objects: HashMap<String, TypeSig>,
    aliases: HashMap<String, TypeRef>,
    type_visibilities: HashMap<String, Visibility>,
    extensions: HashMap<String, HashMap<String, Vec<FunctionSig>>>,
    global_binding_stmts: Vec<crate::ast::BindingStmt>,
}

impl ModuleInfo {
    fn from_loaded(module: &LoadedModule) -> Self {
        let mut info = Self {
            path: module.path.clone(),
            display_path: module.display_path.clone(),
            module_id: module_identity(&module.path, &module.program),
            program: module.program.clone(),
            imports: module.imports.clone(),
            symbol_imports: module.symbol_imports.clone(),
            extension_imports: module.extension_imports.clone(),
            functions: HashMap::new(),
            types: HashMap::new(),
            objects: HashMap::new(),
            aliases: HashMap::new(),
            type_visibilities: HashMap::new(),
            extensions: HashMap::new(),
            global_binding_stmts: Vec::new(),
        };
        info.collect_items();
        info
    }

    fn from_program(display_path: &str, program: &Program) -> Self {
        let path = PathBuf::from(display_path);
        let mut info = Self {
            module_id: module_identity(&path, program),
            path,
            display_path: display_path.to_string(),
            program: program.clone(),
            imports: HashMap::new(),
            symbol_imports: HashMap::new(),
            extension_imports: Vec::new(),
            functions: HashMap::new(),
            types: HashMap::new(),
            objects: HashMap::new(),
            aliases: HashMap::new(),
            type_visibilities: HashMap::new(),
            extensions: HashMap::new(),
            global_binding_stmts: Vec::new(),
        };
        info.collect_items();
        info
    }

    fn collect_items(&mut self) {
        let items = self.program.items.clone();

        // Collect aliases first so signatures do not depend on declaration order.
        for item in &items {
            match item {
                Item::TypeAlias(alias) => {
                    self.aliases
                        .insert(alias.name.clone(), alias.target.clone());
                    self.type_visibilities
                        .insert(alias.name.clone(), alias.visibility);
                }
                Item::Type(decl) => {
                    self.type_visibilities
                        .insert(decl.name.clone(), decl.visibility);
                }
                _ => {}
            }
        }

        for item in &items {
            match item {
                Item::Function(function) => {
                    self.functions
                        .entry(function.name.clone())
                        .or_default()
                        .push(function_sig_from_function(function, &[]));
                }
                Item::Type(decl) => {
                    let mut sig = type_sig_from_decl(decl);
                    sig.module_id = self.module_id.clone();
                    if decl.kind == TypeKind::Object {
                        self.objects.insert(decl.name.clone(), sig);
                    } else {
                        self.types.insert(decl.name.clone(), sig);
                    }
                }
                Item::TypeAlias(_) => {}
                Item::Statement(Stmt::Binding(binding)) => {
                    self.global_binding_stmts.push(binding.clone());
                }
                Item::Extension(block) => {
                    self.collect_extension(block);
                }
                _ => {}
            }
        }

        self.canonicalize_nested_types_in_signatures();
        self.expand_aliases_in_signatures();
    }

    fn effective_type_visibility(&self, name: &str) -> Option<Visibility> {
        let mut visibility = *self.type_visibilities.get(name)?;
        let mut path = name;
        while let Some((parent, _)) = path.rsplit_once('.') {
            let parent_visibility = *self.type_visibilities.get(parent)?;
            visibility = restricted_visibility(visibility, parent_visibility);
            path = parent;
        }
        Some(visibility)
    }

    fn canonicalize_nested_types_in_signatures(&mut self) {
        let names = self
            .types
            .keys()
            .chain(self.objects.keys())
            .chain(self.aliases.keys())
            .cloned()
            .collect::<HashSet<_>>();
        for (name, target) in &mut self.aliases {
            if let Some((owner, _)) = name.rsplit_once('.') {
                canonicalize_nested_type_ref(target, owner, &names);
            }
        }
        for sig in self.types.values_mut().chain(self.objects.values_mut()) {
            canonicalize_nested_type_sig(sig, &names);
        }
    }

    fn expand_aliases_in_signatures(&mut self) {
        let aliases = self.aliases.clone();
        for overloads in self.functions.values_mut() {
            for sig in overloads {
                expand_aliases_in_function_sig(sig, &aliases);
            }
        }
        for sig in self.types.values_mut().chain(self.objects.values_mut()) {
            expand_aliases_in_type_sig(sig, &aliases);
        }
        for methods in self.extensions.values_mut() {
            for overloads in methods.values_mut() {
                for sig in overloads {
                    expand_aliases_in_function_sig(sig, &aliases);
                }
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

    fn canonicalize_qualified_types_in_signatures(
        &mut self,
        modules: &HashMap<PathBuf, ModuleInfo>,
    ) {
        let owner = self.clone();
        for overloads in self.functions.values_mut() {
            for sig in overloads {
                canonicalize_qualified_function_sig(sig, &owner, modules);
            }
        }
        for sig in self.types.values_mut().chain(self.objects.values_mut()) {
            canonicalize_qualified_type_sig(sig, &owner, modules);
        }

        let mut extensions = HashMap::new();
        for (target, mut methods) in std::mem::take(&mut self.extensions) {
            for overloads in methods.values_mut() {
                for sig in overloads {
                    canonicalize_qualified_function_sig(sig, &owner, modules);
                }
            }
            let mut target_ty = Ty::Named(target.clone(), Vec::new());
            canonicalize_qualified_ty(&mut target_ty, &owner, modules);
            let target = match target_ty {
                Ty::Named(name, _) => name,
                _ => target,
            };
            extensions
                .entry(target)
                .or_insert_with(HashMap::new)
                .extend(methods);
        }
        self.extensions = extensions;
    }
}

#[derive(Debug, Clone, Default)]
struct AmbientInfo {
    functions: HashMap<String, Vec<FunctionSig>>,
    types: HashMap<String, TypeSig>,
    objects: HashMap<String, TypeSig>,
    aliases: HashMap<String, TypeRef>,
    enum_cases: HashMap<String, Vec<EnumCaseSig>>,
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
            for (name, overloads) in module.functions.clone() {
                let visible = overloads
                    .into_iter()
                    .filter(|function| function.visibility == Visibility::Default)
                    .collect::<Vec<_>>();
                if !visible.is_empty() {
                    ambient.functions.insert(name, visible);
                }
            }
            for (target, methods) in module.extensions.clone() {
                let target_methods = ambient.extensions.entry(target).or_default();
                for (name, overloads) in methods {
                    target_methods.entry(name).or_default().extend(
                        overloads
                            .into_iter()
                            .filter(|method| method.visibility == Visibility::Default),
                    );
                }
            }
            for (name, sig) in module.types {
                for (case_name, case_sig) in &sig.enum_cases {
                    ambient
                        .enum_cases
                        .entry(case_name.clone())
                        .or_default()
                        .push(case_sig.clone());
                }
                ambient.types.insert(name, sig);
            }
            for (name, sig) in module.objects {
                ambient.objects.insert(name, sig);
            }
            ambient.aliases.extend(module.aliases);
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
        TypeKind::Mixin => "mixin",
        TypeKind::Enum => "declared union",
    }
}

fn visibility_label(visibility: Visibility) -> &'static str {
    match visibility {
        Visibility::Default => "public",
        Visibility::Internal => "internal",
        Visibility::Private => "private",
    }
}

fn visibility_rank(visibility: Visibility) -> u8 {
    match visibility {
        Visibility::Private => 0,
        Visibility::Internal => 1,
        Visibility::Default => 2,
    }
}

fn restricted_visibility(left: Visibility, right: Visibility) -> Visibility {
    if visibility_rank(left) <= visibility_rank(right) {
        left
    } else {
        right
    }
}

fn custom_constructor_error(sig: &TypeSig) -> String {
    match sig.kind {
        TypeKind::Mixin => format!("mixin '{}' cannot declare constructors", sig.name),
        TypeKind::Annotation => format!(
            "only classes can declare custom constructors; annotation '{}' cannot declare constructors",
            sig.name
        ),
        TypeKind::Record => format!(
            "only classes can declare custom constructors; shape '{}' uses structural brace construction",
            sig.name
        ),
        TypeKind::Enum => format!(
            "only classes can declare custom constructors; declared union '{}' uses variants for construction",
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

fn is_builtin_primitive_type_name(name: &str) -> bool {
    matches!(name, "Bool" | "Decimal" | "Float" | "Int" | "Rune" | "Str")
}

fn primitive_implementation_diagnostic(name: &str, span: crate::source::Span) -> Diagnostic {
    Diagnostic::error(
        "invalid_primitive_implementation",
        format!("primitive type '{name}' is closed and cannot be used as an implementation target"),
        span,
    )
    .with_help(format!(
        "remove 'with {name}'; only built-in runtime values can have type '{name}'"
    ))
}

fn builtin_extension_type_sig(name: &str) -> Option<TypeSig> {
    if !is_builtin_primitive_type_name(name) {
        return None;
    }
    Some(TypeSig {
        kind: TypeKind::Class,
        name: name.to_string(),
        module_id: "<builtin>".to_string(),
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
        let mut world = Self {
            ambient: default_inline_ambient(),
            root_module: Some(ModuleInfo::from_program("<memory>", program)),
            ..Self::default()
        };
        world.compose_mixin_signatures();
        world
    }

    fn from_graph(graph: ModuleGraph, root: PathBuf, stdlib_dir: PathBuf) -> Result<Self, String> {
        let ambient = AmbientInfo::load(&stdlib_dir)?;
        let mut modules = HashMap::new();
        for (path, loaded) in graph.modules {
            modules.insert(path, ModuleInfo::from_loaded(&loaded));
        }
        let module_snapshot = modules.clone();
        for module in modules.values_mut() {
            module.canonicalize_qualified_types_in_signatures(&module_snapshot);
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
        let mut world = Self {
            modules,
            order,
            ambient,
            checked_globals: HashMap::new(),
            root_module: None,
        };
        world.compose_mixin_signatures();
        Ok(world)
    }

    fn compose_mixin_signatures(&mut self) {
        let mut additions = Vec::new();
        for module in self.modules.values().chain(self.root_module.iter()) {
            let mut checker = Checker::new(self, module);
            for item in &module.program.items {
                let Item::Type(decl) = item else { continue };
                if decl.kind != TypeKind::Class {
                    continue;
                }
                checker.current_type_paths.push(decl.name.clone());
                checker.push_ast_type_params(&decl.type_params, &decl.type_conditions);
                let mut methods = module.types[&decl.name].methods.clone();
                for (_, mixin, subst) in checker.mixin_uses(decl) {
                    for (name, overloads) in mixin.methods {
                        for method in overloads {
                            let method = instantiate_function_sig(method, &subst);
                            let target = methods.entry(name.clone()).or_default();
                            if !target.iter().any(|existing| {
                                mixin_signature_key(&name, existing)
                                    == mixin_signature_key(&name, &method)
                            }) {
                                target.push(method);
                            }
                        }
                    }
                }
                additions.push((module.path.clone(), decl.name.clone(), methods));
                checker.pop_type_params();
                checker.current_type_paths.pop();
            }
        }
        for (path, name, methods) in additions {
            let module = self.modules.get_mut(&path).or_else(|| {
                self.root_module
                    .as_mut()
                    .filter(|module| module.path == path)
            });
            if let Some(sig) = module.and_then(|module| module.types.get_mut(&name)) {
                sig.methods = methods;
            }
        }
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
        let (import_name, suffix) = name
            .split_once('.')
            .map_or((name, None), |(root, suffix)| (root, Some(suffix)));
        let imported = module.symbol_imports.get(import_name)?;
        let original_name = suffix
            .map(|suffix| format!("{}.{}", imported.original_name, suffix))
            .unwrap_or_else(|| imported.original_name.clone());
        if suffix.is_some()
            && matches!(
                imported.kind,
                ImportedKind::Type | ImportedKind::Interface | ImportedKind::Object
            )
        {
            let source = self.modules.get(&imported.module_path)?;
            return source
                .types
                .get(&original_name)
                .cloned()
                .or_else(|| source.objects.get(&original_name).cloned());
        }
        match imported.kind {
            ImportedKind::Type | ImportedKind::Interface => self
                .modules
                .get(&imported.module_path)?
                .types
                .get(&original_name)
                .cloned(),
            ImportedKind::Object => self
                .modules
                .get(&imported.module_path)?
                .objects
                .get(&original_name)
                .cloned(),
            _ => None,
        }
    }

    fn lookup_imported_alias(
        &self,
        module: &ModuleInfo,
        name: &str,
    ) -> Option<(PathBuf, String, TypeRef)> {
        let (import_name, suffix) = name
            .split_once('.')
            .map_or((name, None), |(root, suffix)| (root, Some(suffix)));
        let imported = module.symbol_imports.get(import_name)?;
        let original_name = suffix
            .map(|suffix| format!("{}.{}", imported.original_name, suffix))
            .unwrap_or_else(|| imported.original_name.clone());
        if suffix.is_some()
            && !matches!(
                imported.kind,
                ImportedKind::Type | ImportedKind::Interface | ImportedKind::Object
            )
            || suffix.is_none() && imported.kind != ImportedKind::TypeAlias
        {
            return None;
        }
        let source = self.modules.get(&imported.module_path)?;
        source
            .aliases
            .get(&original_name)
            .cloned()
            .map(|target| (source.path.clone(), original_name, target))
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

    fn lookup_bare_enum_cases(&self, module: &ModuleInfo, name: &str) -> Vec<NamedEnumCaseSig> {
        let mut local = module
            .types
            .values()
            .filter_map(|sig| {
                sig.enum_cases
                    .get(name)
                    .cloned()
                    .map(|case| NamedEnumCaseSig {
                        owner: sig.name.clone(),
                        case,
                    })
            })
            .collect::<Vec<_>>();
        if !local.is_empty() {
            local.sort_by(|left, right| left.owner.cmp(&right.owner));
            return local;
        }

        if let Some(imported) = module.symbol_imports.get(name)
            && imported.kind == ImportedKind::UnionAlternative
            && let Some(owner) = imported.object_name.as_deref()
            && let Some(case) = self
                .modules
                .get(&imported.module_path)
                .and_then(|source| source.types.get(owner))
                .and_then(|sig| sig.enum_cases.get(&imported.original_name))
        {
            return vec![NamedEnumCaseSig {
                owner: owner.to_string(),
                case: case.clone(),
            }];
        }

        let mut ambient = self
            .ambient
            .enum_cases
            .get(name)
            .into_iter()
            .flatten()
            .cloned()
            .map(|case| NamedEnumCaseSig {
                owner: enum_case_result_owner(&case).unwrap_or_default(),
                case,
            })
            .collect::<Vec<_>>();
        ambient.sort_by(|left, right| left.owner.cmp(&right.owner));
        ambient
    }

    fn lookup_enum_case(&self, module: &ModuleInfo, name: &str) -> Option<EnumCaseSig> {
        let mut candidates = self.lookup_bare_enum_cases(module, name);
        (candidates.len() == 1).then(|| candidates.remove(0).case)
    }

    fn ambiguous_enum_case_owners(&self, module: &ModuleInfo, name: &str) -> Vec<String> {
        let candidates = self.lookup_bare_enum_cases(module, name);
        if candidates.len() < 2 {
            return Vec::new();
        }
        candidates
            .into_iter()
            .map(|candidate| candidate.owner)
            .collect()
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
    warnings: Vec<Diagnostic>,
    scopes: Vec<HashMap<String, ValueInfo>>,
    mutable_bindings: Vec<HashMap<String, MutableBindingUsage>>,
    type_params: Vec<TypeParamScope>,
    current_return: Ty,
    current_owner: Option<TypeSig>,
    current_method: Option<String>,
    current_getter: bool,
    current_extension_target: Option<String>,
    callable_depth: usize,
    local_functions_allowed: bool,
    loop_depth: usize,
    defer_depth: usize,
    next_capture_id: usize,
    capture_labels: HashMap<usize, String>,
    globals: HashMap<String, ValueInfo>,
    anonymous_types: HashMap<String, TypeSig>,
    current_type_paths: Vec<String>,
    reported_member_name_collisions: HashSet<String>,
}

impl<'a> Checker<'a> {
    fn new(world: &'a World, module: &'a ModuleInfo) -> Self {
        Self {
            world,
            module,
            diagnostics: Vec::new(),
            warnings: Vec::new(),
            scopes: Vec::new(),
            mutable_bindings: Vec::new(),
            type_params: Vec::new(),
            current_return: Ty::Unknown,
            current_owner: None,
            current_method: None,
            current_getter: false,
            current_extension_target: None,
            callable_depth: 0,
            local_functions_allowed: false,
            loop_depth: 0,
            defer_depth: 0,
            next_capture_id: 0,
            capture_labels: HashMap::new(),
            globals: HashMap::new(),
            anonymous_types: HashMap::new(),
            current_type_paths: Vec::new(),
            reported_member_name_collisions: HashSet::new(),
        }
    }

    fn check_module(&mut self) {
        self.check_global_bindings();
        self.check_api_type_visibility();
        for item in &self.module.program.items {
            match item {
                Item::Function(function) => self.check_function(function),
                Item::TypeAlias(alias) => self.check_type_alias(alias),
                Item::Type(decl) => self.check_type_decl(decl),
                Item::Extension(block) => self.check_extension(block),
                _ => {}
            }
        }
        self.check_constructor_delegation_cycles();
    }

    fn check_api_type_visibility(&mut self) {
        let items = self.module.program.items.clone();
        for item in &items {
            match item {
                Item::Function(function) => {
                    self.check_callable_api_type_visibility(
                        function.visibility,
                        &format!("function '{}'", function.name),
                        &function.type_params,
                        &function.type_conditions,
                        &function.params,
                        function.return_type.as_ref(),
                        None,
                    );
                }
                Item::TypeAlias(alias) => {
                    let Some(exposure) = self.module.effective_type_visibility(&alias.name) else {
                        continue;
                    };
                    self.check_api_type_ref(
                        &alias.target,
                        exposure,
                        &format!("type alias '{}'", alias.name),
                        &HashSet::new(),
                        alias.name.rsplit_once('.').map(|(owner, _)| owner),
                    );
                }
                Item::Type(decl) => self.check_type_api_visibility(decl),
                Item::Statement(Stmt::Binding(binding)) => {
                    if binding.visibility == Visibility::Private {
                        continue;
                    }
                    for local in &binding.bindings {
                        let api = format!("binding '{}'", local.name);
                        if let Some(reference) = &local.ty {
                            self.check_api_type_ref(
                                reference,
                                binding.visibility,
                                &api,
                                &HashSet::new(),
                                None,
                            );
                        } else if let Some(info) = self.globals.get(&local.name) {
                            let ty = info.ty.clone();
                            self.check_api_ty(&ty, binding.visibility, &api, local.span);
                        }
                    }
                }
                Item::Extension(_) | Item::Statement(_) => {}
            }
        }
    }

    fn check_type_api_visibility(&mut self, decl: &TypeDecl) {
        let Some(owner_exposure) = self.module.effective_type_visibility(&decl.name) else {
            return;
        };
        if owner_exposure == Visibility::Private {
            return;
        }

        let owner_type_params = decl
            .type_params
            .iter()
            .map(|param| param.name.clone())
            .collect::<HashSet<_>>();
        let api = format!("{} '{}'", type_kind_label(decl.kind), decl.name);
        self.check_api_generic_clause(
            &decl.type_params,
            &decl.type_conditions,
            owner_exposure,
            &api,
            &owner_type_params,
            Some(&decl.name),
        );
        for member in &decl.members {
            match member {
                TypeMember::Field(field) => {
                    let exposure = restricted_visibility(owner_exposure, field.visibility);
                    if exposure == Visibility::Private {
                        continue;
                    }
                    if let Some(reference) = &field.ty {
                        self.check_api_type_ref(
                            reference,
                            exposure,
                            &format!("field '{}.{}'", decl.name, field.name),
                            &owner_type_params,
                            Some(&decl.name),
                        );
                    }
                }
                TypeMember::Method(method) => {
                    let exposure = restricted_visibility(owner_exposure, method.visibility);
                    let label = if method.name == "new" {
                        format!("constructor for '{}'", decl.name)
                    } else if method.getter {
                        format!("getter '{}.{}'", decl.name, method.name)
                    } else {
                        format!("method '{}.{}'", decl.name, method.name)
                    };
                    self.check_callable_api_type_visibility(
                        exposure,
                        &label,
                        &method.type_params,
                        &method.type_conditions,
                        &method.params,
                        method.return_type.as_ref(),
                        Some((&owner_type_params, decl.name.as_str())),
                    );
                }
                TypeMember::Case(case) => {
                    for field in &case.fields {
                        if let Some(reference) = &field.ty {
                            self.check_api_type_ref(
                                reference,
                                owner_exposure,
                                &format!(
                                    "declared-union alternative '{}.{}'",
                                    decl.name, case.name
                                ),
                                &owner_type_params,
                                Some(&decl.name),
                            );
                        }
                    }
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn check_callable_api_type_visibility(
        &mut self,
        exposure: Visibility,
        api: &str,
        type_params: &[TypeParam],
        conditions: &[GenericCondition],
        params: &[Param],
        return_type: Option<&TypeRef>,
        owner: Option<(&HashSet<String>, &str)>,
    ) {
        if exposure == Visibility::Private {
            return;
        }
        let mut names = owner.map(|(names, _)| names.clone()).unwrap_or_default();
        names.extend(type_params.iter().map(|param| param.name.clone()));
        let owner_name = owner.map(|(_, name)| name);
        self.check_api_generic_clause(type_params, conditions, exposure, api, &names, owner_name);
        for param in params {
            if let Some(reference) = &param.ty {
                self.check_api_type_ref(reference, exposure, api, &names, owner_name);
            }
        }
        if let Some(reference) = return_type {
            self.check_api_type_ref(reference, exposure, api, &names, owner_name);
        }
    }

    fn check_api_generic_clause(
        &mut self,
        params: &[TypeParam],
        conditions: &[GenericCondition],
        exposure: Visibility,
        api: &str,
        type_params: &HashSet<String>,
        owner: Option<&str>,
    ) {
        for param in params {
            for bound in &param.bounds {
                self.check_api_type_ref(bound, exposure, api, type_params, owner);
            }
        }
        for condition in conditions {
            match condition {
                GenericCondition::Bound { subject, bound, .. } => {
                    self.check_api_type_ref(subject, exposure, api, type_params, owner);
                    self.check_api_type_ref(bound, exposure, api, type_params, owner);
                }
                GenericCondition::Equal { left, right, .. } => {
                    self.check_api_type_ref(left, exposure, api, type_params, owner);
                    self.check_api_type_ref(right, exposure, api, type_params, owner);
                }
            }
        }
    }

    fn check_api_type_ref(
        &mut self,
        reference: &TypeRef,
        exposure: Visibility,
        api: &str,
        type_params: &HashSet<String>,
        owner: Option<&str>,
    ) {
        match reference {
            TypeRef::Wildcard { .. } => {}
            TypeRef::Named { name, args, span } => {
                if !type_params.contains(name)
                    && let Some((visibility, declared_name)) =
                        self.named_type_visibility(name, owner)
                    && visibility_rank(visibility) < visibility_rank(exposure)
                {
                    self.diagnostics.push(
                        Diagnostic::error(
                            "inaccessible_type_in_api",
                            format!(
                                "{} API {} exposes {} type '{}'",
                                visibility_label(exposure),
                                api,
                                visibility_label(visibility),
                                declared_name
                            ),
                            *span,
                        )
                        .with_help(format!(
                            "make '{}' at least {}, or reduce the visibility of {}",
                            declared_name,
                            visibility_label(exposure),
                            api
                        )),
                    );
                }
                for arg in args {
                    self.check_api_type_ref(arg, exposure, api, type_params, owner);
                }
            }
            TypeRef::Tuple { fields, .. } => {
                for field in fields {
                    self.check_api_type_ref(&field.ty, exposure, api, type_params, owner);
                }
            }
            TypeRef::Record { fields, .. } => {
                for field in fields {
                    self.check_api_type_ref(&field.ty, exposure, api, type_params, owner);
                }
            }
            TypeRef::Function { params, ret, .. } => {
                for param in params {
                    self.check_api_type_ref(param, exposure, api, type_params, owner);
                }
                self.check_api_type_ref(ret, exposure, api, type_params, owner);
            }
            TypeRef::Union { members, .. } => {
                for member in members {
                    self.check_api_type_ref(member, exposure, api, type_params, owner);
                }
            }
        }
    }

    fn check_api_ty(
        &mut self,
        ty: &Ty,
        exposure: Visibility,
        api: &str,
        span: crate::source::Span,
    ) {
        match ty {
            Ty::Named(name, args) => {
                if let Some((visibility, declared_name)) = self.named_type_visibility(name, None)
                    && visibility_rank(visibility) < visibility_rank(exposure)
                {
                    self.add_error(
                        "inaccessible_type_in_api",
                        format!(
                            "{} API {} exposes {} type '{}'",
                            visibility_label(exposure),
                            api,
                            visibility_label(visibility),
                            declared_name
                        ),
                        span,
                    );
                }
                for arg in args {
                    self.check_api_ty(arg, exposure, api, span);
                }
            }
            Ty::Tuple(fields) | Ty::Union(fields) => {
                for field in fields {
                    self.check_api_ty(field, exposure, api, span);
                }
            }
            Ty::Record(fields) => {
                for (_, field) in fields {
                    self.check_api_ty(field, exposure, api, span);
                }
            }
            Ty::Function(params, ret) => {
                for param in params {
                    self.check_api_ty(param, exposure, api, span);
                }
                self.check_api_ty(ret, exposure, api, span);
            }
            Ty::Wildcard | Ty::Capture(_) | Ty::TypeParam(_) | Ty::Never | Ty::Unknown => {}
        }
    }

    fn named_type_visibility(
        &self,
        name: &str,
        owner: Option<&str>,
    ) -> Option<(Visibility, String)> {
        if let Some(visibility) = self.module.effective_type_visibility(name) {
            return Some((visibility, name.to_string()));
        }

        if !name.contains('.')
            && let Some(owner) = owner
            && let Some(candidate) = lexical_nested_name(owner, name, |candidate| {
                self.module.type_visibilities.contains_key(candidate)
            })
            && let Some(visibility) = self.module.effective_type_visibility(&candidate)
        {
            return Some((visibility, candidate));
        }

        let (root, suffix) = name
            .split_once('.')
            .map_or((name, None), |(root, suffix)| (root, Some(suffix)));
        if let Some(imported) = self.module.symbol_imports.get(root) {
            let source = self.world.modules.get(&imported.module_path)?;
            let declared_name = suffix
                .map(|suffix| format!("{}.{}", imported.original_name, suffix))
                .unwrap_or_else(|| imported.original_name.clone());
            if let Some(visibility) = source.effective_type_visibility(&declared_name) {
                return Some((visibility, declared_name));
            }
        }

        if let Some(suffix) = suffix
            && let Some(path) = self.module.imports.get(root)
            && let Some(source) = self.world.modules.get(path)
            && let Some(visibility) = source.effective_type_visibility(suffix)
        {
            return Some((visibility, suffix.to_string()));
        }

        let mut imported_matches = self.module.symbol_imports.values().filter_map(|imported| {
            (imported.original_name == name).then(|| {
                let source = self.world.modules.get(&imported.module_path)?;
                let visibility = source.effective_type_visibility(&imported.original_name)?;
                Some((visibility, imported.original_name.clone()))
            })?
        });
        let first = imported_matches.next();
        if first.is_some() && imported_matches.next().is_none() {
            return first;
        }

        None
    }

    fn check_type_alias(&mut self, alias: &crate::ast::TypeAliasDecl) {
        if let Some((owner, _)) = alias.name.rsplit_once('.') {
            self.current_type_paths.push(owner.to_string());
        }
        let mut visiting = vec![(self.module.path.clone(), alias.name.clone())];
        if self.type_ref_has_alias_cycle(&alias.target, self.module, &mut visiting) {
            self.add_error(
                "cyclic_type_alias",
                format!("type alias '{}' contains a cycle", alias.name),
                alias.span,
            );
        }
        self.validate_type_ref_generic_applications(&alias.target);
        if alias.name.contains('.') {
            self.current_type_paths.pop();
        }
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
            .unwrap_or_else(Ty::unit);
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
        self.check_declared_member_name_collisions(decl);
        self.current_type_paths.push(decl.name.clone());
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
        self.check_mixin_composition(decl, &type_sig);
        for bound in &decl.with_bounds {
            let Ty::Named(name, args) = self.ty_from_type_ref(bound) else {
                continue;
            };
            if self
                .lookup_any_type(&name)
                .is_some_and(|sig| sig.kind == TypeKind::Mixin)
            {
                self.add_error(
                    "mixin_not_a_type",
                    "mixins are adopted inside classes with 'use', not listed as interface bounds",
                    bound.span(),
                );
            }
            if args.is_empty() && is_builtin_primitive_type_name(&name) {
                self.diagnostics
                    .push(primitive_implementation_diagnostic(&name, bound.span()));
            }
        }
        for member in &decl.members {
            match member {
                TypeMember::Field(field) => {
                    if field.name == "referenceId" {
                        self.add_error(
                            "reserved_synthetic_member",
                            "'referenceId' is synthetic instance metadata and cannot be declared as a stored field",
                            field.span,
                        );
                    }
                    if let Some(ty) = &field.ty {
                        self.validate_type_ref_generic_applications(ty);
                    }
                }
                TypeMember::Case(case) => {
                    for field in &case.fields {
                        if field.name == "referenceId" {
                            self.add_error(
                                "reserved_synthetic_member",
                                "'referenceId' is synthetic instance metadata and cannot be declared as a stored field",
                                field.span,
                            );
                        }
                        if let Some(ty) = &field.ty {
                            self.validate_type_ref_generic_applications(ty);
                        }
                    }
                }
                TypeMember::Method(method) => {
                    if method.name == "referenceId" {
                        self.add_error(
                            "reserved_synthetic_member",
                            "'referenceId' is synthetic instance metadata and cannot be declared as a method or getter",
                            method.span,
                        );
                    }
                }
            }
        }

        for member in &decl.members {
            match member {
                TypeMember::Field(field) => {
                    if decl.kind == TypeKind::Interface {
                        self.diagnostics
                            .push(typecheck_diagnostics::interface_field(
                                &field.name,
                                field.span,
                            ));
                        continue;
                    }
                    if decl.kind == TypeKind::Annotation {
                        if field.visibility != Visibility::Default {
                            self.add_error(
                                "invalid_annotation_field",
                                format!(
                                    "annotation '{}' cannot declare {} field '{}'; annotation fields are public metadata",
                                    decl.name,
                                    visibility_label(field.visibility),
                                    field.name
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
                }
                TypeMember::Method(method) => {
                    if decl.kind == TypeKind::Interface
                        && let Some(body) = &method.body
                    {
                        self.diagnostics
                            .push(typecheck_diagnostics::interface_member_body(body.span()));
                        continue;
                    }
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
                        if case.kind == TypeKind::Record && field.mutable {
                            self.add_error(
                                "invalid_shape_field",
                                format!(
                                    "shape union variant '{}' cannot declare mutable field '{}'",
                                    case.name, field.name
                                ),
                                field.span,
                            );
                        }
                    }
                }
            }
        }

        self.check_implicit_constructor_contract(decl);
        self.check_interface_implementation(&type_sig, decl.span);
        self.check_type_field_initializers(decl, &type_sig);

        self.pop_type_params();
        self.current_type_paths.pop();
    }

    fn check_declared_member_name_collisions(&mut self, decl: &TypeDecl) {
        let mut members = HashMap::<String, (MemberNamespaceKind, crate::source::Span)>::new();

        for member in &decl.members {
            let entries = match member {
                TypeMember::Field(field) => {
                    vec![(&field.name, MemberNamespaceKind::Field, field.span)]
                }
                TypeMember::Method(method) => vec![(
                    &method.name,
                    if method.getter {
                        MemberNamespaceKind::Getter
                    } else {
                        MemberNamespaceKind::Method
                    },
                    method.span,
                )],
                TypeMember::Case(case) => case
                    .fields
                    .iter()
                    .map(|field| (&field.name, MemberNamespaceKind::Field, field.span))
                    .collect(),
            };

            for (name, kind, span) in entries {
                if let Some((previous_kind, _)) = members.get(name).copied() {
                    if previous_kind.conflicts_with(kind) {
                        self.report_member_name_collision(
                            &decl.name,
                            name,
                            previous_kind,
                            kind,
                            span,
                        );
                    }
                } else {
                    members.insert(name.clone(), (kind, span));
                }
            }
        }
    }

    fn report_member_name_collision(
        &mut self,
        owner: &str,
        name: &str,
        previous: MemberNamespaceKind,
        current: MemberNamespaceKind,
        span: crate::source::Span,
    ) {
        let key = format!("{owner}::{name}");
        if !self.reported_member_name_collisions.insert(key) {
            return;
        }
        let message = if previous == MemberNamespaceKind::Getter
            && current == MemberNamespaceKind::Getter
        {
            format!(
                "getter '{name}' is declared more than once on '{owner}'; getters cannot be overloaded"
            )
        } else {
            format!(
                "member '{name}' on '{owner}' is declared as both a {} and a {}; stored fields, getters, and methods must use distinct names",
                previous.label(),
                current.label()
            )
        };
        self.add_error("member_name_collision", message, span);
    }

    fn check_implicit_constructor_contract(&mut self, decl: &TypeDecl) {
        let has_explicit_constructor = decl
            .members
            .iter()
            .any(|member| matches!(member, TypeMember::Method(method) if method.name == "new"));

        if decl.kind == TypeKind::Class && !has_explicit_constructor {
            for member in &decl.members {
                let TypeMember::Field(field) = member else {
                    continue;
                };
                if field.visibility == Visibility::Default || field.initializer.is_some() {
                    continue;
                }
                self.add_error(
                    "missing_non_public_field_initializer",
                    format!(
                        "class '{}' cannot synthesize an implicit constructor because {} field '{}' has no initializer; add an initializer or declare 'new(...)'",
                        decl.name,
                        visibility_label(field.visibility),
                        field.name
                    ),
                    field.span,
                );
            }
        }
    }

    fn check_interface_implementation(&mut self, sig: &TypeSig, span: crate::source::Span) {
        if matches!(sig.kind, TypeKind::Interface | TypeKind::Mixin) {
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
            if is_builtin_primitive_type_name(interface_name) {
                continue;
            }
            let Some(interface_sig) = self.lookup_any_type(interface_name) else {
                continue;
            };
            if interface_sig.kind != TypeKind::Interface {
                continue;
            }

            self.check_interface_bound_implementation(
                sig,
                bound,
                &interface_sig.name,
                &format!("{} '{}'", type_kind_label(sig.kind), sig.name),
                span,
            );
        }
    }

    fn mixin_uses(&self, decl: &TypeDecl) -> Vec<(TypeRef, TypeSig, HashMap<String, Ty>)> {
        decl.mixins
            .iter()
            .filter_map(|target| {
                let Ty::Named(name, args) = self.ty_from_type_ref(target) else {
                    return None;
                };
                let sig = self.lookup_any_type(&name)?;
                if sig.kind != TypeKind::Mixin || sig.type_params.len() != args.len() {
                    return None;
                }
                let subst = sig.type_params.iter().cloned().zip(args).collect();
                Some((target.clone(), sig, subst))
            })
            .collect()
    }

    fn check_mixin_composition(&mut self, decl: &TypeDecl, owner: &TypeSig) {
        if decl.kind == TypeKind::Mixin {
            for bound in &decl.with_bounds {
                let ty = self.ty_from_type_ref(bound);
                if !matches!(&ty, Ty::Named(name, _) if self.lookup_any_type(name).is_some_and(|sig| sig.kind == TypeKind::Interface))
                {
                    self.add_error(
                        "invalid_mixin_requirement",
                        "mixin requirements must be interfaces",
                        bound.span(),
                    );
                }
            }
        }
        let mut seen = HashSet::new();
        for target in &decl.mixins {
            let ty = self.ty_from_type_ref(target);
            let sig = match &ty {
                Ty::Named(name, _) => self.lookup_any_type(name),
                _ => None,
            };
            if !sig.as_ref().is_some_and(|sig| sig.kind == TypeKind::Mixin) {
                self.add_error(
                    "invalid_mixin_use",
                    "'use' in a class must name a mixin declaration",
                    target.span(),
                );
            } else if let Ty::Named(_, args) = &ty {
                let sig = sig.unwrap();
                if args.len() != sig.type_params.len() {
                    self.add_error(
                        "invalid_type_argument_count",
                        format!(
                            "mixin '{}' expects {} type arguments, got {}",
                            sig.name,
                            sig.type_params.len(),
                            args.len()
                        ),
                        target.span(),
                    );
                }
            }
            if !seen.insert(ty.describe()) {
                self.add_error(
                    "duplicate_mixin_use",
                    "a class cannot adopt the same mixin more than once",
                    target.span(),
                );
            }
        }
        let mut candidates =
            HashMap::<String, Vec<(String, String, FunctionSig, crate::source::Span)>>::new();
        for (target, mixin, subst) in self.mixin_uses(decl) {
            for requirement in &mixin.with_bounds {
                let required = substitute_type(requirement, &subst);
                if !self.is_assignable(&self.owner_self_ty(owner), &required) {
                    self.add_error(
                        "missing_mixin_requirement",
                        format!(
                            "class '{}' must implement '{}' to use mixin '{}'",
                            decl.name,
                            required.describe(),
                            mixin.name
                        ),
                        target.span(),
                    );
                } else if !self.mixin_requirement_members_are_compatible(owner, &required) {
                    self.add_error("incompatible_mixin_requirement", format!("class '{}' does not provide compatible public members for mixin '{}' requirement '{}'", decl.name, mixin.name, required.describe()), target.span());
                }
            }
            self.check_call_generic_conditions(&mixin.generic_conditions, &subst, target.span());
            for (name, overloads) in mixin.methods {
                for method in overloads {
                    let method = instantiate_function_sig(method, &subst);
                    let key = mixin_signature_key(&name, &method);
                    candidates.entry(key).or_default().push((
                        mixin.name.clone(),
                        name.clone(),
                        method,
                        target.span(),
                    ));
                }
            }
        }
        let own_methods = decl
            .members
            .iter()
            .filter_map(|member| match member {
                TypeMember::Method(method) => Some(method),
                _ => None,
            })
            .collect::<Vec<_>>();
        for group in candidates.values() {
            let (_, name, supplied, span) = &group[0];
            if owner.fields.iter().any(|field| field.name == *name) {
                self.add_error(
                    "mixin_member_kind_mismatch",
                    format!("stored field '{name}' cannot replace a mixin method or getter"),
                    *span,
                );
            }
            let replacements = own_methods
                .iter()
                .filter_map(|method| {
                    let mut sig = function_sig_from_method(method, &owner.type_params);
                    expand_aliases_in_function_sig(&mut sig, &self.module.aliases);
                    (mixin_signature_key(&method.name, &sig) == mixin_signature_key(name, supplied))
                        .then_some((*method, sig))
                })
                .collect::<Vec<_>>();
            if replacements.is_empty() && group.len() > 1 {
                self.add_error("mixin_member_conflict", format!("mixins {} both supply '{name}'; resolve the conflict with a compatible declaration in class '{}'", group.iter().map(|(owner, _, _, _)| format!("'{owner}'")).collect::<Vec<_>>().join(" and "), decl.name), *span);
            }
            for (method, replacement) in &replacements {
                let replacement = normalized_mixin_signature(replacement);
                for (mixin, _, supplied, _) in group {
                    let supplied = normalized_mixin_signature(supplied);
                    let compatible = replacement.getter == supplied.getter
                        && visibility_rank(replacement.visibility)
                            >= visibility_rank(supplied.visibility)
                        && replacement.params.iter().zip(&supplied.params).all(
                            |(actual, expected)| {
                                actual.name == expected.name
                                    && (!expected.has_initializer || actual.has_initializer)
                            },
                        )
                        && self.is_assignable(&replacement.ret, &supplied.ret)
                        && replacement.generic_conditions == supplied.generic_conditions
                        && replacement.reified_type_params == supplied.reified_type_params;
                    if !compatible {
                        self.add_error("incompatible_mixin_override", format!("member '{name}' on '{}' is incompatible with the declaration supplied by mixin '{mixin}'", decl.name), method.span);
                    }
                }
            }
            if own_methods
                .iter()
                .any(|method| method.name == *name && method.getter != supplied.getter)
                || candidates
                    .values()
                    .any(|other| other[0].1 == *name && other[0].2.getter != supplied.getter)
            {
                self.add_error("mixin_member_kind_mismatch", format!("'{name}' must have the same member kind in the class and all adopted mixins"), *span);
            }
        }
    }

    fn mixin_requirement_members_are_compatible(&self, owner: &TypeSig, requirement: &Ty) -> bool {
        let required = self.effective_interface_methods(requirement, &mut HashSet::new());
        let actual = self.owner_self_ty(owner);
        required.iter().all(|(name, overloads)| {
            overloads.iter().all(|expected| {
                if expected.getter
                    && owner.fields.iter().any(|field| {
                        field.name == *name
                            && field.visibility == Visibility::Default
                            && self.is_assignable(&field.ty, &expected.ret)
                    })
                {
                    return true;
                }
                let methods = if expected.getter {
                    self.member_getter_sig(&actual, name)
                        .map(|method| vec![method])
                } else {
                    self.member_method_sigs(&actual, name)
                };
                methods.is_some_and(|methods| {
                    methods.iter().any(|method| {
                        if method.visibility != Visibility::Default
                            || mixin_signature_key(name, method)
                                != mixin_signature_key(name, expected)
                        {
                            return false;
                        }
                        let method = normalized_mixin_signature(method);
                        let expected = normalized_mixin_signature(expected);
                        self.is_assignable(&method.ret, &expected.ret)
                            && method.generic_conditions == expected.generic_conditions
                            && method.reified_type_params == expected.reified_type_params
                    })
                })
            })
        })
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
                                score += if actual == &expected { 3 } else { 2 };
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
        self.check_extension_member_name_collisions(target_name, &type_sig, block);
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

    fn check_extension_member_name_collisions(
        &mut self,
        target_name: &str,
        type_sig: &TypeSig,
        block: &ExtensionBlock,
    ) {
        let mut checked = HashSet::new();
        for method in &block.methods {
            if !checked.insert(method.name.as_str()) {
                continue;
            }
            let current_kind = if method.getter {
                MemberNamespaceKind::Getter
            } else {
                MemberNamespaceKind::Method
            };
            if type_sig
                .fields
                .iter()
                .any(|field| field.name == method.name)
            {
                self.report_member_name_collision(
                    target_name,
                    &method.name,
                    MemberNamespaceKind::Field,
                    current_kind,
                    method.span,
                );
                continue;
            }

            let declared = type_sig.methods.get(&method.name).into_iter().flatten();
            let extensions = self
                .module
                .extensions
                .get(target_name)
                .and_then(|methods| methods.get(&method.name))
                .into_iter()
                .flatten();
            let overloads = declared.chain(extensions).collect::<Vec<_>>();
            if overloads.len() <= 1 || !overloads.iter().any(|candidate| candidate.getter) {
                continue;
            }
            let previous_kind = if current_kind == MemberNamespaceKind::Method
                || overloads.iter().all(|candidate| candidate.getter)
            {
                MemberNamespaceKind::Getter
            } else {
                MemberNamespaceKind::Method
            };
            self.report_member_name_collision(
                target_name,
                &method.name,
                previous_kind,
                current_kind,
                method.span,
            );
        }
    }

    fn check_method(&mut self, method: &MethodDecl, owner: &TypeSig) {
        let previous_return = self.current_return.clone();
        let previous_owner = self.current_owner.clone();
        let previous_method = self.current_method.clone();
        let previous_getter = self.current_getter;
        let previous_defer_depth = self.defer_depth;
        let previous_callable_depth = self.callable_depth;
        self.push_ast_type_params(&method.type_params, &method.type_conditions);
        if method.getter && (!method.type_params.is_empty() || !method.type_conditions.is_empty()) {
            self.add_error(
                "generic_getter",
                format!(
                    "getter '{}' cannot declare type parameters or generic conditions",
                    method.name
                ),
                method.span,
            );
        }
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
            .unwrap_or_else(Ty::unit);
        if method.getter && expected_return == Ty::unit() {
            self.add_error(
                "getter_unit_return",
                format!(
                    "getter '{}' cannot return Unit; use a method for an operation that does not produce a value",
                    method.name
                ),
                method
                    .return_type
                    .as_ref()
                    .map(TypeRef::span)
                    .unwrap_or(method.span),
            );
        }
        self.current_return = expected_return.clone();
        self.defer_depth = 0;
        self.callable_depth += 1;
        self.current_owner = Some(owner.clone());
        self.current_method = Some(method.name.clone());
        self.current_getter = method.getter;
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
        self.current_getter = previous_getter;
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
        let is_none_literal = matches!(initializer, Expr::Identifier { name, .. } if name == "None")
            && matches!(actual, Ty::Named(ref name, ref args) if name == "Option" && args.len() == 1);
        if infer_literal_type(initializer).is_none() && !is_none_literal {
            self.add_error(
                "invalid_constructor_default",
                "parameter defaults must be literal constants or None for now",
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
            | Expr::Decimal { .. }
            | Expr::String { .. }
            | Expr::Bool { .. }
            | Expr::Unit { .. } => {}
            Expr::Spread { value, .. } => {
                self.check_field_initializer_expr(value, owner, initialized_fields);
            }
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
            Expr::ContextualNew { args, .. } => {
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
            Expr::RecordLiteral { fields, values, .. } => {
                for field in fields {
                    self.check_field_initializer_expr(&field.value, owner, initialized_fields);
                }
                for value in values {
                    self.check_field_initializer_expr(value, owner, initialized_fields);
                }
            }
            Expr::AnonymousObject { .. } | Expr::Lambda { .. } => {}
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
                condition_clauses,
                then_block,
                else_branch,
                ..
            } => {
                for clause in condition_clauses {
                    self.check_field_initializer_expr(
                        condition_clause_expr(clause),
                        owner,
                        initialized_fields,
                    );
                }
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
                let previous = self.local_functions_allowed;
                self.local_functions_allowed = true;
                let result =
                    self.check_block_against_current_scope(block, &self.current_return.clone());
                self.local_functions_allowed = previous;
                result
            }
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
                        .filter(|field| self.can_access_field(&sig, field))
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

    fn check_block_against(&mut self, block: &Block, expected: &Ty) -> Ty {
        let previous = self.local_functions_allowed;
        self.local_functions_allowed = false;
        let result = self.check_block_against_current_scope(block, expected);
        self.local_functions_allowed = previous;
        result
    }

    fn check_block_against_current_scope(&mut self, block: &Block, expected: &Ty) -> Ty {
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
                        for narrowing in self.type_narrowings_for_condition(condition, true) {
                            self.define_local(&narrowing.name, narrowing.ty, false);
                        }
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
        let then_narrowings = stmt
            .condition
            .as_ref()
            .map(|condition| self.type_narrowings_for_condition(condition, true))
            .unwrap_or_default();
        let else_narrowings = stmt
            .condition
            .as_ref()
            .map(|condition| self.type_narrowings_for_condition(condition, false))
            .unwrap_or_default();
        let then_ty =
            self.check_block_against_with_narrowings(&stmt.then_block, expected, &then_narrowings);
        let else_ty = stmt
            .else_branch
            .as_ref()
            .map(|branch| {
                self.check_else_branch_value_with_narrowings(branch, expected, &else_narrowings)
            })
            .unwrap_or_else(Ty::unit);

        let then_exits = self.block_guarantees_control_exit(&stmt.then_block);
        let else_exits = stmt
            .else_branch
            .as_ref()
            .is_some_and(|branch| self.else_branch_guarantees_control_exit(branch));
        if then_exits && !else_exits {
            for narrowing in else_narrowings {
                self.define_local(&narrowing.name, narrowing.ty, false);
            }
        } else if else_exits && !then_exits {
            for narrowing in then_narrowings {
                self.define_local(&narrowing.name, narrowing.ty, false);
            }
        }

        join_types(&then_ty, &else_ty)
    }

    fn check_block_against_with_narrowings(
        &mut self,
        block: &Block,
        expected: &Ty,
        narrowings: &[TypeNarrowing],
    ) -> Ty {
        self.push_scope();
        for narrowing in narrowings {
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

    fn check_else_branch_value_with_narrowings(
        &mut self,
        branch: &ElseBranch,
        expected: &Ty,
        narrowings: &[TypeNarrowing],
    ) -> Ty {
        self.push_scope();
        for narrowing in narrowings {
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
        self.check_match_exhaustiveness(&value_ty, &stmt.cases, stmt.span);
        result
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
                    if binding.mutable {
                        self.track_mutable_binding(binding);
                    }
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
                            for narrowing in self.type_narrowings_for_condition(condition, true) {
                                self.define_local(&narrowing.name, narrowing.ty, false);
                            }
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
                if !self.local_functions_allowed {
                    self.add_error(
                        "invalid_local_function_scope",
                        format!(
                            "local function '{}' must be declared directly in a callable body, not inside a nested block",
                            function.name
                        ),
                        function.span,
                    );
                }
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
        if self.callable_depth == 0 {
            self.add_error(
                "invalid_let_else",
                "let else used outside callable body",
                stmt.span,
            );
            return;
        }
        let mut binding_names = Vec::new();
        let overlapping_alias = if !stmt.clauses.is_empty() {
            stmt.clauses
                .iter()
                .find_map(|clause| overlapping_whole_pattern_alias(&clause.pattern))
        } else {
            overlapping_whole_pattern_alias(&stmt.pattern)
        };
        if !stmt.clauses.is_empty() {
            for clause in &stmt.clauses {
                let value_ty = self.check_expr(&clause.value);
                self.bind_pattern(&clause.pattern, &value_ty);
                for name in crate::ast::pattern_binding_names(&clause.pattern) {
                    if !binding_names.contains(&name) {
                        binding_names.push(name);
                    }
                }
            }
        } else {
            let value_ty = self.check_expr(&stmt.value);
            self.bind_pattern(&stmt.pattern, &value_ty);
            binding_names = crate::ast::pattern_binding_names(&stmt.pattern);
        }

        let mut bindings = Vec::new();
        if let Some(scope) = self.scopes.last_mut() {
            for name in &binding_names {
                if let Some(value) = scope.remove(name) {
                    bindings.push((name.clone(), value));
                }
            }
        }

        self.check_let_else_fallback(&stmt.else_block, &bindings, overlapping_alias);

        if let Some(scope) = self.scopes.last_mut() {
            for (name, value) in bindings {
                scope.insert(name, value);
            }
        }
    }

    fn check_let_else_fallback(
        &mut self,
        block: &Block,
        bindings: &[(String, ValueInfo)],
        overlapping_alias: Option<String>,
    ) {
        if self.block_guarantees_control_exit(block) {
            self.check_block(block);
            return;
        }

        if let Some(alias) = overlapping_alias {
            self.check_block(block);
            self.add_error(
                "invalid_let_else_fallback",
                format!(
                    "pattern alias '{}' binds a whole value and components inside it; its let else fallback must exit control flow instead of supplying independent replacement bindings",
                    alias
                ),
                block.span,
            );
            return;
        }

        if bindings.is_empty() {
            self.check_block(block);
            self.add_error(
                "invalid_let_else_fallback",
                "let else fallback cannot supply a value because the pattern introduces no bindings; exit control flow instead",
                block.span,
            );
            return;
        }

        if bindings.len() == 1 {
            if !matches!(block.statements.last(), Some(Stmt::Expr(_))) {
                self.check_block(block);
                self.add_error(
                    "invalid_let_else_fallback",
                    "let else fallback must end in a value expression or exit control flow",
                    block.span,
                );
                return;
            }
            let expected = &bindings[0].1.ty;
            let actual = self.check_block_against(block, expected);
            self.require_assignable(
                &actual,
                expected,
                block.span,
                "invalid_let_else_fallback",
                format!(
                    "let else fallback for '{}' has type '{}' but expects '{}'",
                    bindings[0].0,
                    actual.describe(),
                    expected.describe()
                ),
            );
            return;
        }

        let record_expected = Ty::Record(
            bindings
                .iter()
                .map(|(name, value)| (name.clone(), value.ty.clone()))
                .collect(),
        );
        let tail = block
            .statements
            .last()
            .and_then(|statement| match statement {
                Stmt::Expr(statement) => Some(&statement.expr),
                _ => None,
            });
        if tail.is_none() {
            self.check_block(block);
            self.add_error(
                "invalid_let_else_fallback",
                "let else fallback must end in a value expression or exit control flow",
                block.span,
            );
            return;
        }
        let expected = match tail {
            Some(Expr::RecordLiteral { .. })
            | Some(Expr::ContextualNew {
                uses_brace_syntax: true,
                ..
            }) => Some(&record_expected),
            _ => None,
        };
        let actual = match expected {
            Some(expected) => self.check_block_against(block, expected),
            None => self.check_block(block),
        };

        if matches!(actual, Ty::Never) {
            return;
        }
        match &actual {
            Ty::Tuple(_) => self.add_error(
                "invalid_let_else_fallback",
                "positional tuple fallbacks are not allowed for multiple let bindings; use a shape whose fields match the binding names",
                block.span,
            ),
            _ if self.shape_target_fields(&actual).is_some() => {
                let fields = self.shape_target_fields(&actual).unwrap_or_default();
                let expected_names = bindings
                    .iter()
                    .map(|(name, _)| name.as_str())
                    .collect::<Vec<_>>();
                let actual_names = fields
                    .iter()
                    .map(|field| field.name.as_str())
                    .collect::<Vec<_>>();
                if expected_names.len() != actual_names.len()
                    || expected_names
                        .iter()
                        .any(|name| !actual_names.contains(name))
                {
                    self.add_error(
                        "invalid_let_else_fallback",
                        format!(
                            "shape fallback fields must exactly match let bindings: expected {{{}}}, got {{{}}}",
                            expected_names.join(", "),
                            actual_names.join(", ")
                        ),
                        block.span,
                    );
                    return;
                }
                for (name, value) in bindings {
                    if let Some(field) = fields.iter().find(|field| field.name == *name) {
                        self.require_assignable(
                            &field.ty,
                            &value.ty,
                            block.span,
                            "invalid_let_else_fallback",
                            format!(
                                "shape fallback field '{}' has type '{}' but binding expects '{}'",
                                name,
                                field.ty.describe(),
                                value.ty.describe()
                            ),
                        );
                    }
                }
            }
            _ => self.add_error(
                "invalid_let_else_fallback",
                format!(
                    "let else fallback for {} bindings must produce a shape with fields {{{}}}, got '{}'",
                    bindings.len(),
                    bindings
                        .iter()
                        .map(|(name, _)| name.as_str())
                        .collect::<Vec<_>>()
                        .join(", "),
                    actual.describe()
                ),
                block.span,
            ),
        }
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
        stmt.cases
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
            Pattern::Extract { .. } | Pattern::Literal { .. } => false,
            Pattern::Alias { inner, .. } => self.for_pattern_is_irrefutable(inner, scrutinee),
            Pattern::Type { target, .. } => {
                !matches!(scrutinee, Ty::Unknown)
                    && self.is_assignable(scrutinee, &self.ty_from_type_ref(target))
            }
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
                let Some((false, destructured_ty, fields)) =
                    self.lookup_record_pattern_target(path, scrutinee)
                else {
                    return false;
                };
                self.is_assignable(scrutinee, &destructured_ty)
                    && fields.len() <= 1
                    && args.len() == fields.len()
                    && args
                        .iter()
                        .zip(fields.iter())
                        .all(|(pattern, (_, ty))| self.for_pattern_is_irrefutable(pattern, ty))
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
            (Pattern::Alias { inner, .. }, source) => {
                self.source_expr_proves_pattern_match(inner, source)
            }
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
                parse_integer_literal(left, false) == parse_integer_literal(right, false)
            }
            (Expr::Float { raw: left, .. }, Expr::Float { raw: right, .. }) => {
                parse_float_literal(left) == parse_float_literal(right)
            }
            (Expr::Decimal { raw: left, .. }, Expr::Decimal { raw: right, .. }) => {
                Decimal::literal(left) == Decimal::literal(right)
            }
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
        if allow_lifted {
            if let Some((family, item_ty)) = self.unwrap_known_lifted_type(source_ty) {
                return (item_ty, Some(family));
            }
        }
        if let Some(item_ty) = self.known_iterable_item_type(source_ty) {
            return (item_ty, Some(ForYieldFamily::Iterable));
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

    fn expected_for_yield_payload(
        &self,
        family: Option<&ForYieldFamily>,
        expected: &Ty,
    ) -> Option<Ty> {
        match (family?, expected) {
            (ForYieldFamily::Iterable, Ty::Named(name, args))
                if (name == "Vector" || name == "Iterable") && args.len() == 1 =>
            {
                Some(args[0].clone())
            }
            (ForYieldFamily::Option, Ty::Named(name, args))
                if name == "Option" && args.len() == 1 =>
            {
                Some(args[0].clone())
            }
            (ForYieldFamily::Result { error }, Ty::Named(name, args))
                if name == "Result" && args.len() == 2 && self.is_assignable(error, &args[1]) =>
            {
                Some(args[0].clone())
            }
            (ForYieldFamily::Either { left }, Ty::Named(name, args))
                if name == "Either" && args.len() == 2 && self.is_assignable(left, &args[0]) =>
            {
                Some(args[1].clone())
            }
            _ => None,
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
            .map(|(name, ty, inaccessible_visibility)| (name, (ty, inaccessible_visibility)))
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
                let Some((field_ty, inaccessible)) = field_map.get(&field_name).cloned() else {
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
                if let Some(visibility) = inaccessible {
                    self.add_error(
                        "invalid_destructure",
                        format!(
                            "cannot destructure {} field '{}' from this scope",
                            visibility_label(visibility),
                            field_name
                        ),
                        binding.span,
                    );
                    return Ty::Unknown;
                }
                field_ty
            })
            .collect()
    }

    fn destructure_record_fields(&self, ty: &Ty) -> Option<Vec<(String, Ty, Option<Visibility>)>> {
        match ty {
            Ty::Record(fields) => Some(
                fields
                    .iter()
                    .map(|(name, ty)| (name.clone(), ty.clone(), None))
                    .collect(),
            ),
            Ty::Named(name, _) => self.lookup_any_type(name).map(|sig| {
                sig.fields
                    .iter()
                    .map(|field| {
                        (
                            field.name.clone(),
                            field.ty.clone(),
                            (!self.can_access_field(&sig, field)).then_some(field.visibility),
                        )
                    })
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
                        .filter(|field| self.can_access_field(&sig, field))
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
        if self.current_getter {
            for target in &assignment.targets {
                if let Some(field_name) = self.getter_mutated_field(target) {
                    self.add_error(
                        "getter_mutation",
                        format!(
                            "getter '{}' cannot mutate field '{}'; getters may only read class or shape state",
                            self.current_method.as_deref().unwrap_or("<getter>"),
                            field_name
                        ),
                        target.span(),
                    );
                }
            }
        }
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

    fn getter_mutated_field(&self, target: &Expr) -> Option<String> {
        match target {
            Expr::Identifier { name, .. }
                if self.lookup_scoped_value(name).is_none()
                    && self.lookup_implicit_field(name).is_some() =>
            {
                Some(name.clone())
            }
            Expr::Member { receiver, name, .. } => {
                let receiver_ty = self.probe_expr_type(receiver);
                self.field_sig_for_member(&receiver_ty, name)
                    .map(|_| name.clone())
            }
            Expr::Index { receiver, .. } => self.getter_mutated_field(receiver),
            _ => None,
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
                    } else {
                        self.mark_local_reassigned(name);
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
                            format!("cannot reassign read-only field '{}'", name),
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
                if name == "referenceId" {
                    let ty = self.check_reference_id_access(&receiver_ty, *span);
                    if !matches!(ty, Ty::Unknown) {
                        self.add_error(
                            "assign_synthetic_member",
                            "'referenceId' is read-only synthetic metadata and cannot be assigned",
                            *span,
                        );
                    }
                    return ty;
                }
                if self.reject_inaccessible_member(&receiver_ty, name, *span) {
                    return Ty::Unknown;
                }
                if let Some(field) = self.field_sig_for_member(&receiver_ty, name) {
                    if self.extension_this_private_field(receiver, &receiver_ty, name) {
                        self.add_extension_private_access_error("field", name, *span);
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
                            format!("cannot reassign read-only field '{}'", name),
                            *span,
                        );
                    }
                    return field.ty.clone();
                }
                if self.member_getter_sig(&receiver_ty, name).is_some() {
                    self.add_error(
                        "assign_immutable",
                        format!("getter '{name}' is read-only and cannot be assigned"),
                        *span,
                    );
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
                ..
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
            Ty::Record(_) => "tuple values cannot construct anonymous shapes; use named fields like '{ field: value }'".to_string(),
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

    fn check_expr_against(&mut self, expr: &Expr, expected: &Ty) -> Ty {
        match expr {
            Expr::Identifier { name, span } => {
                if let Some(value) = self.lookup_scoped_value(name) {
                    return value.ty;
                }
                if let Some(field) = self.lookup_implicit_field(name) {
                    return field.ty;
                }
                if let Some(getter) = self.lookup_implicit_getter(name) {
                    return getter.ret;
                }
                if let Some(value) = self.lookup_global_value(name) {
                    return value.ty;
                }
                if let Some(functions) = self.lookup_functions(name) {
                    return self.select_callable_reference_type(name, &functions, expected, *span);
                }
                if let Some(methods) = self.lookup_implicit_method_functions(name) {
                    return self.select_callable_reference_type(name, &methods, expected, *span);
                }
                if self.report_ambiguous_union_alternative(name, *span) {
                    return Ty::Unknown;
                }
                if let Some(ty) = self.lookup_bare_enum_case_value_type(name, expected) {
                    return ty;
                }
                if let Some(ty) = self.lookup_named_constructor_type(name) {
                    if self
                        .lookup_any_type(name)
                        .is_some_and(|sig| sig.kind == TypeKind::Mixin)
                    {
                        self.add_error(
                            "mixin_not_a_value",
                            "a mixin is reusable behavior, not a value or constructor",
                            *span,
                        );
                        return Ty::Unknown;
                    }
                    return ty;
                }
                self.add_error("undefined_name", self.undefined_value_message(name), *span);
                Ty::Unknown
            }
            Expr::Placeholder { span } => {
                self.add_error(
                    "invalid_placeholder_expr",
                    "'_' is not a value; it only marks an ignored pattern or explicit lambda parameter slot",
                    *span,
                );
                let _ = expected;
                Ty::Unknown
            }
            Expr::Integer { raw, span } => {
                self.check_integer_literal(raw, false, *span);
                Ty::int()
            }
            Expr::Float { .. } => Ty::float(),
            Expr::Decimal { raw, span } => {
                if Decimal::literal(raw).is_none() {
                    self.add_error("invalid_decimal_literal", "decimal literal has invalid digits or a scale outside the signed 32-bit range", *span);
                }
                Ty::named("Decimal")
            }
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
                        Ty::Named(name, args) if name == "Iterable" && args.len() == 1 => {
                            Ty::Named(
                                "Vector".to_string(),
                                args.iter().map(materialize_type).collect(),
                            )
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
                let expected_item_ty = match expected {
                    Ty::Named(name, args)
                        if matches!(name.as_str(), "Vector" | "Iterable") && args.len() == 1 =>
                    {
                        Some(args[0].clone())
                    }
                    _ => None,
                };
                let mut items_fit_expected = true;
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
                        expected_item_ty
                            .as_ref()
                            .map(|expected| self.check_expr_against(item, expected))
                            .unwrap_or_else(|| self.check_expr(item))
                    };
                    if let Some(expected_item) = &expected_item_ty
                        && !self.is_assignable(&current, expected_item)
                    {
                        items_fit_expected = false;
                    }
                    item_ty = join_types(&item_ty, &current);
                }
                if items_fit_expected && let Some(expected_item) = expected_item_ty {
                    return Ty::list(materialize_type(&expected_item));
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
                    let expected_items = match expected {
                        Ty::Tuple(expected_items) if expected_items.len() == items.len() => {
                            Some(expected_items)
                        }
                        _ => None,
                    };
                    Ty::Tuple(
                        items
                            .iter()
                            .enumerate()
                            .map(|(index, item)| {
                                expected_items
                                    .and_then(|expected| expected.get(index))
                                    .map(|expected| self.check_expr_against(item, expected))
                                    .unwrap_or_else(|| self.check_expr(item))
                            })
                            .collect(),
                    )
                }
            }
            Expr::Call {
                callee,
                args,
                uses_brace_syntax,
                span,
            } => self.check_call(callee, args, *uses_brace_syntax, *span, expected),
            Expr::ContextualNew {
                args,
                uses_brace_syntax,
                default_initialize,
                span,
            } => {
                if *default_initialize {
                    self.check_default_initialization_expr(args, *span, expected)
                } else {
                    self.check_contextual_new_expr(args, *uses_brace_syntax, *span, expected)
                }
            }
            Expr::Member {
                receiver,
                name,
                span,
            } => {
                if let Some(functions) = self.module_member_functions(expr) {
                    return self.select_callable_reference_type(name, &functions, expected, *span);
                }
                if let Some(ty) = self.module_member_value_type(expr) {
                    return ty;
                }
                if self.reject_inaccessible_static_member(receiver, name, *span) {
                    return Ty::Unknown;
                }
                if let Some(methods) = self.static_callable_reference_sigs(receiver, name) {
                    return self.select_callable_reference_type(name, &methods, expected, *span);
                }
                if let Some(ty) = self.static_member_value_type(receiver, name, expected) {
                    return ty;
                }
                if let Some(sig) = self
                    .brace_call_type_sig(receiver)
                    .filter(|sig| sig.kind != TypeKind::Object)
                    && self
                        .method_sigs_for_type(&sig, name)
                        .is_some_and(|methods| !methods.is_empty())
                {
                    self.add_error(
                        "unbound_method_reference",
                        format!(
                            "cannot take method reference '{}.{}' from {} '{}' without an instance; use an instance receiver or declare the method on a named object",
                            sig.name,
                            name,
                            type_kind_label(sig.kind),
                            sig.name
                        ),
                        *span,
                    );
                    return Ty::Unknown;
                }
                let receiver_ty = self.check_expr(receiver);
                if name == "runtimeType" {
                    return Ty::value_runtime_type(receiver_ty);
                }
                if name == "referenceId" {
                    return self.check_reference_id_access(&receiver_ty, *span);
                }
                if self.extension_this_private_field(receiver, &receiver_ty, name) {
                    self.add_extension_private_access_error("field", name, *span);
                    return Ty::Unknown;
                }
                if self.extension_this_private_method(receiver, &receiver_ty, name) {
                    self.add_extension_private_access_error("method", name, *span);
                    return Ty::Unknown;
                }
                if self.reject_inaccessible_member(&receiver_ty, name, *span) {
                    return Ty::Unknown;
                }
                let value_member = self.field_sig_for_member(&receiver_ty, name).is_some()
                    || self.member_getter_sig(&receiver_ty, name).is_some();
                if !value_member
                    && let Some(methods) = self.member_method_sigs(&receiver_ty, name)
                    && !methods.is_empty()
                {
                    return self.select_callable_reference_type(name, &methods, expected, *span);
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
                ..
            } => {
                let receiver_ty = self.check_expr(receiver);
                let index_ty = self.check_expr(index);
                if let Ty::Tuple(items) = &receiver_ty {
                    let Some(index_value) = static_integer_literal(index) else {
                        self.add_error(
                            "tuple_index_must_be_literal",
                            "tuple index must be a compile-time integer literal",
                            index.span(),
                        );
                        return Ty::Unknown;
                    };
                    let Ok(index_value) = usize::try_from(index_value) else {
                        self.add_error(
                            "tuple_index_out_of_bounds",
                            format!(
                                "tuple index {} is out of bounds for tuple with {} elements",
                                index_value,
                                items.len()
                            ),
                            index.span(),
                        );
                        return Ty::Unknown;
                    };
                    return items.get(index_value).cloned().unwrap_or_else(|| {
                        self.add_error(
                            "tuple_index_out_of_bounds",
                            format!(
                                "tuple index {} is out of bounds for tuple with {} elements",
                                index_value,
                                items.len()
                            ),
                            index.span(),
                        );
                        Ty::Unknown
                    });
                }
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
            Expr::RecordLiteral { fields, values, .. } => {
                if let Ty::Named(name, _) = expected
                    && let Some(sig) = self.lookup_any_type(name)
                {
                    let span = expr.span();
                    let args = [crate::ast::CallArg {
                        from_shape_spread: false,
                        name: None,
                        ty: None,
                        value: expr.clone(),
                        span,
                    }];
                    if sig.kind == TypeKind::Class {
                        self.add_error(
                            "class_construction_requires_new",
                            format!(
                                "bare field braces cannot construct class '{}'; use 'new {{ ... }}' or '{} {{ ... }}'",
                                sig.name, sig.name
                            ),
                            span,
                        );
                        self.check_contextual_new_expr(&args, true, span, expected);
                        return materialize_type(expected);
                    }
                    if sig.kind == TypeKind::Record {
                        return self.check_contextual_new_expr(&args, true, span, expected);
                    }
                }
                if fields.is_empty() && values.is_empty() {
                    if matches!(expected, Ty::Record(_)) {
                        return Ty::Record(Vec::new());
                    }
                    self.diagnostics.push(
                        Diagnostic::error(
                            "missing_empty_construction_target",
                            "empty construction '{}' requires an expected concrete type",
                            expr.span(),
                        )
                        .with_help("use '()' for Unit or specify the intended construction type"),
                    );
                    return Ty::Unknown;
                }
                if !fields.is_empty() {
                    let expected_fields = match expected {
                        Ty::Record(fields) => Some(fields.as_slice()),
                        _ => None,
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
                            if expected_fields.is_some_and(|fields| {
                                !fields
                                    .iter()
                                    .any(|(expected_name, _)| expected_name == name)
                            }) {
                                self.add_error(
                                    "invalid_contextual_construction",
                                    format!(
                                        "anonymous shape target has no construction field '{}'",
                                        name
                                    ),
                                    field.span,
                                );
                            }
                            let expected_ty = annotated_ty.clone().unwrap_or_else(|| {
                                expected_fields
                                    .and_then(|fields| {
                                        fields
                                            .iter()
                                            .find(|(expected_name, _)| expected_name == name)
                                    })
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
                            let Expr::Spread { value, .. } = &field.value else {
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

                                    if let Some(provider) = providers.get_mut(&name) {
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
                                    "field '{}' is provided by both '{}' and '{}'; select a value explicitly",
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
            Expr::AnonymousObject {
                kind,
                interfaces,
                fields,
                methods,
                span,
            } => self.check_anonymous_object_expr(*kind, interfaces, fields, methods, *span),
            Expr::Try { value, span } => self.check_try_expr(value, *span),
            Expr::ExtractOr {
                value,
                fallback,
                span,
            } => self.check_extract_or_expr(value, fallback, *span),
            Expr::Return { value, span } => self.check_return_control_expr(value.as_deref(), *span),
            Expr::Break { span } => self.check_break_control_expr(*span),
            Expr::Continue { span } => self.check_continue_control_expr(*span),
            Expr::Unary {
                op: crate::ast::UnaryOp::OptionWrap,
                expr,
                ..
            } => self.check_option_wrap_expr(expr, expected),
            Expr::Unary {
                op: crate::ast::UnaryOp::Neg,
                expr,
                span,
            } if matches!(expr.as_ref(), Expr::Integer { .. }) => {
                let Expr::Integer { raw, .. } = expr.as_ref() else {
                    unreachable!();
                };
                self.check_integer_literal(raw, true, *span);
                Ty::int()
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
                    crate::ast::UnaryOp::OptionWrap => unreachable!(),
                    crate::ast::UnaryOp::UnsafeExtract => {
                        let extracted = self.unwrap_inner_type(&inner);
                        if matches!(extracted, Ty::Unknown) && !matches!(inner, Ty::Unknown) {
                            self.add_error(
                                "invalid_unsafe_extract",
                                format!(
                                    "unsafe extraction '!' requires Option[T], Result[T, E], or Either[L, R], got '{}'",
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
                let right_narrowings = match op {
                    BinaryOp::And => self.type_narrowings_for_condition(left, true),
                    BinaryOp::Or => self.type_narrowings_for_condition(left, false),
                    _ => Vec::new(),
                };
                let right_ty = if right_narrowings.is_empty() {
                    self.check_expr(right)
                } else {
                    self.push_scope();
                    for narrowing in right_narrowings {
                        self.define_local(&narrowing.name, narrowing.ty, false);
                    }
                    let ty = self.check_expr(right);
                    self.pop_scope();
                    ty
                };
                self.check_binary_expr(&left_ty, *op, &right_ty, *span)
            }
            Expr::Is { left, target, .. } => {
                self.check_expr(left);
                let target_ty = match target {
                    TypeRef::Named { name, args, span } if args.is_empty() => {
                        if self.report_ambiguous_union_alternative(name, *span) {
                            Ty::Unknown
                        } else {
                            self.world
                                .lookup_enum_case(self.module, name)
                                .map(|case| case.result)
                                .unwrap_or_else(|| self.ty_from_type_ref(target))
                        }
                    }
                    _ => self.ty_from_type_ref(target),
                };
                self.validate_runtime_type_ref(target, &target_ty, "type tests");
                Ty::bool()
            }
            Expr::TypeOf { ty, .. } => {
                let represented = self.ty_from_type_ref(ty);
                self.check_typeof_type_ref(ty, &represented);
                Ty::exact_runtime_type(represented)
            }
            Expr::If {
                condition_clauses,
                then_block,
                else_branch,
                ..
            } => {
                self.push_scope();
                for clause in condition_clauses {
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
                            for narrowing in self.type_narrowings_for_condition(condition, true) {
                                self.define_local(&narrowing.name, narrowing.ty, false);
                            }
                        }
                    }
                }
                let then_ty = self.check_block_against(then_block, expected);
                self.pop_scope();
                let else_narrowings = match condition_clauses.as_slice() {
                    [IfConditionClause::Expr(condition)] => {
                        self.type_narrowings_for_condition(condition, false)
                    }
                    _ => Vec::new(),
                };
                let else_ty = self.check_else_expr_branch_against_with_narrowings(
                    else_branch,
                    expected,
                    &else_narrowings,
                );
                join_types(&then_ty, &else_ty)
            }
            Expr::Block { body, .. } => self.check_block_against(body, expected),
            Expr::Match { value, cases, span } => {
                let value_ty = self.check_expr(value);
                let mut result = Ty::Unknown;
                for case in cases {
                    let current = self.check_match_case_against(case, &value_ty, expected);
                    result = join_types(&result, &current);
                }
                self.check_match_exhaustiveness(&value_ty, cases, *span);
                result
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
                let expected_yield = self.expected_for_yield_payload(family.as_ref(), expected);
                let yield_ty = if let Some(expected_yield) = &expected_yield {
                    self.check_block_against(yield_body, expected_yield)
                } else {
                    self.check_block(yield_body)
                };
                self.loop_depth -= 1;
                self.pop_scope();
                self.wrap_for_yield_type(family.as_ref(), yield_ty)
            }
            Expr::Lambda { params, body, .. } => self.check_lambda_expr(params, body, expected),
            Expr::Group { inner, .. } => self.check_expr_against(inner, expected),
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
        if let Some(ty) = &param.ty {
            self.validate_type_ref_generic_applications(ty);
        }
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
        let member_callee = match callee {
            Expr::Index { receiver, .. } => receiver.as_ref(),
            other => other,
        };
        if let Expr::Member { receiver, .. } = member_callee
            && !matches!(receiver.as_ref(), Expr::Identifier { name, .. } if self.lookup_value(name).is_some())
            && self
                .brace_call_type_sig(receiver)
                .is_some_and(|sig| sig.kind == TypeKind::Mixin)
        {
            self.add_error("mixin_not_a_value", "mixin members are called on a consuming class instance, not on the mixin declaration", span);
            return Ty::Unknown;
        }
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
        // Bracket map literals are represented as Map calls. Their spreads
        // combine map entries rather than expand named callable arguments.
        if matches!(callee, Expr::Identifier { name, .. } if name == "Map")
            && let Some(ty) = self.try_check_constructor_call(
                callee,
                &normalized_args,
                uses_brace_syntax,
                span,
                expected,
            )
        {
            return ty;
        }
        let source_args = normalized_args;
        let has_shape_spread = source_args.iter().any(|arg| {
            matches!(&arg.value, Expr::Spread { value, .. } if self.shape_target_fields(&self.probe_shape_argument_source(value)).is_some())
        });
        let normalized_args = self.expand_shape_call_arguments(&source_args);
        if let Expr::Member { receiver, name, .. } = callee
            && normalized_args.len() == 1
            && name == "equals"
        {
            let left = self.check_expr(receiver);
            let right = self.check_expr(&normalized_args[0].value);
            self.check_equality_operands(&left, &right, span);
            return Ty::bool();
        }
        if self.is_builtin_panic_call(callee) {
            for arg in &normalized_args {
                self.check_expr(&arg.value);
            }
            return Ty::never();
        }
        if self.is_builtin_print_call(callee) {
            if has_shape_spread {
                self.add_error(
                    "invalid_shape_argument_spread_target",
                    "shape argument expansion requires a callable with named parameters",
                    span,
                );
            }
            for arg in &normalized_args {
                self.check_expr(&arg.value);
            }
            return Ty::unit();
        }
        if !has_shape_spread
            && let Some(ty) =
                self.check_builtin_static_method_call(callee, &normalized_args, span, expected)
        {
            return ty;
        }
        if !has_shape_spread
            && let Some(ty) = self.check_builtin_static_factory_call(callee, &normalized_args, span)
        {
            return ty;
        }
        self.check_shape_call_argument_layout(&source_args);
        if self.check_extension_private_method_call(callee, &normalized_args) {
            return Ty::Unknown;
        }
        if let Expr::Member {
            receiver,
            name,
            span: member_span,
        } = callee
        {
            if self.reject_inaccessible_static_member(receiver, name, *member_span) {
                for arg in &normalized_args {
                    self.check_expr(&arg.value);
                }
                return Ty::Unknown;
            }
            let receiver_ty = self.probe_expr_type(receiver);
            if self.reject_inaccessible_member(&receiver_ty, name, *member_span) {
                for arg in &normalized_args {
                    self.check_expr(&arg.value);
                }
                return Ty::Unknown;
            }
            if let Some(getter) = self.member_getter_sig(&receiver_ty, name) {
                if !matches!(&getter.ret, Ty::Function(_, _)) {
                    for arg in &normalized_args {
                        self.check_expr(&arg.value);
                    }
                    self.add_error(
                        "getter_call_syntax",
                        format!(
                            "getter '{}' is called with parentheses; write '{}.{}'",
                            name,
                            self.describe_member_path(receiver)
                                .unwrap_or_else(|| "<value>".to_string()),
                            name
                        ),
                        span,
                    );
                    return getter.ret;
                }
            }
        } else if let Expr::Identifier { name, .. } = callee
            && let Some(getter) = self.lookup_implicit_getter(name)
        {
            if !matches!(&getter.ret, Ty::Function(_, _)) {
                for arg in &normalized_args {
                    self.check_expr(&arg.value);
                }
                self.add_error(
                    "getter_call_syntax",
                    format!(
                        "getter '{}' is called with parentheses; write '{}'",
                        name, name
                    ),
                    span,
                );
                return getter.ret;
            }
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
            return self.check_callable_selection_call(
                &selection,
                callee,
                &normalized_args,
                span,
                expected,
            );
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

    fn expand_shape_call_arguments(
        &self,
        args: &[crate::ast::CallArg],
    ) -> Vec<crate::ast::CallArg> {
        args.iter()
            .flat_map(|arg| {
                let Expr::Spread { value, .. } = &arg.value else {
                    return vec![arg.clone()];
                };
                let Some(fields) =
                    self.shape_target_fields(&self.probe_shape_argument_source(value))
                else {
                    return vec![arg.clone()];
                };
                fields
                    .into_iter()
                    .map(|field| crate::ast::CallArg {
                        from_shape_spread: true,
                        name: Some(field.name.clone()),
                        ty: None,
                        value: Expr::Member {
                            receiver: value.clone(),
                            name: field.name,
                            span: arg.span,
                        },
                        span: arg.span,
                    })
                    .collect()
            })
            .collect()
    }

    fn check_shape_call_argument_layout(&mut self, args: &[crate::ast::CallArg]) {
        let shapes = args
            .iter()
            .map(|arg| match &arg.value {
                Expr::Spread { value, .. } => {
                    self.shape_target_fields(&self.probe_shape_argument_source(value))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        let has_shape = shapes.iter().any(Option::is_some);
        let mut named = false;
        let mut spread = false;
        for (arg, fields) in args.iter().zip(shapes) {
            if let Some(fields) = fields {
                spread = true;
                if fields.is_empty()
                    && let Expr::Spread { value, .. } = &arg.value
                {
                    self.check_expr(value);
                }
            } else {
                if has_shape && (spread || (named && arg.name.is_none())) {
                    self.add_error("shape_spread_argument_order", "shape argument expansion requires positional arguments, then explicit named arguments, then shape spreads", arg.span);
                }
                named |= arg.name.is_some();
                if let Expr::Spread { value, .. } = &arg.value {
                    let ty = self.probe_expr_type(value);
                    if !matches!(&ty, Ty::Named(name, _) if name == "Vector") {
                        self.add_error("invalid_shape_argument_spread", format!("call argument spread requires a statically known shape or a vector vararg value, got '{}'", ty.describe()), arg.span);
                    }
                }
            }
        }
    }

    fn probe_shape_argument_source(&self, source: &Expr) -> Ty {
        match source {
            Expr::Group { inner, .. } => self.probe_shape_argument_source(inner),
            Expr::ContextualNew {
                args,
                uses_brace_syntax: true,
                default_initialize: false,
                ..
            } => args
                .first()
                .map(|arg| self.probe_expr_type(&arg.value))
                .unwrap_or(Ty::Unknown),
            _ => self.probe_expr_type(source),
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

    fn validate_anonymous_interfaces(
        &mut self,
        interfaces: &[TypeRef],
        implementation: &TypeSig,
        span: crate::source::Span,
    ) -> Vec<Ty> {
        let mut result_tys = Vec::new();

        for interface in interfaces {
            let interface_ty = self.ty_from_type_ref(interface);
            let Some(interface_sig) = self.interface_sig_from_type_ref(interface) else {
                continue;
            };
            self.check_interface_bound_implementation(
                implementation,
                &interface_ty,
                &interface_sig.name,
                "anonymous implementation",
                span,
            );
            result_tys.push(interface_ty);
        }
        result_tys
    }

    fn check_anonymous_object_expr(
        &mut self,
        kind: TypeKind,
        interfaces: &[TypeRef],
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
                visibility: field.visibility,
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
        let mut sig = TypeSig {
            kind,
            name: name.clone(),
            module_id: self.module.module_id.clone(),
            type_params: Vec::new(),
            generic_conditions: Vec::new(),
            with_bounds: Vec::new(),
            fields: field_sigs,
            methods: method_sigs,
            enum_cases: HashMap::new(),
        };
        sig.with_bounds = self.validate_anonymous_interfaces(interfaces, &sig, span);
        self.anonymous_types.insert(name.clone(), sig.clone());
        for method in methods {
            self.check_method(method, &sig);
        }

        Ty::Named(name, Vec::new())
    }

    fn check_contextual_new_expr(
        &mut self,
        args: &[crate::ast::CallArg],
        uses_brace_syntax: bool,
        span: crate::source::Span,
        expected: &Ty,
    ) -> Ty {
        let reject = |checker: &mut Self, message: String| {
            for arg in args {
                checker.check_expr(call_arg_value_expr(arg));
            }
            checker.add_error("invalid_contextual_construction", message, span);
            Ty::Unknown
        };

        if !uses_brace_syntax
            && (matches!(expected, Ty::Unknown) || expected == &Ty::unit())
            && self.current_method.as_deref() == Some("new")
            && self.current_owner.is_some()
        {
            let replacement = if uses_brace_syntax {
                "`this { ... }`"
            } else {
                "`this(...)`"
            };
            return reject(
                self,
                format!(
                    "constructor delegation uses {replacement}; `new` only declares constructors"
                ),
            );
        }

        if uses_brace_syntax {
            let concrete_target = match expected {
                Ty::Named(name, _) => self
                    .lookup_any_type(name)
                    .is_some_and(|sig| matches!(sig.kind, TypeKind::Class | TypeKind::Record)),
                _ => false,
            };
            if !concrete_target {
                let [
                    crate::ast::CallArg {
                        from_shape_spread: false,
                        value: record @ Expr::RecordLiteral { .. },
                        ..
                    },
                ] = args
                else {
                    return reject(
                        self,
                        "'new { ... }' requires construction fields".to_string(),
                    );
                };
                if matches!(
                    record,
                    Expr::RecordLiteral { fields, values, .. }
                        if fields.is_empty() && values.is_empty()
                ) {
                    return match expected {
                        Ty::Record(fields) if fields.is_empty() => Ty::Record(Vec::new()),
                        Ty::Record(fields) => {
                            for (name, _) in fields {
                                self.add_error(
                                    "invalid_contextual_construction",
                                    format!("missing required field '{name}'"),
                                    span,
                                );
                            }
                            materialize_type(expected)
                        }
                        _ => {
                            self.diagnostics.push(
                                Diagnostic::error(
                                    "missing_empty_construction_target",
                                    "empty construction 'new {}' requires an expected concrete type",
                                    span,
                                )
                                .with_help(
                                    "use '()' for Unit or specify the intended construction type, such as 'value {} = new {}'",
                                ),
                            );
                            Ty::Unknown
                        }
                    };
                }
                let record_expected = if matches!(expected, Ty::Record(_)) {
                    expected
                } else {
                    &Ty::Unknown
                };
                return self.check_expr_against(record, record_expected);
            }
        } else if expected.is_any() {
            return reject(
                self,
                "contextual 'new(...)' cannot target Any; use an explicit concrete constructor"
                    .to_string(),
            );
        }
        if !uses_brace_syntax && let Ty::Record(fields) = expected {
            if args.iter().any(|arg| arg.name.is_some()) {
                return reject(
                    self,
                    "contextual anonymous-shape construction uses positional arguments; use 'new(value, ...)' or field braces"
                        .to_string(),
                );
            }
            if args.len() != fields.len() {
                for arg in args {
                    self.check_expr(call_arg_value_expr(arg));
                }
                self.add_error(
                    "invalid_argument_count",
                    format!(
                        "anonymous shape construction expects {} argument{}, got {}",
                        fields.len(),
                        if fields.len() == 1 { "" } else { "s" },
                        args.len()
                    ),
                    span,
                );
                return materialize_type(expected);
            }
            for (arg, (name, field_ty)) in args.iter().zip(fields) {
                let actual = self.check_expr_against(call_arg_value_expr(arg), field_ty);
                self.require_assignable(
                    &actual,
                    field_ty,
                    arg.span,
                    "invalid_argument_type",
                    format!(
                        "argument for field '{}' has type '{}' but expects '{}'",
                        name,
                        actual.describe(),
                        field_ty.describe()
                    ),
                );
            }
            return materialize_type(expected);
        }
        let Ty::Named(name, type_args) = expected else {
            let message = match expected {
                Ty::Unknown => "contextual 'new(...)' requires an expected class or shape type"
                    .to_string(),
                Ty::Union(_) => "contextual 'new(...)' cannot choose an alternative from a union; use an explicit constructor"
                    .to_string(),
                Ty::TypeParam(_) => "contextual 'new(...)' requires an exact declared class or shape, not an unconstrained type parameter"
                    .to_string(),
                other => format!(
                    "contextual 'new(...)' requires an expected class or shape type, got '{}'",
                    other.describe()
                ),
            };
            return reject(self, message);
        };
        let Some(mut sig) = self.lookup_any_type(name) else {
            return reject(
                self,
                format!("contextual 'new' target '{}' is not a declared type", name),
            );
        };
        if is_builtin_primitive_type_name(&sig.name) {
            let guidance = match sig.name.as_str() {
                "Int" => {
                    "use value.toInt() for Float input or Int.parse(text) for string input; contextual construction does not perform primitive conversions"
                }
                "Float" => {
                    "use value.toFloat() for Int input or Float.parse(text) for string input; contextual construction does not perform primitive conversions"
                }
                "Str" => "use value.toStr() to convert a value to Str",
                "Decimal" => {
                    "use a decimal literal such as 0.1d, Decimal.parse(text), or an explicit toDecimal() conversion"
                }
                _ => {
                    "use an explicit operation that produces the primitive value; primitive types are not construction targets"
                }
            };
            return reject(
                self,
                format!(
                    "primitive type '{}' is not constructible; {guidance}",
                    sig.name
                ),
            );
        }
        if !matches!(sig.kind, TypeKind::Class | TypeKind::Record) {
            let guidance = if sig.kind == TypeKind::Interface {
                format!(
                    "interface '{}' is not constructible; use 'object with {} {{ ... }}'",
                    sig.name, sig.name
                )
            } else {
                format!(
                    "contextual 'new' target '{}' must be a class or named shape",
                    sig.name
                )
            };
            return reject(self, guidance);
        }

        let subst = sig
            .type_params
            .iter()
            .cloned()
            .zip(type_args.iter().cloned())
            .collect::<HashMap<_, _>>();
        for field in &mut sig.fields {
            field.ty = substitute_type(&field.ty, &subst);
        }
        for methods in sig.methods.values_mut() {
            for method in methods {
                *method = instantiate_function_sig(method.clone(), &subst);
            }
        }
        sig.with_bounds = sig
            .with_bounds
            .iter()
            .map(|bound| substitute_type(bound, &subst))
            .collect();
        sig.generic_conditions = sig
            .generic_conditions
            .iter()
            .map(|condition| substitute_generic_condition(condition, &subst))
            .collect();
        sig.type_params.clear();

        let structural_record_arg = call_uses_structural_record_arg(args, uses_brace_syntax);
        self.check_named_type_constructor(
            &sig,
            args,
            span,
            uses_brace_syntax,
            structural_record_arg,
            expected,
            &[],
        );
        materialize_type(expected)
    }

    fn check_default_initialization_expr(
        &mut self,
        args: &[crate::ast::CallArg],
        span: crate::source::Span,
        expected: &Ty,
    ) -> Ty {
        let [
            crate::ast::CallArg {
                from_shape_spread: false,
                value: record @ Expr::RecordLiteral { fields, values, .. },
                ..
            },
        ] = args
        else {
            for arg in args {
                self.check_expr(call_arg_value_expr(arg));
            }
            self.add_error(
                "invalid_default_initialization",
                "default initialization requires construction fields",
                span,
            );
            return Ty::Unknown;
        };

        if !values.is_empty() {
            for value in values {
                self.check_expr(value);
            }
            self.add_error(
                "invalid_default_initialization",
                "default initialization uses named fields, not positional values",
                span,
            );
            return materialize_type(expected);
        }

        let (target_name, target_kind, target_fields, explicit_constructors) = match expected {
            Ty::Record(fields) => (
                None,
                TypeKind::Record,
                fields
                    .iter()
                    .map(|(name, ty)| FieldSig {
                        name: name.clone(),
                        ty: ty.clone(),
                        mutable: false,
                        visibility: Visibility::Default,
                        has_initializer: false,
                        variadic: false,
                    })
                    .collect::<Vec<_>>(),
                Vec::new(),
            ),
            Ty::Named(name, type_args) => {
                let Some(mut sig) = self.lookup_any_type(name) else {
                    self.check_expr(record);
                    self.add_error(
                        "invalid_default_initialization",
                        format!(
                            "default initialization target '{}' is not a declared type",
                            name
                        ),
                        span,
                    );
                    return Ty::Unknown;
                };
                if !matches!(sig.kind, TypeKind::Class | TypeKind::Record) {
                    self.check_expr(record);
                    self.add_error(
                        "invalid_default_initialization",
                        format!(
                            "default initialization requires a concrete class or shape target, got '{}'",
                            expected.describe()
                        ),
                        span,
                    );
                    return Ty::Unknown;
                }

                let subst = sig
                    .type_params
                    .iter()
                    .cloned()
                    .zip(type_args.iter().cloned())
                    .collect::<HashMap<_, _>>();
                for field in &mut sig.fields {
                    field.ty = substitute_type(&field.ty, &subst);
                }
                for methods in sig.methods.values_mut() {
                    for method in methods {
                        *method = instantiate_function_sig(method.clone(), &subst);
                    }
                }

                if sig.kind == TypeKind::Class
                    && sig.fields.iter().any(|field| {
                        field.visibility != Visibility::Default && !field.has_initializer
                    })
                {
                    self.check_expr(record);
                    self.add_error(
                        "invalid_default_initialization",
                        format!(
                            "class '{}' cannot be default-initialized because non-public fields without initializers are not construction inputs",
                            sig.name
                        ),
                        span,
                    );
                    return materialize_type(expected);
                }

                let explicit_constructors = sig.methods.get("new").cloned().unwrap_or_default();
                let target_fields = sig
                    .fields
                    .iter()
                    .filter(|field| {
                        if sig.kind == TypeKind::Class {
                            field.visibility == Visibility::Default
                        } else {
                            self.can_access_field(&sig, field)
                        }
                    })
                    .cloned()
                    .collect::<Vec<_>>();
                (
                    Some(sig.name),
                    sig.kind,
                    target_fields,
                    explicit_constructors,
                )
            }
            _ => {
                self.check_expr(record);
                self.add_error(
                    "invalid_default_initialization",
                    format!(
                        "default initialization requires an expected class or shape type, got '{}'",
                        expected.describe()
                    ),
                    span,
                );
                return Ty::Unknown;
            }
        };

        if !explicit_constructors.is_empty() {
            let accessible_zero_arg = explicit_constructors.iter().any(|constructor| {
                self.can_access_constructor_sig(target_name.as_deref(), constructor)
                    && arrange_param_args(&constructor.params, &[]).missing_required == 0
            });
            if !accessible_zero_arg {
                self.check_expr(record);
                self.add_error(
                    "invalid_default_initialization",
                    format!(
                        "'{}' cannot be constructed with zero arguments",
                        target_name.as_deref().unwrap_or("target")
                    ),
                    span,
                );
                return materialize_type(expected);
            }
            if !fields.is_empty() {
                self.check_expr(record);
                self.add_error(
                    "invalid_default_initialization",
                    "default initialization cannot override fields of a class with explicit constructors",
                    span,
                );
                return materialize_type(expected);
            }
            return materialize_type(expected);
        }

        for field in fields.iter().filter(|field| field.name.is_some()) {
            let name = field.name.as_deref().expect("filtered named field");
            if !target_fields.iter().any(|target| target.name == name) {
                self.add_error(
                    "invalid_default_initialization",
                    format!(
                        "{} '{}' has no construction field '{}'",
                        type_kind_label(target_kind),
                        target_name.as_deref().unwrap_or("anonymous shape"),
                        name
                    ),
                    field.span,
                );
            }
        }

        let expected_record = Ty::Record(
            target_fields
                .iter()
                .map(|field| (field.name.clone(), field.ty.clone()))
                .collect(),
        );
        let actual = self.check_expr_against(record, &expected_record);
        let actual_fields = self.structural_fields_for_type(&actual).unwrap_or_default();

        for field in &target_fields {
            if let Some((_, actual_ty)) = actual_fields.iter().find(|(name, _)| name == &field.name)
            {
                self.require_assignable(
                    actual_ty,
                    &field.ty,
                    span,
                    "invalid_argument_type",
                    format!(
                        "field '{}' expects '{}' but got '{}'",
                        field.name,
                        field.ty.describe(),
                        actual_ty.describe()
                    ),
                );
                continue;
            }
            if field.has_initializer || self.type_supports_default_initialization(&field.ty) {
                continue;
            }
            self.add_error(
                "invalid_default_initialization",
                format!(
                    "field '{}' requires an explicit value: '{}' cannot be constructed with zero arguments",
                    field.name,
                    field.ty.describe()
                ),
                span,
            );
        }

        materialize_type(expected)
    }

    fn can_access_constructor_sig(
        &self,
        owner_name: Option<&str>,
        constructor: &FunctionSig,
    ) -> bool {
        let Some(owner_name) = owner_name else {
            return false;
        };
        self.lookup_any_type(owner_name)
            .is_some_and(|owner| self.can_access_constructor(&owner, constructor.visibility))
    }

    fn type_supports_default_initialization(&self, ty: &Ty) -> bool {
        match ty {
            Ty::Named(name, _)
                if matches!(
                    name.as_str(),
                    "Bool" | "Int" | "Float" | "Decimal" | "Str" | "Rune" | "Unit"
                ) =>
            {
                true
            }
            Ty::Named(name, _) if name == "Option" => true,
            Ty::Record(fields) => fields.is_empty(),
            Ty::Named(name, type_args) => {
                let Some(mut sig) = self.lookup_any_type(name) else {
                    return false;
                };
                if !matches!(sig.kind, TypeKind::Class | TypeKind::Record) {
                    return false;
                }
                let subst = sig
                    .type_params
                    .iter()
                    .cloned()
                    .zip(type_args.iter().cloned())
                    .collect::<HashMap<_, _>>();
                for methods in sig.methods.values_mut() {
                    for method in methods {
                        *method = instantiate_function_sig(method.clone(), &subst);
                    }
                }
                if let Some(constructors) = sig.methods.get("new")
                    && !constructors.is_empty()
                {
                    return constructors.iter().any(|constructor| {
                        self.can_access_constructor(&sig, constructor.visibility)
                            && arrange_param_args(&constructor.params, &[]).missing_required == 0
                    });
                }
                if sig.kind == TypeKind::Class
                    && sig.fields.iter().any(|field| {
                        field.visibility != Visibility::Default && !field.has_initializer
                    })
                {
                    return false;
                }
                sig.fields
                    .iter()
                    .filter(|field| {
                        sig.kind != TypeKind::Class || field.visibility == Visibility::Default
                    })
                    .all(|field| field.has_initializer)
            }
            _ => false,
        }
    }

    fn interface_sig_from_type_ref(&mut self, interface: &TypeRef) -> Option<TypeSig> {
        if let Ty::Named(name, args) = self.ty_from_type_ref(interface)
            && args.is_empty()
            && is_builtin_primitive_type_name(&name)
        {
            self.diagnostics
                .push(primitive_implementation_diagnostic(&name, interface.span()));
            return None;
        }
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

    fn effective_interface_methods(
        &self,
        interface_ty: &Ty,
        seen: &mut HashSet<String>,
    ) -> HashMap<String, Vec<FunctionSig>> {
        let Ty::Named(name, args) = interface_ty else {
            return HashMap::new();
        };
        let key = interface_ty.describe();
        if !seen.insert(key.clone()) {
            return HashMap::new();
        }
        let Some(sig) = self.lookup_any_type(name) else {
            seen.remove(&key);
            return HashMap::new();
        };
        if sig.kind != TypeKind::Interface {
            seen.remove(&key);
            return HashMap::new();
        }

        let subst = sig
            .type_params
            .iter()
            .cloned()
            .zip(args.iter().cloned())
            .collect::<HashMap<_, _>>();
        let mut methods = HashMap::<String, Vec<FunctionSig>>::new();
        for bound in &sig.with_bounds {
            let bound = substitute_type(bound, &subst);
            for (name, inherited) in self.effective_interface_methods(&bound, seen) {
                methods.entry(name).or_default().extend(inherited);
            }
        }
        for (name, overloads) in &sig.methods {
            methods.insert(
                name.clone(),
                overloads
                    .iter()
                    .cloned()
                    .map(|method| instantiate_function_sig(method, &subst))
                    .collect(),
            );
        }
        seen.remove(&key);
        methods
    }

    fn check_interface_bound_implementation(
        &mut self,
        implementation: &TypeSig,
        interface_ty: &Ty,
        interface_name: &str,
        implementation_label: &str,
        span: crate::source::Span,
    ) {
        let methods = self.effective_interface_methods(interface_ty, &mut HashSet::new());
        let mut names = methods.keys().cloned().collect::<Vec<_>>();
        names.sort();

        for name in names {
            let requirements = &methods[&name];
            let field = implementation
                .fields
                .iter()
                .find(|field| field.name == name && field.visibility == Visibility::Default);

            if requirements.iter().all(|method| method.getter)
                && let Some(field) = field
            {
                let incompatible = requirements
                    .iter()
                    .find(|getter| !self.is_assignable(&field.ty, &getter.ret));
                if let Some(getter) = incompatible {
                    self.add_error(
                        "invalid_interface_getter_field_type",
                        format!(
                            "public field '{}' on {} has type '{}' but interface '{}' getter '{}' returns '{}'",
                            name,
                            implementation_label,
                            field.ty.describe(),
                            interface_name,
                            name,
                            getter.ret.describe()
                        ),
                        span,
                    );
                }
                continue;
            }

            if requirements.is_empty() {
                continue;
            }
            let implemented = if requirements.iter().all(|method| method.getter) {
                implementation.methods.get(&name).is_some_and(|methods| {
                    methods.iter().any(|method| {
                        method.has_body
                            && method.visibility == Visibility::Default
                            && method.getter
                            && requirements.iter().all(|requirement| {
                                self.is_assignable(&method.ret, &requirement.ret)
                            })
                    })
                })
            } else {
                self.type_declares_method_body(implementation, &name)
            };
            if implemented {
                continue;
            }

            let member_kind = if requirements.iter().all(|method| method.getter) {
                "getter"
            } else {
                "method"
            };
            let message = if implementation_label == "anonymous implementation" {
                format!(
                    "anonymous implementation of interface '{}' is missing {} '{}'",
                    interface_name, member_kind, name
                )
            } else {
                format!(
                    "{} declares interface '{}' but does not implement required {} '{}'",
                    implementation_label, interface_name, member_kind, name
                )
            };
            self.add_error("missing_interface_member", message, span);
        }
    }

    fn type_declares_method_body(&self, sig: &TypeSig, name: &str) -> bool {
        sig.methods.get(name).is_some_and(|methods| {
            methods.iter().any(|method| {
                method.has_body && !method.getter && method.visibility == Visibility::Default
            })
        }) || self
            .module
            .extensions
            .get(&sig.name)
            .and_then(|methods| methods.get(name))
            .is_some_and(|methods| {
                methods.iter().any(|method| {
                    method.has_body && !method.getter && method.visibility == Visibility::Default
                })
            })
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
        expected: &Ty,
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
            (collection @ ("Array" | "Vector"), "fill") => {
                if args.len() != 2 {
                    self.add_error(
                        "invalid_argument_count",
                        format!("{collection}.fill expects 2 arguments, got {}", args.len()),
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
                            "{collection}.fill expects Int length, got '{}'",
                            ty.describe()
                        ),
                    );
                }
                let expected_item = match expected {
                    Ty::Named(name, type_args) if name == collection && type_args.len() == 1 => {
                        type_args.first()
                    }
                    _ => None,
                };
                let item_ty = args
                    .get(1)
                    .map(|arg| {
                        expected_item
                            .map(|expected| self.check_expr_against(&arg.value, expected))
                            .unwrap_or_else(|| self.check_expr(&arg.value))
                    })
                    .unwrap_or(Ty::Unknown);
                Some(Ty::Named(collection.to_string(), vec![item_ty]))
            }
            (collection @ ("Array" | "Vector"), "generate") => {
                if args.len() != 2 {
                    self.add_error(
                        "invalid_argument_count",
                        format!(
                            "{collection}.generate expects 2 arguments, got {}",
                            args.len()
                        ),
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
                            "{collection}.generate expects Int length, got '{}'",
                            ty.describe()
                        ),
                    );
                }
                let expected_item = match expected {
                    Ty::Named(name, type_args) if name == collection && type_args.len() == 1 => {
                        type_args[0].clone()
                    }
                    _ => Ty::Unknown,
                };
                let item_ty = args
                    .get(1)
                    .map(|arg| {
                        let generator =
                            Ty::Function(vec![Ty::int()], Box::new(expected_item.clone()));
                        let generator_ty = self.check_expr_against(&arg.value, &generator);
                        match generator_ty {
                            Ty::Function(params, ret) if params.len() == 1 => {
                                if !matches!(params[0], Ty::Unknown)
                                    && !self.is_assignable(&Ty::int(), &params[0])
                                {
                                    self.add_error(
                                        "invalid_argument_type",
                                        format!(
                                            "{collection}.generate expects fn(Int) T generator, got '{}'",
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
                                        "{collection}.generate expects fn(Int) T generator, got '{}'",
                                        other.describe()
                                    ),
                                    arg.span,
                                );
                                Ty::Unknown
                            }
                        }
                    })
                    .unwrap_or(Ty::Unknown);
                Some(Ty::Named(collection.to_string(), vec![item_ty]))
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
                let expected_value = match expected {
                    Ty::Named(name, type_args) if name == "Option" && type_args.len() == 1 => {
                        type_args.first()
                    }
                    _ => None,
                };
                let value_ty = args
                    .get(1)
                    .map(|arg| {
                        expected_value
                            .map(|expected| self.check_expr_against(&arg.value, expected))
                            .unwrap_or_else(|| self.check_expr(&arg.value))
                    })
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
            Expr::RecordLiteral { fields, values, .. }
                if fields.is_empty() && values.is_empty() => {}
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
        self.check_signature_call_with_subst(params, ret, args, span, HashMap::new(), None)
            .0
    }

    fn check_callable_selection_call(
        &mut self,
        selection: &CallableSelection,
        callee: &Expr,
        args: &[crate::ast::CallArg],
        span: crate::source::Span,
        expected: &Ty,
    ) -> Ty {
        let mut subst = self.explicit_type_arg_subst(
            &selection.sig.type_params,
            &selection.explicit_type_args,
            span,
        );
        self.infer_type_subst_from_generic_conditions(
            &selection.sig.generic_conditions,
            &mut subst,
        );
        if !matches!(expected, Ty::Unknown) {
            infer_type_subst(&selection.sig.ret, expected, &mut subst);
        }
        let callee_name = callable_name_for_diagnostic(callee);
        let (ret, subst, argument_shape_valid) = self.check_signature_call_with_subst(
            &selection.sig.params,
            &selection.sig.ret,
            args,
            span,
            subst,
            Some(&callee_name),
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

    fn infer_type_subst_from_generic_conditions(
        &self,
        conditions: &[GenericConditionSig],
        subst: &mut HashMap<String, Ty>,
    ) {
        for condition in conditions {
            let GenericConditionSig::Bound { subject, bound } = condition else {
                continue;
            };
            let subject = substitute_type(subject, subst);
            let bound = substitute_type(bound, subst);
            let Ty::Named(bound_name, bound_args) = bound else {
                continue;
            };
            if bound_name != "Iterable" || bound_args.len() != 1 {
                continue;
            }
            let Some(item) = self.known_iterable_item_type(&subject) else {
                continue;
            };
            infer_type_subst(&bound_args[0], &item, subst);
        }
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
        callee_name: Option<&str>,
    ) -> (Ty, HashMap<String, Ty>, bool) {
        let arrangement = arrange_param_args(params, args);
        let unknown_named_args = args
            .iter()
            .filter(|arg| {
                arg.name
                    .as_ref()
                    .is_some_and(|name| params.iter().all(|param| param.name != *name))
            })
            .collect::<Vec<_>>();
        for arg in &unknown_named_args {
            let name = arg.name.as_deref().unwrap_or("<unknown>");
            self.add_error(
                "unknown_argument_name",
                format!("callable has no parameter named '{name}'"),
                arg.span,
            );
            self.check_expr(call_arg_value_expr(arg));
        }
        let min_required = params
            .iter()
            .filter(|param| !param.variadic && !param.has_initializer)
            .count();
        let max_allowed = if params.last().is_some_and(|param| param.variadic) {
            usize::MAX
        } else {
            params.len()
        };
        for (name, spread, span) in &arrangement.conflicts {
            self.add_error(if *spread { "conflicting_shape_arguments" } else { "duplicate_argument" },
                if *spread { format!("shape argument spreads both supply '{name}'; provide an explicit argument before the spreads to choose its value") }
                else { format!("parameter '{name}' is supplied more than once explicitly") }, *span);
        }
        let argument_shape_valid = arrangement.overflow == 0 && arrangement.missing_required == 0;
        if !argument_shape_valid && unknown_named_args.is_empty() {
            let prefix_message = args
                .iter()
                .all(|arg| arg.name.is_none())
                .then(|| {
                    positional_callable_prefix_message(
                        callee_name.unwrap_or("callable"),
                        params,
                        args.len(),
                    )
                })
                .flatten();
            self.add_error(
                "invalid_argument_count",
                prefix_message.unwrap_or_else(|| {
                    format!(
                        "call expects {}..{} arguments, got {}",
                        min_required,
                        if max_allowed == usize::MAX {
                            "many".to_string()
                        } else {
                            max_allowed.to_string()
                        },
                        args.len()
                    )
                }),
                span,
            );
        }
        // Independently known values seed inference before contextual spread fields.
        if args.iter().any(|arg| arg.from_shape_spread) {
            for (index, param) in params.iter().enumerate() {
                for arg in &arrangement.slots[index] {
                    let expected = call_arg_expected_ty_for_arg(param.variadic, &param.ty, arg);
                    infer_type_subst(
                        &expected,
                        &self.probe_expr_type(call_arg_value_expr(arg)),
                        &mut subst,
                    );
                }
            }
        }
        let mut spread_types = HashMap::new();
        for arg in args.iter().filter(|arg| arg.from_shape_spread) {
            let Expr::Member { receiver, .. } = &arg.value else {
                continue;
            };
            if spread_types.contains_key(&(receiver.span().start, receiver.span().end)) {
                continue;
            }
            let source_ty = self.probe_shape_argument_source(receiver);
            let expected = if matches!(source_ty, Ty::Record(_)) {
                Ty::Record(
                    self.shape_target_fields(&source_ty)
                        .unwrap_or_default()
                        .into_iter()
                        .map(|field| {
                            let active = params.iter().enumerate().find(|(index, param)| {
                                param.name == field.name
                                    && arrangement.slots[*index]
                                        .iter()
                                        .any(|slot| slot.from_shape_spread && slot.span == arg.span)
                            });
                            (
                                field.name,
                                active
                                    .map(|(_, param)| {
                                        materialize_type(&substitute_type(&param.ty, &subst))
                                    })
                                    .unwrap_or(field.ty),
                            )
                        })
                        .collect(),
                )
            } else {
                source_ty
            };
            let actual = self.check_expr_against(receiver, &expected);
            spread_types.insert((receiver.span().start, receiver.span().end), actual);
        }
        let mut checked_args = Vec::new();
        for (index, param) in params.iter().enumerate() {
            let slot = arrangement
                .slots
                .get(index)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            for arg in slot {
                if param.lazy && !arg.from_shape_spread {
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
                let actual = if arg.from_shape_spread {
                    match &arg.value {
                        Expr::Member { receiver, name, .. } => spread_types
                            .get(&(receiver.span().start, receiver.span().end))
                            .and_then(|ty| self.member_type(ty, name))
                            .unwrap_or(Ty::Unknown),
                        _ => Ty::Unknown,
                    }
                } else {
                    self.check_call_arg_expr_against(call_arg_value_expr(arg), &check_expected)
                };
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
        let original_callee = callee;
        let (callee, explicit_type_args) = self.split_generic_call_callee(callee);
        if matches!(original_callee, Expr::Index { .. })
            && explicit_type_args.is_empty()
            && matches!(callee, Expr::Index { receiver, .. }
                if matches!(receiver.as_ref(), Expr::Identifier { name, .. }
                    if self.lookup_any_type(name).is_some()))
        {
            for arg in args {
                self.check_expr(call_arg_value_expr(arg));
            }
            self.add_error(
                "invalid_constructor_type_arguments",
                "generic construction requires all concrete type arguments or none; omit '[...]' to infer them",
                original_callee.span(),
            );
            return Some(Ty::Unknown);
        }
        let structural_record_arg = call_uses_structural_record_arg(args, uses_brace_syntax);
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
                                expected,
                                &explicit_type_args,
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
                        expected,
                        &explicit_type_args,
                    ) {
                        return Some(ty);
                    }
                }
                if self.report_ambiguous_union_alternative(name, callee.span()) {
                    for arg in args {
                        self.check_expr(call_arg_value_expr(arg));
                    }
                    return Some(Ty::Unknown);
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
                        expected,
                        &explicit_type_args,
                    ));
                }
                if let Some(sig) = self.world.lookup_imported_type(self.module, name) {
                    return Some(self.check_named_type_constructor(
                        &sig,
                        args,
                        span,
                        uses_brace_syntax,
                        structural_record_arg,
                        expected,
                        &explicit_type_args,
                    ));
                }
                if let Some(sig) = self.world.ambient.types.get(name).cloned() {
                    return Some(self.check_named_type_constructor(
                        &sig,
                        args,
                        span,
                        uses_brace_syntax,
                        structural_record_arg,
                        expected,
                        &explicit_type_args,
                    ));
                }
                if let Some(sig) = self.lookup_any_object(name) {
                    return Some(self.check_named_type_constructor(
                        &sig,
                        args,
                        span,
                        uses_brace_syntax,
                        structural_record_arg,
                        expected,
                        &explicit_type_args,
                    ));
                }
                None
            }
            Expr::Member { receiver, name, .. } => {
                if let Some(path) = expr_path_for_known_value(callee) {
                    let qualified = path.join(".");
                    if let Some(sig) = self
                        .lookup_type_local(&qualified)
                        .or_else(|| self.world.lookup_imported_type(self.module, &qualified))
                    {
                        return Some(self.check_named_type_constructor(
                            &sig,
                            args,
                            span,
                            uses_brace_syntax,
                            structural_record_arg,
                            expected,
                            &explicit_type_args,
                        ));
                    }
                }
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
                            expected,
                            &explicit_type_args,
                        ));
                    }
                    if let Some(sig) = module_info.objects.get(&member).cloned() {
                        return Some(self.check_named_type_constructor(
                            &sig,
                            args,
                            span,
                            uses_brace_syntax,
                            structural_record_arg,
                            expected,
                            &explicit_type_args,
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
        expected: &Ty,
        explicit_type_args: &[Ty],
    ) -> Option<Ty> {
        match name {
            "Vector" | "Set" => {
                self.reject_parenthesized_constructor_fields(args, uses_brace_syntax, span);
                let mut type_args = self.constructor_type_argument_seed(
                    name,
                    1,
                    explicit_type_args,
                    expected,
                    span,
                );
                let mut item = type_args.pop().unwrap_or(Ty::Unknown);
                for arg in args {
                    let actual = if matches!(item, Ty::Unknown) {
                        self.check_expr(&arg.value)
                    } else {
                        self.check_expr_against(&arg.value, &item)
                    };
                    if matches!(item, Ty::Unknown) {
                        item = join_types(&item, &actual);
                    }
                }
                if explicit_type_args.is_empty() {
                    self.require_inferred_constructor_type_args(name, &[item.clone()], span);
                }
                Some(Ty::Named(name.to_string(), vec![item]))
            }
            "Map" => {
                self.reject_parenthesized_constructor_fields(args, uses_brace_syntax, span);
                let mut type_args = self.constructor_type_argument_seed(
                    name,
                    2,
                    explicit_type_args,
                    expected,
                    span,
                );
                let mut value = type_args.pop().unwrap_or(Ty::Unknown);
                let mut key = type_args.pop().unwrap_or(Ty::Unknown);
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
                            if matches!(key, Ty::Unknown) {
                                key = join_types(&key, &items[0]);
                            } else {
                                self.require_assignable(
                                    &items[0],
                                    &key,
                                    arg.span,
                                    "invalid_argument_type",
                                    self.diagnostic_type_mismatch_message(
                                        "map key",
                                        &items[0],
                                        "constructor type argument",
                                        &key,
                                    ),
                                );
                            }
                            if matches!(value, Ty::Unknown) {
                                value = join_types(&value, &items[1]);
                            } else {
                                self.require_assignable(
                                    &items[1],
                                    &value,
                                    arg.span,
                                    "invalid_argument_type",
                                    self.diagnostic_type_mismatch_message(
                                        "map value",
                                        &items[1],
                                        "constructor type argument",
                                        &value,
                                    ),
                                );
                            }
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
                if explicit_type_args.is_empty() {
                    self.require_inferred_constructor_type_args(
                        name,
                        &[key.clone(), value.clone()],
                        span,
                    );
                }
                self.require_hashable_map_key(&key, span);
                Some(Ty::Named("Map".to_string(), vec![key, value]))
            }
            _ => None,
        }
    }

    fn constructor_type_argument_seed(
        &mut self,
        name: &str,
        arity: usize,
        explicit_type_args: &[Ty],
        expected: &Ty,
        span: crate::source::Span,
    ) -> Vec<Ty> {
        if !explicit_type_args.is_empty() {
            if explicit_type_args.len() != arity {
                self.add_error(
                    "invalid_type_argument_count",
                    format!(
                        "generic type '{}' expects {} type arguments, got {}",
                        name,
                        arity,
                        explicit_type_args.len()
                    ),
                    span,
                );
            }
            if explicit_type_args
                .iter()
                .any(|ty| matches!(ty, Ty::Wildcard))
            {
                self.add_error(
                    "invalid_constructor_type_arguments",
                    "constructor type arguments must be concrete; omit them to request inference",
                    span,
                );
            }
            return (0..arity)
                .map(|index| {
                    explicit_type_args
                        .get(index)
                        .cloned()
                        .unwrap_or(Ty::Unknown)
                })
                .collect();
        }

        if let Ty::Named(expected_name, expected_args) = expected
            && expected_name == name
            && expected_args.len() == arity
        {
            return expected_args
                .iter()
                .map(|ty| {
                    if matches!(ty, Ty::Wildcard | Ty::Capture(_)) {
                        Ty::Unknown
                    } else {
                        ty.clone()
                    }
                })
                .collect();
        }
        vec![Ty::Unknown; arity]
    }

    fn require_inferred_constructor_type_args(
        &mut self,
        name: &str,
        args: &[Ty],
        span: crate::source::Span,
    ) {
        if args.iter().any(type_contains_unresolved_constructor_type) {
            self.add_error(
                "cannot_infer_constructor_type",
                format!(
                    "cannot infer type arguments for '{}'; add explicit type arguments or an expected type",
                    name
                ),
                span,
            );
        }
    }

    fn constructor_type_argument_subst(
        &mut self,
        sig: &TypeSig,
        explicit_type_args: &[Ty],
        expected: &Ty,
        span: crate::source::Span,
    ) -> HashMap<String, Ty> {
        let args = self.constructor_type_argument_seed(
            &sig.name,
            sig.type_params.len(),
            explicit_type_args,
            expected,
            span,
        );
        sig.type_params
            .iter()
            .cloned()
            .zip(args)
            .filter(|(_, ty)| !matches!(ty, Ty::Unknown | Ty::Wildcard | Ty::Capture(_)))
            .collect()
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
        expected: &Ty,
        explicit_type_args: &[Ty],
    ) -> Ty {
        let generic_ret = Ty::Named(
            sig.name.clone(),
            sig.type_params
                .iter()
                .map(|name| Ty::TypeParam(name.clone()))
                .collect(),
        );
        let subst = self.constructor_type_argument_subst(sig, explicit_type_args, expected, span);
        if explicit_type_args.is_empty()
            && !sig.type_params.is_empty()
            && subst.is_empty()
            && args.is_empty()
        {
            self.add_error(
                "cannot_infer_constructor_type",
                format!(
                    "cannot infer type arguments for '{}'; add explicit type arguments or an expected type",
                    sig.name
                ),
                span,
            );
        }
        let ret = substitute_type(&generic_ret, &subst);
        let mut instantiated_sig = sig.clone();
        for field in &mut instantiated_sig.fields {
            field.ty = substitute_type(&field.ty, &subst);
        }
        for methods in instantiated_sig.methods.values_mut() {
            for method in methods {
                *method = instantiate_function_sig(method.clone(), &subst);
            }
        }
        let sig = &instantiated_sig;

        self.reject_parenthesized_constructor_fields(args, uses_brace_syntax, span);

        if is_builtin_primitive_type_name(&sig.name) {
            let help = match sig.name.as_str() {
                "Int" => "use value.toInt() for Float input or Int.parse(text) for string input",
                "Float" => {
                    "use value.toFloat() for Int input or Float.parse(text) for string input"
                }
                "Str" => "use value.toStr() to convert a value to Str",
                "Decimal" => {
                    "use a decimal literal such as 0.1d, Decimal.parse(text), or an explicit toDecimal() conversion"
                }
                _ => {
                    "use an explicit operation that produces the primitive value; primitive types are not constructors"
                }
            };
            self.diagnostics.push(
                Diagnostic::error(
                    "invalid_primitive_conversion",
                    format!(
                        "cannot convert to primitive type '{}' with constructor syntax",
                        sig.name
                    ),
                    span,
                )
                .with_help(help),
            );
            return ret;
        }

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

        if sig.kind == TypeKind::Mixin {
            self.add_error(
                "invalid_mixin_construction",
                format!(
                    "mixin '{}' cannot be constructed; adopt it inside a class with 'use {}'",
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
                let constructed =
                    self.check_record_constructor_conversion(sig, &ret, &args[0].value, span);
                self.check_constructed_type_conditions(sig, &constructed, span);
                return constructed;
            }
        }

        if let Some(overloads) = constructor_overloads {
            let visible = overloads
                .iter()
                .filter(|ctor| self.can_access_constructor(sig, ctor.visibility))
                .cloned()
                .collect::<Vec<_>>();
            let candidates = self
                .best_overload_candidates(&visible, args)
                .into_iter()
                .cloned()
                .collect::<Vec<_>>();
            if !candidates.is_empty() {
                self.diagnose_best_overload_candidates(&candidates, args, span);
                let ctor = &candidates[0];
                let params = constructor_field_sigs_from_params(&ctor.params);
                let constructed = self.check_constructor_signature(&params, &ret, args, span);
                self.check_constructed_type_conditions(sig, &constructed, span);
                return constructed;
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
            let private = overloads
                .iter()
                .filter(|ctor| {
                    ctor.visibility == Visibility::Private
                        && !self.can_access_constructor(sig, ctor.visibility)
                })
                .cloned()
                .collect::<Vec<_>>();
            if !self.best_overload_candidates(&private, args).is_empty() {
                let help = self.private_constructor_factory_help(&sig.name, args);
                self.diagnostics
                    .push(typecheck_diagnostics::private_field_constructor(
                        span, &sig.name, help,
                    ));
                return ret;
            }
            let internal = overloads
                .iter()
                .filter(|ctor| {
                    ctor.visibility == Visibility::Internal
                        && !self.can_access_constructor(sig, ctor.visibility)
                })
                .cloned()
                .collect::<Vec<_>>();
            if !self.best_overload_candidates(&internal, args).is_empty() {
                self.add_error(
                    "inaccessible_constructor",
                    format!(
                        "constructor for '{}' is internal and can be called only from module '{}'",
                        sig.name, sig.module_id
                    ),
                    span,
                );
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
            let constructed =
                self.check_positional_record_constructor_conversion(sig, &ret, args, span);
            self.check_constructed_type_conditions(sig, &constructed, span);
            return constructed;
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

    fn check_constructed_type_conditions(
        &mut self,
        sig: &TypeSig,
        constructed: &Ty,
        span: crate::source::Span,
    ) {
        if sig.generic_conditions.is_empty() {
            return;
        }
        let Ty::Named(name, args) = constructed else {
            return;
        };
        if name != &sig.name || args.len() != sig.type_params.len() {
            return;
        }
        let subst = sig
            .type_params
            .iter()
            .cloned()
            .zip(args.iter().cloned())
            .collect::<HashMap<_, _>>();
        self.check_call_generic_conditions(&sig.generic_conditions, &subst, span);
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
                "invalid_union_variant_call",
                format!(
                    "union variant '{case_name}' does not accept call syntax; use '{case_name}'"
                ),
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
                visibility: param.visibility,
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
                    let sig = self.select_overload_for_call(&functions, args, span)?;
                    return Some(CallableSelection {
                        sig,
                        explicit_type_args,
                    });
                }
                if let Some(methods) = self.lookup_implicit_method_functions(name) {
                    let sig = self.select_overload_for_call(&methods, args, span)?;
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
                        let sig = self.select_overload_for_call(functions, args, span)?;
                        return Some(CallableSelection {
                            sig,
                            explicit_type_args,
                        });
                    }
                }
                if let Some(sigs) = self.static_method_sigs(receiver, name) {
                    let sig = self.select_overload_for_call(&sigs, args, span)?;
                    return Some(CallableSelection {
                        sig,
                        explicit_type_args,
                    });
                }
                let receiver_ty = self.check_expr(receiver);
                let methods = self.member_method_sigs(&receiver_ty, name)?;
                let method = self.select_overload_for_call(&methods, args, span)?;
                Some(CallableSelection {
                    sig: method,
                    explicit_type_args,
                })
            }
            _ => None,
        }
    }

    fn select_overload_for_call(
        &mut self,
        overloads: &[FunctionSig],
        args: &[crate::ast::CallArg],
        span: crate::source::Span,
    ) -> Option<FunctionSig> {
        let candidates = self.best_overload_candidates(overloads, args);
        if candidates.is_empty() {
            return self.closest_overload(overloads, args).cloned();
        }

        let candidates = candidates.into_iter().cloned().collect::<Vec<_>>();
        self.diagnose_best_overload_candidates(&candidates, args, span);

        // Keep checking after an ambiguity so the compiler can report errors in
        // nested argument expressions. The ambiguity diagnostic prevents this
        // recovery choice from becoming observable program behavior.
        candidates.into_iter().next()
    }

    fn diagnose_best_overload_candidates(
        &mut self,
        candidates: &[FunctionSig],
        args: &[crate::ast::CallArg],
        span: crate::source::Span,
    ) {
        let diagnostics_before = self.diagnostics.len();
        self.diagnose_ambiguous_contextual_construction_call(&candidates, args, span);
        if candidates.len() > 1 && self.diagnostics.len() == diagnostics_before {
            let signatures = candidates
                .iter()
                .map(function_sig_type)
                .map(|ty| ty.describe())
                .collect::<Vec<_>>()
                .join(", ");
            self.add_error(
                "ambiguous_overload",
                format!(
                    "call matches multiple overloads: {signatures}; add type information or select the intended overload explicitly"
                ),
                span,
            );
        }
    }

    fn diagnose_ambiguous_contextual_construction_call(
        &mut self,
        overloads: &[FunctionSig],
        args: &[crate::ast::CallArg],
        span: crate::source::Span,
    ) {
        for (contextual_arg, construction) in args.iter().filter_map(|arg| {
            contextual_construction_expr(&arg.value).map(|construction| (arg, construction))
        }) {
            let actual = self.probe_expr_type(&contextual_arg.value);
            let mut targets = HashSet::new();
            for sig in overloads {
                let arrangement = arrange_param_args(&sig.params, args);
                if arrangement.overflow > 0 || arrangement.missing_required > 0 {
                    continue;
                }
                for (index, param) in sig.params.iter().enumerate() {
                    let matches_arg = arrangement
                        .slots
                        .get(index)
                        .into_iter()
                        .flatten()
                        .any(|candidate| std::ptr::eq(*candidate, contextual_arg));
                    if !matches_arg {
                        continue;
                    }
                    let expected =
                        call_arg_expected_ty_for_arg(param.variadic, &param.ty, contextual_arg);
                    if !matches!(actual, Ty::Unknown)
                        && !self.arg_matches_expected(contextual_arg, &actual, &expected)
                    {
                        continue;
                    }
                    let Ty::Named(name, _) = &expected else {
                        continue;
                    };
                    if self.lookup_any_type(name).is_some_and(|target| {
                        matches!(target.kind, TypeKind::Class | TypeKind::Record)
                    }) {
                        targets.insert(expected.describe());
                    }
                }
            }
            if targets.len() > 1 {
                self.add_error(
                    "ambiguous_contextual_construction",
                    "contextual construction matches multiple concrete overload targets; name the target explicitly, for example 'User { ... }' or 'User(...)'",
                    construction.span(),
                );
            }
        }

        let empty_collection_span = args
            .iter()
            .find_map(|arg| empty_collection_literal_span(&arg.value));
        if empty_collection_span.is_none() {
            return;
        }
        let candidate_count = self.shape_context_candidate_count(overloads, args);
        if candidate_count <= 1 {
            return;
        }
        self.add_error(
            "ambiguous_empty_collection",
            "empty collection '[]' matches multiple vector/map overloads; add an intermediate typed binding",
            empty_collection_span.unwrap_or(span),
        );
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

    fn callable_signature_for_args_probe(
        &self,
        callee: &Expr,
        args: &[crate::ast::CallArg],
        uses_brace_syntax: bool,
    ) -> Option<(Vec<ParamSig>, Ty)> {
        self.callable_signature_for_args_probe_with_locals(
            callee,
            args,
            uses_brace_syntax,
            &HashMap::new(),
        )
    }

    fn callable_signature_for_args_probe_with_locals(
        &self,
        callee: &Expr,
        args: &[crate::ast::CallArg],
        _uses_brace_syntax: bool,
        locals: &HashMap<String, Ty>,
    ) -> Option<(Vec<ParamSig>, Ty)> {
        let (callee, explicit_type_args) = self.split_generic_call_callee(callee);
        match callee {
            Expr::Identifier { name, .. } => {
                if let Some(functions) = self.lookup_functions(name) {
                    let sig = self
                        .choose_overload_with_locals(&functions, args, locals)?
                        .clone();
                    return Some(function_sig_parts_for_probe(sig, &explicit_type_args));
                }
                let methods = self.lookup_implicit_method_functions(name)?;
                let sig = self
                    .choose_overload_with_locals(&methods, args, locals)?
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
                            .choose_overload_with_locals(functions, args, locals)?
                            .clone();
                        return Some(function_sig_parts_for_probe(sig, &explicit_type_args));
                    }
                }
                if let Some(sigs) = self.static_method_sigs(receiver, name) {
                    let sig = self
                        .choose_overload_with_locals(&sigs, args, locals)?
                        .clone();
                    return Some(function_sig_parts_for_probe(sig, &explicit_type_args));
                }
                let receiver_ty = self.probe_expr_type_with_locals(receiver, locals);
                let methods = self.member_method_sigs(&receiver_ty, name)?;
                let method = self
                    .choose_overload_with_locals(&methods, args, locals)?
                    .clone();
                Some(function_sig_parts_for_probe(method, &explicit_type_args))
            }
            _ => None,
        }
    }

    fn is_explicit_generic_call_target(&self, callee: &Expr) -> bool {
        match callee {
            Expr::Identifier { name, .. } => {
                if self.lookup_scoped_value(name).is_some()
                    || self.lookup_implicit_field(name).is_some()
                    || self.lookup_implicit_getter(name).is_some()
                    || self.lookup_global_value(name).is_some()
                {
                    return false;
                }
                if self.lookup_functions(name).is_some_and(|functions| {
                    functions.iter().any(|sig| !sig.type_params.is_empty())
                }) || self
                    .lookup_implicit_method_functions(name)
                    .is_some_and(|methods| methods.iter().any(|sig| !sig.type_params.is_empty()))
                {
                    return true;
                }
                self.brace_call_type_sig(callee)
                    .is_some_and(|sig| !sig.type_params.is_empty())
                    || matches!(
                        name.as_str(),
                        "Vector" | "LinkedList" | "Array" | "Set" | "Map"
                    )
            }
            Expr::Member { receiver, name, .. } => {
                if let Some((module, member)) =
                    module_alias_and_member(callee).and_then(|(alias, member)| {
                        self.world
                            .lookup_module_alias(self.module, &alias)
                            .map(|module| (module, member))
                    })
                    && module.functions.get(&member).is_some_and(|functions| {
                        functions.iter().any(|sig| !sig.type_params.is_empty())
                    })
                {
                    return true;
                }
                if self
                    .static_method_sigs(receiver, name)
                    .is_some_and(|methods| methods.iter().any(|sig| !sig.type_params.is_empty()))
                {
                    return true;
                }
                let receiver_ty = self.probe_expr_type(receiver);
                if self
                    .member_method_sigs(&receiver_ty, name)
                    .is_some_and(|methods| methods.iter().any(|sig| !sig.type_params.is_empty()))
                {
                    return true;
                }
                self.brace_call_type_sig(callee)
                    .is_some_and(|sig| !sig.type_params.is_empty())
            }
            _ => false,
        }
    }

    fn split_generic_call_callee<'expr>(&self, callee: &'expr Expr) -> (&'expr Expr, Vec<Ty>) {
        let Expr::Index {
            receiver,
            index,
            explicit_type_args,
            ..
        } = callee
        else {
            return (callee, Vec::new());
        };
        if !self.is_explicit_generic_call_target(receiver) {
            return (callee, Vec::new());
        }
        let Some(type_args) = explicit_type_args
            .clone()
            .or_else(|| type_arg_refs_from_expr(index))
        else {
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
        let callee = match callee {
            Expr::Index {
                receiver,
                index,
                explicit_type_args,
                ..
            } if explicit_type_args.is_some() || type_arg_refs_from_expr(index).is_some() => {
                receiver.as_ref()
            }
            _ => callee,
        };
        let class_sig = match callee {
            Expr::Identifier { name, .. } => self
                .lookup_type_local(name)
                .or_else(|| self.world.lookup_imported_type(self.module, name))
                .or_else(|| self.lookup_unique_module_type(name))
                .or_else(|| self.lookup_any_object(name))
                .or_else(|| self.world.ambient.types.get(name).cloned()),
            Expr::Member { .. } => expr_path_for_known_value(callee)
                .map(|path| path.join("."))
                .and_then(|name| {
                    self.lookup_type_local(&name)
                        .or_else(|| self.world.lookup_imported_type(self.module, &name))
                })
                .or_else(|| {
                    module_alias_and_member(callee).and_then(|(alias, member)| {
                        self.world
                            .lookup_module_alias(self.module, &alias)
                            .and_then(|module| {
                                module
                                    .types
                                    .get(&member)
                                    .cloned()
                                    .or_else(|| module.objects.get(&member).cloned())
                            })
                    })
                }),
            _ => None,
        };

        class_sig
    }

    fn probe_named_constructor_result_type(&self, callee: &Expr) -> Option<Ty> {
        let (_, explicit_type_args) = self.split_generic_call_callee(callee);
        let sig = self.brace_call_type_sig(callee)?;
        let type_args = if explicit_type_args.is_empty() {
            sig.type_params.iter().map(|_| Ty::Unknown).collect()
        } else {
            explicit_type_args
        };
        Some(Ty::Named(sig.name, type_args))
    }

    fn static_member_value_type(&self, receiver: &Expr, name: &str, expected: &Ty) -> Option<Ty> {
        if let Some(mut path) = expr_path_for_known_value(receiver) {
            path.push(name.to_string());
            let qualified = path.join(".");
            if let Some(sig) = self
                .lookup_object_local(&qualified)
                .or_else(|| self.world.lookup_imported_type(self.module, &qualified))
                .filter(|sig| sig.kind == TypeKind::Object)
            {
                return Some(Ty::Named(sig.name, Vec::new()));
            }
        }
        let Expr::Identifier {
            name: type_name, ..
        } = receiver
        else {
            return None;
        };
        if let Some(sig) = self.lookup_any_object(type_name) {
            if let Some(methods) = self.method_sigs_for_type(&sig, name) {
                let first = methods.first()?;
                return Some(if first.getter {
                    first.ret.clone()
                } else {
                    Ty::Function(
                        first.params.iter().map(|param| param.ty.clone()).collect(),
                        Box::new(first.ret.clone()),
                    )
                });
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

    fn static_callable_reference_sigs(
        &self,
        receiver: &Expr,
        name: &str,
    ) -> Option<Vec<FunctionSig>> {
        let Expr::Identifier {
            name: type_name, ..
        } = receiver
        else {
            return None;
        };
        let sig = self.lookup_any_object(type_name)?;
        let methods = self
            .method_sigs_for_type(&sig, name)?
            .into_iter()
            .filter(|method| !method.getter)
            .collect::<Vec<_>>();
        (!methods.is_empty()).then_some(methods)
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

    fn private_constructor_factory_help(
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
        if matches!(
            op,
            BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Mod
        ) && (left.is_decimal() || right.is_decimal())
            && !(op == BinaryOp::Add && (left.is_str() || right.is_str()))
        {
            if left.is_decimal() && right.is_decimal() {
                return Ty::named("Decimal");
            }
            if !matches!(left, Ty::Unknown) && !matches!(right, Ty::Unknown) {
                self.add_error("invalid_binary_operand", "Decimal arithmetic requires two Decimal operands; use an explicit toDecimal() conversion", span);
            }
            return Ty::Unknown;
        }
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
                } else if matches!(left, Ty::Unknown) || matches!(right, Ty::Unknown) {
                    Ty::Unknown
                } else if let Some(ret) = self.binary_operator_method_return_type(left, "+", right)
                {
                    ret
                } else {
                    self.add_error(
                        "invalid_binary_operand",
                        format!(
                            "operator '+' expects numeric operands, a Str operand, or a compatible '+' method; got '{}' and '{}'",
                            left.describe(),
                            right.describe()
                        ),
                        span,
                    );
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
            BinaryOp::Less | BinaryOp::LessEq | BinaryOp::Greater | BinaryOp::GreaterEq => {
                self.check_ordered_operands(left, right, span);
                Ty::bool()
            }
            BinaryOp::And | BinaryOp::Or => {
                self.require_bool(left, span, "logical operator expects Bool operands");
                self.require_bool(right, span, "logical operator expects Bool operands");
                Ty::bool()
            }
            BinaryOp::Colon => Ty::Unknown,
        }
    }

    fn binary_operator_method_return_type(
        &self,
        receiver: &Ty,
        method_name: &str,
        argument: &Ty,
    ) -> Option<Ty> {
        self.member_method_sigs(receiver, method_name)?
            .into_iter()
            .find(|method| {
                !method.getter
                    && method.params.len() == 1
                    && self.is_assignable(argument, &method.params[0].ty)
            })
            .map(|method| method.ret)
    }

    fn check_ordered_operands(&mut self, left: &Ty, right: &Ty, span: crate::source::Span) {
        if matches!(left, Ty::Unknown) || matches!(right, Ty::Unknown) {
            return;
        }
        if (left.is_numeric() && right.is_numeric() && left.is_decimal() == right.is_decimal())
            || (left.is_str() && right.is_str())
            || self.is_assignable(
                left,
                &Ty::Named("Comparable".to_string(), vec![right.clone()]),
            )
        {
            return;
        }
        self.add_error(
            "invalid_comparison_operands",
            format!(
                "comparison requires numeric values, Str values, or a left operand implementing Comparable[{}]; got '{}' and '{}'",
                right.describe(),
                left.describe(),
                right.describe()
            ),
            span,
        );
    }

    fn check_equality_operands(&mut self, left: &Ty, right: &Ty, span: crate::source::Span) {
        if matches!(left, Ty::Unknown) || matches!(right, Ty::Unknown) {
            return;
        }
        if left.is_any() || right.is_any() {
            self.add_error(
                "any_equality_not_supported",
                format!(
                    "'==' and '!=' do not compare Any values; narrow the value before comparing it (got '{}' and '{}')",
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

        if !self.is_equality_type(left, &mut HashSet::new()) {
            self.add_error(
                "missing_equality_contract",
                format!(
                    "type '{}' has no equality contract; every tuple or union member must support Eq, and classes must declare 'with Eq[{}]'",
                    left.describe(),
                    left.describe()
                ),
                span,
            );
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

        let mut reported = HashSet::new();
        for operand in [left, right] {
            if reported.insert(operand.describe())
                && !self.is_equality_type(operand, &mut HashSet::new())
            {
                self.add_error(
                    "missing_equality_contract",
                    format!(
                        "type '{}' has no equality contract; a shape derives equality only when every field is public and satisfies Eq, otherwise declare 'with Eq[{}]' and implement equals(other)",
                        operand.describe(),
                        operand.describe()
                    ),
                    span,
                );
            }
        }
    }

    fn supports_reference_id(&self, ty: &Ty) -> bool {
        let Ty::Named(name, _) = ty else {
            return false;
        };
        if matches!(
            name.as_str(),
            "Any" | "Bool" | "Decimal" | "Float" | "Int" | "ReferenceId" | "Rune" | "Str" | "Unit"
        ) {
            return false;
        }
        self.lookup_any_type(name)
            .is_some_and(|sig| matches!(sig.kind, TypeKind::Class | TypeKind::Object))
    }

    fn check_reference_id_access(&mut self, receiver: &Ty, span: crate::source::Span) -> Ty {
        if matches!(receiver, Ty::Unknown) {
            return Ty::Unknown;
        }
        if self.supports_reference_id(receiver) {
            return Ty::named("ReferenceId");
        }
        self.add_error(
            "invalid_reference_id_receiver",
            format!(
                "'referenceId' is available only on class, object, or concrete collection values; '{}' is not reference-bearing",
                receiver.describe()
            ),
            span,
        );
        Ty::Unknown
    }

    fn check_else_expr_branch_against_with_narrowings(
        &mut self,
        branch: &ElseExprBranch,
        expected: &Ty,
        narrowings: &[TypeNarrowing],
    ) -> Ty {
        self.push_scope();
        for narrowing in narrowings {
            self.define_local(&narrowing.name, narrowing.ty.clone(), false);
        }
        let result = match branch {
            ElseExprBranch::If(expr) => self.check_expr_against(expr, expected),
            ElseExprBranch::Block(block) => self.check_block_against(block, expected),
        };
        self.pop_scope();
        result
    }

    fn type_narrowings_for_condition(
        &self,
        condition: &Expr,
        condition_is_true: bool,
    ) -> Vec<TypeNarrowing> {
        match condition {
            Expr::Group { inner, .. } => {
                self.type_narrowings_for_condition(inner, condition_is_true)
            }
            Expr::Unary {
                op: crate::ast::UnaryOp::Not,
                expr,
                ..
            } => self.type_narrowings_for_condition(expr, !condition_is_true),
            Expr::Is { left, target, .. } if condition_is_true => {
                let Expr::Identifier { name, .. } = left.as_ref() else {
                    return Vec::new();
                };
                let Some(value) = self.lookup_scoped_value(name) else {
                    return Vec::new();
                };
                if !value.stable || runtime_type_ref_has_arguments(target) {
                    return Vec::new();
                }
                let ty = self.ty_from_type_ref(target);
                if matches!(ty, Ty::TypeParam(_) | Ty::Wildcard) {
                    return Vec::new();
                }
                vec![TypeNarrowing {
                    name: name.clone(),
                    ty,
                }]
            }
            Expr::Binary {
                left,
                op: BinaryOp::And,
                right,
                ..
            } if condition_is_true => {
                let mut narrowings = self.type_narrowings_for_condition(left, true);
                narrowings.extend(self.type_narrowings_for_condition(right, true));
                narrowings
            }
            Expr::Binary {
                left,
                op: BinaryOp::Or,
                right,
                ..
            } if !condition_is_true => {
                let mut narrowings = self.type_narrowings_for_condition(left, false);
                narrowings.extend(self.type_narrowings_for_condition(right, false));
                narrowings
            }
            _ => Vec::new(),
        }
    }

    fn validate_runtime_type_ref(&mut self, reference: &TypeRef, ty: &Ty, construct: &str) {
        if let Ty::Named(name, _) = ty
            && self
                .lookup_any_type(name)
                .is_some_and(|sig| sig.kind == TypeKind::Mixin)
        {
            self.add_error(
                "mixin_not_a_type",
                "mixins cannot be used in runtime type tests or patterns",
                reference.span(),
            );
            return;
        }
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
            TypeRef::Union { span, .. } => {
                self.add_error(
                    "invalid_runtime_union_type",
                    format!(
                        "{construct} require one concrete runtime type; match union alternatives separately"
                    ),
                    *span,
                );
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
            Pattern::Alias { inner, name, .. } => {
                self.bind_pattern(inner, scrutinee);
                if name != "_" {
                    let binding_ty = self.whole_pattern_binding_type(inner, scrutinee);
                    self.define_local(name, binding_ty, false);
                }
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
                                "sequence pattern requires a Vector or Array value, got '{}'",
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
                if let [name] = path.as_slice()
                    && self.report_ambiguous_union_alternative(name, pattern.span())
                {
                    for field in fields {
                        self.bind_pattern(&field.pattern, &Ty::Unknown);
                    }
                    return;
                }
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
                                "headless record pattern requires a concrete class, shape, anonymous shape, or declared-union value; got '{}'",
                                scrutinee.describe()
                            )
                        } else {
                            format!("unknown record pattern '{target_name}'")
                        },
                        pattern.span(),
                    );
                    return;
                };
                if fields.is_empty() {
                    let message = if path.is_empty() {
                        "empty field patterns are redundant; use '_' to ignore the matched value or '_ as value' to bind it".to_string()
                    } else if is_enum_case && target_fields.is_empty() {
                        format!(
                            "empty field pattern '{} {{}}' is redundant; use the bare zero-payload case '{}'",
                            target_name, target_name
                        )
                    } else if is_enum_case {
                        format!(
                            "empty field pattern '{} {{}}' does not select its payload; use '{}(_)' to ignore a unary payload or select fields by name",
                            target_name, target_name
                        )
                    } else {
                        format!(
                            "empty field pattern '{} {{}}' is redundant; use '_ {}' to ignore the matched value or 'value {}' to bind it",
                            target_name, target_name, target_name
                        )
                    };
                    self.add_error("empty_record_pattern", message, pattern.span());
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
            Pattern::Constructor {
                path,
                args,
                parenthesized,
                option_shorthand,
                ..
            } => {
                if *parenthesized && !*option_shorthand {
                    self.diagnostics
                        .push(typecheck_diagnostics::parenthesized_pattern(
                            path,
                            pattern.span(),
                        ));
                    for argument in args {
                        self.bind_pattern(argument, &Ty::Unknown);
                    }
                    return;
                }
                if let [name] = path.as_slice()
                    && self.report_ambiguous_union_alternative(name, pattern.span())
                {
                    for pattern in args {
                        self.bind_pattern(pattern, &Ty::Unknown);
                    }
                    return;
                }
                if *option_shorthand
                    && !matches!(scrutinee, Ty::Named(name, args) if name == "Option" && args.len() == 1)
                {
                    for pattern in args {
                        self.bind_pattern(pattern, &Ty::Unknown);
                    }
                    self.add_error(
                        "invalid_option_pattern",
                        format!(
                            "option pattern shorthand '^pattern' requires an Option source, got '{}'",
                            scrutinee.describe()
                        ),
                        pattern.span(),
                    );
                    return;
                }
                let case_name = path.last().cloned().unwrap_or_default();
                if let Some(case) = self.lookup_case_by_pattern(path, scrutinee) {
                    let extractable = self.enum_case_extractable_params(&case);
                    if !*parenthesized && !extractable.is_empty() {
                        self.add_error(
                            "bare_payload_pattern",
                            bare_payload_pattern_diagnostic(
                                &case_name,
                                &extractable
                                    .iter()
                                    .map(|field| field.name.clone())
                                    .collect::<Vec<_>>(),
                            ),
                            pattern.span(),
                        );
                    }
                    let mut subst = HashMap::new();
                    infer_type_subst(&case.result, scrutinee, &mut subst);
                    for (pattern, param) in args.iter().zip(extractable.iter()) {
                        self.bind_pattern(
                            pattern,
                            &materialize_type(&substitute_type(&param.ty, &subst)),
                        );
                    }
                } else if let Some(object) = self.lookup_object_pattern(path) {
                    if *parenthesized && !args.is_empty() {
                        self.add_error(
                            "invalid_singleton_pattern",
                            format!(
                                "singleton object '{}' is matched by its bare name",
                                object.name
                            ),
                            pattern.span(),
                        );
                    }
                } else if let Some((false, _, fields)) =
                    self.lookup_record_pattern_target(path, scrutinee)
                {
                    if !*parenthesized {
                        self.add_error(
                            "bare_type_pattern",
                            bare_type_pattern_diagnostic(&case_name),
                            pattern.span(),
                        );
                    }
                    for (pattern, (_, field_ty)) in args.iter().zip(fields.iter()) {
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
            Pattern::Alias { inner, .. } => self.pattern_is_irrefutable(inner, scrutinee),
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
                let Some((false, destructured_ty, fields)) =
                    self.lookup_record_pattern_target(path, scrutinee)
                else {
                    return false;
                };
                self.is_assignable(scrutinee, &destructured_ty)
                    && fields.len() <= 1
                    && args.len() == fields.len()
                    && args
                        .iter()
                        .zip(fields.iter())
                        .all(|(pattern, (_, ty))| self.pattern_is_irrefutable(pattern, ty))
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
            match pattern_without_alias(&case.pattern) {
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
            format!(
                "match does not cover union variants: {}",
                missing.join(", ")
            ),
            span,
        );
    }

    fn match_case_name_for_enum(&self, path: &[String], enum_name: &str) -> Option<String> {
        let case = self.lookup_case_by_path(path)?;
        if !matches!(&case.result, Ty::Named(name, _) if name == enum_name) {
            return None;
        }
        if let [name] = path
            && let Some(imported) = self.module.symbol_imports.get(name)
            && imported.kind == ImportedKind::UnionAlternative
        {
            return Some(imported.original_name.clone());
        }
        path.last().cloned()
    }

    fn lookup_case_by_path(&self, path: &[String]) -> Option<EnumCaseSig> {
        let (case_name, owner_path) = path.split_last()?;
        if owner_path.is_empty() {
            return self.world.lookup_enum_case(self.module, case_name);
        }
        self.lookup_pattern_type_path(owner_path)
            .filter(|sig| sig.kind == TypeKind::Enum)
            .and_then(|sig| sig.enum_cases.get(case_name).cloned())
    }

    fn lookup_case_by_pattern(&self, path: &[String], scrutinee: &Ty) -> Option<EnumCaseSig> {
        let _ = scrutinee;
        self.lookup_case_by_path(path)
    }

    fn lookup_object_pattern(&self, path: &[String]) -> Option<TypeSig> {
        let sig = self.lookup_pattern_type_path(path)?;
        (sig.kind == TypeKind::Object).then_some(sig)
    }

    fn whole_pattern_binding_type(&self, pattern: &Pattern, scrutinee: &Ty) -> Ty {
        match pattern {
            Pattern::Alias { inner, .. } => self.whole_pattern_binding_type(inner, scrutinee),
            Pattern::Type { target, .. } => self.ty_from_type_ref(target),
            Pattern::Record { path, .. } if !path.is_empty() => {
                if let Some(case) = self.lookup_case_by_pattern(path, scrutinee) {
                    return self.enum_case_view_type(path, &case, scrutinee);
                }
                self.lookup_record_pattern_target(path, scrutinee)
                    .map(|(_, target, _)| target)
                    .unwrap_or_else(|| scrutinee.clone())
            }
            Pattern::Constructor { path, .. } => {
                if let Some(case) = self.lookup_case_by_pattern(path, scrutinee) {
                    return self.enum_case_view_type(path, &case, scrutinee);
                }
                if let Some(object) = self.lookup_object_pattern(path) {
                    return Ty::Named(object.name, Vec::new());
                }
                self.lookup_record_pattern_target(path, scrutinee)
                    .map(|(_, target, _)| target)
                    .unwrap_or_else(|| scrutinee.clone())
            }
            _ => scrutinee.clone(),
        }
    }

    fn enum_case_view_type(&self, path: &[String], case: &EnumCaseSig, scrutinee: &Ty) -> Ty {
        let mut subst = HashMap::new();
        infer_type_subst(&case.result, scrutinee, &mut subst);
        let result = materialize_type(&substitute_type(&case.result, &subst));
        let Ty::Named(owner, args) = result else {
            return scrutinee.clone();
        };
        let case_name = path.last().cloned().unwrap_or_default();
        Ty::Named(enum_case_view_name(&owner, &case_name), args)
    }

    fn enum_case_extractable_params(&self, case: &EnumCaseSig) -> Vec<FieldSig> {
        let shared_fields = match &case.result {
            Ty::Named(enum_name, _) => self
                .lookup_any_type(enum_name)
                .map(|sig| {
                    sig.fields
                        .iter()
                        .map(|field| field.name.clone())
                        .collect::<HashSet<_>>()
                })
                .unwrap_or_default(),
            _ => HashSet::new(),
        };
        case.params
            .iter()
            .filter(|field| {
                field.visibility != Visibility::Private && !shared_fields.contains(&field.name)
            })
            .cloned()
            .collect()
    }

    fn lookup_destructured_type_pattern(&self, path: &[String]) -> Option<(Ty, Vec<Ty>)> {
        let sig = self.lookup_pattern_type_path(path)?;
        if !matches!(sig.kind, TypeKind::Class | TypeKind::Record) {
            return None;
        }
        let ty = Ty::Named(
            sig.name.clone(),
            sig.type_params.iter().map(|_| Ty::Unknown).collect(),
        );
        let fields = sig
            .fields
            .iter()
            .filter(|field| self.can_access_field(&sig, field))
            .map(|field| field.ty.clone())
            .collect();
        Some((ty, fields))
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
                            .filter(|field| self.can_access_field(&sig, field))
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
                .filter(|field| field.visibility != Visibility::Private)
                .map(|field| {
                    (
                        field.name.clone(),
                        materialize_type(&substitute_type(&field.ty, &subst)),
                    )
                })
                .collect();
            return Some((true, case.result, fields));
        }

        let sig = self.lookup_pattern_type_path(path)?;
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
            .filter(|field| self.can_access_field(&sig, field))
            .map(|field| {
                (
                    field.name.clone(),
                    materialize_type(&substitute_type(&field.ty, &subst)),
                )
            })
            .collect();
        Some((false, target, fields))
    }

    fn lookup_pattern_type_path(&self, path: &[String]) -> Option<TypeSig> {
        let joined = path.join(".");
        if let Some(sig) = self.lookup_any_type(&joined) {
            return Some(sig);
        }
        let (module_alias, nested_path) = path.split_first()?;
        if nested_path.is_empty() {
            return None;
        }
        let name = nested_path.join(".");
        self.world
            .lookup_module_alias(self.module, module_alias)
            .and_then(|module| {
                module
                    .types
                    .get(&name)
                    .cloned()
                    .or_else(|| module.objects.get(&name).cloned())
            })
    }

    fn unwrap_inner_type(&self, ty: &Ty) -> Ty {
        match ty {
            Ty::Named(name, args) if name == "Option" && args.len() == 1 => args[0].clone(),
            Ty::Named(name, args) if name == "Result" && args.len() >= 1 => args[0].clone(),
            Ty::Named(name, args) if name == "Either" && args.len() == 2 => args[1].clone(),
            _ => Ty::Unknown,
        }
    }

    fn check_option_wrap_expr(&mut self, value: &Expr, expected: &Ty) -> Ty {
        let expected_payload = match expected {
            Ty::Named(name, args) if name == "Option" && args.len() == 1 => args.first().cloned(),
            _ => None,
        };

        let actual = if let Some(payload) = expected_payload.as_ref() {
            self.check_expr_against(value, payload)
        } else {
            self.check_expr(value)
        };

        if let Some(payload) = expected_payload {
            self.require_assignable(
                &actual,
                &payload,
                value.span(),
                "invalid_optional_wrap_value",
                format!(
                    "optional wrapping value has type {} but the expected Option payload type is {}",
                    self.diagnostic_type_phrase(&actual),
                    self.diagnostic_type_phrase(&payload)
                ),
            );
            Ty::option(materialize_type(&payload))
        } else {
            Ty::option(actual)
        }
    }

    fn list_element_type(&self, ty: &Ty) -> Option<Ty> {
        match ty {
            Ty::Named(name, args)
                if matches!(name.as_str(), "Vector" | "Array") && args.len() == 1 =>
            {
                args.first().cloned()
            }
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
        let lifted = matches!(
            &value_ty,
            Ty::Named(name, args)
                if (name == "Option" && args.len() == 1)
                    || (name == "Result" && !args.is_empty())
                    || (name == "Either" && args.len() == 2)
        );
        if inner == Ty::Unknown && !matches!(value_ty, Ty::Unknown) && !lifted {
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

        let fallback_ty = if inner == Ty::Unknown {
            self.check_expr(fallback)
        } else {
            self.check_expr_against(fallback, &inner)
        };
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
        if lifted && inner == Ty::Unknown {
            materialize_type(&fallback_ty)
        } else {
            inner
        }
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
        self.known_iterable_item_type_inner(ty, &mut HashSet::new())
    }

    fn known_iterable_item_type_inner(&self, ty: &Ty, seen: &mut HashSet<String>) -> Option<Ty> {
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
            Ty::Named(name, args) => {
                if !seen.insert(name.clone()) {
                    return None;
                }
                let sig = self.lookup_any_type(name)?;
                let subst = sig
                    .type_params
                    .iter()
                    .cloned()
                    .zip(args.iter().cloned())
                    .collect::<HashMap<_, _>>();
                let item = sig.with_bounds.iter().find_map(|bound| {
                    let bound = substitute_type(bound, &subst);
                    self.known_iterable_item_type_inner(&bound, seen)
                });
                seen.remove(name);
                item
            }
            Ty::TypeParam(name) => self
                .type_param_bounds(name)
                .into_iter()
                .find_map(|bound| self.known_iterable_item_type_inner(&bound, seen)),
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
        let Ty::Named(type_name, args) = receiver else {
            return None;
        };
        let sig = self.lookup_any_type(type_name)?;
        let mut field = sig.fields.iter().find(|field| field.name == name)?.clone();
        let subst = sig
            .type_params
            .iter()
            .cloned()
            .zip(args.iter().cloned())
            .collect();
        field.ty = substitute_type(&field.ty, &subst);
        Some(field)
    }

    fn can_access_member_visibility(
        &self,
        owner_name: &str,
        owner_module: &str,
        visibility: Visibility,
    ) -> bool {
        match visibility {
            Visibility::Default => true,
            Visibility::Internal => self.module.module_id == owner_module,
            Visibility::Private => {
                self.current_extension_target.is_none()
                    && self.current_owner.as_ref().is_some_and(|owner| {
                        owner.name == owner_name && owner.module_id == owner_module
                    })
            }
        }
    }

    fn declared_member_access(
        &self,
        sig: &TypeSig,
        name: &str,
        seen: &mut HashSet<String>,
    ) -> Option<(&'static str, Visibility, String, String)> {
        let key = format!("{}::{}", sig.module_id, sig.name);
        if !seen.insert(key) {
            return None;
        }
        if let Some(field) = sig.fields.iter().find(|field| field.name == name) {
            return Some((
                "field",
                field.visibility,
                sig.name.clone(),
                sig.module_id.clone(),
            ));
        }
        if let Some(methods) = sig.methods.get(name) {
            let visibility = methods
                .iter()
                .find(|method| {
                    self.can_access_member_visibility(&sig.name, &sig.module_id, method.visibility)
                })
                .or_else(|| methods.first())
                .map(|method| method.visibility)
                .unwrap_or(Visibility::Default);
            return Some((
                "method",
                visibility,
                sig.name.clone(),
                sig.module_id.clone(),
            ));
        }
        for bound in &sig.with_bounds {
            let Ty::Named(bound_name, _) = bound else {
                continue;
            };
            let Some(bound_sig) = self.lookup_any_type(bound_name) else {
                continue;
            };
            if let Some(access) = self.declared_member_access(&bound_sig, name, seen) {
                return Some(access);
            }
        }
        None
    }

    fn inaccessible_member(
        &self,
        receiver: &Ty,
        name: &str,
    ) -> Option<(&'static str, Visibility, String)> {
        let Ty::Named(type_name, _) = receiver else {
            return None;
        };
        let sig = self.lookup_any_type(type_name)?;
        let (kind, visibility, owner_name, owner_module) =
            self.declared_member_access(&sig, name, &mut HashSet::new())?;
        (!self.can_access_member_visibility(&owner_name, &owner_module, visibility))
            .then_some((kind, visibility, owner_name))
    }

    fn reject_inaccessible_member(
        &mut self,
        receiver: &Ty,
        name: &str,
        span: crate::source::Span,
    ) -> bool {
        let Some((kind, visibility, owner)) = self.inaccessible_member(receiver, name) else {
            return false;
        };
        let scope = match visibility {
            Visibility::Internal => "declaring module",
            Visibility::Private => "declaring type",
            Visibility::Default => return false,
        };
        self.add_error(
            "inaccessible_member",
            format!(
                "cannot access {visibility_name} {kind} '{name}' on '{owner}'; it is accessible only from the {scope}",
                visibility_name = visibility_label(visibility),
            ),
            span,
        );
        true
    }

    fn reject_inaccessible_static_member(
        &mut self,
        receiver: &Expr,
        name: &str,
        span: crate::source::Span,
    ) -> bool {
        let Expr::Identifier {
            name: type_name, ..
        } = receiver
        else {
            return false;
        };
        let Some(sig) = self.lookup_any_object(type_name) else {
            return false;
        };
        self.reject_inaccessible_member(&Ty::Named(sig.name, Vec::new()), name, span)
    }

    fn extension_this_private_field(&self, receiver: &Expr, receiver_ty: &Ty, name: &str) -> bool {
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
            .is_some_and(|field| field.visibility == Visibility::Private)
    }

    fn extension_this_private_method(&self, receiver: &Expr, receiver_ty: &Ty, name: &str) -> bool {
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
                    .any(|method| method.visibility == Visibility::Private)
            })
        })
    }

    fn check_extension_private_method_call(
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
        if !self.extension_this_private_method(receiver, &receiver_ty, name) {
            return false;
        }
        self.add_extension_private_access_error("method", name, *span);
        for arg in args {
            self.check_expr(&arg.value);
        }
        true
    }

    fn add_extension_private_access_error(
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
                "extension method cannot access private {member_kind} '{member_name}'; extension methods can access only visible members of '{target_name}'"
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
        if name == "equals"
            && !matches!(receiver, Ty::Unknown)
            && !self.is_equality_type(receiver, &mut HashSet::new())
        {
            return None;
        }
        if name == "hash" && self.is_hashable_type(receiver, &mut HashSet::new()) {
            return Some(Ty::Function(Vec::new(), Box::new(Ty::int())));
        }
        match receiver {
            Ty::Named(type_name, args) => {
                if let Some((owner_name, case_name)) = enum_case_view_parts(type_name) {
                    let owner = self.lookup_any_type(owner_name)?;
                    let subst = owner
                        .type_params
                        .iter()
                        .cloned()
                        .zip(args.iter().cloned())
                        .collect::<HashMap<_, _>>();
                    if let Some(field) = owner
                        .enum_cases
                        .get(case_name)
                        .and_then(|case| case.params.iter().find(|field| field.name == name))
                    {
                        return Some(substitute_type(&field.ty, &subst));
                    }
                    return self
                        .member_type(&Ty::Named(owner_name.to_string(), args.clone()), name);
                }
                let Some(sig) = self.lookup_any_type(type_name) else {
                    let extension_methods =
                        self.extension_method_sigs_for_named_type(type_name, args, name);
                    if let Some(first) = extension_methods.first() {
                        return Some(if first.getter {
                            first.ret.clone()
                        } else {
                            Ty::Function(
                                first.params.iter().map(|param| param.ty.clone()).collect(),
                                Box::new(first.ret.clone()),
                            )
                        });
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
                    return Some(if first.getter {
                        substitute_type(&first.ret, &subst)
                    } else {
                        Ty::Function(
                            first
                                .params
                                .iter()
                                .map(|param| substitute_type(&param.ty, &subst))
                                .collect(),
                            Box::new(substitute_type(&first.ret, &subst)),
                        )
                    });
                }
                let extension_methods =
                    self.extension_method_sigs_for_named_type(type_name, args, name);
                if let Some(first) = extension_methods.first() {
                    return Some(if first.getter {
                        first.ret.clone()
                    } else {
                        Ty::Function(
                            first.params.iter().map(|param| param.ty.clone()).collect(),
                            Box::new(first.ret.clone()),
                        )
                    });
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
                    if method.getter {
                        method.ret.clone()
                    } else {
                        Ty::Function(
                            method.params.iter().map(|param| param.ty.clone()).collect(),
                            Box::new(method.ret.clone()),
                        )
                    }
                })
                .or_else(|| universal_member_type(name)),
            Ty::Unknown => Some(Ty::Unknown),
            _ => universal_member_type(name),
        }
    }

    fn can_access_constructor(&self, owner: &TypeSig, visibility: Visibility) -> bool {
        self.can_access_member_visibility(&owner.name, &owner.module_id, visibility)
    }

    fn can_access_field(&self, owner: &TypeSig, field: &FieldSig) -> bool {
        self.can_access_member_visibility(&owner.name, &owner.module_id, field.visibility)
    }

    fn member_method_sigs(&self, receiver: &Ty, name: &str) -> Option<Vec<FunctionSig>> {
        if name == "equals"
            && !matches!(receiver, Ty::Unknown)
            && !self.is_equality_type(receiver, &mut HashSet::new())
        {
            return None;
        }
        if name == "hash" && self.is_hashable_type(receiver, &mut HashSet::new()) {
            return Some(vec![hash_method_sig()]);
        }
        match receiver {
            Ty::Named(type_name, args) => {
                let Some(sig) = self.lookup_any_type(type_name) else {
                    let extension_methods =
                        self.extension_method_sigs_for_named_type(type_name, args, name);
                    let extension_methods = extension_methods
                        .into_iter()
                        .filter(|method| !method.getter)
                        .collect::<Vec<_>>();
                    if extension_methods.is_empty() {
                        return universal_method_sigs(name);
                    }
                    return Some(extension_methods);
                };
                let Some(methods) = self.method_sigs_for_type(&sig, name) else {
                    let extension_methods =
                        self.extension_method_sigs_for_named_type(type_name, args, name);
                    let extension_methods = extension_methods
                        .into_iter()
                        .filter(|method| !method.getter)
                        .collect::<Vec<_>>();
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
                let methods = methods
                    .into_iter()
                    .filter(|method| !method.getter)
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
                        getter: method.getter,
                    })
                    .collect::<Vec<_>>();
                if methods.is_empty() {
                    let extension_methods = self
                        .extension_method_sigs_for_named_type(type_name, args, name)
                        .into_iter()
                        .filter(|method| !method.getter)
                        .collect::<Vec<_>>();
                    if extension_methods.is_empty() {
                        universal_method_sigs(name)
                    } else {
                        Some(extension_methods)
                    }
                } else {
                    Some(methods)
                }
            }
            Ty::Unknown => universal_method_sigs(name),
            Ty::TypeParam(param) => {
                let methods = self
                    .type_param_method_sigs(param, name)
                    .into_iter()
                    .filter(|method| !method.getter)
                    .collect::<Vec<_>>();
                if methods.is_empty() {
                    universal_method_sigs(name)
                } else {
                    Some(methods)
                }
            }
            _ => universal_method_sigs(name),
        }
    }

    fn member_getter_sig(&self, receiver: &Ty, name: &str) -> Option<FunctionSig> {
        match receiver {
            Ty::Named(type_name, args) => {
                if let Some((owner_name, _)) = enum_case_view_parts(type_name) {
                    return self
                        .member_getter_sig(&Ty::Named(owner_name.to_string(), args.clone()), name);
                }
                if let Some(sig) = self.lookup_any_type(type_name) {
                    if sig.fields.iter().any(|field| field.name == name) {
                        return None;
                    }
                    let subst = sig
                        .type_params
                        .iter()
                        .cloned()
                        .zip(args.iter().cloned())
                        .collect::<HashMap<_, _>>();
                    if let Some(getter) = self
                        .method_sigs_for_type(&sig, name)
                        .and_then(|methods| methods.into_iter().find(|method| method.getter))
                    {
                        return Some(instantiate_function_sig(getter, &subst));
                    }
                }
                self.extension_method_sigs_for_named_type(type_name, args, name)
                    .into_iter()
                    .find(|method| method.getter)
            }
            Ty::TypeParam(param) => self
                .type_param_method_sigs(param, name)
                .into_iter()
                .find(|method| method.getter),
            _ => None,
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
                getter: method.getter,
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
            return Some(
                methods
                    .iter()
                    .filter(|method| {
                        self.can_access_member_visibility(
                            &sig.name,
                            &sig.module_id,
                            method.visibility,
                        )
                    })
                    .cloned()
                    .collect(),
            );
        }
        for bound in &sig.with_bounds {
            let Ty::Named(bound_name, bound_args) = bound else {
                continue;
            };
            let Some(bound_sig) = self.lookup_any_type(bound_name) else {
                continue;
            };
            if let Some(methods) = self.method_sigs_for_type_inner(&bound_sig, name, seen) {
                let subst = bound_sig
                    .type_params
                    .iter()
                    .cloned()
                    .zip(bound_args.iter().cloned())
                    .collect();
                return Some(
                    methods
                        .into_iter()
                        .map(|method| instantiate_function_sig(method, &subst))
                        .collect(),
                );
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

    fn module_member_functions(&self, expr: &Expr) -> Option<Vec<FunctionSig>> {
        let (alias, member) = module_alias_and_member(expr)?;
        let module = self.world.lookup_module_alias(self.module, &alias)?;
        module.functions.get(&member).cloned()
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
        let owner = self.current_owner.as_ref()?;
        owner
            .fields
            .iter()
            .find(|field| field.name == name && self.can_access_field(owner, field))
            .cloned()
    }

    fn lookup_implicit_getter(&self, name: &str) -> Option<FunctionSig> {
        if self.lookup_scoped_value(name).is_some()
            || self.lookup_implicit_field(name).is_some()
            || self.lookup_global_value(name).is_some()
        {
            return None;
        }
        let owner = self.current_owner.as_ref()?;
        self.method_sigs_for_type(owner, name)?
            .into_iter()
            .find(|method| method.getter)
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
        if name == "getOr"
            && !matches!(receiver_ty, Ty::Unknown)
            && self.unwrap_known_lifted_type(receiver_ty).is_some()
        {
            return format!(
                "method 'getOr' was removed from '{}'; use '<value> ?? fallback' for lazy fallback extraction",
                receiver_ty.describe(),
            );
        }

        if name == "orPanic"
            && !matches!(receiver_ty, Ty::Unknown)
            && self.unwrap_known_lifted_type(receiver_ty).is_some()
        {
            return format!(
                "method 'orPanic' was removed from '{}'; use postfix '!' for unsafe extraction",
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

    fn check_typeof_type_ref(&mut self, ty_ref: &TypeRef, represented: &Ty) {
        if let Ty::Named(name, _) = represented
            && self
                .lookup_any_type(name)
                .is_some_and(|sig| sig.kind == TypeKind::Mixin)
        {
            self.add_error(
                "mixin_not_a_type",
                "mixins have no runtime type metadata",
                ty_ref.span(),
            );
            return;
        }
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

    fn choose_overload_with_locals<'b>(
        &self,
        overloads: &'b [FunctionSig],
        args: &[crate::ast::CallArg],
        locals: &HashMap<String, Ty>,
    ) -> Option<&'b FunctionSig> {
        let candidates = self.best_overload_candidates_with_locals(overloads, args, locals);
        (candidates.len() == 1).then(|| candidates[0])
    }

    fn best_overload_candidates<'b>(
        &self,
        overloads: &'b [FunctionSig],
        args: &[crate::ast::CallArg],
    ) -> Vec<&'b FunctionSig> {
        self.best_overload_candidates_with_locals(overloads, args, &HashMap::new())
    }

    fn best_overload_candidates_with_locals<'b>(
        &self,
        overloads: &'b [FunctionSig],
        args: &[crate::ast::CallArg],
        locals: &HashMap<String, Ty>,
    ) -> Vec<&'b FunctionSig> {
        let arg_types = args
            .iter()
            .map(|arg| self.probe_expr_type_with_locals(call_arg_value_expr(arg), locals))
            .collect::<Vec<_>>();
        let ranked = overloads
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
                                score += if actual == &expected { 3 } else { 2 };
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
            .collect::<Vec<_>>();
        let Some(best_score) = ranked.iter().map(|(score, _)| *score).max() else {
            return Vec::new();
        };
        let best = ranked
            .into_iter()
            .filter_map(|(score, sig)| (score == best_score).then_some(sig))
            .collect::<Vec<_>>();
        self.collapse_external_signature_duplicates(best)
    }

    fn collapse_external_signature_duplicates<'b>(
        &self,
        candidates: Vec<&'b FunctionSig>,
    ) -> Vec<&'b FunctionSig> {
        let mut collapsed: Vec<&'b FunctionSig> = Vec::new();
        for candidate in candidates {
            let Some(index) = collapsed.iter().position(|existing| {
                !existing.has_body
                    && !candidate.has_body
                    && existing.type_params == candidate.type_params
                    && existing.generic_conditions.is_empty()
                    && candidate.generic_conditions.is_empty()
                    && overload_inputs_are_equivalent(existing, candidate)
            }) else {
                collapsed.push(candidate);
                continue;
            };

            let existing = collapsed[index];
            if candidate.ret == existing.ret {
                continue;
            }
            let candidate_is_narrower = self.is_assignable(&candidate.ret, &existing.ret);
            let existing_is_narrower = self.is_assignable(&existing.ret, &candidate.ret);
            if candidate_is_narrower && !existing_is_narrower {
                collapsed[index] = candidate;
            } else if !existing_is_narrower {
                // Equal inputs with unrelated returns are genuinely ambiguous.
                collapsed.push(candidate);
            }
        }
        collapsed
    }

    fn closest_overload<'b>(
        &self,
        overloads: &'b [FunctionSig],
        args: &[crate::ast::CallArg],
    ) -> Option<&'b FunctionSig> {
        overloads.iter().min_by_key(|sig| {
            let arrangement = arrange_param_args(&sig.params, args);
            arrangement.overflow + arrangement.missing_required
        })
    }

    fn arg_matches_expected(&self, arg: &crate::ast::CallArg, actual: &Ty, expected: &Ty) -> bool {
        let _ = arg;
        self.is_assignable(actual, expected)
    }

    fn probe_expr_type(&self, expr: &Expr) -> Ty {
        self.probe_expr_type_with_locals(expr, &HashMap::new())
    }

    fn probe_expr_type_with_locals(&self, expr: &Expr, locals: &HashMap<String, Ty>) -> Ty {
        match expr {
            Expr::Identifier { name, .. } => self
                .probe_local_type(locals, name)
                .or_else(|| self.lookup_scoped_value(name).map(|value| value.ty))
                .or_else(|| self.lookup_implicit_field(name).map(|field| field.ty))
                .or_else(|| self.lookup_implicit_getter(name).map(|getter| getter.ret))
                .or_else(|| self.lookup_global_value(name).map(|value| value.ty))
                .or_else(|| self.lookup_function_type(name))
                .or_else(|| self.lookup_named_constructor_type(name))
                .unwrap_or(Ty::Unknown),
            Expr::Integer { .. } => Ty::int(),
            Expr::Float { .. } => Ty::float(),
            Expr::Decimal { .. } => Ty::named("Decimal"),
            Expr::String { .. } => Ty::str(),
            Expr::Bool { .. } => Ty::bool(),
            Expr::Unit { .. } => Ty::unit(),
            Expr::Spread { value, .. } => self.probe_expr_type_with_locals(value, locals),
            Expr::ListLiteral { items, .. } => {
                if items.is_empty() {
                    return Ty::Unknown;
                }
                if items.iter().all(|item| {
                    matches!(
                        item,
                        Expr::Binary {
                            op: BinaryOp::Colon,
                            ..
                        }
                    )
                }) {
                    let mut keys = Vec::new();
                    let mut values = Vec::new();
                    for item in items {
                        let Expr::Binary { left, right, .. } = item else {
                            unreachable!();
                        };
                        keys.push(self.probe_expr_type_with_locals(left, locals));
                        values.push(self.probe_expr_type_with_locals(right, locals));
                    }
                    return Ty::Named(
                        "Map".to_string(),
                        vec![join_many_types(&keys), join_many_types(&values)],
                    );
                }
                if items.iter().any(|item| {
                    matches!(
                        item,
                        Expr::Binary {
                            op: BinaryOp::Colon,
                            ..
                        }
                    )
                }) {
                    return Ty::Unknown;
                }
                let item_types = items
                    .iter()
                    .map(|item| match item {
                        Expr::Spread { value, .. } => {
                            let spread = self.probe_expr_type_with_locals(value, locals);
                            self.known_iterable_item_type(&spread)
                                .unwrap_or(Ty::Unknown)
                        }
                        _ => self.probe_expr_type_with_locals(item, locals),
                    })
                    .collect::<Vec<_>>();
                Ty::list(join_many_types(&item_types))
            }
            Expr::TupleLiteral { items, .. } => Ty::Tuple(
                items
                    .iter()
                    .map(|item| self.probe_expr_type_with_locals(item, locals))
                    .collect(),
            ),
            Expr::RecordLiteral { fields, .. } => {
                let mut out = Vec::new();
                for field in fields {
                    if let Some(name) = &field.name {
                        let ty = field
                            .ty
                            .as_ref()
                            .map(|ty| self.ty_from_type_ref(ty))
                            .unwrap_or_else(|| {
                                self.probe_expr_type_with_locals(&field.value, locals)
                            });
                        upsert_shape_field(&mut out, name.clone(), ty);
                    } else if let Ty::Record(spread_fields) =
                        self.probe_expr_type_with_locals(&field.value, locals)
                    {
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
                let receiver_ty = self.probe_expr_type_with_locals(receiver, locals);
                if name == "runtimeType" {
                    return Ty::value_runtime_type(receiver_ty);
                }
                if name == "referenceId" {
                    return self
                        .supports_reference_id(&receiver_ty)
                        .then(|| Ty::named("ReferenceId"))
                        .unwrap_or(Ty::Unknown);
                }
                self.member_type(&receiver_ty, name).unwrap_or(Ty::Unknown)
            }
            Expr::Index {
                receiver, index, ..
            } => {
                let receiver_ty = self.probe_expr_type_with_locals(receiver, locals);
                if let Ty::Tuple(items) = &receiver_ty
                    && let Some(index) = static_integer_literal(index)
                    && let Ok(index) = usize::try_from(index)
                {
                    return items.get(index).cloned().unwrap_or(Ty::Unknown);
                }
                self.index_result_type(&receiver_ty)
            }
            Expr::Call {
                callee,
                args,
                uses_brace_syntax,
                ..
            } => {
                if let Some(ty) = self.probe_named_constructor_result_type(callee) {
                    return ty;
                }
                let normalized_args =
                    self.normalize_trailing_brace_call_args(callee, args, *uses_brace_syntax);
                let normalized_args = self.expand_shape_call_arguments(&normalized_args);
                self.callable_signature_for_args_probe_with_locals(
                    callee,
                    &normalized_args,
                    *uses_brace_syntax,
                    locals,
                )
                .map(|(_, ret)| ret)
                .unwrap_or(Ty::Unknown)
            }
            Expr::ContextualNew { .. } | Expr::AnonymousObject { .. } => Ty::Unknown,
            Expr::Try { value, .. } => {
                self.unwrap_inner_type(&self.probe_expr_type_with_locals(value, locals))
            }
            Expr::ExtractOr {
                value, fallback, ..
            } => {
                let value =
                    self.unwrap_inner_type(&self.probe_expr_type_with_locals(value, locals));
                let fallback = self.probe_expr_type_with_locals(fallback, locals);
                join_types(&value, &fallback)
            }
            Expr::Unary { op, expr, .. } => {
                let inner = self.probe_expr_type_with_locals(expr, locals);
                match op {
                    crate::ast::UnaryOp::Neg if inner.is_numeric() => inner,
                    crate::ast::UnaryOp::Not => Ty::bool(),
                    crate::ast::UnaryOp::OptionWrap => Ty::option(inner),
                    crate::ast::UnaryOp::UnsafeExtract => self.unwrap_inner_type(&inner),
                    crate::ast::UnaryOp::Neg => Ty::Unknown,
                }
            }
            Expr::Binary {
                left, op, right, ..
            } => {
                let left = self.probe_expr_type_with_locals(left, locals);
                let right = self.probe_expr_type_with_locals(right, locals);
                self.probe_binary_expr_type(&left, *op, &right)
            }
            Expr::Is { .. } => Ty::bool(),
            Expr::TypeOf { ty, .. } => Ty::exact_runtime_type(self.ty_from_type_ref(ty)),
            Expr::If {
                condition_clauses,
                then_block,
                else_branch,
                ..
            } => {
                let mut then_locals = locals.clone();
                for clause in condition_clauses {
                    if let IfConditionClause::Let(clause) = clause {
                        let value_ty =
                            self.probe_expr_type_with_locals(&clause.value, &then_locals);
                        self.probe_pattern_bindings(&clause.pattern, &value_ty, &mut then_locals);
                    }
                }
                let then_ty = self.probe_block_result_type_with_locals(then_block, &then_locals);
                let else_ty = match else_branch.as_ref() {
                    ElseExprBranch::If(expr) => self.probe_expr_type_with_locals(expr, locals),
                    ElseExprBranch::Block(block) => {
                        self.probe_block_result_type_with_locals(block, locals)
                    }
                };
                join_types(&then_ty, &else_ty)
            }
            Expr::Block { body, .. } => self.probe_block_result_type_with_locals(body, locals),
            Expr::Match { value, cases, .. } => {
                let value_ty = self.probe_expr_type_with_locals(value, locals);
                self.probe_match_result_type(cases, &value_ty, locals)
            }
            Expr::Lambda { params, body, .. } => {
                let mut body_locals = locals.clone();
                let param_types = params
                    .iter()
                    .map(|param| {
                        let ty = param
                            .ty
                            .as_ref()
                            .map(|ty| self.ty_from_type_ref(ty))
                            .unwrap_or(Ty::Unknown);
                        body_locals.insert(param.name.clone(), ty.clone());
                        ty
                    })
                    .collect::<Vec<_>>();
                let ret = match body {
                    LambdaBody::Expr(expr) => self.probe_expr_type_with_locals(expr, &body_locals),
                    LambdaBody::Block(block) => {
                        self.probe_block_result_type_with_locals(block, &body_locals)
                    }
                };
                Ty::Function(param_types, Box::new(ret))
            }
            Expr::Group { inner, .. } => self.probe_expr_type_with_locals(inner, locals),
            Expr::Placeholder { .. }
            | Expr::Return { .. }
            | Expr::Break { .. }
            | Expr::Continue { .. }
            | Expr::ForYield { .. } => Ty::Unknown,
        }
    }

    fn probe_block_result_type_with_locals(
        &self,
        block: &Block,
        outer_locals: &HashMap<String, Ty>,
    ) -> Ty {
        let mut locals = outer_locals.clone();
        let mut result = Ty::unit();
        for statement in &block.statements {
            result = match statement {
                Stmt::Binding(binding) => {
                    self.probe_binding_stmt(binding, &mut locals);
                    Ty::unit()
                }
                Stmt::PatternBinding(binding) => {
                    if binding.clauses.is_empty() {
                        let value_ty = self.probe_expr_type_with_locals(&binding.value, &locals);
                        self.probe_pattern_bindings(&binding.pattern, &value_ty, &mut locals);
                    } else {
                        for clause in &binding.clauses {
                            let value_ty = self.probe_expr_type_with_locals(&clause.value, &locals);
                            self.probe_pattern_bindings(&clause.pattern, &value_ty, &mut locals);
                        }
                    }
                    Ty::unit()
                }
                Stmt::LetElse(binding) => {
                    if binding.clauses.is_empty() {
                        let value_ty = self.probe_expr_type_with_locals(&binding.value, &locals);
                        self.probe_pattern_bindings(&binding.pattern, &value_ty, &mut locals);
                    } else {
                        for clause in &binding.clauses {
                            let value_ty = self.probe_expr_type_with_locals(&clause.value, &locals);
                            self.probe_pattern_bindings(&clause.pattern, &value_ty, &mut locals);
                        }
                    }
                    Ty::unit()
                }
                Stmt::Expr(statement) => self.probe_expr_type_with_locals(&statement.expr, &locals),
                Stmt::If(statement) => self.probe_if_stmt_type(statement, &locals),
                Stmt::Match(statement) => {
                    let value_ty = self.probe_expr_type_with_locals(&statement.value, &locals);
                    self.probe_match_result_type(&statement.cases, &value_ty, &locals)
                }
                Stmt::LocalFunction(function) => {
                    let sig = function_sig_from_function(function, &[]);
                    locals.insert(function.name.clone(), function_sig_type(&sig));
                    Ty::unit()
                }
                _ => Ty::unit(),
            };
        }
        result
    }

    fn probe_local_type(&self, locals: &HashMap<String, Ty>, name: &str) -> Option<Ty> {
        locals.get(name).cloned()
    }

    fn probe_binding_stmt(&self, binding: &BindingStmt, locals: &mut HashMap<String, Ty>) {
        let value_types = binding
            .values
            .iter()
            .map(|value| self.probe_expr_type_with_locals(value, locals))
            .collect::<Vec<_>>();
        let inferred = match binding.destructure {
            Some(DestructureKind::Tuple) => self.destructure_slots(
                value_types.first().unwrap_or(&Ty::Unknown),
                binding.bindings.len(),
                DestructureKind::Tuple,
            ),
            Some(DestructureKind::Record) => {
                let fields = value_types
                    .first()
                    .and_then(|ty| self.destructure_record_fields(ty))
                    .unwrap_or_default();
                binding
                    .bindings
                    .iter()
                    .map(|binding| {
                        let field_name = binding
                            .field_name
                            .as_deref()
                            .unwrap_or(binding.name.as_str());
                        fields
                            .iter()
                            .find(|(name, _, _)| name == field_name)
                            .map(|(_, ty, _)| ty.clone())
                            .unwrap_or(Ty::Unknown)
                    })
                    .collect()
            }
            None => (0..binding.bindings.len())
                .map(|index| value_types.get(index).cloned().unwrap_or(Ty::Unknown))
                .collect(),
        };
        for (index, binding) in binding.bindings.iter().enumerate() {
            let ty = binding
                .ty
                .as_ref()
                .map(|ty| self.ty_from_type_ref(ty))
                .unwrap_or_else(|| inferred.get(index).cloned().unwrap_or(Ty::Unknown));
            if binding.name != "_" {
                locals.insert(binding.name.clone(), ty);
            }
        }
    }

    fn probe_if_stmt_type(&self, statement: &IfStmt, locals: &HashMap<String, Ty>) -> Ty {
        let mut then_locals = locals.clone();
        for clause in &statement.condition_clauses {
            if let IfConditionClause::Let(clause) = clause {
                let value_ty = self.probe_expr_type_with_locals(&clause.value, &then_locals);
                self.probe_pattern_bindings(&clause.pattern, &value_ty, &mut then_locals);
            }
        }
        let then_ty = self.probe_block_result_type_with_locals(&statement.then_block, &then_locals);
        let else_ty = match statement.else_branch.as_ref() {
            Some(ElseBranch::If(branch)) => self.probe_if_stmt_type(branch, locals),
            Some(ElseBranch::Block(block)) => {
                self.probe_block_result_type_with_locals(block, locals)
            }
            None => Ty::unit(),
        };
        join_types(&then_ty, &else_ty)
    }

    fn probe_match_result_type(
        &self,
        cases: &[MatchCase],
        value_ty: &Ty,
        locals: &HashMap<String, Ty>,
    ) -> Ty {
        let results = cases
            .iter()
            .map(|case| {
                let mut case_locals = locals.clone();
                self.probe_pattern_bindings(&case.pattern, value_ty, &mut case_locals);
                match &case.body {
                    MatchCaseBody::Block(block) => {
                        self.probe_block_result_type_with_locals(block, &case_locals)
                    }
                    MatchCaseBody::Expr(expr) => {
                        self.probe_expr_type_with_locals(expr, &case_locals)
                    }
                }
            })
            .collect::<Vec<_>>();
        join_many_types(&results)
    }

    fn probe_pattern_bindings(
        &self,
        pattern: &Pattern,
        scrutinee: &Ty,
        locals: &mut HashMap<String, Ty>,
    ) {
        match pattern {
            Pattern::Wildcard { .. } | Pattern::Literal { .. } => {}
            Pattern::Extract { inner, .. } => {
                let inner_ty = self.unwrap_inner_type(scrutinee);
                self.probe_pattern_bindings(inner, &inner_ty, locals);
            }
            Pattern::Alias { inner, name, .. } => {
                self.probe_pattern_bindings(inner, scrutinee, locals);
                if name != "_" {
                    locals.insert(
                        name.clone(),
                        self.whole_pattern_binding_type(inner, scrutinee),
                    );
                }
            }
            Pattern::Binding { name, .. } => {
                if name != "_" {
                    locals.insert(name.clone(), scrutinee.clone());
                }
            }
            Pattern::Type { name, target, .. } => {
                if let Some(name) = name
                    && name != "_"
                {
                    locals.insert(name.clone(), self.ty_from_type_ref(target));
                }
            }
            Pattern::Tuple { elements, .. } => {
                let items = match scrutinee {
                    Ty::Tuple(items) => items.clone(),
                    _ => vec![Ty::Unknown; elements.len()],
                };
                for (pattern, item) in elements.iter().zip(items.iter()) {
                    self.probe_pattern_bindings(pattern, item, locals);
                }
            }
            Pattern::List { elements, rest, .. } => {
                let item = self.list_element_type(scrutinee).unwrap_or(Ty::Unknown);
                for pattern in elements {
                    self.probe_pattern_bindings(pattern, &item, locals);
                }
                if let Some(rest) = rest
                    && rest.name != "_"
                {
                    locals.insert(rest.name.clone(), Ty::list(item));
                }
            }
            Pattern::Record { path, fields, .. } => {
                let target_fields = self
                    .lookup_record_pattern_target(path, scrutinee)
                    .map(|(_, _, fields)| fields)
                    .unwrap_or_default();
                for field in fields {
                    let field_ty = target_fields
                        .iter()
                        .find(|(name, _)| name == &field.name)
                        .map(|(_, ty)| ty.clone())
                        .unwrap_or(Ty::Unknown);
                    self.probe_pattern_bindings(&field.pattern, &field_ty, locals);
                }
            }
            Pattern::Constructor { path, args, .. } => {
                let fields = if let Some(case) = self.lookup_case_by_pattern(path, scrutinee) {
                    let mut subst = HashMap::new();
                    infer_type_subst(&case.result, scrutinee, &mut subst);
                    self.enum_case_extractable_params(&case)
                        .into_iter()
                        .map(|field| materialize_type(&substitute_type(&field.ty, &subst)))
                        .collect::<Vec<_>>()
                } else {
                    self.lookup_record_pattern_target(path, scrutinee)
                        .map(|(_, _, fields)| fields.into_iter().map(|(_, ty)| ty).collect())
                        .unwrap_or_default()
                };
                for (pattern, field_ty) in args.iter().zip(fields.iter()) {
                    self.probe_pattern_bindings(pattern, field_ty, locals);
                }
            }
        }
    }

    fn probe_binary_expr_type(&self, left: &Ty, op: BinaryOp, right: &Ty) -> Ty {
        if matches!(left, Ty::Unknown) || matches!(right, Ty::Unknown) {
            return Ty::Unknown;
        }
        match op {
            BinaryOp::Add if left.is_str() || right.is_str() => Ty::str(),
            BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Mod
                if left.is_decimal() && right.is_decimal() =>
            {
                Ty::named("Decimal")
            }
            BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Mod
                if left.is_decimal() || right.is_decimal() =>
            {
                Ty::Unknown
            }
            BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Mod
                if left.is_numeric() && right.is_numeric() =>
            {
                if left.is_float_like() || right.is_float_like() {
                    Ty::float()
                } else {
                    Ty::int()
                }
            }
            BinaryOp::Add => self
                .binary_operator_method_return_type(left, "+", right)
                .unwrap_or(Ty::Unknown),
            BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Mod => Ty::Unknown,
            BinaryOp::Eq
            | BinaryOp::NotEq
            | BinaryOp::Less
            | BinaryOp::LessEq
            | BinaryOp::Greater
            | BinaryOp::GreaterEq
            | BinaryOp::And
            | BinaryOp::Or => Ty::bool(),
            BinaryOp::Colon => Ty::Unknown,
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
        if sig.kind == TypeKind::Class
            && sig
                .fields
                .iter()
                .any(|field| field.visibility != Visibility::Default && !field.has_initializer)
        {
            return materialize_type(ret);
        }

        let constructor_fields = sig
            .fields
            .iter()
            .filter(|field| {
                if sig.kind == TypeKind::Class {
                    field.visibility == Visibility::Default
                } else {
                    self.can_access_field(sig, field)
                }
            })
            .collect::<Vec<_>>();

        for arg in fields.iter().filter(|arg| arg.name.is_some()) {
            let name = arg.name.as_deref().expect("filtered named field");
            if !constructor_fields.iter().any(|field| field.name == name) {
                self.add_error(
                    "no_matching_overload",
                    format!(
                        "{} '{}' has no public constructor field '{}'",
                        type_kind_label(sig.kind),
                        sig.name,
                        name
                    ),
                    arg.span,
                );
                return materialize_type(ret);
            }
        }

        let expected_record = Ty::Record(
            constructor_fields
                .iter()
                .map(|field| (field.name.clone(), field.ty.clone()))
                .collect(),
        );
        let actual = self.check_expr_against(expr, &expected_record);
        let actual_fields = self.structural_fields_for_type(&actual).unwrap_or_default();

        for field in &constructor_fields {
            let Some((_, actual_ty)) = actual_fields.iter().find(|(name, _)| name == &field.name)
            else {
                if !field.has_initializer {
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
                }
                continue;
            };
            self.require_assignable(
                actual_ty,
                &field.ty,
                span,
                "invalid_argument_type",
                format!(
                    "field '{}' in {} '{}' expects '{}' but got '{}'",
                    field.name,
                    type_kind_label(sig.kind),
                    sig.name,
                    field.ty.describe(),
                    actual_ty.describe()
                ),
            );
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
        if sig.kind == TypeKind::Class
            && sig
                .fields
                .iter()
                .any(|field| field.visibility != Visibility::Default && !field.has_initializer)
        {
            return materialize_type(ret);
        }

        let params = sig
            .fields
            .iter()
            .filter(|field| {
                if sig.kind == TypeKind::Class {
                    field.visibility == Visibility::Default
                } else {
                    self.can_access_field(sig, field)
                }
            })
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
        let methods = self
            .method_sigs_for_type(owner, name)?
            .into_iter()
            .filter(|method| !method.getter)
            .collect::<Vec<_>>();
        (!methods.is_empty()).then_some(methods)
    }

    fn select_callable_reference_type(
        &mut self,
        name: &str,
        overloads: &[FunctionSig],
        expected: &Ty,
        span: crate::source::Span,
    ) -> Ty {
        let candidates = overloads
            .iter()
            .filter(|sig| !sig.getter)
            .map(function_sig_type)
            .collect::<Vec<_>>();

        if candidates.is_empty() {
            return Ty::Unknown;
        }
        if candidates.len() == 1 {
            return candidates[0].clone();
        }

        if matches!(expected, Ty::Function(_, _)) {
            let exact = candidates
                .iter()
                .filter(|candidate| *candidate == expected)
                .cloned()
                .collect::<Vec<_>>();
            if exact.len() == 1 {
                return exact[0].clone();
            }
            if exact.len() > 1 {
                self.add_ambiguous_callable_reference_error(name, expected, &exact, span);
                return Ty::Unknown;
            }

            let compatible = candidates
                .iter()
                .filter(|candidate| self.is_assignable(candidate, expected))
                .cloned()
                .collect::<Vec<_>>();
            if compatible.len() == 1 {
                return compatible[0].clone();
            }
            if compatible.len() > 1 {
                self.add_ambiguous_callable_reference_error(name, expected, &compatible, span);
                return Ty::Unknown;
            }

            // Preserve the normal assignment diagnostic when no overload can
            // satisfy the requested callable contract.
            return candidates[0].clone();
        }

        self.add_ambiguous_callable_reference_error(name, expected, &candidates, span);
        Ty::Unknown
    }

    fn add_ambiguous_callable_reference_error(
        &mut self,
        name: &str,
        expected: &Ty,
        candidates: &[Ty],
        span: crate::source::Span,
    ) {
        let signatures = candidates
            .iter()
            .map(Ty::describe)
            .collect::<Vec<_>>()
            .join(", ");
        let message = if matches!(expected, Ty::Function(_, _)) {
            format!(
                "callable reference '{name}' is ambiguous for expected type '{}'; matching overloads: {signatures}",
                expected.describe()
            )
        } else {
            format!(
                "callable reference '{name}' is overloaded; add a function type annotation to select one overload: {signatures}"
            )
        };
        self.add_error("ambiguous_callable_reference", message, span);
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

    fn report_ambiguous_union_alternative(
        &mut self,
        name: &str,
        span: crate::source::Span,
    ) -> bool {
        let owners = self.world.ambiguous_enum_case_owners(self.module, name);
        if owners.is_empty() {
            return false;
        }
        let choices = owners
            .iter()
            .map(|owner| format!("{owner}.{name}"))
            .collect::<Vec<_>>()
            .join(", ");
        self.add_error(
            "ambiguous_union_alternative",
            format!("union alternative '{name}' is ambiguous: {choices}; use a qualified name"),
            span,
        );
        true
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
            .or_else(|| self.lookup_lexical_type(name))
    }

    fn lookup_object_local(&self, name: &str) -> Option<TypeSig> {
        self.module
            .objects
            .get(name)
            .cloned()
            .or_else(|| self.lookup_lexical_object(name))
    }

    fn lookup_lexical_type(&self, name: &str) -> Option<TypeSig> {
        if name.contains('.') {
            return None;
        }
        let owner = self
            .current_type_paths
            .last()
            .or_else(|| self.current_owner.as_ref().map(|owner| &owner.name))?;
        lexical_nested_name(owner, name, |candidate| {
            self.module.types.contains_key(candidate) || self.module.objects.contains_key(candidate)
        })
        .and_then(|candidate| {
            self.module
                .types
                .get(&candidate)
                .cloned()
                .or_else(|| self.module.objects.get(&candidate).cloned())
        })
    }

    fn lookup_lexical_object(&self, name: &str) -> Option<TypeSig> {
        if name.contains('.') {
            return None;
        }
        let owner = self
            .current_type_paths
            .last()
            .or_else(|| self.current_owner.as_ref().map(|owner| &owner.name))?;
        lexical_nested_name(owner, name, |candidate| {
            self.module.objects.contains_key(candidate)
        })
        .and_then(|candidate| self.module.objects.get(&candidate).cloned())
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

    fn lookup_alias_target(
        &self,
        module: Option<&ModuleInfo>,
        name: &str,
    ) -> Option<(PathBuf, String, TypeRef)> {
        if let Some(module) = module {
            if let Some(target) = module.aliases.get(name) {
                return Some((module.path.clone(), name.to_string(), target.clone()));
            }
            if module.path == self.module.path
                && !name.contains('.')
                && let Some(owner) = self
                    .current_type_paths
                    .last()
                    .or_else(|| self.current_owner.as_ref().map(|owner| &owner.name))
                && let Some(canonical) = lexical_nested_name(owner, name, |candidate| {
                    module.aliases.contains_key(candidate)
                })
                && let Some(target) = module.aliases.get(&canonical)
            {
                return Some((module.path.clone(), canonical, target.clone()));
            }
            if let Some(imported) = self.world.lookup_imported_alias(module, name) {
                return Some(imported);
            }
        }
        self.world
            .ambient
            .aliases
            .get(name)
            .cloned()
            .map(|target| (PathBuf::from("<ambient>"), name.to_string(), target))
    }

    fn module_for_alias_path(&self, path: &Path) -> Option<&ModuleInfo> {
        if path == self.module.path {
            Some(self.module)
        } else {
            self.world.modules.get(path)
        }
    }

    fn resolve_named_type_in_module(
        &self,
        module: Option<&ModuleInfo>,
        name: &str,
        args: Vec<Ty>,
    ) -> Ty {
        if module.is_some_and(|module| module.path == self.module.path) {
            return self.resolve_named_type(name, args);
        }
        if name == "Never" && args.is_empty() {
            return Ty::Never;
        }
        let sig = module
            .and_then(|module| {
                module
                    .types
                    .get(name)
                    .or_else(|| module.objects.get(name))
                    .cloned()
                    .or_else(|| self.world.lookup_imported_type(module, name))
            })
            .or_else(|| self.world.ambient.types.get(name).cloned())
            .or_else(|| self.world.ambient.objects.get(name).cloned());
        match sig {
            Some(sig) => Ty::Named(sig.name, args),
            None => Ty::Named(name.to_string(), args),
        }
    }

    fn qualified_type_context<'m, 'n>(
        &'m self,
        module: Option<&'m ModuleInfo>,
        name: &'n str,
    ) -> (Option<&'m ModuleInfo>, &'n str) {
        let Some((module_alias, member)) = name.split_once('.') else {
            return (module, name);
        };
        let Some(owner) =
            module.and_then(|module| self.world.lookup_module_alias(module, module_alias))
        else {
            return (module, name);
        };
        (Some(owner), member)
    }

    fn ty_from_type_ref_in_module(
        &self,
        reference: &TypeRef,
        module: Option<&ModuleInfo>,
        visiting: &mut Vec<(PathBuf, String)>,
    ) -> Ty {
        match reference {
            TypeRef::Wildcard { .. } => Ty::Wildcard,
            TypeRef::Named { name, args, .. } => {
                let (type_module, type_name) = self.qualified_type_context(module, name);
                if args.is_empty() {
                    if let Some((path, alias_name, target)) =
                        self.lookup_alias_target(type_module, type_name)
                    {
                        let key = (path.clone(), alias_name);
                        if visiting.contains(&key) {
                            return Ty::Unknown;
                        }
                        visiting.push(key);
                        let owner = self.module_for_alias_path(&path);
                        let expanded = self.ty_from_type_ref_in_module(&target, owner, visiting);
                        visiting.pop();
                        return expanded;
                    }
                }
                let args = args
                    .iter()
                    .map(|arg| self.ty_from_type_ref_in_module(arg, module, visiting))
                    .collect();
                self.resolve_named_type_in_module(type_module, type_name, args)
            }
            TypeRef::Tuple { fields, .. } => Ty::Tuple(
                fields
                    .iter()
                    .map(|field| self.ty_from_type_ref_in_module(&field.ty, module, visiting))
                    .collect(),
            ),
            TypeRef::Record { fields, .. } => Ty::Record(
                fields
                    .iter()
                    .map(|field| {
                        (
                            field.name.clone(),
                            self.ty_from_type_ref_in_module(&field.ty, module, visiting),
                        )
                    })
                    .collect(),
            ),
            TypeRef::Function { params, ret, .. } => Ty::Function(
                params
                    .iter()
                    .map(|param| self.ty_from_type_ref_in_module(param, module, visiting))
                    .collect(),
                Box::new(self.ty_from_type_ref_in_module(ret, module, visiting)),
            ),
            TypeRef::Union { members, .. } => Ty::union(
                members
                    .iter()
                    .map(|member| self.ty_from_type_ref_in_module(member, module, visiting)),
            ),
        }
    }

    fn type_ref_has_alias_cycle(
        &self,
        reference: &TypeRef,
        module: &ModuleInfo,
        visiting: &mut Vec<(PathBuf, String)>,
    ) -> bool {
        match reference {
            TypeRef::Named { name, args, .. } => {
                if args
                    .iter()
                    .any(|arg| self.type_ref_has_alias_cycle(arg, module, visiting))
                {
                    return true;
                }
                let (type_module, type_name) = self.qualified_type_context(Some(module), name);
                let Some((path, alias_name, target)) =
                    self.lookup_alias_target(type_module, type_name)
                else {
                    return false;
                };
                let key = (path.clone(), alias_name);
                if visiting.contains(&key) {
                    return true;
                }
                let Some(owner) = self.module_for_alias_path(&path) else {
                    return false;
                };
                visiting.push(key);
                let cyclic = self.type_ref_has_alias_cycle(&target, owner, visiting);
                visiting.pop();
                cyclic
            }
            TypeRef::Tuple { fields, .. } => fields
                .iter()
                .any(|field| self.type_ref_has_alias_cycle(&field.ty, module, visiting)),
            TypeRef::Record { fields, .. } => fields
                .iter()
                .any(|field| self.type_ref_has_alias_cycle(&field.ty, module, visiting)),
            TypeRef::Function { params, ret, .. } => {
                params
                    .iter()
                    .any(|param| self.type_ref_has_alias_cycle(param, module, visiting))
                    || self.type_ref_has_alias_cycle(ret, module, visiting)
            }
            TypeRef::Union { members, .. } => members
                .iter()
                .any(|member| self.type_ref_has_alias_cycle(member, module, visiting)),
            TypeRef::Wildcard { .. } => false,
        }
    }

    fn ty_from_type_ref(&self, reference: &TypeRef) -> Ty {
        self.ty_from_type_ref_in_module(reference, Some(self.module), &mut Vec::new())
    }

    fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
        self.mutable_bindings.push(HashMap::new());
    }

    fn pop_scope(&mut self) {
        if let Some(bindings) = self.mutable_bindings.pop() {
            let mut bindings = bindings.into_values().collect::<Vec<_>>();
            bindings.sort_by_key(|binding| binding.span.start);
            for binding in bindings {
                if !binding.reassigned {
                    self.warnings.push(
                        Diagnostic::warning(
                            "unused_mutability",
                            format!(
                                "binding '{}' is declared mutable but never reassigned",
                                binding.name
                            ),
                            binding.span,
                        )
                        .with_label("this binding does not need 'var'")
                        .with_help(format!(
                            "remove 'var' and declare it as '{} = ...'",
                            binding.name
                        )),
                    );
                }
            }
        }
        self.scopes.pop();
    }

    fn track_mutable_binding(&mut self, binding: &crate::ast::Binding) {
        if binding.name == "_" {
            return;
        }
        if self.mutable_bindings.is_empty() {
            self.push_scope();
        }
        if let Some(scope) = self.mutable_bindings.last_mut() {
            scope.insert(
                binding.name.clone(),
                MutableBindingUsage {
                    name: binding.name.clone(),
                    span: binding.span,
                    reassigned: false,
                },
            );
        }
    }

    fn mark_local_reassigned(&mut self, name: &str) {
        for scope in self.mutable_bindings.iter_mut().rev() {
            if let Some(binding) = scope.get_mut(name) {
                binding.reassigned = true;
                return;
            }
        }
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
        if is_builtin_primitive_type_name(&name) {
            self.diagnostics
                .push(primitive_implementation_diagnostic(&name, span));
            return;
        }
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
                if sig.kind == TypeKind::Mixin {
                    self.add_error("mixin_not_a_type", format!("mixin '{}' cannot be used as a value's type; use an interface for the contract", sig.name), *span);
                    return;
                }
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
            TypeRef::Union { members, .. } => {
                for member in members {
                    self.validate_type_ref_generic_applications(member);
                }
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
        if let Some(targets) = self.ambiguous_shape_union_projection_targets(actual, expected) {
            let targets = targets
                .iter()
                .map(|target| format!("'{}'", target.describe()))
                .collect::<Vec<_>>()
                .join(", ");
            self.add_error(
                "ambiguous_shape_union_projection",
                format!(
                    "value of type '{}' can project to multiple members of union '{}': {targets}; construct the intended shape explicitly, for example 'Target {{ ...value }}'",
                    actual.describe(),
                    expected.describe(),
                ),
                span,
            );
            return;
        }
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

    fn check_integer_literal(&mut self, raw: &str, negative: bool, span: crate::source::Span) {
        if parse_integer_literal(raw, negative).is_some() {
            return;
        }
        let spelling = if negative {
            format!("-{raw}")
        } else {
            raw.to_string()
        };
        self.add_error(
            "integer_literal_out_of_range",
            format!("integer literal '{spelling}' is outside the signed 64-bit Int range"),
            span,
        );
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
                        .filter(|field| self.can_access_field(&sig, field))
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
                        visibility: Visibility::Default,
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
                        .filter(|field| self.can_access_field(&sig, field))
                        .map(|field| FieldSig {
                            name: field.name.clone(),
                            ty: substitute_type(&field.ty, &subst),
                            mutable: field.mutable,
                            visibility: field.visibility,
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

        if matches!(actual, Ty::Named(name, _) if self
            .lookup_any_type(name)
            .is_some_and(|sig| sig.kind == TypeKind::Class))
        {
            return false;
        }

        let Some(actual_fields) = self.structural_fields_for_type(actual) else {
            return false;
        };

        expected_fields.iter().all(|expected| {
            actual_fields
                .iter()
                .find(|(actual_name, _)| actual_name == &expected.name)
                .map(|(_, actual_ty)| self.is_assignable(actual_ty, &expected.ty))
                .unwrap_or(false)
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

    fn shape_union_projection_candidates(&self, actual: &Ty, members: &[Ty]) -> Vec<Ty> {
        members
            .iter()
            .filter(|member| self.structurally_assignable_to_shape(actual, member))
            .cloned()
            .collect()
    }

    fn union_accepts_without_shape_projection(&self, actual: &Ty, members: &[Ty]) -> bool {
        if matches!(actual, Ty::Unknown | Ty::Never)
            || members.iter().any(|member| member == actual)
        {
            return true;
        }
        members.iter().any(|member| {
            self.shape_target_fields(member).is_none() && self.is_assignable(actual, member)
        })
    }

    fn shape_union_projection_targets(&self, actual: &Ty, members: &[Ty]) -> Vec<Ty> {
        if self.union_accepts_without_shape_projection(actual, members) {
            return Vec::new();
        }
        let candidates = self.shape_union_projection_candidates(actual, members);
        let exact = candidates
            .iter()
            .filter(|member| self.shape_fields_match_exactly(actual, member))
            .cloned()
            .collect::<Vec<_>>();
        if exact.is_empty() { candidates } else { exact }
    }

    fn ambiguous_shape_union_projection_targets(
        &self,
        actual: &Ty,
        expected: &Ty,
    ) -> Option<Vec<Ty>> {
        let Ty::Union(members) = expected else {
            return None;
        };
        match actual {
            Ty::Union(actual_members) => actual_members.iter().find_map(|actual_member| {
                self.ambiguous_shape_union_projection_targets(actual_member, expected)
            }),
            actual => {
                let targets = self.shape_union_projection_targets(actual, members);
                (targets.len() > 1).then_some(targets)
            }
        }
    }

    fn is_assignable_to_union(
        &self,
        actual: &Ty,
        members: &[Ty],
        seen: &mut HashSet<(String, String)>,
    ) -> bool {
        if let Ty::Union(actual_members) = actual {
            return actual_members
                .iter()
                .all(|member| self.is_assignable_to_union(member, members, seen));
        }
        if self.union_accepts_without_shape_projection(actual, members) {
            return true;
        }
        let targets = self.shape_union_projection_targets(actual, members);
        if targets.len() == 1 {
            return true;
        }

        // Preserve ordinary non-structural assignability for special and
        // constrained types that cannot be classified as shape projections.
        members.iter().any(|member| {
            let mut branch_seen = seen.clone();
            self.is_assignable_inner(actual, member, &mut branch_seen)
                && self.shape_target_fields(member).is_none()
        })
    }

    fn implicitly_satisfies_value_bound(&self, actual: &Ty, expected: &Ty) -> bool {
        match expected {
            Ty::Named(name, args) if name == "Eq" && args.len() == 1 => {
                (self.generic_types_are_equal(actual, &args[0])
                    || self.shape_fields_match_exactly(actual, &args[0]))
                    && self.is_equality_type(actual, &mut HashSet::new())
            }
            Ty::Named(name, args) if name == "Hashed" && args.len() == 1 => {
                (self.generic_types_are_equal(actual, &args[0])
                    || self.shape_fields_match_exactly(actual, &args[0]))
                    && self.is_hashable_type(actual, &mut HashSet::new())
            }
            _ => false,
        }
    }

    fn is_equality_type(&self, ty: &Ty, seen: &mut HashSet<String>) -> bool {
        match ty {
            Ty::Named(name, args)
                if args.is_empty()
                    && matches!(
                        name.as_str(),
                        "Bool" | "Float" | "Decimal" | "Int" | "Rune" | "Str" | "Unit"
                    ) =>
            {
                true
            }
            Ty::Record(fields) => fields
                .iter()
                .all(|(_, field_ty)| self.is_equality_type(field_ty, seen)),
            Ty::Tuple(items) | Ty::Union(items) => {
                items.iter().all(|item| self.is_equality_type(item, seen))
            }
            Ty::Named(name, args) => {
                if let Some((owner_name, _)) = enum_case_view_parts(name) {
                    return self
                        .lookup_any_type(owner_name)
                        .is_some_and(|sig| sig.kind == TypeKind::Enum);
                }
                let Some(sig) = self.lookup_any_type(name) else {
                    return false;
                };
                match sig.kind {
                    TypeKind::Record => {
                        if self.type_sig_has_equality_bound(&sig, ty, seen) {
                            return true;
                        }
                        let key = format!("shape-equality:{}", ty.describe());
                        if !seen.insert(key.clone()) {
                            return true;
                        }
                        let subst = sig
                            .type_params
                            .iter()
                            .cloned()
                            .zip(args.iter().cloned())
                            .collect::<HashMap<_, _>>();
                        let derives = sig.fields.iter().all(|field| {
                            field.visibility == Visibility::Default
                                && self.is_equality_type(&substitute_type(&field.ty, &subst), seen)
                        });
                        seen.remove(&key);
                        derives
                    }
                    TypeKind::Enum | TypeKind::Object => true,
                    TypeKind::Class | TypeKind::Interface => {
                        self.type_sig_has_equality_bound(&sig, ty, seen)
                    }
                    TypeKind::Annotation | TypeKind::Mixin => false,
                }
            }
            Ty::TypeParam(name) => self.type_param_bounds(name).iter().any(|bound| {
                matches!(
                    bound,
                    Ty::Named(bound_name, args)
                        if matches!(bound_name.as_str(), "Eq" | "Hashed")
                            && args.len() == 1
                            && matches!(&args[0], Ty::TypeParam(bound_param) if bound_param == name)
                )
            }),
            Ty::Unknown | Ty::Wildcard | Ty::Capture(_) | Ty::Never | Ty::Function(_, _) => false,
        }
    }

    fn is_hashable_type(&self, ty: &Ty, seen: &mut HashSet<String>) -> bool {
        match ty {
            Ty::Named(name, args)
                if args.is_empty()
                    && matches!(
                        name.as_str(),
                        "Bool" | "Float" | "Decimal" | "Int" | "Rune" | "Str" | "Unit"
                    ) =>
            {
                true
            }
            Ty::Record(fields) => fields
                .iter()
                .all(|(_, field_ty)| self.is_hashable_type(field_ty, seen)),
            Ty::Tuple(items) | Ty::Union(items) => {
                items.iter().all(|item| self.is_hashable_type(item, seen))
            }
            Ty::Named(name, args) => {
                if let Some((owner_name, case_name)) = enum_case_view_parts(name) {
                    let Some(sig) = self.lookup_any_type(owner_name) else {
                        return false;
                    };
                    return self.enum_sig_is_hashable(&sig, args, Some(case_name), seen);
                }
                let Some(sig) = self.lookup_any_type(name) else {
                    return false;
                };
                match sig.kind {
                    TypeKind::Enum => self.enum_sig_is_hashable(&sig, args, None, seen),
                    TypeKind::Object => true,
                    TypeKind::Record => {
                        if self.type_sig_has_hashed_bound(&sig, ty, seen) {
                            return true;
                        }
                        if self.type_sig_has_equality_bound(&sig, ty, seen) {
                            return false;
                        }
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
                            field.visibility == Visibility::Default
                                && self.is_hashable_type(&substitute_type(&field.ty, &subst), seen)
                        });
                        seen.remove(&key);
                        hashable
                    }
                    TypeKind::Class => self.type_sig_has_hashed_bound(&sig, ty, seen),
                    TypeKind::Annotation | TypeKind::Interface | TypeKind::Mixin => false,
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
            Ty::Unknown | Ty::Wildcard | Ty::Capture(_) | Ty::Never | Ty::Function(_, _) => false,
        }
    }

    fn enum_sig_is_hashable(
        &self,
        sig: &TypeSig,
        args: &[Ty],
        case_name: Option<&str>,
        seen: &mut HashSet<String>,
    ) -> bool {
        let key = format!(
            "union:{}<{}>:{}",
            sig.name,
            args.iter().map(Ty::describe).collect::<Vec<_>>().join(","),
            case_name.unwrap_or("*")
        );
        if !seen.insert(key.clone()) {
            return true;
        }
        let subst = sig
            .type_params
            .iter()
            .cloned()
            .zip(args.iter().cloned())
            .collect::<HashMap<_, _>>();
        let shared_fields_hashable = sig
            .fields
            .iter()
            .all(|field| self.is_hashable_type(&substitute_type(&field.ty, &subst), seen));
        let cases_hashable = match case_name {
            Some(case_name) => sig.enum_cases.get(case_name).is_some_and(|case| {
                case.params
                    .iter()
                    .all(|field| self.is_hashable_type(&substitute_type(&field.ty, &subst), seen))
            }),
            None => sig.enum_cases.values().all(|case| {
                case.params
                    .iter()
                    .all(|field| self.is_hashable_type(&substitute_type(&field.ty, &subst), seen))
            }),
        };
        seen.remove(&key);
        shared_fields_hashable && cases_hashable
    }

    fn type_sig_has_equality_bound(
        &self,
        sig: &TypeSig,
        actual: &Ty,
        seen: &mut HashSet<String>,
    ) -> bool {
        let key = format!("equality:{}:{}", sig.name, actual.describe());
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
            if matches!(name.as_str(), "Eq" | "Hashed") {
                return args.len() == 1 && self.generic_types_are_equal(&args[0], actual);
            }
            self.lookup_any_type(name)
                .is_some_and(|parent| self.type_sig_has_equality_bound(&parent, &bound, seen))
        });
        seen.remove(&key);
        found
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
        if let (
            Ty::Function(actual_params, actual_ret),
            Ty::Function(expected_params, expected_ret),
        ) = (actual, expected)
        {
            return actual_params.len() == expected_params.len()
                && actual_params
                    .iter()
                    .zip(expected_params)
                    .all(|(actual, expected)| self.is_assignable_inner(expected, actual, seen))
                && self.is_assignable_inner(actual_ret, expected_ret, seen);
        }
        if let Ty::Union(expected_members) = expected {
            return self.is_assignable_to_union(actual, expected_members, seen);
        }
        if let Ty::Union(actual_members) = actual {
            return actual_members
                .iter()
                .all(|actual_member| self.is_assignable(actual_member, expected));
        }
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

fn bare_payload_pattern_diagnostic(case_name: &str, field_names: &[String]) -> String {
    if field_names.len() == 1 {
        return format!(
            "bare case pattern '{case_name}' has a payload; use '{case_name} {{ {} }}' to unpack it or '{case_name} {{ {}: _ }}' to ignore it",
            field_names[0], field_names[0]
        );
    }
    format!(
        "bare case pattern '{case_name}' has {} payload fields; use '{case_name} {{ {} }}' for named field matching",
        field_names.len(),
        field_names.join(", ")
    )
}

fn bare_type_pattern_diagnostic(type_name: &str) -> String {
    format!(
        "bare type pattern '{type_name}' is not supported; use '_ {type_name}' to ignore the matched value, 'value {type_name}' to bind it, or '{type_name} {{ field }}' to extract fields"
    )
}

fn pattern_without_alias(mut pattern: &Pattern) -> &Pattern {
    while let Pattern::Alias { inner, .. } = pattern {
        pattern = inner;
    }
    pattern
}

fn overlapping_whole_pattern_alias(pattern: &Pattern) -> Option<String> {
    match pattern {
        Pattern::Alias { inner, name, .. } => {
            if name != "_" && !crate::ast::pattern_binding_names(inner).is_empty() {
                return Some(name.clone());
            }
            overlapping_whole_pattern_alias(inner)
        }
        Pattern::Extract { inner, .. } => overlapping_whole_pattern_alias(inner),
        Pattern::Tuple { elements, .. } | Pattern::List { elements, .. } => {
            elements.iter().find_map(overlapping_whole_pattern_alias)
        }
        Pattern::Record { fields, .. } => fields
            .iter()
            .find_map(|field| overlapping_whole_pattern_alias(&field.pattern)),
        Pattern::Constructor { args, .. } => args.iter().find_map(overlapping_whole_pattern_alias),
        Pattern::Wildcard { .. }
        | Pattern::Binding { .. }
        | Pattern::Type { .. }
        | Pattern::Literal { .. } => None,
    }
}

fn constructor_field_sigs_from_params(params: &[ParamSig]) -> Vec<FieldSig> {
    params
        .iter()
        .map(|param| FieldSig {
            name: param.name.clone(),
            ty: param.ty.clone(),
            mutable: false,
            visibility: Visibility::Default,
            has_initializer: param.has_initializer,
            variadic: param.variadic,
        })
        .collect()
}

fn function_sig_type(sig: &FunctionSig) -> Ty {
    Ty::Function(
        sig.params.iter().map(|param| param.ty.clone()).collect(),
        Box::new(sig.ret.clone()),
    )
}

fn overload_inputs_are_equivalent(left: &FunctionSig, right: &FunctionSig) -> bool {
    left.params.len() == right.params.len()
        && left.params.iter().zip(&right.params).all(|(left, right)| {
            left.ty == right.ty
                && left.variadic == right.variadic
                && left.lazy == right.lazy
                && left.has_initializer == right.has_initializer
        })
}

fn constructor_target_name(ret: &Ty) -> Option<&str> {
    match ret {
        Ty::Named(name, _) => Some(name.as_str()),
        _ => None,
    }
}

struct PositionalPrefixGap {
    supplied: Option<String>,
    required: String,
    defaulted: String,
}

fn positional_prefix_gap<'a>(
    inputs: impl IntoIterator<Item = (&'a str, bool, bool)>,
    arg_count: usize,
) -> Option<PositionalPrefixGap> {
    let inputs = inputs.into_iter().collect::<Vec<_>>();
    if arg_count > inputs.len() {
        return None;
    }
    let (required_index, required) = inputs
        .iter()
        .enumerate()
        .skip(arg_count)
        .find(|(_, (_, has_default, variadic))| !*has_default && !*variadic)?;
    let defaulted = inputs[..required_index]
        .iter()
        .find(|(_, has_default, _)| *has_default)?;
    Some(PositionalPrefixGap {
        supplied: arg_count
            .checked_sub(1)
            .and_then(|index| inputs.get(index))
            .map(|(name, _, _)| (*name).to_string()),
        required: required.0.to_string(),
        defaulted: defaulted.0.to_string(),
    })
}

fn positional_constructor_prefix_message(
    target_name: &str,
    params: &[FieldSig],
    arg_count: usize,
) -> Option<String> {
    let gap = positional_prefix_gap(
        params
            .iter()
            .map(|param| (param.name.as_str(), param.has_initializer, param.variadic)),
        arg_count,
    )?;
    let supplied = gap
        .supplied
        .as_deref()
        .map(|name| format!("the last supplied value initializes construction input '{name}'; "))
        .unwrap_or_else(|| "no construction inputs are supplied; ".to_string());
    Some(format!(
        "positional construction for {target_name} with {arg_count} {} binds a declaration-order prefix: {supplied}required input '{}' remains unset because positional arguments do not skip defaulted input '{}'. Use {target_name} {{ {}: ... }} to omit the defaulted input, or pass the complete positional prefix.",
        if arg_count == 1 {
            "argument"
        } else {
            "arguments"
        },
        gap.required,
        gap.defaulted,
        gap.required,
    ))
}

fn positional_callable_prefix_message(
    callable_name: &str,
    params: &[ParamSig],
    arg_count: usize,
) -> Option<String> {
    let gap = positional_prefix_gap(
        params
            .iter()
            .map(|param| (param.name.as_str(), param.has_initializer, param.variadic)),
        arg_count,
    )?;
    let supplied = gap
        .supplied
        .as_deref()
        .map(|name| format!("the last supplied value initializes parameter '{name}'; "))
        .unwrap_or_else(|| "no parameters are supplied; ".to_string());
    Some(format!(
        "positional call to {callable_name} with {arg_count} {} binds a declaration-order prefix: {supplied}required parameter '{}' remains unset because positional arguments do not skip defaulted parameter '{}'. Use a named argument such as {callable_name}({} = ...), or pass the complete positional prefix.",
        if arg_count == 1 {
            "argument"
        } else {
            "arguments"
        },
        gap.required,
        gap.defaulted,
        gap.required,
    ))
}

fn empty_collection_literal_span(expr: &Expr) -> Option<crate::source::Span> {
    match expr {
        Expr::ListLiteral { items, span } if items.is_empty() => Some(*span),
        Expr::Group { inner, .. } => empty_collection_literal_span(inner),
        _ => None,
    }
}

fn contextual_construction_expr(expr: &Expr) -> Option<&Expr> {
    match expr {
        Expr::Group { inner, .. } => contextual_construction_expr(inner),
        Expr::RecordLiteral { .. } | Expr::ContextualNew { .. } => Some(expr),
        Expr::If {
            then_block,
            else_branch,
            ..
        } => contextual_construction_in_block(then_block).or_else(|| match else_branch.as_ref() {
            ElseExprBranch::If(expr) => contextual_construction_expr(expr),
            ElseExprBranch::Block(block) => contextual_construction_in_block(block),
        }),
        Expr::Block { body, .. } => contextual_construction_in_block(body),
        _ => None,
    }
}

fn contextual_construction_in_block(block: &Block) -> Option<&Expr> {
    block_tail_expr(block).and_then(contextual_construction_expr)
}

fn block_tail_expr(block: &Block) -> Option<&Expr> {
    match block.statements.last() {
        Some(Stmt::Expr(statement)) => Some(&statement.expr),
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
        Expr::ListLiteral { items, .. } | Expr::TupleLiteral { items, .. } => items
            .iter()
            .find_map(loop_control_targeting_current_loop_in_expr),
        Expr::Call { callee, args, .. } => loop_control_targeting_current_loop_in_expr(callee)
            .or_else(|| {
                args.iter()
                    .find_map(|arg| loop_control_targeting_current_loop_in_expr(&arg.value))
            }),
        Expr::ContextualNew { args, .. } => args
            .iter()
            .find_map(|arg| loop_control_targeting_current_loop_in_expr(&arg.value)),
        Expr::Member { receiver, .. } => loop_control_targeting_current_loop_in_expr(receiver),
        Expr::Index {
            receiver, index, ..
        } => loop_control_targeting_current_loop_in_expr(receiver)
            .or_else(|| loop_control_targeting_current_loop_in_expr(index)),
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
            condition_clauses,
            then_block,
            else_branch,
            ..
        } => condition_clauses
            .iter()
            .find_map(|clause| {
                loop_control_targeting_current_loop_in_expr(condition_clause_expr(clause))
            })
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
        // Nested callables and anonymous object methods are separate control-flow boundaries.
        Expr::Lambda { .. } | Expr::AnonymousObject { .. } => None,
        Expr::Identifier { .. }
        | Expr::Placeholder { .. }
        | Expr::Integer { .. }
        | Expr::Float { .. }
        | Expr::Decimal { .. }
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
        Expr::ListLiteral { items, .. } | Expr::TupleLiteral { items, .. } => {
            items.iter().find_map(lazy_arg_forbidden_control_flow_span)
        }
        Expr::Call { callee, args, .. } => {
            lazy_arg_forbidden_control_flow_span(callee).or_else(|| {
                args.iter()
                    .find_map(|arg| lazy_arg_forbidden_control_flow_span(&arg.value))
            })
        }
        Expr::ContextualNew { args, .. } => args
            .iter()
            .find_map(|arg| lazy_arg_forbidden_control_flow_span(&arg.value)),
        Expr::Member { receiver, .. } => lazy_arg_forbidden_control_flow_span(receiver),
        Expr::Index {
            receiver, index, ..
        } => lazy_arg_forbidden_control_flow_span(receiver)
            .or_else(|| lazy_arg_forbidden_control_flow_span(index)),
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
            condition_clauses,
            then_block,
            else_branch,
            ..
        } => condition_clauses
            .iter()
            .find_map(|clause| lazy_arg_forbidden_control_flow_span(condition_clause_expr(clause)))
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
        Expr::Lambda { .. } | Expr::AnonymousObject { .. } => None,
        Expr::Group { inner, .. } => lazy_arg_forbidden_control_flow_span(inner),
        Expr::Identifier { .. }
        | Expr::Placeholder { .. }
        | Expr::Integer { .. }
        | Expr::Float { .. }
        | Expr::Decimal { .. }
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
            .unwrap_or_else(Ty::unit),
        visibility: function.visibility,
        has_body: true,
        getter: false,
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
            .unwrap_or_else(Ty::unit),
        visibility: method.visibility,
        has_body: method.body.is_some(),
        getter: method.getter,
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
                visibility: field.visibility,
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
                        visibility: field.visibility,
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
        module_id: String::new(),
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
        TypeRef::Union { members, .. } => Ty::union(
            members
                .iter()
                .map(|member| convert_type_ref(member, type_params)),
        ),
    }
}

fn infer_literal_type(expr: &Expr) -> Option<Ty> {
    match expr {
        Expr::Group { inner, .. } => infer_literal_type(inner),
        Expr::Integer { .. } => Some(Ty::int()),
        Expr::Float { .. } => Some(Ty::float()),
        Expr::Decimal { .. } => Some(Ty::named("Decimal")),
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
        TypeRef::Union { members, .. } => members.iter().find_map(unit_function_param_span),
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
    conflicts: Vec<(String, bool, crate::source::Span)>,
}

fn arrange_param_args<'a>(
    params: &[ParamSig],
    args: &'a [crate::ast::CallArg],
) -> ArgArrangement<'a> {
    let mut slots = vec![Vec::new(); params.len()];
    let mut positional_index = 0usize;
    let mut overflow = 0usize;
    let mut conflicts = Vec::new();
    for arg in args {
        if let Some(name) = &arg.name {
            if let Some(index) = params.iter().position(|param| param.name == *name) {
                if slots[index].is_empty() {
                    slots[index].push(arg);
                } else if arg.from_shape_spread && !slots[index][0].from_shape_spread {
                    // Explicit arguments take precedence over every shape source.
                } else {
                    overflow += 1;
                    conflicts.push((
                        name.clone(),
                        arg.from_shape_spread && slots[index][0].from_shape_spread,
                        arg.span,
                    ));
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
        conflicts,
    }
}

fn arrange_constructor_args<'a>(
    params: &[FieldSig],
    args: &'a [crate::ast::CallArg],
) -> ArgArrangement<'a> {
    let mut slots = vec![Vec::new(); params.len()];
    let mut positional_index = 0usize;
    let mut overflow = 0usize;
    let mut conflicts = Vec::new();
    for arg in args {
        if let Some(name) = &arg.name {
            if let Some(index) = params.iter().position(|param| param.name == *name) {
                if slots[index].is_empty() {
                    slots[index].push(arg);
                } else if arg.from_shape_spread && !slots[index][0].from_shape_spread {
                } else {
                    overflow += 1;
                    conflicts.push((
                        name.clone(),
                        arg.from_shape_spread && slots[index][0].from_shape_spread,
                        arg.span,
                    ));
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
        conflicts,
    }
}

fn call_uses_structural_record_arg(args: &[crate::ast::CallArg], uses_brace_syntax: bool) -> bool {
    uses_brace_syntax && has_single_record_literal_arg(args)
}

fn has_single_record_literal_arg(args: &[crate::ast::CallArg]) -> bool {
    matches!(
        args,
        [crate::ast::CallArg {
            from_shape_spread: false,
            name: None,
            value: Expr::RecordLiteral { .. },
            ..
        }]
    )
}

fn brace_record_constructor_args(args: &[crate::ast::CallArg]) -> Option<Vec<crate::ast::CallArg>> {
    let [
        crate::ast::CallArg {
            from_shape_spread: false,
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
            from_shape_spread: false,
            name: None,
            value,
            ..
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

fn format_factory_help_args(args: &[crate::ast::CallArg]) -> Option<String> {
    if let [
        crate::ast::CallArg {
            from_shape_spread: false,
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
        Expr::Integer { raw, .. } | Expr::Float { raw, .. } | Expr::Decimal { raw, .. } => {
            raw.clone()
        }
        Expr::String { raw, .. } if raw.starts_with('"') || raw.starts_with("raw\"") => raw.clone(),
        Expr::String { raw, .. } => format!("{raw:?}"),
        Expr::Bool { value, .. } => value.to_string(),
        Expr::Unit { .. } => "()".to_string(),
        _ => "...".to_string(),
    }
}

fn static_integer_literal(expr: &Expr) -> Option<i64> {
    match expr {
        Expr::Integer { raw, .. } => parse_integer_literal(raw, false),
        Expr::Unary {
            op: crate::ast::UnaryOp::Neg,
            expr,
            ..
        } => static_integer_literal(expr)?.checked_neg(),
        Expr::Group { inner, .. } => static_integer_literal(inner),
        _ => None,
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
        Ty::Union(expected_members) => {
            if let Ty::Union(actual_members) = actual {
                for (expected_member, actual_member) in
                    expected_members.iter().zip(actual_members.iter())
                {
                    infer_type_subst(expected_member, actual_member, subst);
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
        Ty::Union(members) => {
            Ty::union(members.iter().map(|member| substitute_type(member, subst)))
        }
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
        Ty::Named(_, args) | Ty::Tuple(args) | Ty::Union(args) => {
            args.iter().any(type_contains_wildcard)
        }
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
        getter: method.getter,
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
        Ty::Named(_, args) | Ty::Tuple(args) | Ty::Union(args) => {
            args.iter().any(type_contains_type_param)
        }
        Ty::Record(fields) => fields.iter().any(|(_, ty)| type_contains_type_param(ty)),
        Ty::Function(params, ret) => {
            params.iter().any(type_contains_type_param) || type_contains_type_param(ret)
        }
        Ty::Never | Ty::Unknown => false,
    }
}

fn type_contains_unresolved_constructor_type(ty: &Ty) -> bool {
    match ty {
        Ty::Unknown | Ty::Wildcard | Ty::Capture(_) => true,
        Ty::Named(_, args) | Ty::Tuple(args) | Ty::Union(args) => {
            args.iter().any(type_contains_unresolved_constructor_type)
        }
        Ty::Record(fields) => fields
            .iter()
            .any(|(_, ty)| type_contains_unresolved_constructor_type(ty)),
        Ty::Function(params, ret) => {
            params.iter().any(type_contains_unresolved_constructor_type)
                || type_contains_unresolved_constructor_type(ret)
        }
        Ty::Never | Ty::TypeParam(_) => false,
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
        Ty::Union(members) => Ty::union(members.iter().map(materialize_type)),
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
        Expr::ListLiteral { items, span } if items.len() == 1 => Some(TypeRef::Named {
            name: "Vector".to_string(),
            args: vec![type_ref_from_expr(&items[0])?],
            span: *span,
        }),
        Expr::Identifier { name, span } => Some(TypeRef::Named {
            name: name.clone(),
            args: Vec::new(),
            span: *span,
        }),
        Expr::Member { span, .. } => Some(TypeRef::Named {
            name: expr_path_for_known_value(expr)?.join("."),
            args: Vec::new(),
            span: *span,
        }),
        Expr::Index {
            receiver,
            index,
            span,
            ..
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
    if let Ty::Union(expected_members) = expected {
        return match actual {
            Ty::Union(actual_members) => actual_members.iter().all(|actual_member| {
                expected_members
                    .iter()
                    .any(|expected_member| is_assignable(actual_member, expected_member))
            }),
            actual => expected_members
                .iter()
                .any(|expected_member| is_assignable(actual, expected_member)),
        };
    }
    if let Ty::Union(actual_members) = actual {
        return actual_members
            .iter()
            .all(|actual_member| is_assignable(actual_member, expected));
    }
    match (actual, expected) {
        (Ty::Never, Ty::Never) => true,
        (Ty::Wildcard, Ty::Wildcard) => true,
        (Ty::Capture(left), Ty::Capture(right)) => left == right,
        (Ty::TypeParam(left), Ty::TypeParam(right)) => left == right,
        (Ty::Named(left, left_args), Ty::Named(right, right_args)) => {
            (left == right
                && left_args.len() == right_args.len()
                && type_args_assignable(left, left_args, right_args))
                || enum_case_view_parts(left).is_some_and(|(owner, _)| {
                    owner == right
                        && left_args.len() == right_args.len()
                        && type_args_assignable(owner, left_args, right_args)
                })
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
                    .all(|(left, right)| is_assignable(right, left))
                && is_assignable(left_ret, right_ret)
        }
        _ => false,
    }
}

fn enum_case_view_name(owner: &str, case_name: &str) -> String {
    format!("{owner}::{case_name}")
}

fn enum_case_view_parts(name: &str) -> Option<(&str, &str)> {
    let (owner, case_name) = name.split_once("::")?;
    (!owner.is_empty() && !case_name.is_empty()).then_some((owner, case_name))
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

    if generic_type_is_covariant(type_name) {
        return actual_args
            .iter()
            .zip(expected_args.iter())
            .all(|(actual, expected)| is_assignable(actual, expected));
    }

    // A generic value may expose mutation through its own API. Keep type
    // arguments invariant unless the type has an explicitly audited read-only
    // contract, so aliases cannot widen the type of writable storage.
    actual_args
        .iter()
        .zip(expected_args.iter())
        .all(|(actual, expected)| {
            matches!(expected, Ty::Wildcard)
                || (is_assignable(actual, expected) && is_assignable(expected, actual))
        })
}

fn generic_type_is_covariant(type_name: &str) -> bool {
    matches!(
        type_name,
        "Iterable" | "Iterator" | "Option" | "Result" | "Either"
    )
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
            | "UnionType"
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
    Ty::union([left.clone(), right.clone()])
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
        TypeRef::Union { .. } => true,
        TypeRef::Wildcard { .. } => false,
    }
}

fn module_alias_and_member(expr: &Expr) -> Option<(String, String)> {
    let path = expr_path_for_known_value(expr)?;
    let (alias, member) = path.split_first()?;
    (!member.is_empty()).then(|| (alias.clone(), member.join(".")))
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
    fn checks_stateless_mixins_and_rejects_unsafe_composition() {
        let accepted = check_path(workspace_root().join("examples/features/mixins.lum")).unwrap();
        assert!(
            accepted.diagnostics.is_empty(),
            "{:#?}",
            accepted.diagnostics
        );
        let cases = [
            (
                "mixin_member_conflict",
                "mixin A { get value Int = 1 }\nmixin B { get value Int = 2 }\nclass C { use A\nuse B }",
            ),
            (
                "incompatible_mixin_override",
                "mixin A { get value Bool = true }\nclass C { use A\nget value Int = 1 }",
            ),
            (
                "mixin_member_kind_mismatch",
                "mixin A { get value Int = 1 }\nclass C { use A\ndef value() Int = 2 }",
            ),
            (
                "mixin_member_kind_mismatch",
                "mixin A { get value Int = 1 }\nclass C { use A\nvalue Int }",
            ),
            (
                "missing_mixin_requirement",
                "interface Sized { get size Int }\nmixin A with Sized { get nonEmpty Bool = this.size > 0 }\nclass C { use A }",
            ),
            ("invalid_mixin_use", "class A {}\nclass C { use A }"),
            (
                "duplicate_mixin_use",
                "mixin A {}\nclass C { use A\nuse A }",
            ),
            ("mixin_not_a_type", "mixin A {}\ndef f(value A) {}"),
            ("mixin_not_a_type", "mixin A {}\ntype Alias = A"),
            (
                "invalid_mixin_construction",
                "mixin A {}\ndef main() { value = A() }",
            ),
            (
                "mixin_not_a_type",
                "mixin A {}\ndef main() { value = typeOf[A] }",
            ),
            (
                "invalid_type_argument_count",
                "mixin A[T] {}\nclass C { use A }",
            ),
            (
                "incompatible_mixin_requirement",
                "interface Readable { def read() Str }\nmixin A with Readable { def text() Str = this.read() }\nclass C with Readable { use A\ndef read() Int = 1 }",
            ),
            (
                "assign_immutable",
                "interface Sized { get size Int }\nmixin A with Sized { def update() { this.size := 1 } }",
            ),
            (
                "mixin_not_a_type",
                "mixin A {}\ndef main() { callback = (value A) => value }",
            ),
            (
                "mixin_not_a_value",
                "mixin A { def value() Int = 1 }\ndef main() { value = A.value() }",
            ),
        ];
        for (code, source) in cases {
            let result = check_program(&parse_inline(source));
            assert!(
                result
                    .diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.code == code),
                "expected {code} for {source}: {:#?}",
                result.diagnostics
            );
        }
    }

    #[test]
    fn bare_union_alternatives_require_unique_scope_resolution() {
        let ambiguous = parse_inline(
            r#"
type ConnectionState = object Ready {} | object Closed {}
type JobState = object Ready {} | object Running {}

def main() Unit {
    value ConnectionState = ConnectionState.Ready
    other = Ready
    match value {
        case Ready => ()
        case ConnectionState.Closed => ()
    }
}
"#,
        );
        let result = check_program(&ambiguous);
        let ambiguities = result
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "ambiguous_union_alternative")
            .count();
        assert_eq!(ambiguities, 2, "{:#?}", result.diagnostics);

        let qualified = parse_inline(
            r#"
type ConnectionState = object Ready {} | object Closed {}
type JobState = object Ready {} | object Running {}

def main() Unit {
    connection ConnectionState = ConnectionState.Ready
    job JobState = JobState.Ready
    match connection {
        case ConnectionState.Ready => ()
        case ConnectionState.Closed => ()
    }
}
"#,
        );
        let result = check_program(&qualified);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn rejects_invalid_known_addition_operands_without_cascading_unknowns() {
        let invalid = parse_inline(
            r#"
def broken() Int = true + false
"#,
        );
        let invalid_result = check_program(&invalid);
        assert_eq!(
            invalid_result.diagnostics.len(),
            1,
            "{:#?}",
            invalid_result.diagnostics
        );
        assert!(
            invalid_result.diagnostics.iter().any(|diagnostic| {
                diagnostic.code == "invalid_binary_operand"
                    && diagnostic.message.contains("Bool")
                    && diagnostic.message.contains("operator '+'")
            }),
            "{:#?}",
            invalid_result.diagnostics
        );

        let unknown = parse_inline(
            r#"
def broken() Int = missing + false
"#,
        );
        let unknown_result = check_program(&unknown);
        assert!(
            unknown_result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "undefined_name"),
            "{:#?}",
            unknown_result.diagnostics
        );
        assert!(
            unknown_result
                .diagnostics
                .iter()
                .all(|diagnostic| diagnostic.code != "invalid_binary_operand"),
            "{:#?}",
            unknown_result.diagnostics
        );
    }

    #[test]
    fn checks_complete_signed_integer_literal_boundary() {
        let valid = parse_inline(
            r#"
def maximum() Int = 9223372036854775807
def minimum() Int = -9223372036854775808
"#,
        );
        let valid_result = check_program(&valid);
        assert!(
            valid_result.diagnostics.is_empty(),
            "{:#?}",
            valid_result.diagnostics
        );

        let invalid = parse_inline(
            r#"
def tooHigh() Int = 9223372036854775808
def tooLow() Int = -9223372036854775809
"#,
        );
        let invalid_result = check_program(&invalid);
        let range_errors = invalid_result
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "integer_literal_out_of_range")
            .collect::<Vec<_>>();
        assert_eq!(range_errors.len(), 2, "{:#?}", invalid_result.diagnostics);
        assert!(
            range_errors
                .iter()
                .any(|diagnostic| diagnostic.message.contains("9223372036854775808")),
            "{:#?}",
            invalid_result.diagnostics
        );
        assert!(
            range_errors
                .iter()
                .any(|diagnostic| diagnostic.message.contains("-9223372036854775809")),
            "{:#?}",
            invalid_result.diagnostics
        );
    }

    #[test]
    fn preserves_compatible_user_defined_addition() {
        let program = parse_inline(
            r#"
class Pair {
    left Int
    right Int

    def +(other Pair) Pair = Pair(left + other.left, right + other.right)
}

def combine(left Pair, right Pair) Pair = left + right
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn function_parameters_are_contravariant_and_returns_are_covariant() {
        let program = parse_inline(
            r#"
def ignore(value Any) Str = "ignored"

def main() Unit {
    acceptsAny fn(Any) Str = ignore
    acceptsStr fn(Str) Any = acceptsAny
    result Any = acceptsStr("value")
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn rejects_function_assignment_with_a_narrower_actual_parameter() {
        let program = parse_inline(
            r#"
def textLength(value Str) Int = value.size

def main() Unit {
    onlyText fn(Str) Int = textLength
    anyValue fn(Any) Int = onlyText
    println(anyValue(123))
}
"#,
        );
        let result = check_program(&program);
        assert_eq!(result.diagnostics.len(), 1, "{:#?}", result.diagnostics);
        assert!(
            result.diagnostics.iter().any(|diagnostic| {
                diagnostic.code == "invalid_binding_type"
                    && diagnostic.message.contains("fn(Str) Int")
                    && diagnostic.message.contains("fn(Any) Int")
            }),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn rejects_unsafe_function_parameter_variance_inside_a_collection() {
        let program = parse_inline(
            r#"
def textLength(value Str) Int = value.size

def main() Unit {
    onlyText [fn(Str) Int] = [textLength]
    anyValue [fn(Any) Int] = onlyText
}
"#,
        );
        let result = check_program(&program);
        assert_eq!(result.diagnostics.len(), 1, "{:#?}", result.diagnostics);
        assert!(
            result.diagnostics.iter().any(|diagnostic| {
                diagnostic.code == "invalid_binding_type"
                    && diagnostic.message.contains("Vector[fn(Str) Int]")
                    && diagnostic.message.contains("Vector[fn(Any) Int]")
            }),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn rejects_widening_existing_mutable_generic_aliases() {
        let program = parse_inline(
            r#"
class Cell[T] {
    var value T
}

def main() Unit {
    integers [Int] = [1]
    arbitrary [Any] = integers

    original Cell[Int] = Cell(1)
    widened Cell[Any] = original
}
"#,
        );
        let result = check_program(&program);
        assert_eq!(result.diagnostics.len(), 2, "{:#?}", result.diagnostics);
        assert!(
            result.diagnostics.iter().any(|diagnostic| {
                diagnostic.code == "invalid_binding_type"
                    && diagnostic.message.contains("Vector[Int]")
                    && diagnostic.message.contains("Vector[Any]")
            }),
            "{:#?}",
            result.diagnostics
        );
        assert!(
            result.diagnostics.iter().any(|diagnostic| {
                diagnostic.code == "invalid_binding_type"
                    && diagnostic.message.contains("Cell[Int]")
                    && diagnostic.message.contains("Cell[Any]")
            }),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn allows_fresh_wider_generic_values_and_read_only_views() {
        let program = parse_inline(
            r#"
class Cell[T] {
    var value T
}

def main() Unit {
    arbitrary [Any] = [1]
    arbitrary.add("valid")

    fresh Cell[Any] = Cell(1)

    integers [Int] = [1]
    exact Iterable[Int] = integers
    wider Iterable[Any] = integers
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn rejects_api_signatures_that_expose_less_visible_types() {
        let program = parse_inline(
            r#"
class FileDetails {
    value Int
}

internal class ModuleDetails {
    value Int
}

public def inspect(value ModuleDetails) FileDetails = FileDetails(value.value)
internal def share(value ModuleDetails) ModuleDetails = value
internal def leak(value FileDetails) FileDetails = value

public type ExposedDetails = FileDetails

public class Api {
    details ModuleDetails
    private cached FileDetails = FileDetails(0)

    def read(value ModuleDetails) ModuleDetails = value
    private def helper(value FileDetails) FileDetails = value
}

class LocalApi {
    def read(value FileDetails) FileDetails = value
}
"#,
        );
        let result = check_program(&program);
        let visibility_errors = result
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "inaccessible_type_in_api")
            .collect::<Vec<_>>();
        assert_eq!(visibility_errors.len(), 8, "{:#?}", result.diagnostics);
        assert!(
            visibility_errors.iter().any(|diagnostic| {
                diagnostic.message.contains("public API function 'inspect'")
                    && diagnostic.message.contains("internal type 'ModuleDetails'")
            }),
            "{:#?}",
            result.diagnostics
        );
        assert!(
            visibility_errors.iter().any(|diagnostic| {
                diagnostic.message.contains("internal API function 'leak'")
                    && diagnostic.message.contains("private type 'FileDetails'")
            }),
            "{:#?}",
            result.diagnostics
        );
        assert!(
            !visibility_errors
                .iter()
                .any(|diagnostic| diagnostic.message.contains("function 'share'")),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn propagates_expected_types_through_groups_tuples_and_for_yield() {
        let program = parse_inline(
            r#"
shape Point {
    x Int
    y Int
}

def main() Unit {
    grouped [Str] = ([])
    groupedPoint Point = (new(1, 2))
    configuration ([Str], Bool) = ([], false)
    pair (Point, Point) = (new(1, 2), new(3, 4))
    points [Point] = for value <- [1, 2] yield new(value, 0)
    rows [[Int]] = for value <- [1, 2] yield []
    optionalSource Int? = Some(1)
    optionalPoint Point? = for value <- optionalSource yield new(value, 0)
    resultSource Result[Int, Str] = Ok(2)
    resultPoint Result[Point, Str] = for value <- resultSource yield new(value, 0)
    eitherSource Either[Str, Int] = Right(3)
    eitherPoint Either[Str, Point] = for value <- eitherSource yield new(value, 0)
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn does_not_choose_for_yield_family_from_the_expected_type() {
        let program = parse_inline(
            r#"
shape Point {
    x Int
    y Int
}

def points(source) [Point] =
    for value <- source yield new(value, 0)
"#,
        );
        let result = check_program(&program);
        assert!(
            result.diagnostics.iter().any(|diagnostic| {
                diagnostic.code == "invalid_contextual_construction"
                    && diagnostic
                        .message
                        .contains("requires an expected class or shape")
            }),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn rejects_local_functions_inside_nested_executable_blocks() {
        let program = parse_inline(
            r#"
def main(flag Bool) Unit {
    def direct(value Int) Int = value

    if flag {
        def inIf(value Int) Int = value
        println(inIf(1))
    }

    while flag {
        def inWhile(value Int) Int = value
        println(inWhile(1))
        break
    }

    for value <- [1] {
        def inFor(input Int) Int = input
        println(inFor(value))
    }

    match flag {
        case true => {
            def inMatch(value Int) Int = value
            println(inMatch(1))
        }
        case false => ()
    }

    println(direct(1))
}
"#,
        );
        let result = check_program(&program);
        assert_eq!(
            result
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.code == "invalid_local_function_scope")
                .count(),
            4,
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn allows_semantic_equality_and_reference_ids() {
        let program = parse_inline(
            r#"
class Box with Eq[Box] {
    value Int

    def equals(other Box) Bool = this.value == other.value
}

object Shared {
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

shape Point {
    x Int
}

def main() Unit {
    first = Box(1)
    alias = first
    sameValue = first == Box(1)
    differentValue = first != Box(2)
    sameInstance = first.referenceId == alias.referenceId

    values = [1, 2]
    valuesAlias = values
    sameVectorReference = values.referenceId == valuesAlias.referenceId
    sameObject = Shared == Shared
    objectReference ReferenceId = Shared.referenceId

    left Identified = Entry(1)
    right Identified = AlternateEntry(1)
    interfaceEquality = left == right

    sameInt = 1 == 1
    sameShape = Point(1) == Point(1)
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn equality_requires_an_equality_contract_and_valid_reference_receivers() {
        let program = parse_inline(
            r#"
class Box {
    value Int
}

interface Named {
    def name() Str
}

class Person with Named {
    label Str
    def name() Str = this.label
}

shape Point {
    x Int
}

def main() Unit {
    missingClassContract = Box(1) == Box(1)
    left Named = Person("Ada")
    right Named = left
    missingInterfaceContract = left == right
    unknown Any = Any(Box(1))
    anyEquality = unknown == unknown
    number = 1
    invalidPrimitiveReference = number.referenceId
    invalidShapeReference = Point(1).referenceId
    invalidInterfaceReference = left.referenceId
}
"#,
        );
        let result = check_program(&program);
        assert_eq!(
            result
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.code == "missing_equality_contract")
                .count(),
            2,
            "{:#?}",
            result.diagnostics
        );
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diagnostic| { diagnostic.code == "any_equality_not_supported" })
        );
        assert!(
            result
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.code == "invalid_reference_id_receiver")
                .count()
                == 3
        );
    }

    #[test]
    fn rejects_equality_between_unrelated_equality_domains() {
        let program = parse_inline(
            r#"
class FirstBox with Eq[FirstBox] {
    def equals(other FirstBox) Bool = true
}

class SecondBox with Eq[SecondBox] {
    def equals(other SecondBox) Bool = true
}

def main() Unit {
    same = FirstBox {} == SecondBox {}
}
"#,
        );
        let result = check_program(&program);
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "incompatible_equality_operands"),
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
}
"#,
        );
        let result = check_program(&program);
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "any_equality_not_supported"),
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
type State =
    object Ready {}
    | object Busy {}

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
    fn derives_shape_equality_only_from_public_equality_fields() {
        let program = parse_inline(
            r#"
class Token {
    value Int
}

shape Entry {
    token Token
}

shape Nested {
    entry Entry
}

shape Box[T] {
    value T
}

shape PrivateEntry {
    value Int
    private marker Int = 0
}

shape InternalEntry {
    value Int
    internal marker Int = 0
}

def requireEq[T with Eq[T]](value T) Unit = ()

def main() Unit {
    entry = Entry(Token(1))
    nested = Nested(entry)
    privateEntry = PrivateEntry(1)
    internalEntry = InternalEntry(1)

    _ = entry == entry
    _ = nested == nested
    _ = privateEntry == privateEntry
    _ = internalEntry == internalEntry
    requireEq(Box(1))
    requireEq(Box(Token(1)))
}
"#,
        );
        let result = check_program(&program);
        assert_eq!(
            result
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.code == "missing_equality_contract")
                .count(),
            4,
            "{:#?}",
            result.diagnostics
        );
        assert!(
            result.diagnostics.iter().any(|diagnostic| {
                diagnostic.code == "generic_bound_not_satisfied"
                    && diagnostic.message.contains("Box[Token]")
            }),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn explicit_shape_equality_allows_non_public_and_non_equality_fields() {
        let program = parse_inline(
            r#"
class Token {
    value Int
}

shape Entry with Eq[Entry] {
    token Token
    private marker Int = 0

    def equals(other Entry) Bool =
        this.token.referenceId == other.token.referenceId
}

def requireEq[T with Eq[T]](value T) Unit = ()

def main() Unit {
    token = Token(1)
    first = Entry(token)
    alias = Entry(token)
    other = Entry(Token(1))

    requireEq(first)
    _ = first == alias
    _ = first != other
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn derives_shape_hashing_only_from_public_hashed_fields() {
        let program = parse_inline(
            r#"
shape PublicKey {
    id Int
}

shape PrivateKey {
    id Int
    private marker Int = 0
}

shape CustomEquality with Eq[CustomEquality] {
    id Int
    note Str

    def equals(other CustomEquality) Bool = this.id == other.id
}

shape ExplicitKey with Hashed[ExplicitKey] {
    id Int
    private marker Int = 0

    def equals(other ExplicitKey) Bool = this.id == other.id
    def hash() Int = this.id
}

def requireHash[T with Hashed[T]](value T) Unit = ()

def main() Unit {
    requireHash(PublicKey(1))
    requireHash(PrivateKey(1))
    requireHash(CustomEquality(1, "one"))
    requireHash(ExplicitKey(1))
}
"#,
        );
        let result = check_program(&program);
        assert_eq!(
            result
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.code == "generic_bound_not_satisfied")
                .count(),
            2,
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn derives_equality_and_hashing_recursively_for_tuples_and_declared_unions() {
        let program = parse_inline(
            r#"
type LookupKey =
    class Number { value Int }
    | object Default {}

def requireEq[T with Eq[T]](value T) Unit = ()
def requireHash[T with Hashed[T]](value T) Unit = ()

def main() Unit {
    pair = (10, "ten")
    requireEq(pair)
    requireHash(pair)
    _ [(Int, Str): Str] = [pair: "tuple"]

    key LookupKey = LookupKey.Number(10)
    requireEq(key)
    requireHash(key)
    _ = [key: "union"]
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn rejects_tuple_and_declared_union_hashing_when_a_payload_is_not_hashable() {
        let program = parse_inline(
            r#"
class MutableReference {
    id Int
}

type UnsafeKey =
    class Wrapped { value MutableReference }
    | object Missing {}

def requireHash[T with Hashed[T]](value T) Unit = ()

def main() Unit {
    pair = (MutableReference(1), 2)
    _ = pair == (MutableReference(1), 2)
    requireHash(pair)

    key UnsafeKey = UnsafeKey.Wrapped(MutableReference(1))
    requireHash(key)
}
"#,
        );
        let result = check_program(&program);
        let failures = result
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "generic_bound_not_satisfied")
            .count();
        assert_eq!(failures, 2, "{:#?}", result.diagnostics);
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
    fn rejects_equality_and_hashed_bounds_for_shape_with_uncontracted_class_field() {
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
            2,
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
    fn interface_implementations_cannot_reduce_member_visibility() {
        let program = parse_inline(
            r#"
interface Reader {
    def read() Str
}

class PrivateReader with Reader {
    private def read() Str = "private"
}

class InternalReader with Reader {
    internal def read() Str = "internal"
}

class PublicReader with Reader {
    def read() Str = "public"
}
"#,
        );
        let result = check_program(&program);
        let missing = result
            .diagnostics
            .iter()
            .filter(|diagnostic| {
                diagnostic.code == "missing_interface_member" && diagnostic.message.contains("read")
            })
            .count();
        assert_eq!(missing, 2, "{:#?}", result.diagnostics);
    }

    #[test]
    fn public_fields_satisfy_read_only_interface_getters() {
        let program = parse_inline(
            r#"
interface Identified[T] {
    get id T
}

class User with Identified[Int] {
    id Int
    name Str
}

class MutableUser with Identified[Int] {
    var id Int
}

shape UserSnapshot with Identified[Int] {
    id Int
}

def main() Unit {
    user Identified[Int] = User(7, "Ada")
    mutable Identified[Int] = MutableUser(8)
    snapshot Identified[Int] = UserSnapshot(9)
    anonymous Identified[Int] = object with Identified[Int] {
        id Int = 10
    }
    println(user.id + mutable.id + snapshot.id + anonymous.id)
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn concrete_types_can_implement_interface_getters_with_fields_or_getters() {
        let result = check_path(
            workspace_root().join("examples/features/interface_getter_implementations.lum"),
        )
        .unwrap();
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);

        for owner in ["class", "shape", "object"] {
            for (requirement, implementation, code) in [
                (
                    "get id Int",
                    "private id Int = 1",
                    "missing_interface_member",
                ),
                (
                    "get id Int",
                    "internal id Int = 1",
                    "missing_interface_member",
                ),
                (
                    "get id Int",
                    "id Str = \"bad\"",
                    "invalid_interface_getter_field_type",
                ),
                (
                    "get id Int",
                    "get id Str = \"bad\"",
                    "missing_interface_member",
                ),
                (
                    "get id Int",
                    "private get id Int = 1",
                    "missing_interface_member",
                ),
                (
                    "get id Int",
                    "internal get id Int = 1",
                    "missing_interface_member",
                ),
                ("get id Int", "def id() Int = 1", "missing_interface_member"),
                ("def id() Int", "get id Int = 1", "missing_interface_member"),
                ("def id() Int", "id Int = 1", "missing_interface_member"),
            ] {
                let source = format!(
                    "interface Contract {{ {requirement} }}\n{owner} Implementation with Contract {{ {implementation} }}"
                );
                let checked = check_program(&parse_inline(&source));
                assert!(
                    checked
                        .diagnostics
                        .iter()
                        .any(|diagnostic| diagnostic.code == code),
                    "expected {code} for {source}: {:?}",
                    checked.diagnostics
                );
            }
        }
    }

    #[test]
    fn fields_must_be_public_and_compatible_to_satisfy_interface_getters() {
        let program = parse_inline(
            r#"
interface Identified {
    get id Int
}

class HiddenId with Identified {
    private id Int = 1
}

class WrongId with Identified {
    id Str
}

class MethodId with Identified {
    def id() Int = 1
}
"#,
        );
        let result = check_program(&program);
        let missing = result
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "missing_interface_member")
            .count();
        assert_eq!(missing, 2, "{:#?}", result.diagnostics);
        assert!(
            result.diagnostics.iter().any(|diagnostic| {
                diagnostic.code == "invalid_interface_getter_field_type"
                    && diagnostic.message.contains("WrongId")
                    && diagnostic.message.contains("Str")
                    && diagnostic.message.contains("Int")
            }),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn field_backed_interface_getters_remain_read_only() {
        let program = parse_inline(
            r#"
            interface Identified { get id Int }
            class User with Identified { var id Int }
            def main() {
                user Identified = User(1)
                user.id := 2
            }
            "#,
        );
        let checked = check_program(&program);
        assert!(
            checked
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "assign_immutable"),
            "{:?}",
            checked.diagnostics
        );
    }

    #[test]
    fn checks_shape_argument_spread_precedence() {
        let program = parse_inline(
            r#"
def consume(first Int, second Str = "default") Int = first
def main() {
    options = { first: "ignored", second: "spread" }
    consume(1, ...options)
    consume(first = 1, ...options)
    consume(1, "explicit", ...options, ...{ first: false })
    consume(...{ first: 1 })
}
"#,
        );
        let checked = check_program(&program);
        assert!(checked.diagnostics.is_empty(), "{:?}", checked.diagnostics);
    }

    #[test]
    fn rejects_invalid_shape_argument_spreads() {
        for (body, code) in [
            (
                "consume(...{ value: 1 }, value = 2)",
                "shape_spread_argument_order",
            ),
            ("consume(...{ value: 1 }, 2)", "shape_spread_argument_order"),
            (
                "consume(...{ value: 1 }, ...{ value: 2 })",
                "conflicting_shape_arguments",
            ),
            (
                "consume(1, value = 2, ...{ value: 3 })",
                "duplicate_argument",
            ),
            ("consume(...{ value: 1, typo: 2 })", "unknown_argument_name"),
            ("consume(...{ value: \"wrong\" })", "invalid_argument_type"),
            ("consume(...Options(1))", "invalid_shape_argument_spread"),
            (
                "consume(...[\"value\": 1])",
                "invalid_shape_argument_spread",
            ),
            ("consume(...{ unrelated: 1 })", "unknown_argument_name"),
        ] {
            let program = parse_inline(&format!(
                "class Options {{ value Int }}\ndef consume(value Int) {{}}\ndef main() {{ {body} }}"
            ));
            let checked = check_program(&program);
            assert!(
                checked
                    .diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.code == code),
                "expected {code} for {body}: {:?}",
                checked.diagnostics
            );
        }
    }

    #[test]
    fn shape_argument_spreads_require_static_fields_and_parameter_names() {
        for source in [
            "def consume(value Int) {}\ndef bad(options Any) { consume(...options) }",
            "def consume(value Int) {}\ndef bad(options { value Int } | { other Int }) { consume(...options) }",
            "def bad(consume fn(Int) Int) Int = consume(...{ value: 1 })",
        ] {
            let checked = check_program(&parse_inline(source));
            assert!(
                checked.diagnostics.iter().any(|diagnostic| matches!(
                    diagnostic.code,
                    "invalid_shape_argument_spread" | "unknown_argument_name"
                )),
                "{source}: {:?}",
                checked.diagnostics
            );
        }
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
    fn checks_punned_qualified_and_generic_constructors() {
        let temp = workspace_root().join("rust/target/typecheck-constructor-target-test");
        let _ = fs::remove_dir_all(&temp);
        fs::create_dir_all(&temp).expect("create temp dir");
        fs::write(
            temp.join("models.lum"),
            r#"
module models

public shape Point {
    x Int
    y Int
}

public class Box[T] {
    value T
}
"#,
        )
        .expect("write models");
        let source = temp.join("app.lum");
        fs::write(
            &source,
            r#"
module app

use models
use models/{Point as ModelPoint, Box as ModelBox}

shape Point {
    x Int
    y Int
}

class Outer {
    shape Point {
        x Int
        y Int
    }
}

class Box[T] {
    value T
}

def main() Unit {
    x = 1
    y = 2
    value = 3

    local = Point { x, y }
    qualified = models.Point { x, y }
    nested = Outer.Point { x, y }
    generic = Box[Int] { value }
    qualifiedGeneric = models.Box[Int] { value }
    aliased = ModelPoint { x, y }
    aliasedGeneric = ModelBox[Int] { value }
}
"#,
        )
        .expect("write source");

        let result = check_path(&source).expect("typecheck");
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn checks_imported_nested_aliases_and_declared_unions() {
        let temp = workspace_root().join("rust/target/typecheck-nested-alias-import-test");
        let _ = fs::remove_dir_all(&temp);
        fs::create_dir_all(&temp).expect("create temp dir");
        fs::write(
            temp.join("model.lum"),
            r#"
module model

public class Parser {
    type Text = Str
    type Outcome =
        class Parsed { text Text }
        | object Empty {}

    def parse(text Text) Outcome = Outcome.Parsed(text)
}
"#,
        )
        .expect("write model");
        let source = temp.join("app.lum");
        fs::write(
            &source,
            r#"
module app

use model/{Parser}

def main() Str {
    outcome Parser.Outcome = Parser().parse("ok")
    return match outcome {
        case Parser.Outcome.Parsed { text } => text
        case Parser.Outcome.Empty => "empty"
    }
}
"#,
        )
        .expect("write source");

        let result = check_path(&source).expect("typecheck");
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn checks_explicitly_imported_union_alternative_aliases() {
        let temp = workspace_root().join("rust/target/typecheck-union-alternative-import-test");
        let _ = fs::remove_dir_all(&temp);
        fs::create_dir_all(&temp).expect("create temp dir");
        fs::write(
            temp.join("states.lum"),
            r#"
module states

public type ConnectionState = object Ready {} | object Closed {}
public type JobState = object Ready {} | object Running {}
"#,
        )
        .expect("write states module");
        let source = temp.join("app.lum");
        fs::write(
            &source,
            r#"
module app

use states/{ConnectionState, ConnectionState.Ready as ConnectionReady}

def isReady(value ConnectionState) Bool = match value {
    case ConnectionReady => true
    case ConnectionState.Closed => false
}

def main() Bool {
    value ConnectionState = ConnectionReady
    isReady(value)
}
"#,
        )
        .expect("write source");

        let result = check_path(&source).expect("typecheck");
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn allows_internal_members_across_files_in_the_same_module() {
        let temp = workspace_root().join("rust/target/typecheck-internal-same-module-test");
        let _ = fs::remove_dir_all(&temp);
        fs::create_dir_all(&temp).expect("create temp dir");
        fs::write(
            temp.join("support.lum"),
            r#"
module shared

internal class Box {
    internal var value Int = 1

    internal def increment() Unit = this.value += 1
}

internal def moduleValue() Int = 7
"#,
        )
        .expect("write support");
        let source = temp.join("main.lum");
        fs::write(
            &source,
            r#"
module shared

use support/{Box, moduleValue}

def main() Int {
    box = Box {}
    box.increment()
    box.value := box.value + moduleValue()
    box.value
}
"#,
        )
        .expect("write source");

        let result = check_path(&source).expect("typecheck");
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn rejects_internal_members_from_another_module() {
        let temp = workspace_root().join("rust/target/typecheck-internal-cross-module-test");
        let _ = fs::remove_dir_all(&temp);
        fs::create_dir_all(&temp).expect("create temp dir");
        fs::write(
            temp.join("model.lum"),
            r#"
module model

public class Box {
    internal var value Int = 1

    internal def increment() Unit = this.value += 1
}
"#,
        )
        .expect("write model");
        let source = temp.join("app.lum");
        fs::write(
            &source,
            r#"
module app

use model/{Box}

def main() Unit {
    box = Box {}
    box.increment()
    box.value := 3
}
"#,
        )
        .expect("write source");

        let result = check_path(&source).expect("typecheck");
        let inaccessible = result
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.diagnostic.code == "inaccessible_member")
            .count();
        assert_eq!(inaccessible, 2, "{:#?}", result.diagnostics);
        let _ = fs::remove_dir_all(&temp);
    }

    #[test]
    fn keeps_implicit_class_constructor_contract_stable_across_modules() {
        let temp = workspace_root().join("rust/target/typecheck-stable-constructor-test");
        let _ = fs::remove_dir_all(&temp);
        fs::create_dir_all(&temp).expect("create temp dir");
        fs::write(
            temp.join("model.lum"),
            r#"
module model

public class Account {
    owner Str
    internal region Str = "US"
    balance Int
    private cache Int = 0
}

public def localAccount() Account = Account("local", 1)
"#,
        )
        .expect("write model");
        let source = temp.join("app.lum");
        fs::write(
            &source,
            r#"
module app

use model/{Account, localAccount}

def main() Unit {
    _ Account = localAccount()
    _ Account = Account("external", 2)
    _ Account = Account { owner: "named", balance: 3 }
}
"#,
        )
        .expect("write source");

        let result = check_path(&source).expect("typecheck");
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
        let _ = fs::remove_dir_all(&temp);
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
    fn rejects_extension_methods_reading_private_fields() {
        let program = parse_inline(
            r#"
class User {
    private token Str = "secret"
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
    fn rejects_extension_methods_calling_private_methods() {
        let program = parse_inline(
            r#"
class User {
    name Str

    private def secret() Str = this.name
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
    fn selects_public_overload_when_same_named_overload_is_private() {
        let program = parse_inline(
            r#"
class RecordAggregator {
    private def get_rollup(key Str, range Int?) Int = 2
    def get_rollup(key Str) Int = 1
}

def main() Int {
    aggregator = RecordAggregator {}
    aggregator.get_rollup("alice")
}
"#,
        );
        let result = check_program(&program);
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
    private map Map[Int, Str] = Map()
    private var currentTick Int = 0
    private queue [Str] = []
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
    fn rejects_implicit_constructor_when_non_public_field_lacks_initializer() {
        let program = parse_inline(
            r#"
class SecretUser {
    name Str
    private token Str
    internal region Str
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.iter().any(|diag| {
            diag.code == "missing_non_public_field_initializer"
                && diag.message.contains(
                    "private field 'token' has no initializer; add an initializer or declare 'new(...)'",
                )
        }), "{:#?}", result.diagnostics);
        assert!(result.diagnostics.iter().any(|diag| {
            diag.code == "missing_non_public_field_initializer"
                && diag.message.contains(
                    "internal field 'region' has no initializer; add an initializer or declare 'new(...)'",
                )
        }), "{:#?}", result.diagnostics);
    }

    #[test]
    fn allows_explicit_constructor_when_private_field_lacks_initializer() {
        let program = parse_inline(
            r#"
class SecretUser {
    name Str
    private token Str

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
    private token Str

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
    private score Int = 5

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
    fn allows_intermingled_defaults_for_constructor_inputs() {
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

shape Response {
    status Int = 200
    body Str
}

type Outcome =
    shape Success {
        code Int = 200
        message Str
    }
    | object Cancelled {}

def main() Unit {
    _ Page = Page("custom body", "Intro")
    _ Page = Page { title: "Intro" }
    _ Article = Article("custom body", "Intro")
    _ Article = Article { title: "Intro" }
    _ Response = Response(201, "created")
    _ Response = Response { body: "ok" }
    _ Outcome = Success(201, "created")
    _ Outcome = Success { message: "ok" }
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn rejects_positional_constructor_prefix_that_leaves_required_input_unset() {
        let program = parse_inline(
            r#"
class X1 {
    a Str = ""
    b Str
}

class X3 {
    a Str
    b Str = ""
    c Str
}

def main() Unit {
    _ X1 = X1("b")
    _ X3 = X3("a", "c")
}
"#,
        );
        let result = check_program(&program);
        assert_eq!(result.diagnostics.len(), 2, "{:#?}", result.diagnostics);
        assert!(result.diagnostics.iter().all(|diag| {
            diag.code == "invalid_argument_count"
                && diag.message.contains("required input")
                && diag.message.contains("remains unset")
                && diag.message.contains("declaration-order prefix")
                && diag.message.contains("do not skip defaulted input")
        }));
    }

    #[test]
    fn allows_named_callable_arguments_to_omit_intermingled_defaults() {
        let program = parse_inline(
            r#"
def connect(protocol Str = "https", host Str, port Int = 443) Str =
    protocol + "://" + host + ":" + port.toStr()

class Client {
    def connect(protocol Str = "https", host Str, port Int = 443) Str =
        protocol + "://" + host + ":" + port.toStr()
}

def main() Unit {
    _ Str = connect("http", "example.com")
    _ Str = connect(host = "example.com")
    client = Client {}
    _ Str = client.connect("http", "example.com")
    _ Str = client.connect(host = "example.com", port = 8443)
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn rejects_positional_callable_prefix_that_leaves_required_parameter_unset() {
        let program = parse_inline(
            r#"
def connect(protocol Str = "https", host Str, port Int = 443) Str = host

class Client {
    def connect(protocol Str = "https", host Str, port Int = 443) Str = host
}

def main() Unit {
    _ Str = connect("example.com")
    client = Client {}
    _ Str = client.connect("example.com")
}
"#,
        );
        let result = check_program(&program);
        assert_eq!(result.diagnostics.len(), 2, "{:#?}", result.diagnostics);
        assert!(
            result.diagnostics.iter().all(|diag| {
                diag.code == "invalid_argument_count"
                    && diag
                        .message
                        .contains("required parameter 'host' remains unset")
                    && diag.message.contains("named argument")
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
    OS.println(mapped.size)
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
    private var values [Str: Int] = []

    new() {}

    def currentValue() Int = 7

    def store(key Str) Unit {
        values[key] := currentValue()
        values[key] += 5
        values[key] -= 2
        values[key] *= 3
        values[key] /= 2
        values[key] %= 4
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
def consume(value [Int]?) Unit = ()

def main() Unit {
    number Option[Int] = Option.when(true, 7)
    optional [Int]? = Option.when(true, [])
    consume(Option.when(true, []))
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
    fn numeric_static_api_is_declared_on_same_named_objects() {
        let ambient = default_inline_ambient();
        for name in ["Int", "Float"] {
            let primitive = ambient
                .types
                .get(name)
                .unwrap_or_else(|| panic!("missing primitive interface {name}"));
            assert!(
                !primitive.methods.contains_key("parse"),
                "{name}.parse must not be an instance method"
            );

            let static_object = ambient
                .objects
                .get(name)
                .unwrap_or_else(|| panic!("missing same-named object {name}"));
            assert!(static_object.methods.contains_key("parse"));
            assert!(static_object.methods.contains_key("MIN"));
            assert!(static_object.methods.contains_key("MAX"));
        }
    }

    #[test]
    fn allows_intrinsic_numeric_limit_members() {
        let program = parse_inline(
            r#"
def main() Unit {
    smallestInt Int = Int.MIN
    largestInt Int = Int.MAX
    mostNegativeFloat Float = Float.MIN
    largestFloat Float = Float.MAX
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn allows_get_as_a_contextual_local_name() {
        let program = parse_inline(
            r#"
def identity(get Int) Int = get

def main() Unit {
    get = identity(41)
    named Int = identity(get = 42)
    println(get)
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn rejects_primitive_types_as_implementation_targets() {
        let program = parse_inline(
            r#"
class FakeInt with Int {}
shape FakeFloat with Float {}
interface FakeStr with Str {}
object FakeBool with Bool {}
annotation FakeRune with Rune {}

def invalid[T with Int](value T) Unit = ()

def main() Unit {
    fake = object with Str {}
}
"#,
        );
        let result = check_program(&program);
        let diagnostics = result
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "invalid_primitive_implementation")
            .collect::<Vec<_>>();
        assert_eq!(diagnostics.len(), 7, "{:#?}", result.diagnostics);
        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| diagnostic.message.contains("is closed")),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn primitive_constructor_error_recommends_conversion_apis() {
        let program = parse_inline(
            r#"
def main() Unit {
    converted = Int(3.7)
    contextual Int = new(3.7)
}
"#,
        );
        let result = check_program(&program);
        let diagnostic = result
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.code == "invalid_primitive_conversion")
            .expect("primitive construction should be rejected");
        assert!(diagnostic.message.contains("cannot convert"));
        assert!(
            diagnostic
                .helps
                .iter()
                .any(|help| help.contains("Int.parse"))
        );
        assert!(
            diagnostic
                .helps
                .iter()
                .all(|help| !help.contains("object with"))
        );
        assert!(
            result.diagnostics.iter().any(|diagnostic| {
                diagnostic.code == "invalid_contextual_construction"
                    && diagnostic.message.contains("value.toInt()")
            }),
            "{:#?}",
            result.diagnostics
        );
        assert!(
            result
                .diagnostics
                .iter()
                .all(|diagnostic| !diagnostic.message.contains("object with Int")),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn intrinsic_static_parse_is_not_an_instance_method() {
        let program = parse_inline(
            r#"
def main() Unit {
    invalid = (1).parse("2")
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
    fn allows_array_fill_for_primitive_defaults() {
        let program = parse_inline(
            r#"
def main() Unit {
    nul Rune = "\0".runeAt(0)!
    ints Array[Int] = Array.fill(3, 0)
    floats Array[Float] = Array.fill(3, 0.0)
    bools Array[Bool] = Array.fill(3, false)
    texts Array[Str] = Array.fill(3, "")
    runes Array[Rune] = Array.fill(3, nul)
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
def consume(values Array[[Int]]) Unit = ()

def main() Unit {
    values Array[Int] = Array.fill(3, 7)
    nested Array[[Int]] = Array.fill(2, [])
    consume(Array.fill(2, []))
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn rejects_builtin_factory_payloads_incompatible_with_expected_types() {
        let program = parse_inline(
            r#"
def main() Unit {
    array Array[[Int]] = Array.fill(2, ["wrong"])
    optional [Int]? = Option.when(true, ["wrong"])
}
"#,
        );
        let result = check_program(&program);
        assert_eq!(
            result
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.code == "invalid_binding_type")
                .count(),
            2,
            "{:#?}",
            result.diagnostics
        );
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
    fn allows_vector_fill_and_generate_static_helpers() {
        let program = parse_inline(
            r#"
def consume(values [[Int]]) Unit = ()

def main() Unit {
    filled [Int] = Vector.fill(3, 7)
    generated [Int] = Vector.generate(3, idx => idx + 1)
    nestedFill [[Int]] = Vector.fill(2, [])
    nestedGenerate [[Int]] = Vector.generate(2, idx => [])
    consume(Vector.fill(2, []))
    consume(Vector.generate(2, idx => []))
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
    assert(condition = true)
    assert(message = "still true", condition = true)
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn validates_assert_with_the_ordinary_callable_contract() {
        let program = parse_inline(
            r#"
def main() Unit {
    assert(misspelled = true)
    assert(true, "first", "second")
}
"#,
        );
        let result = check_program(&program);
        assert_eq!(result.diagnostics.len(), 2, "{:#?}", result.diagnostics);
        assert!(
            result.diagnostics.iter().any(|diag| {
                diag.code == "unknown_argument_name"
                    && diag.message.contains("no parameter named 'misspelled'")
            }),
            "{:#?}",
            result.diagnostics
        );
        assert!(
            result.diagnostics.iter().any(|diag| {
                diag.code == "invalid_argument_count"
                    && diag.message.contains("expects 1..2 arguments, got 3")
            }),
            "{:#?}",
            result.diagnostics
        );
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
    fn rejects_annotation_mutable_and_private_fields() {
        let program = parse_inline(
            r#"
annotation Route {
    private path Str
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
                    && diag.message.contains("private field 'path'")),
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
    fn allows_list_remove_first_and_seeded_fold() {
        let program = parse_inline(
            r#"
def main() Unit {
    values = Vector(1, 2, 3)
    removed Option[Int] = values.removeFirst()
    total Int = values.fold(0, (acc, value) => acc + value)
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn rejects_removed_collection_aliases() {
        let program = parse_inline(
            r#"
def main() Unit {
    values = Vector(1, 2, 3)
    values.append(4)
    println(values.head)
    println(values.reduce(0, (acc, value) => acc + value))
}
"#,
        );
        let result = check_program(&program);
        assert!(
            result.diagnostics.iter().any(|diagnostic| {
                diagnostic.code == "unknown_member" && diagnostic.message.contains("append")
            }),
            "{:#?}",
            result.diagnostics
        );
        assert!(
            result.diagnostics.iter().any(|diagnostic| {
                diagnostic.code == "unknown_member" && diagnostic.message.contains("head")
            }),
            "{:#?}",
            result.diagnostics
        );
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "invalid_argument_count"),
            "{:#?}",
            result.diagnostics
        );
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
                        .contains("argument has type 'Int' but parameter expects 'Bool'")
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
    _ User = User { name: "Cara" }
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
    fn reports_private_field_constructor_with_factory_help() {
        let program = parse_inline(
            r#"
class User {
    name Str

    private new(name Str) {
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
            Some("field construction is private")
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
    private var count Int = 0
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
    private var count Int = 0

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
    fn allows_parenthesized_anonymous_shape_constructor_arguments() {
        let program = parse_inline(
            r#"
class Holder {
    payload { x Int }
}

def main() Unit {
    payload = { x: 7 }
    fromValue Holder = Holder(payload)
    fromLiteral Holder = Holder({ x: 8 })
    println(fromValue.payload.x, fromLiteral.payload.x)
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn allows_anonymous_shape_fields_typed_by_context() {
        let program = parse_inline(
            r#"
def main() Unit {
    value = "Ada"
    user { name Str, age Int } = {
        name: value
        age: 42
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
    fn allows_field_punning_in_named_and_anonymous_construction() {
        let program = parse_inline(
            r#"
shape Point {
    x Int
    y Int
}

class ClassPoint {
    x Int
    y Int
}

def main() Unit {
    x = 4
    y = 5
    named Point = Point { x, y }
    instance ClassPoint = ClassPoint { x, y }
    anonymous { x Int, y Int } = new { x, y }
    explicit Point = Point { x, y }
    OS.println(named.x + instance.y + anonymous.x + explicit.y)
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn field_punning_matches_anonymous_shape_fields_by_name() {
        let program = parse_inline(
            r#"
def main() Unit {
    x = 4
    y = 5
    point { x Int, z Int } = new { x, y }
}
"#,
        );
        let result = check_program(&program);
        assert!(
            result.diagnostics.iter().any(|diagnostic| {
                diagnostic.code == "invalid_binding_type"
                    && diagnostic.message.contains("y Int")
                    && diagnostic.message.contains("z Int")
            }),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn accepts_explicit_member_expressions_in_construction_fields() {
        let accepted = parse_inline(
            r#"
shape StoredRollup {
    count Int
    total Int
    internalId Int
}

shape Rollup {
    count Int
    total Int
}

def main() Unit {
    source = StoredRollup { count: 2, total: 7, internalId: 99 }
    named Rollup = Rollup { count: source.count, total: source.total }
    explicit Rollup = { count: source.count, total: source.total }
    contextual Rollup = new { count: source.count, total: source.total }
}
"#,
        );
        let accepted_result = check_program(&accepted);
        assert!(
            accepted_result.diagnostics.is_empty(),
            "{:#?}",
            accepted_result.diagnostics
        );
    }

    #[test]
    fn allows_anonymous_shape_spread_copy_and_extend() {
        let program = parse_inline(
            r#"
def main() Unit {
    base = { name: "Ada", age: 10 }
    aged = { ...base, age: 42 }
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
    fn allows_spread_copy_for_named_shapes() {
        let program = parse_inline(
            r#"
shape Amount {
    value Int
}

def main() Unit {
    original = Amount(10)
    updated Amount = { ...original, value: 42 }
    value Int = updated.value
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn shape_spread_literals_receive_field_types_from_the_target() {
        let program = parse_inline(
            r#"
shape Point {
    x Int
    y Int
}

shape Batch {
    items [Int]
    position Point
}

def main() Unit {
    original = Batch([1, 2], Point(3, 4))
    updated Batch = {
        ...original
        items: []
        position: new(0, 0)
    }
    items [Int] = updated.items
    position Point = updated.position
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn shape_fields_are_shallowly_read_only() {
        let program = parse_inline(
            r#"
shape Batch {
    items [Int]
}

def main() Unit {
    batch Batch = Batch([1])
    batch.items.add(42)
    batch.items := [3]
}
"#,
        );
        let result = check_program(&program);
        assert_eq!(result.diagnostics.len(), 1, "{:#?}", result.diagnostics);
        assert!(
            result.diagnostics.iter().any(|diagnostic| {
                diagnostic.code == "assign_immutable"
                    && diagnostic
                        .message
                        .contains("cannot reassign read-only field 'items'")
            }),
            "{:#?}",
            result.diagnostics
        );
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
                    && diag.message.contains("select a value explicitly")
            }),
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
    fn ignores_initialized_non_public_fields_in_implicit_constructor_order() {
        let program = parse_inline(
            r#"
class Account {
    name Str
    internal region Str = "US"
    age Int
    private score Int = 5
}

def main() Unit {
    _ Account = Account("Ada", 10)
    _ Account = Account { name: "Ada", age: 10 }
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
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
                        .contains("tuple pattern requires a tuple value")
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
    fn named_class_destructuring_skips_private_fields() {
        let program = parse_inline(
            r#"
class SecretUser {
    name Str
    private token Str
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
                diag.code == "unknown_pattern_field"
                    && diag.message.contains("has no visible field 'missing'")
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
    fn allows_named_anonymous_shape_assignment() {
        let program = parse_inline(
            r#"
def main() Int {
    point { x Int, y Int } = { x: 4, y: 5 }
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
    fn allows_contextual_new_for_named_shape() {
        let program = parse_inline(
            r#"
shape Point {
    x Int
    y Int
}

def main() Unit {
    point Point = new(4, 5)
    anonymous { x Int, y Int } = new(4, 5)
    reversed { y Int, x Int } = new(4, 5)
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn allows_contextual_new_for_class_and_named_alias() {
        let program = parse_inline(
            r#"
class Worker {
    name Str
    age Int
}

shape Point {
    x Int
    y Int
}

type Position = Point

def main() Unit {
    worker Worker = new("Ada", 42)
    point Position = new { x: 4, y: 5 }
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn checks_nonrecursive_default_initialization() {
        let program = parse_inline(
            r#"
class RetrySettings {
    attempts Int = 3
}

class ExplicitSettings {
    attempts Int

    new(attempts Int = 4) {
        this.attempts = attempts
    }
}

shape NestedDefaults {
    value Str = "nested"
}

shape Point {
    x Int
    y Int
}

shape Settings {
    label Str = "kept"
    enabled Bool
    retries RetrySettings
    explicit ExplicitSettings
    nested NestedDefaults
    note Str?
    files [Str]
    counts [Str: Int]
}

def main() Unit {
    settings Settings = default {}
    overridden Settings = default { label: "changed" }
    anonymous { enabled Bool, files [Str] } = default {}
    empty {} = default {}
    point Point = default { x: 1, y: 2 }
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn rejects_default_initialization_that_would_require_recursive_derivation() {
        let program = parse_inline(
            r#"
shape Point {
    x Int
    y Int
}

shape State {
    origin Point
    files [Str]
}

def main() Unit {
    state State = default {}
}
"#,
        );
        let result = check_program(&program);
        assert!(
            result.diagnostics.iter().any(|diagnostic| {
                diagnostic.code == "invalid_default_initialization"
                    && diagnostic
                        .message
                        .contains("field 'origin' requires an explicit value")
                    && diagnostic.message.contains("Point")
            }),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn checks_empty_braces_from_expression_context() {
        let program = parse_inline(
            r#"
class EmptyClass {}

class DefaultedClass {
    value Int = 7

    def noop() {}
}

class ExplicitEmpty {
    new() {}
}

shape EmptyShape {}

def noop() Unit {}
def noop2() = ()

def makeEmpty() EmptyClass = new {}

def accept(value EmptyClass) Unit = ()

def main() Unit {
    instance EmptyClass = new {}
    defaulted DefaultedClass = new {}
    explicit = ExplicitEmpty()
    marker EmptyShape = {}
    values Vector[Str] = new {}
    lookup Map[Str, Int] = new {}
    set Set[Str] = new {}
    nothing Unit = ()
    anonymous {} = new {}
    structural {} = {}
    defaultedStructural {} = default {}
    widened Any = anonymous
    accept(new {})
    result Unit = noop2()
    methodResult Unit = defaulted.noop()
    callback fn() Unit = () => ()
    shapeCallback fn() {} = () => new {}
    callbackResult = { ...shapeCallback(), value: 1 }
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn rejects_empty_braces_as_unit_expressions() {
        let program = parse_inline(
            r#"
class Legacy {
    new() = {}

    def method() Unit = {}
}

def inferred() = {}
def explicit() Unit = {}

def main() Unit {
    nothing Unit = {}
    callback fn() Unit = () => {}
}
"#,
        );
        let result = check_program(&program);
        let diagnostics = result
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "missing_empty_construction_target")
            .collect::<Vec<_>>();
        assert_eq!(diagnostics.len(), 6, "{:#?}", result.diagnostics);
        assert!(diagnostics.iter().all(|diagnostic| {
            diagnostic
                .helps
                .iter()
                .any(|help| help.contains("use '()' for Unit"))
        }));
    }

    #[test]
    fn rejects_empty_construction_without_a_concrete_target() {
        let program = parse_inline(
            r#"
interface Marker {}

def main() Unit {
    untyped = {}
    forced = new {}
    widened Any = {}
    forcedWidened Any = new {}
    abstract Marker = {}
    forcedAbstract Marker = new {}
}
"#,
        );
        let result = check_program(&program);
        let diagnostics = result
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "missing_empty_construction_target")
            .collect::<Vec<_>>();
        assert_eq!(diagnostics.len(), 6, "{:#?}", result.diagnostics);
        assert!(diagnostics.iter().all(|diagnostic| {
            diagnostic
                .message
                .contains("requires an expected concrete type")
                && diagnostic
                    .helps
                    .iter()
                    .any(|help| help.contains("use '()' for Unit"))
        }));
    }

    #[test]
    fn empty_braces_do_not_bypass_required_construction_or_turn_bodies_into_values() {
        let program = parse_inline(
            r#"
class Required {
    value Int
}

def constructed() Required = new {}

def body() Required {}
"#,
        );
        let result = check_program(&program);
        assert!(
            result.diagnostics.iter().any(|diagnostic| {
                diagnostic.code == "no_matching_overload"
                    && diagnostic
                        .message
                        .contains("missing required field 'value'")
            }),
            "{:#?}",
            result.diagnostics
        );
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "invalid_return_type"),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn allows_field_braces_and_forced_braces_across_contexts() {
        let program = parse_inline(
            r#"
shape Point {
    x Int
    y Int
}

class User {
    name Str
    age Int

    new(name Str, age Int) {
        this.name = name
        this.age = age
    }
}

shape Rollup {
    count Int
    total Int
}

def makePoint(x Int, y Int) Point = {
    x: x
    y: y
}

def save(user User) Unit = ()

def main() Unit {
    x = 4
    y = 5
    name = "Ada"
    age = 42

    point Point = { x: 1, y: 2 }
    punned Point = new { x, y }
    user User = User { name: "Ada", age: 42 }
    other User = new { name, age }
    _ Point = makePoint(6, 7)
    save(new { name: "Ben", age: 30 })

    rollups [Str: Rollup] = []
    rollups["main"] := new { count: 0, total: 0 }

    anonymous = new { x, y }
    widened Any = new { x, y }
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn distinguishes_fresh_class_construction_from_existing_shape_assignment() {
        let program = parse_inline(
            r#"
class User {
    name Str
}

shape UserData {
    name Str
}

def main() Unit {
    fresh User = new { name: "Ada" }
    data = UserData { name: "Grace" }
    explicit User = User { ...data }
    existing User = data
}
"#,
        );
        let result = check_program(&program);
        assert_eq!(result.diagnostics.len(), 1, "{:#?}", result.diagnostics);
        assert!(
            result.diagnostics.iter().any(|diagnostic| {
                diagnostic.code == "invalid_binding_type"
                    && diagnostic
                        .message
                        .contains("cannot assign value of type 'UserData'")
                    && diagnostic
                        .message
                        .contains("to binding 'existing' of type 'User'")
            }),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn allows_wider_construction_spreads_but_rejects_extra_explicit_fields() {
        let program = parse_inline(
            r#"
shape Point {
    x Int
    y Int
}

def main() Unit {
    larger = { x: 1, y: 2, yy: 3 }
    projected Point = { ...larger }
    forcedProjection Point = new { ...larger }

    typo Point = { x: 1, y: 2, yy: 3 }
    forcedTypo Point = new { x: 1, y: 2, yy: 3 }
}
"#,
        );
        let result = check_program(&program);
        let unknown_fields = result
            .diagnostics
            .iter()
            .filter(|diagnostic| {
                diagnostic.code == "no_matching_overload"
                    && diagnostic
                        .message
                        .contains("has no public constructor field 'yy'")
            })
            .count();
        assert_eq!(unknown_fields, 2, "{:#?}", result.diagnostics);
        assert_eq!(result.diagnostics.len(), 2, "{:#?}", result.diagnostics);
    }

    #[test]
    fn anonymous_shape_targets_reject_extra_explicit_fields_but_allow_projection() {
        let program = parse_inline(
            r#"
def main() Unit {
    wider = { x: 1, y: 2, z: 3 }
    assigned { x Int, y Int } = wider
    spread { x Int, y Int } = { ...wider }

    typo { x Int, y Int } = { x: 1, y: 2, yy: 3 }
    forcedTypo { x Int, y Int } = new { x: 1, y: 2, yy: 3 }
}
"#,
        );
        let result = check_program(&program);
        let unknown_fields = result
            .diagnostics
            .iter()
            .filter(|diagnostic| {
                diagnostic.code == "invalid_contextual_construction"
                    && diagnostic
                        .message
                        .contains("anonymous shape target has no construction field 'yy'")
            })
            .count();
        assert_eq!(unknown_fields, 2, "{:#?}", result.diagnostics);
        assert_eq!(result.diagnostics.len(), 2, "{:#?}", result.diagnostics);
    }

    #[test]
    fn shape_assignment_does_not_apply_target_field_initializers() {
        let program = parse_inline(
            r#"
shape Small {
    x Int
}

shape Full {
    x Int
    y Int = 42
}

def main() Unit {
    source = Small(1)
    invalid Full = source
    constructed Full = Full { ...source }
    _ Int = constructed.y
}
"#,
        );
        let result = check_program(&program);
        assert_eq!(result.diagnostics.len(), 1, "{:#?}", result.diagnostics);
        assert!(
            result.diagnostics.iter().any(|diagnostic| {
                diagnostic.code == "invalid_binding_type"
                    && diagnostic
                        .message
                        .contains("cannot assign value of type 'Small'")
                    && diagnostic
                        .message
                        .contains("to binding 'invalid' of type 'Full'")
            }),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn allows_optional_new_for_shapes_and_requires_it_for_contextual_classes() {
        let program = parse_inline(
            r#"
shape Point {
    x Int
    y Int
}

class User {
    name Str
}

def main() Unit {
    point Point = { x: 1, y: 2 }
    explicitPoint Point = new { x: 3, y: 4 }
    user User = User { name: "Ada" }
    explicitUser User = new { name: "Grace" }
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn rejects_bare_field_braces_for_class_construction() {
        let program = parse_inline(
            r#"
class User {
    name Str
}

shape Profile {
    name Str
}

class Team {
    leader User
}

def makeInvalid() User = { name: "Return" }

def accept(user User) Unit = ()

def main() Unit {
    profile Profile = { name: "Ada" }
    contextual User = new { name: "Ada" }
    explicit User = User { name: "Ada" }
    invalid User = { name: "Ada" }
    accept({ name: "Argument" })
    team Team = new { leader: { name: "Nested" } }
    users [Str: User] = []
    users["indexed"] := { name: "Indexed" }
}
"#,
        );
        let result = check_program(&program);
        assert_eq!(result.diagnostics.len(), 5, "{:#?}", result.diagnostics);
        assert!(
            result.diagnostics.iter().all(|diagnostic| {
                diagnostic.code == "class_construction_requires_new"
                    && diagnostic
                        .message
                        .contains("bare field braces cannot construct class 'User'")
                    && diagnostic.message.contains("new { ... }")
                    && diagnostic.message.contains("User { ... }")
            }),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn rejects_parenthesized_contextual_new_for_non_constructible_expected_types() {
        let program = parse_inline(
            r#"
interface Reader {
    def read() Str
}

class Success {}
class Failure {}

def generic[T]() T = new()

def main() Unit {
    reader Reader = new()
    anything Any = new()
    outcome Success | Failure = new()
}
"#,
        );
        let result = check_program(&program);
        let diagnostics = result
            .diagnostics
            .iter()
            .filter(|diag| diag.code == "invalid_contextual_construction")
            .collect::<Vec<_>>();
        assert_eq!(diagnostics.len(), 4, "{:#?}", result.diagnostics);
        assert!(
            diagnostics
                .iter()
                .any(|diag| diag.message.contains("object with Reader")),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn rejects_forced_braces_for_interface_and_union_targets() {
        let program = parse_inline(
            r#"
interface Named {
    get name Str
}

class Success { message Str }
class Failure { message Str }

def main() Unit {
    value Named = new { name: "Ada" }
    result Success | Failure = new { message: "no target" }
}
"#,
        );
        let result = check_program(&program);
        assert!(
            result
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.code == "invalid_binding_type")
                .count()
                >= 2,
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn rejects_contextual_construction_that_would_choose_an_overload_target() {
        let program = parse_inline(
            r#"
class User { name Str }
class Admin { name Str }

class Saver {
    def save(user User) Unit = ()
    def save(admin Admin) Unit = ()
}

def main(ready Bool) Unit {
    saver = Saver()
    saver.save(new { name: "Ada" })
    saver.save((new { name: "Grace" }))
    saver.save(new("Linus"))
    saver.save((new("Margaret")))
    saver.save(if ready {
        new { name: "Barbara" }
    } else {
        new { name: "Evelyn" }
    })
    saver.save({
        new { name: "Annie" }
    })
}
"#,
        );
        let result = check_program(&program);
        assert_eq!(
            result
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.code == "ambiguous_contextual_construction")
                .count(),
            6,
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn allows_explicit_construction_to_resolve_an_overload_target() {
        let program = parse_inline(
            r#"
class User { name Str }
class Admin { name Str }

class Saver {
    def save(user User) Unit = ()
    def save(admin Admin) Unit = ()
}

def main(ready Bool) Unit {
    saver = Saver()
    saver.save(User { name: "Ada" })
    saver.save(User("Grace"))
    saver.save(if ready {
        User { name: "Barbara" }
    } else {
        new { name: "Evelyn" }
    })
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn rejects_contextual_new_without_expected_type() {
        let program = parse_inline(
            r#"
def main() Unit {
    point = new(4, 5)
}
"#,
        );
        let result = check_program(&program);
        assert!(
            result.diagnostics.iter().any(|diag| {
                diag.code == "invalid_contextual_construction"
                    && diag.message.contains("requires an expected class or shape")
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
    fn requires_explicit_construction_from_class_to_shape() {
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
    implicit Point = pixel
    explicit Point = { ...pixel }
    roundTrip Pixel = Pixel { ...explicit }
    explicit.x + explicit.y + roundTrip.x + roundTrip.y
}
"#,
        );
        let result = check_program(&program);
        assert_eq!(result.diagnostics.len(), 1, "{:#?}", result.diagnostics);
        assert!(
            result.diagnostics.iter().any(|diagnostic| {
                diagnostic.code == "invalid_binding_type"
                    && diagnostic
                        .message
                        .contains("cannot assign value of type 'Pixel'")
                    && diagnostic
                        .message
                        .contains("to binding 'implicit' of type 'Point'")
            }),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn allows_shape_with_interface_after_explicit_shape_projection() {
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
    view PointView = { ...pixel }
    named Named = view
    _ Str = named.label()
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn rejects_class_to_interface_through_shape_without_explicit_shape_projection() {
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
    fn rejects_contextual_new_with_wrong_field_type() {
        let program = parse_inline(
            r#"
shape Point {
    x Int
    y Str
}

def main() Unit {
    point Point = new { x: 4, y: 5 }
}
"#,
        );
        let result = check_program(&program);
        assert!(
            result.diagnostics.iter().any(|diag| {
                diag.code == "invalid_argument_type"
                    && diag.message.contains("field 'y' in shape 'Point'")
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
    fn rejects_incompatible_let_else_fallback() {
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
                diag.code == "invalid_let_else_fallback"
                    && diag
                        .message
                        .contains("fallback for 'item' has type 'Unit' but expects 'Int'")
            }),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn allows_single_value_and_named_shape_let_else_fallbacks() {
        let program = parse_inline(
            r#"
class User {
    name Str
    value Int
}

def main() Unit {
    maybePair Option[(Int, Int)] = None
    let Some { value as pair } = maybePair else (1, 2)

    candidate Any = 0
    let User { name, value } = candidate else {
        name = "fallback"
        value = 22
        new { name, value }
    }

    OS.println(pair, name, value)
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn rejects_positional_tuple_for_multiple_let_else_bindings() {
        let program = parse_inline(
            r#"
class User {
    name Str
    value Int
}

def main() Unit {
    candidate Any = 0
    let User { name, value } = candidate else ("fallback", 22)
    OS.println(name, value)
}
"#,
        );
        let result = check_program(&program);
        assert!(
            result.diagnostics.iter().any(|diag| {
                diag.code == "invalid_let_else_fallback"
                    && diag
                        .message
                        .contains("positional tuple fallbacks are not allowed")
            }),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn rejects_value_fallback_for_whole_pattern_alias_with_component_bindings() {
        let program = parse_inline(
            r#"
class User {
    name Str
}

def main() Unit {
    candidate Any = 0
    let User { name } as user = candidate else {
        name: "Unknown"
        user: User("Ada")
    }
    OS.println(name, user.name)
}
"#,
        );
        let result = check_program(&program);
        assert!(
            result.diagnostics.iter().any(|diag| {
                diag.code == "invalid_let_else_fallback"
                    && diag.message.contains(
                        "pattern alias 'user' binds a whole value and components inside it",
                    )
            }),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn allows_exiting_alias_fallback_and_nested_single_binding_value_fallback() {
        let program = parse_inline(
            r#"
class User {
    name Str
}

def read(candidate Any) Str {
    let User { name } as user = candidate else return "missing"
    name + user.name
}

def main() Unit {
    candidate Option[User] = None
    let Some { value: User { name } } = candidate else "Unknown"
    OS.println(name)
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
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
type MaybeInt =
    object NoneX {}
    | class SomeX { value Int }

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
type MaybeInt =
    object NoneX {}
    | class SomeX { value Int }

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
    fn allows_let_else_inside_explicitly_typed_local_function() {
        let program = parse_inline(
            r#"
class IntRange {
    start Int
    end Int
}

def contains(range Option[IntRange], value Int) Bool {
    def outside(candidate Int) Bool {
        let bounds <- range else return false
        candidate < bounds.start || candidate > bounds.end
    }

    !outside(value)
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn rejects_value_return_inside_implicit_unit_local_function() {
        let program = parse_inline(
            r#"
def contains(range Option[Int], value Int) Bool {
    def present() {
        let item <- range else return false
        item == value
    }

    true
}
"#,
        );
        let result = check_program(&program);
        assert!(
            result.diagnostics.iter().any(|diagnostic| {
                diagnostic.code == "invalid_return_type"
                    && diagnostic
                        .message
                        .contains("enclosing callable expects 'Unit'")
            }),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn unannotated_equals_block_local_function_returns_unit() {
        let program = parse_inline(
            r#"
def contains(range Option[Int], value Int) Bool {
    def report() = {
        let item <- range else return
        println(item == value)
    }

    report()
    true
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn rejects_value_body_when_return_type_is_omitted() {
        let program = parse_inline(
            r#"
def isMissing(value Option[Int]) = {
    value is None
}
"#,
        );
        let result = check_program(&program);
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "discarded_expression"),
            "{:#?}",
            result.diagnostics
        );
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
    fn allows_bare_zero_payload_union_variants() {
        let program = parse_inline(
            r#"
type MaybeInt =
    object Missing {}
    | class Present { value Int }

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
    fn materializes_bare_zero_payload_variant_patterns() {
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
    fn rejects_bare_union_variants_with_payloads() {
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
                diag.code == "bare_payload_pattern"
                    && diag
                        .message
                        .contains("bare case pattern 'Some' has a payload")
            }),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn allows_bare_zero_cases_singletons_and_whole_pattern_aliases() {
        let program = parse_inline(
            r#"
class User {
    name Str
}

type Status =
    object Pending {}
    | object Complete {}

type Payload =
    class Item { value Int }
    | object Empty {}

object Ready {}

def main(value Any, status Status, maybe Option[Int], payload Payload) Unit {
    match status {
        case Pending => ()
        case Complete => ()
    }
    match value {
        case User { name } as user => println(name, user.name)
        case _ as other => println(other)
    }
    match Ready {
        case Ready => ()
    }
    match maybe {
        case Some { value as item } as some => println(item, some.value)
        case None => ()
    }
    match payload {
        case Payload.Item { value as item } as whole => println(item, whole.value)
        case Payload.Empty => ()
    }
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn allows_union_variant_default_constructor_shapes() {
        let program = parse_inline(
            r#"
type Outcome =
    class Left {
        value Str
        tag Str = "left"
    }
    | object Empty {}

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
    fn materializes_bare_union_variant_in_expected_shape_field() {
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
    fn rejects_zero_payload_union_variant_call_syntax() {
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
                diag.code == "invalid_union_variant_call"
                    && diag
                        .message
                        .contains("union variant 'None' does not accept call syntax")
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
	    OS.println(mapped.size)
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
    fn rejects_refutable_pattern_in_short_for_yield_generator() {
        let program = parse_inline(
            r#"
def values(options [Int?]) [Int] =
    for let Some { value } <- options yield value
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

type Status =
    object Pending {}
    | object Complete {}

def main() Unit {
    user User = User("Ada")
    declared Type[User] = typeOf[User]
    actual Type[User] = user.runtimeType
    unknown Type[_] = declared
    anyMetadata Type[Any] = typeOf[Any]
    classType ClassType[User] = declared.asClass() !
    unknownClass ClassType[_] = classType
    unionType UnionType[Status] = typeOf[Status].asUnion() !
    fieldType Type[_] = (classType.fields.at(0) !).fieldType
    OS.println(actual.name !, unknown.name !, anyMetadata.kind, unknownClass.name !, unionType.name !, fieldType.name !)
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
    OS.println(values.size)
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
    OS.println(a.size, b.size, c.size, first, sameCapture)
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
def countMap(values [Str: Int]) Int = values.size
def countArray(values [Str]) Int = values.size
def consume(rows Iterable[[Int]]) Unit = ()
def emptyMap() [Str: Int] = []
def emptyArray() [Str] = []

def main() Int {
    directMap [Str: Int] = []
    directArray [Str] = []
    emptyIterable Iterable[[Int]] = []
    nestedVector [[Int]] = [[]]
    nestedIterable Iterable[[Int]] = [[]]
    consume([[]])
    return directMap.size + directArray.size + countMap([]) + countArray([]) + emptyMap().size + emptyArray().size
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn rejects_nested_vector_elements_incompatible_with_iterable_context() {
        let program = parse_inline(
            r#"
def main() Unit {
    rows Iterable[[Int]] = [["wrong"]]
}
"#,
        );
        let result = check_program(&program);
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "invalid_binding_type"),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn allows_declared_union_to_implement_iterable_through_shared_extension() {
        let program = parse_inline(
            r#"
type Maybe[T] with Iterable[T] =
    class Present { value T }
    | object Missing {}

ext Maybe[T] {
    def iterator() Iterator[T] = match this {
        case Present { value } => [value].iterator()
        case Missing => Vector[T]().iterator()
    }
}

def count(values Iterable[Int]) Int = {
    var result = 0
    for _ <- values {
        result += 1
    }
    result
}

def main() Int {
    present Maybe[Int] = Present(1)
    missing Maybe[Int] = Missing
    count(present) + count(missing)
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn allows_vector_flat_map_callback_to_return_option() {
        let program = parse_inline(
            r#"
def keepOdd(value Int) Int? = if value % 2 == 1 { Some(value) } else { None }

def main() [Int] =
    [1, 2, 3].flatMap(value => keepOdd(value))
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

    defaults [Str: Int] = ["one": 1, "shared": 2]
    overrides [Str: Int] = ["shared": 20, "three": 3]
    copy [Str: Int] = [...defaults]
    merged [Str: Int] = [...defaults, "two": 2, ...overrides]
    entries [(Str, Int)] = [...merged.entries()]

    println(values.size, copy.size, merged.size, entries.size)
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
    entries [Str: Int] = ["one": 1]
    mixed = [...values, ...entries]
    invalidList = [0, ...entries]
    invalidMap [Str: Int] = [...values]
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
    new(values [Str: Int]) {}
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

def fromOption(value Option[User]) Str = value!.name
def fromResult(value Result[Int, Str]) Int = value !
def fromEither(value Either[Str, Int]) Int = value !
def invoke(value Option[fn() Str]) Str = value!()
def indexed(value Option[[Str: Int]]) Int = value!["first"]!
def nested(value Option[Option[Int]]) Int = value!!

def main() Unit {
    values LinkedList[Int] = LinkedList()
    values.add(5)
    first Int = values.at(0) !
    println(first)
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn checks_option_wrap_as_some_only() {
        let program = parse_inline(
            r#"
shape Point {
    x Int
    y Int
}

def optionValue() Option[Int] = ^5
def nestedValue() Option[Option[Int]] = ^^9
def nestedNone() Option[Option[Int]] = ^None
def choose(flag Bool) Option[Int] = if flag { ^1 } else { ^2 }
def consume(value Option[Int]) Int = value ?? 0
def inferred(values [Int]) Int = values.flatMap(value => [value]).at(0) ?? 0

def main() Unit {
    inferred = ^1
    println(inferred! + 1)
    option Option[Int] = ^1
    point Point? = ^new(2, 3)
    result Result[Int, Str] = Ok(2)
    either Either[Str, Int] = Right(3)
    println(consume(^4))
    println(point!.x)
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn rejects_option_wrap_outside_option_contexts_and_wrong_payloads() {
        let program = parse_inline(
            r#"
def plainContext() Int = ^5
def resultContext() Result[Int, Str] = ^5
def eitherContext() Either[Str, Int] = ^5
def wrongValue() Option[Int] = ^"five"
"#,
        );
        let result = check_program(&program);
        assert_eq!(
            result
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.code == "invalid_return_type")
                .count(),
            3,
            "{:#?}",
            result.diagnostics
        );
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "invalid_optional_wrap_value"),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn rejects_option_pattern_shorthand_for_non_option_union() {
        let program = parse_inline(
            r#"
type Other =
    class Some { value Int }
    | object Missing {}

def read(value Other) Int = match value {
    case ^item => item
    case _ => 0
}
"#,
        );
        let result = check_program(&program);
        assert!(
            result.diagnostics.iter().any(|diagnostic| {
                diagnostic.code == "invalid_option_pattern"
                    && diagnostic.message.contains("requires an Option source")
            }),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn narrows_stable_identifier_inside_successful_is_branch() {
        let program = parse_inline(
            r#"
def textSize(value Any) Int {
    if value is Str {
        return value.size
    }
    0
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn narrows_logical_right_operands_and_successful_and_branches() {
        let program = parse_inline(
            r#"
def hasTextSize(value Any) Bool = value is Str && value.size == 4

def textSize(value Any, enabled Bool) Int {
    if enabled && value is Str && value.size == 4 {
        return value.size
    }
    0
}

def combinedSize(left Any, right Any) Int {
    if left is Str && right is Str && left.size > 0 && right.size > 0 {
        return left.size + right.size
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
    value.size
}

def textSizeIsNot(value Any) Int {
    if value is not Str {
        return 0
    }
    value.size
}

def textSizeInElse(value Any) Int {
    if value is not Str {
        return 0
    } else {
        return value.size
    }
}

def textSizeWithElse(value Any) Int {
    if value is Str {
    } else {
        return 0
    }
    value.size
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
        return value.size
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
    fn infers_generic_constructor_arguments_from_values_and_expected_types() {
        let program = parse_inline(
            r#"
class Box[T] {
    value T

    new(value T) {
        this.value = value
    }
}

def main() Unit {
    explicitSet Set[Str] = Set[Str]()
    contextualSet Set[Str] = Set()
    contextualNewSet Set[Str] = new()
    contextualBraceSet Set[Str] = new {}
    inferredSet = Set[Str]()
    anySet Set[Any] = Set[Any]()

    explicitMap Map[Str, Int] = Map[Str, Int]()
    contextualMap Map[Str, Int] = Map()
    contextualVector Vector[Int] = Vector()

    expectedBox Box[Str] = Box("hello")
    inferredBox = Box("world")
    explicitBox Box[Any] = Box[Any]("wide")
    contextualNew Box[Str] = new("new")

    println(
        explicitSet.size,
        contextualSet.size,
        contextualNewSet.size,
        contextualBraceSet.size,
        inferredSet.size,
        anySet.size,
        explicitMap.size,
        contextualMap.size,
        contextualVector.size,
        expectedBox.value,
        inferredBox.value,
        explicitBox.value,
        contextualNew.value
    )
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn rejects_contextual_collection_braces_with_named_fields() {
        let program = parse_inline(
            r#"
def main() Unit {
    values Set[Str] = new { item: "Ada" }
}
"#,
        );
        let result = check_program(&program);
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "no_matching_overload"),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn rejects_uninferable_partial_and_wildcard_generic_construction() {
        let program = parse_inline(
            r#"
def main() Unit {
    missing = Set()
    partial = Map[Str]()
    wildcard = Set[_]()
}
"#,
        );
        let result = check_program(&program);
        let codes = result
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.code)
            .collect::<Vec<_>>();
        assert!(
            codes.contains(&"cannot_infer_constructor_type"),
            "{:#?}",
            result.diagnostics
        );
        assert!(
            codes.contains(&"invalid_type_argument_count"),
            "{:#?}",
            result.diagnostics
        );
        assert!(
            codes.contains(&"invalid_constructor_type_arguments"),
            "{:#?}",
            result.diagnostics
        );
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
def invalid(value Int) Int = value !
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
    fn treats_unions_as_unordered_sets_and_allows_widening() {
        let program = parse_inline(
            r#"
class Cat {}
class Dog {}
class Bird {}

type Pet = Cat | Dog

def fromAlias(value Pet) Dog | Cat = value
def widen(value Cat | Dog) Cat | Dog | Bird = value
def reorderAndWiden(value Cat | Dog) Bird | Dog | Cat = value
def flatten(value Cat | Dog) Cat | (Dog | Cat) = value
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn allows_general_transparent_type_aliases() {
        let program = parse_inline(
            r#"
shape User {
    name Str
}

class Cat {}
class Dog {}

type UserId = Int
type Handler = fn(UserId) Str
type Users = [User]
type Scores = [Str: Int]
type Companion = Pet
type Pet = Cat | Dog

def apply(handler Handler, id UserId) Str = handler(id)
def companion(value Companion) Pet = value

def main() Unit {
    id UserId = 7
    handler Handler = (value Int) => value.toStr()
    users Users = [User("Ada")]
    scores Scores = ["Ada": 10]
    pet Companion = Cat()

    println(apply(handler, id))
    println(users[0].name)
    println(scores["Ada"]!)
    _ = companion(pet)
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn allows_anonymous_shape_types_in_inline_positions() {
        let program = parse_inline(
            r#"
shape Session {
    position Str
    start Int
    end Int

    def duration() Int = this.end - this.start
}

def project(value { position Str, start Int }) { position Str } =
    { position: value.position }

def copy(value { position Str }) { position Str } = value

def main() Unit {
    session Session = Session("engineer", 10, 30)
    view { position Str } = project(session)
    copied { position Str } = copy(view)
    println(copied.position, session.duration())
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn rejects_union_assignment_when_a_source_alternative_is_uncovered() {
        let program = parse_inline(
            r#"
class Cat {}
class Dog {}
class Bird {}

def narrow(value Cat | Dog) Dog | Bird = value
"#,
        );
        let result = check_program(&program);
        assert!(
            result.diagnostics.iter().any(|diagnostic| {
                diagnostic.message.contains("Cat | Dog")
                    && diagnostic.message.contains("Bird | Dog")
            }),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn rejects_ambiguous_shape_projection_into_union() {
        let program = parse_inline(
            r#"
shape XY {
    x Int
    y Int
}

shape XZ {
    x Int
    z Int
}

shape XYZ {
    x Int
    y Int
    z Int
}

def main() Unit {
    original = XYZ(1, 2, 3)
    selected XY | XZ = original
    reordered XZ | XY = original
}
"#,
        );
        let result = check_program(&program);
        assert_eq!(
            result
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.code == "ambiguous_shape_union_projection")
                .count(),
            2,
            "{:#?}",
            result.diagnostics
        );
        let diagnostic = result
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.code == "ambiguous_shape_union_projection")
            .expect("ambiguous projection diagnostic");
        assert!(diagnostic.message.contains("XYZ"), "{diagnostic:#?}");
        assert!(diagnostic.message.contains("XY"), "{diagnostic:#?}");
        assert!(diagnostic.message.contains("XZ"), "{diagnostic:#?}");
        assert!(
            diagnostic
                .message
                .contains("construct the intended shape explicitly"),
            "{diagnostic:#?}"
        );
    }

    #[test]
    fn selects_exact_or_unique_shape_projection_into_union() {
        let program = parse_inline(
            r#"
shape X {
    x Int
}

shape XY {
    x Int
    y Int
}

shape XZ {
    x Int
    z Int
}

shape XYZ {
    x Int
    y Int
    z Int
}

shape Position {
    y Int
    x Int
}

shape Label {
    label Str
}

def project(value XYZ | Label) XY | Label = value

def main() Unit {
    original = XYZ(1, 2, 3)
    unique XY | Label = original
    exact XY | X = Position(2, 1)
    resolved XY | XZ = XY { ...original }
    dynamic XY | Label = project(original)
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn rejects_cyclic_type_aliases() {
        let program = parse_inline(
            r#"
type First = Second
type Second = First
"#,
        );
        let result = check_program(&program);
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "cyclic_type_alias"),
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
                    && diag.message.contains("postfix '!'")
            })
            .count();
        assert_eq!(removed, 3, "{:#?}", result.diagnostics);
    }

    #[test]
    fn rejects_removed_get_or_methods_on_lifted_values() {
        let program = parse_inline(
            r#"
def fromOption(value Option[Int]) Int = value.getOr(0)
def fromResult(value Result[Int, Str]) Int = value.getOr(0)
def fromEither(value Either[Str, Int]) Int = value.getOr(0)
"#,
        );
        let result = check_program(&program);
        let removed = result
            .diagnostics
            .iter()
            .filter(|diag| {
                diag.code == "unknown_member"
                    && diag.message.contains("method 'getOr' was removed")
                    && diag.message.contains("?? fallback")
            })
            .count();
        assert_eq!(removed, 3, "{:#?}", result.diagnostics);
    }

    #[test]
    fn rejects_removed_panic_only_helpers() {
        let program = parse_inline(
            r#"
def rune(text Str) Rune = text.expectRuneAt(0)
def reflection(method Method, receiver Any) Any = method.invoke(receiver)
"#,
        );
        let result = check_program(&program);
        for removed in ["expectRuneAt", "invoke"] {
            assert!(
                result.diagnostics.iter().any(|diagnostic| {
                    diagnostic.code == "unknown_member" && diagnostic.message.contains(removed)
                }),
                "missing diagnostic for removed helper '{removed}': {:#?}",
                result.diagnostics
            );
        }
    }

    #[test]
    fn checks_getter_access_as_a_value() {
        let program = parse_inline(
            r#"
interface Named {
    get label Str
}

class Item with Named {
    name Str

    get label Str = this.name
    get repeated Str = label + this.label
}

shape Point {
    x Int
    y Int

    get total Int = this.x + this.y
}

def main() Unit {
    item Item = Item("Ada")
    point Point = Point(2, 3)
    label Str = item.label
    total Int = point.total
    println(label, total, item.repeated)
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn checks_getters_with_collection_function_and_anonymous_shape_return_types() {
        let program = parse_inline(
            r#"
interface Accessors {
    get counts [Str: Int]
    get rows [[Int]]
    get optionalValues [Int?]
    get callbacks [fn(Int) Int]
    get position { x Int, y Int }
}

class Values with Accessors {
    private storedCounts [Str: Int] = []
    private storedRows [[Int]] = []
    private storedOptionalValues [Int?] = []
    private storedCallbacks [fn(Int) Int] = []

    get counts [Str: Int] = this.storedCounts
    get rows [[Int]] = this.storedRows
    get optionalValues [Int?] = this.storedOptionalValues
    get callbacks [fn(Int) Int] = this.storedCallbacks
    get position { x Int, y Int } = { x: 1, y: 2 }
}

def read(values Accessors) Unit {
    counts [Str: Int] = values.counts
    rows [[Int]] = values.rows
    optionalValues [Int?] = values.optionalValues
    callbacks [fn(Int) Int] = values.callbacks
    position { x Int, y Int } = values.position
    println(counts.size, rows.size, optionalValues.size, callbacks.size, position.x)
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn rejects_getters_that_return_unit_directly_or_through_an_alias() {
        let program = parse_inline(
            r#"
type Nothing = Unit

interface InvalidAccessors {
    get ignored Unit
}

class Worker {
    get cleanup Unit = ()
}

shape Marker {
    value Int

    get empty Nothing = ()
}
"#,
        );
        let result = check_program(&program);
        let unit_getters = result
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "getter_unit_return")
            .count();
        assert_eq!(unit_getters, 3, "{:#?}", result.diagnostics);
    }

    #[test]
    fn checks_anonymous_shape_method_signature_before_following_member() {
        let program = parse_inline(
            r#"
interface Positioned {
    def position() { x Int, y Int }
    get label Str
}

class Located with Positioned {
    def position() { x Int, y Int } = { x: 3, y: 4 }
    get label Str = "point"
}

def coordinate(value Positioned) Int = value.position().x

def main() Unit {
    value Positioned = Located()
    println(value.label, coordinate(value))
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn rejects_member_names_shared_across_fields_getters_and_methods() {
        let program = parse_inline(
            r#"
class FieldAndMethod {
    label Str
    def label() Str = this.label
}

class FieldAndGetter {
    value Int
    get value Int = this.value
}

class GetterAndMethod {
    get status Str = "ready"
    def status(verbose Bool) Str = if verbose { "ready" } else { "" }
}

class RepeatedGetter {
    get name Str = "first"
    get name Str = "second"
}
"#,
        );
        let result = check_program(&program);
        let collisions = result
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "member_name_collision")
            .count();
        assert_eq!(collisions, 4, "{:#?}", result.diagnostics);
    }

    #[test]
    fn allows_ordinary_method_overloads_with_the_same_name() {
        let program = parse_inline(
            r#"
class Formatter {
    def format(value Int) Str = value.toStr()
    def format(value Str) Str = value
}

def main(formatter Formatter) Unit {
    println(formatter.format(1), formatter.format("one"))
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn selects_overloads_from_expression_result_types() {
        let program = parse_inline(
            r#"
class Formatter {
    def render(value Int) Str = value.toStr()
    def render(value Str) Str = value

    def classify(value Str) Str = "text"
    def classify(value Any) Int = 1
}

def render(value Int) Str = value.toStr()
def render(value Str) Str = value

def main(formatter Formatter) Unit {
    numbers = [1]
    literal Str = formatter.render(3)
    arithmetic Str = formatter.render(1 + 2)
    grouped Str = formatter.render((1 + 2))
    negative Str = formatter.render(-1)
    indexed Str = formatter.render(numbers[0])
    matched Str = formatter.render(match true {
        case true => 1
        case false => 2
    })
    blockLocal Str = formatter.render({
        value = numbers[0]
        value
    })
    topLevel Str = render(1 + 2)
    exact Str = formatter.classify("value")
    println(
        literal,
        arithmetic,
        grouped,
        negative,
        indexed,
        matched,
        blockLocal,
        topLevel,
        exact
    )
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn filters_contextual_construction_targets_using_other_arguments() {
        let program = parse_inline(
            r#"
class User {
    name Str
}

class Admin {
    name Str
}

class Saver {
    def save(tag Int, value User) Str = "user"
    def save(tag Str, value Admin) Str = "admin"
}

class Wrapper {
    new(tag Int, value User) {}
    new(tag Str, value Admin) {}
}

def main() Unit {
    saver = Saver()
    result Str = saver.save(1, new { name: "Ada" })
    wrapped = Wrapper(1, new { name: "Grace" })
    println(result)
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn selects_overloaded_callable_references_from_expected_function_types() {
        let program = parse_inline(
            r#"
def render(value Int) Str = value.toStr()
def render(value Str) Str = value

class Formatter {
    def render(value Int) Str = value.toStr()
    def render(value Str) Str = value

    def selected() fn(Str) Str = render
}

object SharedFormatter {
    def render(value Int) Str = value.toStr()
    def render(value Str) Str = value
}

def main(formatter Formatter) Unit {
    topLevel fn(Str) Str = render
    instance fn(Str) Str = formatter.render
    shared fn(Str) Str = SharedFormatter.render
    println(topLevel("top"), instance("instance"), shared("shared"))
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn rejects_unbound_class_method_references() {
        let program = parse_inline(
            r#"
class User {
    name Str

    def toDto(user User) Str = user.name
}

def main() Unit {
    convert fn(User) Str = User.toDto
}
"#,
        );
        let result = check_program(&program);
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "unbound_method_reference"),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn rejects_callable_references_when_expected_type_does_not_select_one_overload() {
        let program = parse_inline(
            r#"
class Formatter {
    def render(value Any) Str = "any"
    def render(value Str) Str = value
}

def main(formatter Formatter) Unit {
    untyped = formatter.render
    compatible fn(Str) Any = formatter.render
}
"#,
        );
        let result = check_program(&program);
        assert_eq!(
            result
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.code == "ambiguous_callable_reference")
                .count(),
            2,
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn rejects_extension_members_that_collide_with_fields_or_getters() {
        let program = parse_inline(
            r#"
class Item {
    label Str
    get status Str = "ready"
}

ext Item {
    def label() Str = this.label
    def status(verbose Bool) Str = if verbose { this.status } else { "" }
}
"#,
        );
        let result = check_program(&program);
        let collisions = result
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "member_name_collision")
            .count();
        assert_eq!(collisions, 2, "{:#?}", result.diagnostics);
    }

    #[test]
    fn rejects_call_syntax_for_getters() {
        let program = parse_inline(
            r#"
class Item {
    name Str
    get label Str = this.name
}

def read(item Item) Str = item.label()
"#,
        );
        let result = check_program(&program);
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "getter_call_syntax"),
            "{:#?}",
            result.diagnostics
        );
    }

    #[test]
    fn allows_calling_function_values_returned_by_getters() {
        let program = parse_inline(
            r#"
class Factory {
    get creator fn(Int) Int = (value Int) => value + 1
    get implicitCall Int = creator(4)
}

def read(factory Factory) Int = factory.creator(5)
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    }

    #[test]
    fn warns_when_a_mutable_local_is_never_reassigned() {
        let program = parse_inline(
            r#"
def main() Unit {
    var count = 1
    println(count)
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
        assert_eq!(result.warnings.len(), 1, "{:#?}", result.warnings);
        assert_eq!(result.warnings[0].code, "unused_mutability");
        assert!(
            result.warnings[0]
                .message
                .contains("'count' is declared mutable but never reassigned"),
            "{:#?}",
            result.warnings
        );
    }

    #[test]
    fn mutable_local_warning_tracks_binding_reassignment_only() {
        let program = parse_inline(
            r#"
def main() Unit {
    var count = 0
    if true {
        count += 1
    }

    var label = "before"
    label := "after"

    var values = [1]
    values.add(2)
    println(count, label, values.size)
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
        assert_eq!(result.warnings.len(), 1, "{:#?}", result.warnings);
        assert!(
            result.warnings[0].message.contains("'values'"),
            "{:#?}",
            result.warnings
        );
    }

    #[test]
    fn mutable_fields_do_not_produce_local_binding_warnings() {
        let program = parse_inline(
            r#"
class Counter {
    var value Int = 0
}
"#,
        );
        let result = check_program(&program);
        assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
        assert!(result.warnings.is_empty(), "{:#?}", result.warnings);
    }

    #[test]
    fn rejects_direct_field_mutation_in_getters() {
        let program = parse_inline(
            r#"
class Counter {
    var count Int
    values [Int]

    get incremented Int {
        this.count += 1
        this.count
    }

    get replaced Int {
        this.values[0] := 2
        this.values[0]
    }
}
"#,
        );
        let result = check_program(&program);
        let mutations = result
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "getter_mutation")
            .count();
        assert_eq!(mutations, 2, "{:#?}", result.diagnostics);
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
            "examples/mixin_interface_implementations.lum",
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
