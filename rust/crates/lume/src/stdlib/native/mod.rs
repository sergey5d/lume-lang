//! Native execution bindings. Public contracts live in the parent module.
mod bindings;
mod char_type;
mod constructors;
mod either_type;
mod int_range_type;
mod map_type;
mod option_type;
mod result_type;
mod set_type;
mod str_type;
mod vector_type;

use crate::{
    Diagnostic, Span,
    ast::{self, TypeKind, TypeMember},
    interpreter::{Interpreter, Value},
    ir,
    runtime::{
        BuiltinMethodFn, RuntimeEnumCase, RuntimeEnumCaseId, RuntimeField, RuntimeFieldSlot,
        RuntimeMethod, RuntimeMethodKind, RuntimeMethodSlot, RuntimeMethodTarget, RuntimeType,
        RuntimeTypeId,
    },
};

pub(super) fn invoke_callback(
    interpreter: &mut Interpreter<'_>,
    value: Value,
    span: Option<Span>,
) -> Result<Value, Diagnostic> {
    interpreter.invoke_value(value, Vec::new(), span)
}

pub(super) fn builtin_method(
    owner: &str,
    name: &str,
    arity: usize,
    target: BuiltinMethodFn,
) -> Option<RuntimeMethod> {
    let (param_names, params, return_ty) = super::runtime_method_signature(owner, name, arity)?;
    Some(RuntimeMethod {
        slot: RuntimeMethodSlot(0),
        name: name.into(),
        target: RuntimeMethodTarget::Builtin(target),
        param_names,
        params,
        return_ty,
        kind: RuntimeMethodKind::Instance,
        variadic: false,
        default_count: 0,
        reified_params: 0,
    })
}

pub(super) fn empty_builtin_type(name: &str) -> RuntimeType {
    let declaration = &super::descriptor(name)
        .unwrap_or_else(|| panic!("missing native contract {name}"))
        .declaration;
    materialize(declaration)
}

fn fields(
    declaration: &ast::TypeDecl,
    fields: impl Iterator<Item = ast::FieldDecl>,
) -> Vec<RuntimeField> {
    let variables = declaration
        .type_params
        .iter()
        .map(|param| param.name.as_str())
        .collect::<Vec<_>>();
    fields
        .enumerate()
        .map(|(index, field)| RuntimeField {
            slot: RuntimeFieldSlot(index),
            name: field.name,
            ty: super::type_to_ir(
                field.ty.as_ref().expect("native fields have types"),
                &variables,
            ),
            mutable: field.mutable,
            from_mixin: field.from_mixin,
            visibility: field.visibility,
            has_initializer: field.initializer.is_some(),
            initializer: None,
        })
        .collect()
}

fn materialize(declaration: &ast::TypeDecl) -> RuntimeType {
    RuntimeType {
        id: RuntimeTypeId(usize::MAX),
        native: true,
        ir_type_id: None,
        name: declaration.name.clone(),
        kind: declaration.kind,
        fields: fields(
            declaration,
            declaration
                .members
                .iter()
                .filter_map(|member| match member {
                    TypeMember::Field(field) => Some(field.clone()),
                    _ => None,
                }),
        ),
        field_init: None,
        mixin_init: None,
        methods: Vec::new(),
        enum_cases: declaration
            .members
            .iter()
            .filter_map(|member| match member {
                TypeMember::Case(case) => Some(case),
                _ => None,
            })
            .enumerate()
            .map(|(index, case)| RuntimeEnumCase {
                id: RuntimeEnumCaseId(index),
                name: case.name.clone(),
                fields: fields(declaration, case.fields.iter().cloned()),
            })
            .collect(),
        with_bounds: Vec::new(),
    }
}

fn method_from_declaration(
    owner: &ast::TypeDecl,
    method: &ast::MethodDecl,
    target: BuiltinMethodFn,
) -> RuntimeMethod {
    let variables = owner
        .type_params
        .iter()
        .chain(&method.type_params)
        .map(|param| param.name.as_str())
        .collect::<Vec<_>>();
    RuntimeMethod {
        slot: RuntimeMethodSlot(0),
        name: method.name.clone(),
        target: RuntimeMethodTarget::Builtin(target),
        param_names: method
            .params
            .iter()
            .map(|param| param.name.clone())
            .collect(),
        params: method
            .params
            .iter()
            .map(|param| {
                super::type_to_ir(
                    param.ty.as_ref().expect("native parameter has type"),
                    &variables,
                )
            })
            .collect(),
        return_ty: method
            .return_type
            .as_ref()
            .map(|ty| super::type_to_ir(ty, &variables))
            .unwrap_or(ir::Type::Unit),
        kind: if method.getter {
            RuntimeMethodKind::Getter
        } else if owner.kind == TypeKind::Object {
            RuntimeMethodKind::Static
        } else {
            RuntimeMethodKind::Instance
        },
        variadic: method.params.last().is_some_and(|param| param.variadic),
        default_count: method
            .params
            .iter()
            .filter(|param| param.initializer.is_some())
            .count(),
        reified_params: method
            .type_params
            .iter()
            .filter(|param| param.reified)
            .count(),
    }
}

pub(super) fn bind(ty: &mut RuntimeType, name: &str, target: BuiltinMethodFn) {
    let Some(declaration) = super::type_declaration(&ty.name, ty.kind) else {
        return;
    };
    for method in super::member_declarations(declaration, name) {
        if method.name == "new" {
            continue;
        }
        let metadata = method_from_declaration(declaration, &method, target);
        if !ty
            .methods
            .iter()
            .any(|old| old.name == metadata.name && old.params == metadata.params)
        {
            ty.methods.push(metadata);
        }
    }
}

pub(crate) fn runtime_types() -> Vec<RuntimeType> {
    let mut types = super::descriptors()
        .iter()
        .map(|descriptor| materialize(&descriptor.declaration))
        .collect::<Vec<_>>();
    for raw in [
        char_type::define(),
        str_type::define(),
        option_type::define(),
        result_type::define(),
        either_type::define(),
        int_range_type::define(),
        vector_type::define(),
        vector_type::define_for("Array"),
        vector_type::define_for("LinkedList"),
        set_type::define(),
        map_type::define(),
    ] {
        let Some(ty) = types
            .iter_mut()
            .find(|ty| ty.name == raw.name && ty.kind != TypeKind::Object)
        else {
            continue;
        };
        for binding in raw.methods {
            let RuntimeMethodTarget::Builtin(target) = binding.target else {
                unreachable!()
            };
            bind(ty, &binding.name, target);
        }
    }
    bindings::register(&mut types);
    for ty in &mut types {
        for (index, method) in ty.methods.iter_mut().enumerate() {
            method.slot = RuntimeMethodSlot(index);
        }
    }
    assign_type_ids(&mut types, 0);
    // Bounds are resolved only after every declaration has a dense runtime id.
    let bounds = types
        .iter()
        .map(|ty| {
            super::type_declaration(&ty.name, ty.kind)
                .map(|decl| {
                    decl.with_bounds
                        .iter()
                        .filter_map(|bound| {
                            let ast::TypeRef::Named { name, .. } = bound else {
                                return None;
                            };
                            types
                                .iter()
                                .find(|ty| ty.name == *name && ty.kind != TypeKind::Object)
                                .map(|ty| ty.id)
                        })
                        .collect()
                })
                .unwrap_or_default()
        })
        .collect::<Vec<Vec<RuntimeTypeId>>>();
    for (ty, bounds) in types.iter_mut().zip(bounds) {
        ty.with_bounds = bounds;
    }
    types
}

pub(crate) fn assign_type_ids(types: &mut [RuntimeType], start: usize) {
    for (index, ty) in types.iter_mut().enumerate() {
        ty.id = RuntimeTypeId(start + index);
    }
}

pub(crate) fn value_type(value: &Value) -> Option<(&'static str, TypeKind)> {
    let name = match value {
        Value::Bool(_) => "Bool",
        Value::Int(_) => "Int",
        Value::Float(_) => "Float",
        Value::Decimal(_) => "Decimal",
        Value::Char(_) => "Char",
        Value::String(_) => "Str",
        Value::List(_) => "Vector",
        Value::Set(_) => "Set",
        Value::Map(_) => "Map",
        Value::Iterator(_) => "Iterator",
        Value::ReferenceId(_) => "ReferenceId",
        Value::FileStream(_) => "FileStream",
        Value::TextFileReader(_) => "TextFileReader",
        Value::Regex(_) => "Regex",
        Value::RegexMatch(_) => "RegexMatch",
        Value::RuntimeType(_) => "Type",
        Value::RuntimeField { .. } => "Field",
        Value::RuntimeMethod { .. } => "Method",
        Value::RuntimeParam { .. } => "Param",
        Value::RuntimeEnumCase { .. } => "UnionAlternative",
        Value::Unit => "Unit",
        _ => return None,
    };
    Some((name, super::descriptor(name)?.kind))
}

pub(crate) fn declared_value_type<'a>(
    ty: &'a ir::Type,
    value: &Value,
) -> Option<(&'a str, TypeKind)> {
    let (ir::Type::Named { name, .. }, Value::List(_)) = (ty, value) else {
        return None;
    };
    Some((name, super::descriptor(name)?.kind))
}

const NAMESPACES: &[(&[&str], &str)] = &[
    (&["OS"], "OS"),
    (&["OS", "stdout"], "OS"),
    (&["OS", "stderr"], "OS"),
    (&["Math"], "Math"),
    (&["File"], "File"),
    (&["Json"], "Json"),
    (&["Regex"], "Regex"),
    (&["Vector"], "Vector"),
    (&["Array"], "Array"),
    (&["Set"], "Set"),
    (&["Int"], "Int"),
    (&["Float"], "Float"),
    (&["Decimal"], "Decimal"),
    (&["Option"], "Option"),
];

pub(crate) fn static_namespace(path: &[String]) -> Option<&'static str> {
    NAMESPACES
        .iter()
        .find(|(prefix, _)| {
            prefix.len() == path.len() && prefix.iter().zip(path).all(|(a, b)| *a == b)
        })
        .map(|(_, owner)| *owner)
}

pub(crate) fn bare_variant_owner(name: &str) -> Option<&'static str> {
    ["Option", "Rounding"].into_iter().find(|owner| {
        super::type_declaration(owner, TypeKind::Enum).is_some_and(|declaration| {
            declaration.members.iter().any(|member| {
            matches!(member, TypeMember::Case(case) if case.name == name && case.fields.is_empty())
        })
        })
    })
}

pub(crate) fn opaque_method_error(value: &Value, method: &str) -> Option<String> {
    let owner = match value {
        Value::FileStream(_) => "FileStream",
        Value::TextFileReader(_) => "TextFileReader",
        Value::Regex(_) if method != "toStr" => "Regex",
        Value::RegexMatch(_) if method != "toStr" => "RegexMatch",
        _ => return None,
    };
    Some(format!("unknown {owner} method '{method}'"))
}

pub(crate) use constructors::{
    constructor, constructor_owner, root_function, root_function_for_ir,
};

pub(crate) fn field_getter(owner: &str, kind: TypeKind, name: &str) -> Option<BuiltinMethodFn> {
    const GETTERS: &[(&str, TypeKind, &str, BuiltinMethodFn)] = &[
        ("OS", TypeKind::Object, "stdout", |i, r, a, s| {
            i.native_os_stdout(r, a, s)
        }),
        ("OS", TypeKind::Object, "stderr", |i, r, a, s| {
            i.native_os_stderr(r, a, s)
        }),
    ];
    GETTERS
        .iter()
        .find(|(ty, member_kind, member, _)| {
            *ty == owner && *member_kind == kind && *member == name
        })
        .map(|(_, _, _, handler)| *handler)
}

pub(crate) fn synthetic_getter(name: &str) -> Option<RuntimeMethod> {
    type Getter = (&'static str, fn() -> ir::Type, BuiltinMethodFn);
    const GETTERS: &[Getter] = &[
        (
            "runtimeType",
            || ir::Type::Named {
                name: "Type".into(),
                args: vec![ir::Type::named("Any")],
            },
            |i, r, a, s| i.native_runtime_type(r, a, s),
        ),
        (
            "referenceId",
            || ir::Type::named("ReferenceId"),
            |i, r, a, s| i.native_reference_id(r, a, s),
        ),
    ];
    let (_, return_ty, target) = GETTERS
        .iter()
        .find(|(candidate, _, _)| *candidate == name)?;
    Some(RuntimeMethod {
        slot: RuntimeMethodSlot(0),
        name: name.into(),
        target: RuntimeMethodTarget::Builtin(*target),
        param_names: Vec::new(),
        params: Vec::new(),
        return_ty: return_ty(),
        kind: RuntimeMethodKind::Getter,
        variadic: false,
        default_count: 0,
        reified_params: 0,
    })
}

pub(crate) fn conditional_trait(value: &Value, method: &str) -> Option<&'static str> {
    let hashable = matches!(
        value,
        Value::Unit
            | Value::Bool(_)
            | Value::Char(_)
            | Value::Int(_)
            | Value::Float(_)
            | Value::Decimal(_)
            | Value::String(_)
            | Value::ReferenceId(_)
            | Value::Tuple(_)
            | Value::Record(_)
            | Value::Aggregate(_)
    );
    (method == "hash" && hashable).then_some("Hashed")
}

pub(crate) fn metadata_type(kind: &str) -> &'static str {
    const TYPES: &[(&str, &str)] = &[
        ("Class", "ClassType"),
        ("Shape", "ShapeType"),
        ("Union", "UnionType"),
        ("Interface", "InterfaceType"),
        ("Object", "ObjectType"),
        ("Annotation", "AnnotationType"),
    ];
    TYPES
        .iter()
        .find(|(candidate, _)| *candidate == kind)
        .map(|(_, name)| *name)
        .unwrap_or("Type")
}
