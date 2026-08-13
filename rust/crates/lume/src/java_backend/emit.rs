use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

use crate::{
    ast::{self, TypeKind, TypeRef, Visibility},
    backend::BackendBundle,
    ir::{self, FunctionKind},
    java_backend::{JavaExternalClass, JavaPrimitiveCoercion},
};

pub(crate) struct JavaSource {
    pub(crate) relative_path: PathBuf,
    pub(crate) contents: String,
}

pub(crate) const JAVA_UNSUPPORTED_STUB_MARKER: &str = "unsupported Lume Java backend method body";
const MAX_JAVA_FUNCTION_ARITY: usize = 12;

pub(crate) fn render_declaration_skeletons(
    bundle: &BackendBundle,
    external_classes: &HashMap<String, JavaExternalClass>,
) -> Vec<JavaSource> {
    let package = JavaPackage::from_module(bundle.ir.module.as_deref());
    let names = JavaNames::from_external_classes(external_classes);
    let mut sources = Vec::new();
    let mut source_indexes = HashMap::new();

    let module_path = package.relative_file(&format!("{}.java", module_class_name(bundle)));
    push_java_source(
        &mut sources,
        &mut source_indexes,
        JavaSource {
            relative_path: module_path,
            contents: render_module_wrapper(bundle, &package, &names),
        },
        false,
    );

    if let Some(entrypoint) = render_entrypoint_runner(bundle, &package) {
        push_java_source(&mut sources, &mut source_indexes, entrypoint, false);
    }

    for ty in &bundle.ir.types {
        if names.is_java_type(&ty.name) {
            continue;
        }
        let relative_path = package.relative_file(&format!("{}.java", java_type_name(&ty.name)));
        push_java_source(
            &mut sources,
            &mut source_indexes,
            JavaSource {
                relative_path,
                contents: render_type_shell(bundle, ty, &package, &names),
            },
            is_java_library_placeholder_type(ty),
        );
    }

    sources
}

fn push_java_source(
    sources: &mut Vec<JavaSource>,
    indexes: &mut HashMap<PathBuf, (usize, bool)>,
    source: JavaSource,
    placeholder: bool,
) {
    let key = source.relative_path.clone();
    match indexes.get(&key).copied() {
        Some((index, existing_placeholder)) if existing_placeholder && !placeholder => {
            sources[index] = source;
            indexes.insert(key, (index, placeholder));
        }
        Some(_) => {}
        None => {
            indexes.insert(key, (sources.len(), placeholder));
            sources.push(source);
        }
    }
}

fn is_java_library_placeholder_type(ty: &ir::TypeDef) -> bool {
    ty.kind == TypeKind::Interface
        && ty.type_params.is_empty()
        && ty.with_bounds.is_empty()
        && ty.fields.is_empty()
        && ty.field_init.is_none()
        && ty.methods.is_empty()
        && ty.enum_cases.is_empty()
}

fn render_module_wrapper(
    bundle: &BackendBundle,
    package: &JavaPackage,
    names: &JavaNames,
) -> String {
    let mut out = String::new();
    push_header(&mut out, package);
    out.push_str(&format!("final class {} {{\n", module_class_name(bundle)));
    out.push_str("    private ");
    out.push_str(&module_class_name(bundle));
    out.push_str("() {}\n");

    for global in &bundle.ir.globals {
        out.push('\n');
        out.push_str("    static ");
        out.push_str(&names.value_type(&global.ty));
        out.push(' ');
        out.push_str(&java_member_name(&global.name));
        out.push_str(";\n");
    }

    for function in bundle
        .ir
        .functions
        .iter()
        .filter(|function| matches!(function.kind, FunctionKind::TopLevel))
    {
        out.push('\n');
        out.push_str("    static ");
        push_function_signature(&mut out, function, names);
        push_function_body(&mut out, bundle, function, names);
        let has_fixed_overload = variadic_fixed_arity(function).is_some_and(|arity| {
            bundle.ir.functions.iter().any(|other| {
                other.id != function.id
                    && matches!(other.kind, FunctionKind::TopLevel)
                    && other.name == function.name
                    && other.params.len() == arity
            })
        });
        push_variadic_bridge_method(
            &mut out,
            function,
            names,
            "    static ",
            &function.name,
            has_fixed_overload,
        );
    }

    out.push_str("}\n");
    out
}

fn render_entrypoint_runner(bundle: &BackendBundle, package: &JavaPackage) -> Option<JavaSource> {
    let entry = bundle.ir.entry.and_then(|id| bundle.ir.function(id))?;
    if !matches!(entry.kind, FunctionKind::TopLevel) {
        return None;
    }

    let class_name = runner_class_name(bundle);
    let module_name = module_class_name(bundle);
    let method_name = java_member_name(&entry.name);
    let mut out = String::new();
    push_header(&mut out, package);
    out.push_str(&format!("final class {class_name} {{\n"));
    out.push_str(&format!("    private {class_name}() {{}}\n\n"));
    out.push_str("    public static void main(String[] args) {\n");
    if is_java_void_type(&entry.return_ty) {
        out.push_str(&format!("        {module_name}.{method_name}();\n"));
    } else {
        out.push_str(&format!(
            "        System.out.println({module_name}.{method_name}());\n"
        ));
    }
    out.push_str("    }\n");
    out.push_str("}\n");

    Some(JavaSource {
        relative_path: package.relative_file(&format!("{class_name}.java")),
        contents: out,
    })
}

fn render_type_shell(
    bundle: &BackendBundle,
    ty: &ir::TypeDef,
    package: &JavaPackage,
    names: &JavaNames,
) -> String {
    if is_anonymous_object_type(ty) {
        return render_interface(bundle, ty, package, names);
    }
    match ty.kind {
        TypeKind::Annotation => render_annotation(bundle, ty, package, names),
        TypeKind::Class => render_class(bundle, ty, package, names),
        TypeKind::Record => render_shape(bundle, ty, package, names),
        TypeKind::Object => render_single(bundle, ty, package, names),
        TypeKind::Interface => render_interface(bundle, ty, package, names),
        TypeKind::Enum => render_enum(bundle, ty, package, names),
    }
}

fn render_class(
    bundle: &BackendBundle,
    ty: &ir::TypeDef,
    package: &JavaPackage,
    names: &JavaNames,
) -> String {
    let mut out = String::new();
    push_header(&mut out, package);
    out.push_str(&format!(
        "{}class {}{}{} {{\n",
        java_type_visibility(ty),
        java_type_name(&ty.name),
        java_type_params(&ty.type_params),
        java_implements_clause(ty, names)
    ));
    push_type_descriptor(&mut out, bundle, ty, package, names);
    push_runtime_type_method(&mut out, false);
    push_fields(&mut out, ty, names);
    push_class_field_initializer(&mut out, bundle, ty, names);
    push_class_constructors(&mut out, bundle, ty, names);
    push_instance_methods(&mut out, bundle, ty, MethodShell::StubBody, names);
    out.push_str("}\n");
    out
}

fn render_shape(
    bundle: &BackendBundle,
    ty: &ir::TypeDef,
    package: &JavaPackage,
    names: &JavaNames,
) -> String {
    let mut out = String::new();
    push_header(&mut out, package);
    out.push_str(&format!(
        "{}record {}{}({}){} {{\n",
        java_type_visibility(ty),
        java_type_name(&ty.name),
        java_type_params(&ty.type_params),
        ty.fields
            .iter()
            .map(|field| format!(
                "{} {}",
                names.value_type(&field.ty),
                java_member_name(&field.name)
            ))
            .collect::<Vec<_>>()
            .join(", "),
        java_implements_clause(ty, names)
    ));
    push_type_descriptor(&mut out, bundle, ty, package, names);
    push_runtime_type_method(&mut out, false);
    push_instance_methods(&mut out, bundle, ty, MethodShell::StubBody, names);
    out.push_str("}\n");
    out
}

fn render_single(
    bundle: &BackendBundle,
    ty: &ir::TypeDef,
    package: &JavaPackage,
    names: &JavaNames,
) -> String {
    let mut out = String::new();
    let name = java_type_name(&ty.name);
    push_header(&mut out, package);
    out.push_str(&format!(
        "{}final class {name}{} {{\n",
        java_type_visibility(ty),
        java_implements_clause(ty, names)
    ));
    out.push_str(&format!(
        "    public static final {name} INSTANCE = new {name}();\n"
    ));
    push_type_descriptor(&mut out, bundle, ty, package, names);
    push_runtime_type_method(&mut out, false);
    out.push_str(&format!("    private {name}()"));
    if let Some(field_init) = ty
        .field_init
        .and_then(|id| bundle.ir.function(id))
        .and_then(|function| emit_field_initializer_constructor_body(bundle, function, names))
    {
        out.push_str(&field_init);
    } else {
        out.push_str(" {}\n");
    }
    push_fields(&mut out, ty, names);
    push_instance_methods(&mut out, bundle, ty, MethodShell::StubBody, names);
    out.push_str("}\n");
    out
}

fn render_interface(
    bundle: &BackendBundle,
    ty: &ir::TypeDef,
    package: &JavaPackage,
    names: &JavaNames,
) -> String {
    let mut out = String::new();
    push_header(&mut out, package);
    out.push_str(&format!(
        "{}interface {}{}{} {{\n",
        java_type_visibility(ty),
        java_type_name(&ty.name),
        java_type_params(&ty.type_params),
        java_extends_clause(ty, names)
    ));
    push_type_descriptor(&mut out, bundle, ty, package, names);
    push_runtime_type_method(&mut out, true);
    push_instance_methods(&mut out, bundle, ty, MethodShell::Abstract, names);
    out.push_str("}\n");
    out
}

fn render_annotation(
    bundle: &BackendBundle,
    ty: &ir::TypeDef,
    package: &JavaPackage,
    names: &JavaNames,
) -> String {
    let mut out = String::new();
    push_header(&mut out, package);
    out.push_str(&format!(
        "{}@interface {} {{\n",
        java_type_visibility(ty),
        java_type_name(&ty.name)
    ));
    push_type_descriptor(&mut out, bundle, ty, package, names);
    for field in &ty.fields {
        out.push_str("    ");
        out.push_str(&names.annotation_type(&field.ty));
        out.push(' ');
        out.push_str(&java_member_name(&field.name));
        out.push_str("();\n");
    }
    out.push_str("}\n");
    out
}

fn render_enum(
    bundle: &BackendBundle,
    ty: &ir::TypeDef,
    package: &JavaPackage,
    names: &JavaNames,
) -> String {
    let mut out = String::new();
    let enum_name = java_type_name(&ty.name);
    let type_params = java_type_params(&ty.type_params);
    let type_args = java_type_args(&ty.type_params);
    push_header(&mut out, package);

    if ty.enum_cases.is_empty() {
        out.push_str(&format!(
            "{}interface {enum_name}{type_params}{} {{\n",
            java_type_visibility(ty),
            java_extends_clause(ty, names)
        ));
    } else {
        let permits = ty
            .enum_cases
            .iter()
            .map(|case| format!("{enum_name}.{}", java_type_name(&case.name)))
            .collect::<Vec<_>>()
            .join(", ");
        out.push_str(&format!(
            "{}sealed interface {enum_name}{type_params}{} permits {permits} {{\n",
            java_type_visibility(ty),
            java_extends_clause(ty, names)
        ));
    }

    push_type_descriptor(&mut out, bundle, ty, package, names);
    push_runtime_type_method(&mut out, true);
    push_instance_methods(&mut out, bundle, ty, MethodShell::DefaultBody, names);

    for case in &ty.enum_cases {
        out.push('\n');
        out.push_str("    record ");
        out.push_str(&java_type_name(&case.name));
        out.push_str(&type_params);
        out.push('(');
        out.push_str(
            &case
                .fields
                .iter()
                .map(|field| {
                    format!(
                        "{} {}",
                        names.value_type(&field.ty),
                        java_member_name(&field.name)
                    )
                })
                .collect::<Vec<_>>()
                .join(", "),
        );
        out.push_str(") implements ");
        out.push_str(&enum_name);
        out.push_str(&type_args);
        out.push_str(" {}\n");
    }

    out.push_str("}\n");
    out
}

fn java_type_visibility(ty: &ir::TypeDef) -> &'static str {
    match ty.visibility {
        ast::Visibility::Hidden => "",
        ast::Visibility::Default => "public ",
    }
}

fn is_anonymous_object_type(ty: &ir::TypeDef) -> bool {
    ty.kind == TypeKind::Object && ty.name.starts_with("__LumeObject_")
}

fn java_implements_clause(ty: &ir::TypeDef, names: &JavaNames) -> String {
    java_bound_clause(" implements ", ty, names)
}

fn java_extends_clause(ty: &ir::TypeDef, names: &JavaNames) -> String {
    java_bound_clause(" extends ", ty, names)
}

fn java_bound_clause(prefix: &str, ty: &ir::TypeDef, names: &JavaNames) -> String {
    let bounds = ty
        .with_bounds
        .iter()
        .filter_map(|bound| java_bound_type(bound, names))
        .collect::<Vec<_>>();
    if bounds.is_empty() {
        String::new()
    } else {
        format!("{prefix}{}", bounds.join(", "))
    }
}

fn java_bound_type(bound: &ir::Type, names: &JavaNames) -> Option<String> {
    match bound {
        ir::Type::Named { .. } => Some(names.value_type(bound)),
        _ => None,
    }
}

fn push_header(out: &mut String, package: &JavaPackage) {
    out.push_str("// Generated by Lume Java backend.\n");
    if let Some(name) = &package.name {
        out.push_str("package ");
        out.push_str(name);
        out.push_str(";\n\n");
    }
}

fn push_fields(out: &mut String, ty: &ir::TypeDef, names: &JavaNames) {
    for field in &ty.fields {
        out.push_str("    ");
        out.push_str(&names.value_type(&field.ty));
        out.push(' ');
        out.push_str(&java_member_name(&field.name));
        out.push_str(";\n");
    }
}

fn push_type_descriptor(
    out: &mut String,
    bundle: &BackendBundle,
    ty: &ir::TypeDef,
    package: &JavaPackage,
    names: &JavaNames,
) {
    out.push_str("    public static final lume.core.LumeType TYPE = ");
    out.push_str(&type_descriptor_expr(bundle, ty, package, names));
    out.push_str(";\n");
    out.push_str("    public static final String LUME_KIND = ");
    out.push_str(&java_string_literal(lume_type_kind_name(ty.kind)));
    out.push_str(";\n");
    out.push_str("    public static final String LUME_DEFAULT_FIELDS = ");
    out.push_str(&java_string_literal(
        &ty.fields
            .iter()
            .filter(|field| field.initializer.is_some())
            .map(|field| field.name.as_str())
            .collect::<Vec<_>>()
            .join(","),
    ));
    out.push_str(";\n");
    out.push_str("    public static final String LUME_DEFAULT_FIELD_VALUES = ");
    out.push_str(&java_string_literal(&lume_default_field_values(&ty.fields)));
    out.push_str(";\n");
}

fn lume_type_kind_name(kind: TypeKind) -> &'static str {
    match kind {
        TypeKind::Annotation => "annotation",
        TypeKind::Class => "class",
        TypeKind::Record => "shape",
        TypeKind::Object => "object",
        TypeKind::Interface => "interface",
        TypeKind::Enum => "enum",
    }
}

fn lume_default_field_values(fields: &[ir::Field]) -> String {
    fields
        .iter()
        .filter_map(|field| {
            field.initializer.as_ref().and_then(|initializer| {
                lume_constant_metadata(initializer)
                    .map(|value| format!("{}\t{}", metadata_escape(&field.name), value))
            })
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn lume_constant_metadata(value: &ir::Constant) -> Option<String> {
    let (tag, body) = match value {
        ir::Constant::Unit => ("unit", String::new()),
        ir::Constant::Bool(value) => ("bool", value.to_string()),
        ir::Constant::Int(value) => ("int", value.to_string()),
        ir::Constant::Float(value) => ("float", value.to_string()),
        ir::Constant::String(value) => ("str", metadata_escape(&decode_lume_string_literal(value))),
        ir::Constant::List(_) => return None,
    };
    Some(format!("{tag}\t{body}"))
}

fn metadata_escape(value: &str) -> String {
    let mut out = String::new();
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            ch => out.push(ch),
        }
    }
    out
}

fn push_runtime_type_method(out: &mut String, default_method: bool) {
    out.push('\n');
    out.push_str("    ");
    if default_method {
        out.push_str("default ");
    } else {
        out.push_str("public ");
    }
    out.push_str("lume.core.LumeType runtimeType() {\n");
    out.push_str("        return TYPE;\n");
    out.push_str("    }\n");
}

fn type_descriptor_expr(
    bundle: &BackendBundle,
    ty: &ir::TypeDef,
    package: &JavaPackage,
    names: &JavaNames,
) -> String {
    let name = java_string_literal(&ty.name);
    let qualified_name = qualified_type_name(ty, package);
    let qualified = java_string_literal(&qualified_name);
    let fields = type_field_array_expr(&ty.fields, names, &ty.type_params);
    let methods = type_method_array_expr(bundle, ty, names);
    let annotations = annotation_array_expr(&ty.annotations);
    match ty.kind {
        TypeKind::Annotation => {
            format!(
                "lume.core.LumeType.annotationType({name}, {qualified}, {fields}, {annotations})"
            )
        }
        TypeKind::Class => {
            format!(
                "lume.core.LumeType.classType({name}, {qualified}, {fields}, {methods}, {annotations})"
            )
        }
        TypeKind::Record => {
            format!(
                "lume.core.LumeType.shapeType({name}, {qualified}, {fields}, {methods}, {annotations})"
            )
        }
        TypeKind::Object => {
            format!(
                "lume.core.LumeType.objectType({name}, {qualified}, {fields}, {methods}, {annotations})"
            )
        }
        TypeKind::Interface => {
            format!(
                "lume.core.LumeType.interfaceType({name}, {qualified}, {methods}, {annotations})"
            )
        }
        TypeKind::Enum => format!(
            "lume.core.LumeType.enumType({}, {}, {}, {}, {})",
            name,
            qualified,
            enum_case_array_expr(ty, names, &qualified_name),
            methods,
            annotations
        ),
    }
}

fn qualified_type_name(ty: &ir::TypeDef, package: &JavaPackage) -> String {
    package
        .name
        .as_ref()
        .map(|package| format!("{}.{}", package, java_type_name(&ty.name)))
        .unwrap_or_else(|| java_type_name(&ty.name))
}

fn type_field_array_expr(
    fields: &[ir::Field],
    names: &JavaNames,
    type_params: &[String],
) -> String {
    let items = fields
        .iter()
        .map(|field| {
            format!(
                "lume.core.LumeField.of({}, {}, {}, {})",
                java_string_literal(&field.name),
                type_value_expr_with_params(&field.ty, names, type_params),
                annotation_array_expr(&field.annotations),
                matches!(field.visibility, Visibility::Hidden)
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!("new lume.core.LumeField[] {{{items}}}")
}

fn type_method_array_expr(bundle: &BackendBundle, ty: &ir::TypeDef, names: &JavaNames) -> String {
    let items = ty
        .methods
        .iter()
        .filter_map(|method_id| bundle.ir.function(*method_id))
        .filter(|method| method.name != "new")
        .map(|method| method_descriptor_expr(ty, method, names))
        .collect::<Vec<_>>()
        .join(", ");
    format!("new lume.core.LumeMethod[] {{{items}}}")
}

fn method_descriptor_expr(owner: &ir::TypeDef, method: &ir::Function, names: &JavaNames) -> String {
    let type_params = owner
        .type_params
        .iter()
        .chain(method.type_params.iter())
        .cloned()
        .collect::<Vec<_>>();
    let params = method
        .params
        .iter()
        .filter_map(|param| method.locals.get(param.0))
        .filter(|local| !is_reified_type_param_local(&local.name))
        .map(|local| {
            format!(
                "lume.core.LumeParam.of({}, {})",
                java_string_literal(&local.name),
                type_value_expr_with_params(&local.ty, names, &type_params)
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "lume.core.LumeMethod.of({}, {}, new lume.core.LumeParam[] {{{}}}, {}, {})",
        java_string_literal(&method.name),
        type_value_expr_with_params(&method.return_ty, names, &type_params),
        params,
        annotation_array_expr(&method.annotations),
        method_invoker_expr(owner, method, names)
    )
}

fn method_invoker_expr(owner: &ir::TypeDef, method: &ir::Function, names: &JavaNames) -> String {
    if matches!(owner.kind, TypeKind::Interface | TypeKind::Annotation) {
        return "null".to_string();
    }

    let type_params = owner
        .type_params
        .iter()
        .chain(method.type_params.iter())
        .cloned()
        .collect::<Vec<_>>();
    let owner_type = java_type_name(&owner.name);
    let receiver = format!("(({owner_type}) receiver)");
    let args = method
        .params
        .iter()
        .filter_map(|param| method.locals.get(param.0))
        .enumerate()
        .map(|(index, local)| {
            format!(
                "(({}) args[{}])",
                invoker_erased_value_type(&local.ty, names, &type_params),
                index
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    let call = format!("{}.{}({})", receiver, java_member_name(&method.name), args);
    if is_java_void_type(&method.return_ty) {
        format!("(receiver, args) -> {{ {call}; return lume.core.LumeUnit.INSTANCE; }}")
    } else {
        format!("(receiver, args) -> {call}")
    }
}

fn invoker_erased_value_type(ty: &ir::Type, names: &JavaNames, type_params: &[String]) -> String {
    match ty {
        ir::Type::TypeParam(_) | ir::Type::Unknown => "Object".to_string(),
        ir::Type::Never => "lume.core.LumePanic".to_string(),
        ir::Type::Unit => "lume.core.LumeUnit".to_string(),
        ir::Type::Bool => "Boolean".to_string(),
        ir::Type::Int => "Long".to_string(),
        ir::Type::Float => "Double".to_string(),
        ir::Type::Str => "String".to_string(),
        ir::Type::Function { params, .. } => erased_function_type_name(params.len()),
        ir::Type::Named { name, .. } if is_reflection_type(name) => {
            "lume.core.LumeType".to_string()
        }
        ir::Type::Named { name, args } if is_builtin_container(name) && !args.is_empty() => {
            invoker_erased_container_type(name, names)
        }
        ir::Type::Named { name, args }
            if args.is_empty() && type_params.iter().any(|param| param == name) =>
        {
            "Object".to_string()
        }
        ir::Type::Named { name, args } if args.is_empty() => java_named_builtin_value(name)
            .or_else(|| names.java_types.get(name).cloned())
            .unwrap_or_else(|| java_type_name(name)),
        ir::Type::Named { name, .. } => names.named_type(name),
        ir::Type::Tuple(_) | ir::Type::Record(_) => "Object".to_string(),
    }
}

fn erased_function_type_name(arity: usize) -> String {
    match arity {
        0 => "java.util.function.Supplier".to_string(),
        1 => "java.util.function.Function".to_string(),
        2 => "java.util.function.BiFunction".to_string(),
        3..=MAX_JAVA_FUNCTION_ARITY => format!("lume.core.Function{arity}"),
        _ => "Object".to_string(),
    }
}

fn emit_functional_call(target: &str, args: &[String]) -> Option<String> {
    if args.len() > MAX_JAVA_FUNCTION_ARITY {
        return None;
    }
    if args.is_empty() {
        Some(format!("{target}.get()"))
    } else {
        Some(format!("{target}.apply({})", args.join(", ")))
    }
}

fn is_reified_type_param_local(name: &str) -> bool {
    name.starts_with("__type_")
}

fn invoker_erased_container_type(name: &str, names: &JavaNames) -> String {
    match name {
        "Array" => "lume.core.LumeArray".to_string(),
        "Either" => "lume.core.Either".to_string(),
        "Iterator" => "lume.core.LumeIterator".to_string(),
        "Vector" => "lume.core.LumeVector".to_string(),
        "LinkedList" => "lume.core.LumeLinkedList".to_string(),
        "Map" => "lume.core.LumeMap".to_string(),
        "Option" => "lume.core.Option".to_string(),
        "Result" => "lume.core.Result".to_string(),
        "Set" => "lume.core.LumeSet".to_string(),
        _ => names.named_type(name),
    }
}

fn enum_case_array_expr(ty: &ir::TypeDef, names: &JavaNames, owner_qualified_name: &str) -> String {
    let items = ty
        .enum_cases
        .iter()
        .map(|case| {
            format!(
                "lume.core.LumeEnumCase.of({}, {}, {}, {})",
                java_string_literal(owner_qualified_name),
                java_string_literal(&case.name),
                type_field_array_expr(&case.fields, names, &ty.type_params),
                annotation_array_expr(&case.annotations)
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!("new lume.core.LumeEnumCase[] {{{items}}}")
}

fn annotation_array_expr(annotations: &[ir::Annotation]) -> String {
    let items = annotations
        .iter()
        .map(annotation_expr)
        .collect::<Vec<_>>()
        .join(", ");
    format!("new lume.core.LumeAnnotation[] {{{items}}}")
}

fn annotation_expr(annotation: &ir::Annotation) -> String {
    let fields = annotation
        .fields
        .iter()
        .map(|field| {
            format!(
                "lume.core.LumeAnnotationField.of({}, {})",
                java_string_literal(&field.name),
                annotation_value_expr(&field.value)
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "lume.core.LumeAnnotation.of({}, new lume.core.LumeAnnotationField[] {{{}}})",
        java_string_literal(&annotation.name),
        fields
    )
}

fn annotation_value_expr(value: &ir::AnnotationValue) -> String {
    match value {
        ir::AnnotationValue::Bool(value) => value.to_string(),
        ir::AnnotationValue::Int(value) => format!("{value}L"),
        ir::AnnotationValue::Float(value) => java_float_literal(*value),
        ir::AnnotationValue::String(value) => java_string_literal(value),
        ir::AnnotationValue::List(items) => format!(
            "lume.core.LumeVector.of({})",
            items
                .iter()
                .map(annotation_value_expr)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        ir::AnnotationValue::Record(fields) => {
            let entries = fields
                .iter()
                .map(|field| {
                    format!(
                        "new lume.core.Tuple2<>({}, {})",
                        java_string_literal(&field.name),
                        annotation_value_expr(&field.value)
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            format!("lume.core.LumeMap.fromEntries(lume.core.LumeVector.of({entries}))")
        }
        ir::AnnotationValue::EnumCase(path) => java_string_literal(&path.join(".")),
        ir::AnnotationValue::Unresolved(value) => java_string_literal(value),
    }
}

fn type_value_expr(ty: &ir::Type, names: &JavaNames) -> String {
    type_value_expr_with_params(ty, names, &[])
}

fn type_value_expr_with_params(ty: &ir::Type, names: &JavaNames, type_params: &[String]) -> String {
    match ty {
        ir::Type::Unknown => "lume.core.LumeType.primitive(\"Unknown\")".to_string(),
        ir::Type::Never => "lume.core.LumeType.primitive(\"Never\")".to_string(),
        ir::Type::Unit => "lume.core.LumeType.primitive(\"Unit\")".to_string(),
        ir::Type::Bool => "lume.core.LumeType.primitive(\"Bool\")".to_string(),
        ir::Type::Int => "lume.core.LumeType.primitive(\"Int\")".to_string(),
        ir::Type::Float => "lume.core.LumeType.primitive(\"Float\")".to_string(),
        ir::Type::Str => "lume.core.LumeType.primitive(\"Str\")".to_string(),
        ir::Type::Named { name, args }
            if args.is_empty() && java_named_builtin_value(name).is_some() =>
        {
            format!(
                "lume.core.LumeType.primitive({})",
                java_string_literal(name)
            )
        }
        ir::Type::Named { name, args } if args.is_empty() && type_params.contains(name) => {
            format!(
                "lume.core.LumeType.primitive({})",
                java_string_literal(name)
            )
        }
        ir::Type::Named { name, args } if args.is_empty() && names.is_java_type(name) => {
            format!(
                "lume.core.LumeType.classType({}, {}, new lume.core.LumeField[] {{}}, new lume.core.LumeMethod[] {{}})",
                java_string_literal(name),
                java_string_literal(&names.named_type(name))
            )
        }
        ir::Type::Named { name, args } if args.is_empty() => {
            format!("{}.TYPE", java_type_name(name))
        }
        ir::Type::Named { name, args } => {
            let rendered = format!(
                "{}[{}]",
                name,
                args.iter()
                    .map(|arg| type_descriptor_name(arg))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            format!(
                "lume.core.LumeType.primitive({})",
                java_string_literal(&rendered)
            )
        }
        ir::Type::Tuple(items) => {
            let rendered = format!(
                "({})",
                items
                    .iter()
                    .map(type_descriptor_name)
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            format!(
                "lume.core.LumeType.primitive({})",
                java_string_literal(&rendered)
            )
        }
        ir::Type::Record(_) => "lume.core.LumeType.primitive(\"AnonymousShape\")".to_string(),
        ir::Type::Function { .. } => "lume.core.LumeType.primitive(\"Function\")".to_string(),
        ir::Type::TypeParam(name) => {
            format!(
                "lume.core.LumeType.primitive({})",
                java_string_literal(name)
            )
        }
    }
}

fn type_descriptor_name(ty: &ir::Type) -> String {
    match ty {
        ir::Type::Unknown => "Unknown".to_string(),
        ir::Type::Never => "Never".to_string(),
        ir::Type::Unit => "Unit".to_string(),
        ir::Type::Bool => "Bool".to_string(),
        ir::Type::Int => "Int".to_string(),
        ir::Type::Float => "Float".to_string(),
        ir::Type::Str => "Str".to_string(),
        ir::Type::Named { name, args } if args.is_empty() => name.clone(),
        ir::Type::Named { name, args } => format!(
            "{}[{}]",
            name,
            args.iter()
                .map(type_descriptor_name)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        ir::Type::Tuple(items) => format!(
            "({})",
            items
                .iter()
                .map(type_descriptor_name)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        ir::Type::Record(_) => "AnonymousShape".to_string(),
        ir::Type::Function { .. } => "Function".to_string(),
        ir::Type::TypeParam(name) => name.clone(),
    }
}

fn push_instance_methods(
    out: &mut String,
    bundle: &BackendBundle,
    ty: &ir::TypeDef,
    shell: MethodShell,
    names: &JavaNames,
) {
    for method_id in &ty.methods {
        let Some(function) = bundle.ir.function(*method_id) else {
            continue;
        };
        if function.name == "new" {
            continue;
        }
        out.push('\n');
        out.push_str("    ");
        match shell {
            MethodShell::DefaultBody => out.push_str("default "),
            MethodShell::StubBody => out.push_str("public "),
            MethodShell::Abstract => {}
        }
        push_function_signature(out, function, names);
        match shell {
            MethodShell::Abstract => {
                out.push_str(";\n");
                let has_fixed_overload = variadic_fixed_arity(function).is_some_and(|arity| {
                    ty.methods
                        .iter()
                        .filter_map(|id| bundle.ir.function(*id))
                        .any(|other| {
                            other.id != function.id
                                && other.name == function.name
                                && other.params.len() == arity
                        })
                });
                push_variadic_bridge_method(
                    out,
                    function,
                    names,
                    "    default ",
                    &function.name,
                    has_fixed_overload,
                );
            }
            MethodShell::DefaultBody | MethodShell::StubBody => {
                push_function_body(out, bundle, function, names);
                let prefix = match shell {
                    MethodShell::DefaultBody => "    default ",
                    MethodShell::StubBody => "    public ",
                    MethodShell::Abstract => unreachable!(),
                };
                let has_fixed_overload = variadic_fixed_arity(function).is_some_and(|arity| {
                    ty.methods
                        .iter()
                        .filter_map(|id| bundle.ir.function(*id))
                        .any(|other| {
                            other.id != function.id
                                && other.name == function.name
                                && other.params.len() == arity
                        })
                });
                push_variadic_bridge_method(
                    out,
                    function,
                    names,
                    prefix,
                    &function.name,
                    has_fixed_overload,
                );
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MethodShell {
    Abstract,
    DefaultBody,
    StubBody,
}

fn push_function_signature(out: &mut String, function: &ir::Function, names: &JavaNames) {
    push_function_signature_named(out, function, names, &function.name);
}

fn push_function_signature_named(
    out: &mut String,
    function: &ir::Function,
    names: &JavaNames,
    name: &str,
) {
    if !function.type_params.is_empty() {
        out.push_str(&java_type_params(&function.type_params));
        out.push(' ');
    }
    out.push_str(&names.return_type(&function.return_ty));
    out.push(' ');
    out.push_str(&java_member_name(name));
    out.push('(');
    out.push_str(&java_param_list(function, names, false));
    out.push(')');
}

fn java_param_list(function: &ir::Function, names: &JavaNames, skip_receiver: bool) -> String {
    function
        .params
        .iter()
        .filter_map(|param| function.locals.get(param.0))
        .filter(|local| {
            !(skip_receiver && matches!(local.kind, ir::LocalKind::Param) && local.name == "this")
        })
        .map(|local| format!("{} {}", names.value_type(&local.ty), java_local_name(local)))
        .collect::<Vec<_>>()
        .join(", ")
}

fn variadic_element_type(ty: &ir::Type) -> Option<&ir::Type> {
    match ty {
        ir::Type::Named { name, args } if name == "Vector" && args.len() == 1 => args.first(),
        _ => None,
    }
}

fn variadic_fixed_arity(function: &ir::Function) -> Option<usize> {
    let index = function
        .param_variadic
        .iter()
        .position(|variadic| *variadic)?;
    (index + 1 == function.params.len()).then_some(index)
}

fn push_variadic_bridge_method(
    out: &mut String,
    function: &ir::Function,
    names: &JavaNames,
    prefix: &str,
    method_name: &str,
    has_fixed_overload: bool,
) {
    let Some(variadic_index) = function
        .param_variadic
        .iter()
        .position(|variadic| *variadic)
    else {
        return;
    };
    if variadic_index + 1 != function.params.len() {
        return;
    }
    let Some(variadic_local) = function
        .params
        .get(variadic_index)
        .and_then(|param| function.locals.get(param.0))
    else {
        return;
    };
    let Some(element_ty) = variadic_element_type(&variadic_local.ty) else {
        return;
    };

    let fixed_params = function
        .params
        .iter()
        .take(variadic_index)
        .filter_map(|param| function.locals.get(param.0))
        .collect::<Vec<_>>();
    let fixed_decl = fixed_params
        .iter()
        .map(|local| format!("{} {}", names.value_type(&local.ty), java_local_name(local)))
        .collect::<Vec<_>>();
    let fixed_args = fixed_params
        .iter()
        .map(|local| java_local_name(local))
        .collect::<Vec<_>>();

    if !has_fixed_overload {
        let omitted_variadic_arg = function
            .param_defaults
            .get(variadic_index)
            .and_then(|default| default.as_ref())
            .map(java_constant)
            .unwrap_or_else(|| "lume.core.LumeVector.of()".to_string());
        push_variadic_bridge_overload(
            out,
            function,
            names,
            prefix,
            method_name,
            &fixed_decl,
            &fixed_args,
            omitted_variadic_arg,
        );
    }

    let mut variadic_decl = fixed_decl;
    variadic_decl.push(format!(
        "{}... {}",
        names.value_type(element_ty),
        java_local_name(variadic_local)
    ));
    push_variadic_bridge_overload(
        out,
        function,
        names,
        prefix,
        method_name,
        &variadic_decl,
        &fixed_args,
        format!(
            "lume.core.LumeVector.of({})",
            java_local_name(variadic_local)
        ),
    );
}

fn push_variadic_bridge_overload(
    out: &mut String,
    function: &ir::Function,
    names: &JavaNames,
    prefix: &str,
    method_name: &str,
    params: &[String],
    fixed_args: &[String],
    variadic_arg: String,
) {
    out.push('\n');
    out.push_str(prefix);
    if !function.type_params.is_empty() {
        out.push_str(&java_type_params(&function.type_params));
        out.push(' ');
    }
    out.push_str(&names.return_type(&function.return_ty));
    out.push(' ');
    out.push_str(&java_member_name(method_name));
    out.push('(');
    out.push_str(&params.join(", "));
    out.push_str(") {\n");
    out.push_str("        ");
    if !is_java_void_type(&function.return_ty) {
        out.push_str("return ");
    }
    let mut args = fixed_args.to_vec();
    args.push(variadic_arg);
    out.push_str(&java_member_name(method_name));
    out.push('(');
    out.push_str(&args.join(", "));
    out.push_str(");\n");
    out.push_str("    }\n");
}

fn function_param_types(function: &ir::Function) -> Vec<ir::Type> {
    function
        .params
        .iter()
        .filter_map(|param| function.locals.get(param.0))
        .map(|local| local.ty.clone())
        .collect()
}

#[derive(Clone)]
struct JavaParamSpec {
    ty: ir::Type,
    variadic: bool,
    lazy: bool,
    default: Option<ir::Constant>,
    coercion: Option<JavaPrimitiveCoercion>,
}

fn function_param_specs(function: &ir::Function) -> Vec<JavaParamSpec> {
    function
        .params
        .iter()
        .enumerate()
        .filter_map(|(index, param)| {
            let local = function.locals.get(param.0)?;
            Some(JavaParamSpec {
                ty: local.ty.clone(),
                variadic: function.param_variadic.get(index).copied().unwrap_or(false),
                lazy: function.param_lazy.get(index).copied().unwrap_or(false),
                default: function.param_defaults.get(index).cloned().flatten(),
                coercion: None,
            })
        })
        .collect()
}

fn param_specs_from_types(params: Vec<ir::Type>) -> Vec<JavaParamSpec> {
    params
        .into_iter()
        .map(|ty| JavaParamSpec {
            ty,
            variadic: false,
            lazy: false,
            default: None,
            coercion: None,
        })
        .collect()
}

fn java_param_spec(ty: ir::Type, lazy: bool) -> JavaParamSpec {
    JavaParamSpec {
        ty,
        variadic: false,
        lazy,
        default: None,
        coercion: None,
    }
}

fn lazy_param_value_type(ty: &ir::Type) -> &ir::Type {
    match ty {
        ir::Type::Function { params, ret } if params.is_empty() => ret.as_ref(),
        _ => ty,
    }
}

fn function_accepts_arg_len(function: &ir::Function, arg_len: usize) -> bool {
    param_specs_accept_arg_len(&function_param_specs(function), arg_len)
}

fn param_specs_accept_arg_len(params: &[JavaParamSpec], arg_len: usize) -> bool {
    match params.iter().position(|param| param.variadic) {
        Some(variadic_index) => arg_len >= variadic_index,
        None => params.len() == arg_len,
    }
}

fn push_function_body(
    out: &mut String,
    bundle: &BackendBundle,
    function: &ir::Function,
    names: &JavaNames,
) {
    if let Some(body) = structured_source_function_body(bundle, function, names) {
        out.push_str(&body);
        return;
    }

    match FunctionEmitter::new(bundle, function, names).emit_body() {
        Some(body) => out.push_str(&body),
        None => push_stub_body(out),
    }
}

fn structured_source_function_body(
    bundle: &BackendBundle,
    function: &ir::Function,
    names: &JavaNames,
) -> Option<String> {
    let (owner, expr) = source_method_expr(bundle, function)?;
    SourceBodyEmitter {
        function,
        names,
        owner,
    }
    .emit_body(expr)
}

fn source_method_expr<'a>(
    bundle: &'a BackendBundle,
    function: &ir::Function,
) -> Option<(&'a ir::TypeDef, &'a ast::Expr)> {
    let FunctionKind::Method { owner } = function.kind else {
        return None;
    };
    let owner = bundle.ir.types.get(owner.0)?;
    let method = find_source_method(&bundle.ast, owner, function)?;
    let ast::CallableBody::Expr(expr) = method.body.as_ref()? else {
        return None;
    };
    Some((owner, expr))
}

fn find_source_method<'a>(
    program: &'a ast::Program,
    owner: &ir::TypeDef,
    function: &ir::Function,
) -> Option<&'a ast::MethodDecl> {
    for item in &program.items {
        match item {
            ast::Item::Type(type_decl) if type_decl.name == owner.name => {
                for member in &type_decl.members {
                    let ast::TypeMember::Method(method) = member else {
                        continue;
                    };
                    if source_method_matches(method, function) {
                        return Some(method);
                    }
                }
            }
            ast::Item::Extension(ext_block)
                if type_ref_base_name(&ext_block.target).is_some_and(|name| name == owner.name) =>
            {
                for method in &ext_block.methods {
                    if source_method_matches(method, function) {
                        return Some(method);
                    }
                }
            }
            _ => {}
        }
    }
    None
}

fn source_method_matches(method: &ast::MethodDecl, function: &ir::Function) -> bool {
    if method.name != function.name || method.params.len() != function.params.len() {
        return false;
    }

    method
        .params
        .iter()
        .zip(function_param_types(function))
        .all(|(param, ty)| source_param_shape_matches_ir(param, &ty))
}

fn source_param_shape_matches_ir(param: &ast::Param, ty: &ir::Type) -> bool {
    if param.lazy {
        return matches!(ty, ir::Type::Function { params, .. } if params.is_empty());
    }

    match (&param.ty, ty) {
        (Some(TypeRef::Function { .. }), ir::Type::Function { .. }) => true,
        (Some(TypeRef::Function { .. }), _) => false,
        (Some(_), ir::Type::Function { .. }) => false,
        _ => true,
    }
}

fn type_ref_base_name(ty: &ast::TypeRef) -> Option<&str> {
    match ty {
        ast::TypeRef::Named { name, .. } => Some(name),
        _ => None,
    }
}

struct SourceBodyEmitter<'a> {
    function: &'a ir::Function,
    names: &'a JavaNames,
    owner: &'a ir::TypeDef,
}

impl<'a> SourceBodyEmitter<'a> {
    fn emit_body(&self, expr: &ast::Expr) -> Option<String> {
        let mut out = String::new();
        out.push_str(" {\n");
        self.emit_returning_expr(&mut out, expr, "        ", &HashMap::new(), &HashMap::new())?;
        out.push_str("    }\n");
        Some(out)
    }

    fn emit_returning_expr(
        &self,
        out: &mut String,
        expr: &ast::Expr,
        indent: &str,
        bindings: &HashMap<String, String>,
        binding_types: &HashMap<String, ir::Type>,
    ) -> Option<()> {
        match expr {
            ast::Expr::Match {
                partial: false,
                value,
                cases,
                ..
            } if self.owner.kind == TypeKind::Enum => {
                self.emit_match_return(out, value, cases, indent, bindings, binding_types)
            }
            _ => {
                out.push_str(indent);
                let source_ty = self
                    .expr_type(expr, bindings)
                    .or_else(|| self.pattern_binding_expr_type(expr, binding_types));
                let mut emitted =
                    self.emit_expr_against(expr, bindings, &self.function.return_ty)?;
                if let Some(source_ty) = source_ty
                    && source_ty != self.function.return_ty
                    && self.generic_types_are_equal(&source_ty, &self.function.return_ty)
                {
                    emitted = format!(
                        "(({}) ((Object) {}))",
                        self.names.value_type(&self.function.return_ty),
                        emitted
                    );
                }
                if is_java_void_type(&self.function.return_ty) {
                    out.push_str(&emitted);
                    out.push_str(";\n");
                } else {
                    out.push_str("return ");
                    out.push_str(&emitted);
                    out.push_str(";\n");
                }
                Some(())
            }
        }
    }

    fn emit_match_return(
        &self,
        out: &mut String,
        value: &ast::Expr,
        cases: &[ast::MatchCase],
        indent: &str,
        bindings: &HashMap<String, String>,
        binding_types: &HashMap<String, ir::Type>,
    ) -> Option<()> {
        let value = self.emit_expr(value, bindings)?;
        for (index, case) in cases.iter().enumerate() {
            if case.guard.is_some() {
                return None;
            }
            let matched =
                self.match_case_pattern(&case.pattern, &value, index, bindings, binding_types)?;
            out.push_str(indent);
            out.push_str("if (");
            out.push_str(&matched.condition);
            out.push_str(") {\n");
            self.emit_match_case_body(
                out,
                &case.body,
                &format!("{indent}    "),
                &matched.bindings,
                &matched.binding_types,
            )?;
            out.push_str(indent);
            out.push_str("}\n");
        }
        out.push_str(indent);
        out.push_str("throw new IllegalStateException(\"non-exhaustive Lume match\");\n");
        Some(())
    }

    fn emit_match_case_body(
        &self,
        out: &mut String,
        body: &ast::MatchCaseBody,
        indent: &str,
        bindings: &HashMap<String, String>,
        binding_types: &HashMap<String, ir::Type>,
    ) -> Option<()> {
        match body {
            ast::MatchCaseBody::Expr(expr) => {
                self.emit_returning_expr(out, expr, indent, bindings, binding_types)
            }
            ast::MatchCaseBody::Block(_) => None,
        }
    }

    fn match_case_pattern(
        &self,
        pattern: &ast::Pattern,
        value: &str,
        index: usize,
        parent_bindings: &HashMap<String, String>,
        parent_binding_types: &HashMap<String, ir::Type>,
    ) -> Option<MatchedCase> {
        match pattern {
            ast::Pattern::Constructor { path, args, .. } => {
                let case_name = path.last()?;
                self.enum_case_match(
                    case_name,
                    args,
                    value,
                    index,
                    parent_bindings,
                    parent_binding_types,
                )
            }
            ast::Pattern::Binding { name, .. } if self.enum_case(name).is_some() => self
                .enum_case_match(
                    name,
                    &[],
                    value,
                    index,
                    parent_bindings,
                    parent_binding_types,
                ),
            ast::Pattern::List { .. } => None,
            _ => None,
        }
    }

    fn enum_case_match(
        &self,
        case_name: &str,
        args: &[ast::Pattern],
        value: &str,
        index: usize,
        parent_bindings: &HashMap<String, String>,
        parent_binding_types: &HashMap<String, ir::Type>,
    ) -> Option<MatchedCase> {
        let enum_case = self.enum_case(case_name)?;
        if enum_case.fields.len() != args.len() {
            return None;
        }

        let java_case = java_type_name(case_name);
        let case_local = format!("__case{index}");
        let case_type = format!(
            "{java_case}{}",
            java_wildcard_type_args(self.owner.type_params.len())
        );
        let needs_case_local = args
            .iter()
            .any(|arg| matches!(arg, ast::Pattern::Binding { .. }));
        let condition = if needs_case_local {
            format!("{value} instanceof {case_type} {case_local}")
        } else {
            format!("{value} instanceof {case_type}")
        };
        let mut bindings = parent_bindings.clone();
        let mut binding_types = parent_binding_types.clone();
        for (arg, field) in args.iter().zip(&enum_case.fields) {
            match arg {
                ast::Pattern::Wildcard { .. } => {}
                ast::Pattern::Binding { name, .. } => {
                    bindings.insert(
                        name.clone(),
                        format!(
                            "(({}) {}.{}())",
                            self.names.value_type(&field.ty),
                            case_local,
                            java_member_name(&field.name)
                        ),
                    );
                    binding_types.insert(name.clone(), field.ty.clone());
                }
                ast::Pattern::List { .. } => return None,
                _ => return None,
            }
        }

        Some(MatchedCase {
            condition,
            bindings,
            binding_types,
        })
    }

    fn enum_case(&self, name: &str) -> Option<&'a ir::EnumCase> {
        self.owner.enum_cases.iter().find(|case| case.name == name)
    }

    fn emit_expr(&self, expr: &ast::Expr, bindings: &HashMap<String, String>) -> Option<String> {
        match expr {
            ast::Expr::Identifier { name, .. } if name == "this" => Some("this".to_string()),
            ast::Expr::Identifier { name, .. } if self.enum_case(name).is_some() => {
                Some(format!("new {}<>()", java_type_name(name)))
            }
            ast::Expr::Identifier { name, .. } if core_enum_case_owner(name).is_some() => {
                self.emit_core_enum_case(name, &[])
            }
            ast::Expr::Identifier { name, .. } => bindings
                .get(name)
                .cloned()
                .or_else(|| self.lazy_param_value_reference(name))
                .or_else(|| self.param_reference(name)),
            ast::Expr::Bool { value, .. } => Some(value.to_string()),
            ast::Expr::Integer { raw, .. } => Some(format!("{raw}L")),
            ast::Expr::Float { raw, .. } => Some(raw.clone()),
            ast::Expr::String { raw, .. } => {
                Some(java_string_literal(&decode_lume_string_literal(raw)))
            }
            ast::Expr::Unit { .. } => Some("lume.core.LumeUnit.INSTANCE".to_string()),
            ast::Expr::ListLiteral { items, .. } => {
                if items
                    .iter()
                    .any(|item| matches!(item, ast::Expr::Spread { .. }))
                {
                    let mut out = "lume.core.LumeVector.empty()".to_string();
                    for item in items {
                        match item {
                            ast::Expr::Spread { value, .. } => {
                                out =
                                    format!("{}.addAll({})", out, self.emit_expr(value, bindings)?);
                            }
                            _ => {
                                out = format!("{}.add({})", out, self.emit_expr(item, bindings)?);
                            }
                        }
                    }
                    Some(out)
                } else {
                    let items = items
                        .iter()
                        .map(|item| self.emit_expr(item, bindings))
                        .collect::<Option<Vec<_>>>()?;
                    Some(format!("lume.core.LumeVector.of({})", items.join(", ")))
                }
            }
            ast::Expr::Spread { value, .. } => self.emit_expr(value, bindings),
            ast::Expr::Group { inner, .. } => self.emit_expr(inner, bindings),
            ast::Expr::Call { callee, args, .. } => self.emit_call(callee, args, bindings),
            _ => None,
        }
    }

    fn emit_expr_against(
        &self,
        expr: &ast::Expr,
        bindings: &HashMap<String, String>,
        expected: &ir::Type,
    ) -> Option<String> {
        match expr {
            ast::Expr::ListLiteral { items, .. }
                if items.is_empty()
                    && matches!(
                        expected,
                        ir::Type::Named { name, args } if name == "Map" && args.len() == 2
                    ) =>
            {
                Some("lume.core.LumeMap.empty()".to_string())
            }
            ast::Expr::Group { inner, .. } => self
                .emit_expr_against(inner, bindings, expected)
                .map(|value| format!("({value})")),
            _ => self.emit_expr(expr, bindings),
        }
    }

    fn emit_call(
        &self,
        callee: &ast::Expr,
        args: &[ast::CallArg],
        bindings: &HashMap<String, String>,
    ) -> Option<String> {
        match callee {
            ast::Expr::Identifier { name, .. } if name == "panic" => {
                let message = match args.first() {
                    Some(arg) => self.emit_expr(&arg.value, bindings)?,
                    None => java_string_literal("panic"),
                };
                Some(format!("lume.core.LumePanic.panic({message})"))
            }
            ast::Expr::Identifier { name, .. } if self.enum_case(name).is_some() => {
                let args = args
                    .iter()
                    .map(|arg| self.emit_call_arg(arg, bindings))
                    .collect::<Option<Vec<_>>>()?;
                Some(format!(
                    "new {}<>({})",
                    java_type_name(name),
                    args.join(", ")
                ))
            }
            ast::Expr::Identifier { name, .. } if core_enum_case_owner(name).is_some() => {
                let args = args
                    .iter()
                    .map(|arg| self.emit_call_arg(arg, bindings))
                    .collect::<Option<Vec<_>>>()?;
                self.emit_core_enum_case(name, &args)
            }
            ast::Expr::Identifier { name, .. } if self.function_param(name).is_some() => {
                let args = args
                    .iter()
                    .map(|arg| self.emit_call_arg(arg, bindings))
                    .collect::<Option<Vec<_>>>()?;
                let target = if self.is_lazy_param(name) {
                    format!("{}.get()", self.param_reference(name)?)
                } else {
                    self.param_reference(name)?
                };
                emit_functional_call(&target, &args)
            }
            ast::Expr::Member { receiver, name, .. }
                if name == "iterator"
                    && args.is_empty()
                    && matches!(
                        receiver.as_ref(),
                        ast::Expr::ListLiteral { items, .. } if items.is_empty()
                    ) =>
            {
                Some("lume.core.LumeIterator.from(lume.core.LumeVector.of())".to_string())
            }
            ast::Expr::Member { receiver, name, .. }
                if name == "iterator"
                    && args.is_empty()
                    && matches!(
                        receiver.as_ref(),
                        ast::Expr::Call {
                            callee,
                            args,
                            uses_brace_syntax: false,
                            ..
                        } if args.is_empty()
                            && matches!(
                                callee.as_ref(),
                                ast::Expr::Identifier { name, .. } if name == "Vector"
                            )
                    ) =>
            {
                Some("lume.core.LumeIterator.from(lume.core.LumeVector.of())".to_string())
            }
            ast::Expr::Member { name, .. } if lazy_core_member_call_name(name) => None,
            ast::Expr::Member { receiver, name, .. } => {
                let mut receiver_expr = self.emit_expr(receiver, bindings)?;
                if let Some(receiver_ty) = self.expr_type(receiver, bindings)
                    && let Some(param) = self.type_param_name(&receiver_ty)
                    && let Some(bound) = self.generic_bound_for_type_param(param)
                {
                    receiver_expr =
                        format!("(({}) {})", self.names.value_type(&bound), receiver_expr);
                }
                let args = args
                    .iter()
                    .map(|arg| self.emit_call_arg(arg, bindings))
                    .collect::<Option<Vec<_>>>()?;
                Some(format!(
                    "{}.{}({})",
                    receiver_expr,
                    java_member_name(name),
                    args.join(", ")
                ))
            }
            _ => None,
        }
    }

    fn emit_call_arg(
        &self,
        arg: &ast::CallArg,
        bindings: &HashMap<String, String>,
    ) -> Option<String> {
        match &arg.value {
            ast::Expr::Spread { value, .. } => self.emit_expr(value, bindings),
            _ => self.emit_expr(&arg.value, bindings),
        }
    }

    fn emit_core_enum_case(&self, case: &str, args: &[String]) -> Option<String> {
        let owner = core_enum_case_owner(case)?;
        Some(format!(
            "new lume.core.{}.{}<>({})",
            java_type_name(owner),
            java_type_name(case),
            args.join(", ")
        ))
    }

    fn param_reference(&self, name: &str) -> Option<String> {
        self.function
            .params
            .iter()
            .filter_map(|param| self.function.locals.get(param.0))
            .find(|local| local.name == name)
            .map(java_local_name)
    }

    fn function_param(&self, name: &str) -> Option<&'a ir::Local> {
        self.function
            .params
            .iter()
            .filter_map(|param| self.function.locals.get(param.0))
            .find(|local| local.name == name && matches!(local.ty, ir::Type::Function { .. }))
    }

    fn expr_type(&self, expr: &ast::Expr, _bindings: &HashMap<String, String>) -> Option<ir::Type> {
        match expr {
            ast::Expr::Identifier { name, .. } => self
                .function
                .params
                .iter()
                .filter_map(|param| self.function.locals.get(param.0))
                .find(|local| local.name == *name)
                .map(|local| local.ty.clone()),
            ast::Expr::Group { inner, .. } => self.expr_type(inner, _bindings),
            _ => None,
        }
    }

    fn pattern_binding_expr_type(
        &self,
        expr: &ast::Expr,
        binding_types: &HashMap<String, ir::Type>,
    ) -> Option<ir::Type> {
        match expr {
            ast::Expr::Identifier { name, .. } => binding_types.get(name).cloned(),
            ast::Expr::Group { inner, .. } => self.pattern_binding_expr_type(inner, binding_types),
            _ => None,
        }
    }

    fn type_param_name<'ty>(&self, ty: &'ty ir::Type) -> Option<&'ty str> {
        match ty {
            ir::Type::TypeParam(name) => Some(name),
            ir::Type::Named { name, args }
                if args.is_empty()
                    && (self.function.type_params.contains(name)
                        || self.owner.type_params.contains(name)) =>
            {
                Some(name)
            }
            _ => None,
        }
    }

    fn generic_conditions(&self) -> impl Iterator<Item = &ir::GenericCondition> {
        self.function
            .generic_conditions
            .iter()
            .chain(self.owner.generic_conditions.iter())
    }

    fn generic_bound_for_type_param(&self, name: &str) -> Option<ir::Type> {
        self.generic_conditions()
            .find_map(|condition| match condition {
                ir::GenericCondition::Bound {
                    subject: ir::Type::TypeParam(subject),
                    bound,
                } if subject == name => Some(bound.clone()),
                _ => None,
            })
    }

    fn generic_types_are_equal(&self, left: &ir::Type, right: &ir::Type) -> bool {
        let (Some(left), Some(right)) = (self.type_param_name(left), self.type_param_name(right))
        else {
            return false;
        };
        if left == right {
            return false;
        }
        let mut equivalent = HashSet::from([left.to_string()]);
        loop {
            let mut changed = false;
            for condition in self.generic_conditions() {
                let ir::GenericCondition::Equal { left, right } = condition else {
                    continue;
                };
                let (Some(left), Some(right)) =
                    (self.type_param_name(left), self.type_param_name(right))
                else {
                    continue;
                };
                if equivalent.contains(left) {
                    changed |= equivalent.insert(right.to_string());
                }
                if equivalent.contains(right) {
                    changed |= equivalent.insert(left.to_string());
                }
            }
            if !changed {
                return equivalent.contains(right);
            }
        }
    }

    fn is_lazy_param(&self, name: &str) -> bool {
        self.function
            .params
            .iter()
            .enumerate()
            .filter(|(index, _)| {
                self.function
                    .param_lazy
                    .get(*index)
                    .copied()
                    .unwrap_or(false)
            })
            .filter_map(|(_, param)| self.function.locals.get(param.0))
            .any(|local| local.name == name)
    }

    fn lazy_param_value_reference(&self, name: &str) -> Option<String> {
        self.function
            .params
            .iter()
            .enumerate()
            .filter(|(index, _)| {
                self.function
                    .param_lazy
                    .get(*index)
                    .copied()
                    .unwrap_or(false)
            })
            .filter_map(|(_, param)| self.function.locals.get(param.0))
            .find(|local| local.name == name)
            .map(|local| format!("{}.get()", java_local_name(local)))
    }
}

struct MatchedCase {
    condition: String,
    bindings: HashMap<String, String>,
    binding_types: HashMap<String, ir::Type>,
}

fn java_wildcard_type_args(count: usize) -> String {
    if count == 0 {
        String::new()
    } else {
        format!(
            "<{}>",
            std::iter::repeat_n("?", count)
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}

fn push_stub_body(out: &mut String) {
    out.push_str(" {\n");
    out.push_str("        throw new UnsupportedOperationException(\"");
    out.push_str(JAVA_UNSUPPORTED_STUB_MARKER);
    out.push_str("\");\n");
    out.push_str("    }\n");
}

fn emit_field_initializer_constructor_body(
    bundle: &BackendBundle,
    function: &ir::Function,
    names: &JavaNames,
) -> Option<String> {
    let mut overrides = HashMap::new();
    for param in &function.params {
        let local = function.locals.get(param.0)?;
        if local.name == "this" {
            overrides.insert(local.id, "this".to_string());
        }
    }
    FunctionEmitter::new(bundle, function, names)
        .with_constructor_body()
        .with_capture_overrides(overrides)
        .emit_body()
}

fn push_class_field_initializer(
    out: &mut String,
    bundle: &BackendBundle,
    ty: &ir::TypeDef,
    names: &JavaNames,
) {
    let Some(function) = ty.field_init.and_then(|id| bundle.ir.function(id)) else {
        return;
    };
    let mut overrides = HashMap::new();
    for param in &function.params {
        let local = match function.locals.get(param.0) {
            Some(local) if local.name == "this" => local,
            _ => continue,
        };
        overrides.insert(local.id, "this".to_string());
    }
    let Some(body) = FunctionEmitter::new(bundle, function, names)
        .with_capture_overrides(overrides)
        .emit_body()
    else {
        return;
    };
    out.push('\n');
    out.push_str("    private void __lume_field_init()");
    out.push_str(&body);
}

fn push_class_constructors(
    out: &mut String,
    bundle: &BackendBundle,
    ty: &ir::TypeDef,
    names: &JavaNames,
) {
    let explicit_constructors = ty
        .methods
        .iter()
        .filter_map(|method_id| bundle.ir.function(*method_id))
        .filter(|function| function.name == "new")
        .collect::<Vec<_>>();
    if !explicit_constructors.is_empty() {
        for constructor in explicit_constructors {
            push_explicit_class_constructor(out, bundle, ty, constructor, names);
        }
        return;
    }
    push_implicit_class_constructors(out, ty, names);
}

fn push_explicit_class_constructor(
    out: &mut String,
    bundle: &BackendBundle,
    ty: &ir::TypeDef,
    function: &ir::Function,
    names: &JavaNames,
) {
    out.push('\n');
    out.push_str("    public ");
    out.push_str(&java_type_name(&ty.name));
    out.push('(');
    out.push_str(&java_param_list(function, names, true));
    out.push(')');
    match FunctionEmitter::new(bundle, function, names)
        .with_prologue_line(ty.field_init.map(|_| "this.__lume_field_init();"))
        .with_constructor_body()
        .emit_body()
    {
        Some(body) => out.push_str(&body),
        None => push_stub_body(out),
    }
}

fn push_implicit_class_constructors(out: &mut String, ty: &ir::TypeDef, names: &JavaNames) {
    let name = java_type_name(&ty.name);
    let has_field_init = ty.field_init.is_some();
    out.push('\n');
    out.push_str("    public ");
    out.push_str(&name);
    out.push_str("() {\n");
    if has_field_init {
        out.push_str("        this.__lume_field_init();\n");
    }
    out.push_str("    }\n");
    if ty.fields.is_empty() {
        return;
    }

    out.push('\n');
    out.push_str("    public ");
    out.push_str(&name);
    out.push('(');
    out.push_str(
        &ty.fields
            .iter()
            .enumerate()
            .map(|(index, field)| {
                format!(
                    "{} {}",
                    names.value_type(&field.ty),
                    constructor_param_name(field, index)
                )
            })
            .collect::<Vec<_>>()
            .join(", "),
    );
    out.push_str(") {\n");
    if has_field_init {
        out.push_str("        this.__lume_field_init();\n");
    }
    for (index, field) in ty.fields.iter().enumerate() {
        out.push_str("        this.");
        out.push_str(&java_member_name(&field.name));
        out.push_str(" = ");
        out.push_str(&constructor_param_name(field, index));
        out.push_str(";\n");
    }
    out.push_str("    }\n");
}

fn constructor_param_name(field: &ir::Field, index: usize) -> String {
    format!("{}_arg{index}", java_member_name(&field.name))
}

struct FunctionEmitter<'a> {
    bundle: &'a BackendBundle,
    function: &'a ir::Function,
    names: &'a JavaNames,
    module_class: String,
    constructor_body: bool,
    capture_overrides: HashMap<ir::LocalId, String>,
    inferred_local_types: HashMap<ir::LocalId, ir::Type>,
    assignment_counts: HashMap<ir::LocalId, usize>,
    captured_locals: HashSet<ir::LocalId>,
    control_var: String,
    local_prefix: String,
    prologue_lines: Vec<String>,
}

impl<'a> FunctionEmitter<'a> {
    fn new(bundle: &'a BackendBundle, function: &'a ir::Function, names: &'a JavaNames) -> Self {
        let mut emitter = Self {
            bundle,
            function,
            names,
            module_class: module_class_name(bundle),
            constructor_body: false,
            capture_overrides: HashMap::new(),
            inferred_local_types: HashMap::new(),
            assignment_counts: HashMap::new(),
            captured_locals: HashSet::new(),
            control_var: "__block".to_string(),
            local_prefix: String::new(),
            prologue_lines: Vec::new(),
        };
        emitter.index_local_usage();
        emitter.infer_local_types();
        emitter
    }

    fn with_constructor_body(mut self) -> Self {
        self.constructor_body = true;
        self
    }

    fn with_capture_overrides(mut self, capture_overrides: HashMap<ir::LocalId, String>) -> Self {
        self.capture_overrides = capture_overrides;
        self
    }

    fn with_control_var(mut self, control_var: impl Into<String>) -> Self {
        self.control_var = control_var.into();
        self
    }

    fn with_local_prefix(mut self, local_prefix: impl Into<String>) -> Self {
        self.local_prefix = local_prefix.into();
        self
    }

    fn with_prologue_line(mut self, line: Option<&str>) -> Self {
        if let Some(line) = line {
            self.prologue_lines.push(line.to_string());
        }
        self
    }

    fn index_local_usage(&mut self) {
        for block in &self.function.blocks {
            for statement in &block.statements {
                let ir::StatementKind::Assign { target, value } = &statement.kind else {
                    continue;
                };
                if let ir::Place::Local(local_id) = target {
                    *self.assignment_counts.entry(*local_id).or_insert(0) += 1;
                }
                self.index_rvalue_usage(value);
            }
        }
    }

    fn index_rvalue_usage(&mut self, value: &ir::RValue) {
        if let ir::RValue::Closure { captures, .. } = value {
            for capture in captures {
                if let Some(local_id) = operand_local_id(capture) {
                    self.captured_locals.insert(local_id);
                }
            }
        }
    }

    fn infer_local_types(&mut self) {
        let mut changed = true;
        while changed {
            changed = false;
            for block in &self.function.blocks {
                for statement in &block.statements {
                    let ir::StatementKind::Assign { target, value } = &statement.kind else {
                        continue;
                    };
                    let ir::Place::Local(local_id) = target else {
                        continue;
                    };
                    let Some(local) = self.function.locals.get(local_id.0) else {
                        continue;
                    };
                    if !matches!(local.ty, ir::Type::Unknown)
                        || self.inferred_local_types.contains_key(local_id)
                    {
                        continue;
                    }
                    let Some(inferred) = self.rvalue_type(value) else {
                        continue;
                    };
                    if matches!(inferred, ir::Type::Unknown) {
                        continue;
                    }
                    if self.type_has_unbound_type_params(&inferred) {
                        continue;
                    }
                    self.inferred_local_types.insert(*local_id, inferred);
                    changed = true;
                }
            }
        }
    }

    fn emit_body(&self) -> Option<String> {
        let mut out = String::new();
        out.push_str(" {\n");
        for line in &self.prologue_lines {
            out.push_str("        ");
            out.push_str(line);
            out.push('\n');
        }
        for local in &self.function.locals {
            if self.local_is_declared_elsewhere(local) {
                continue;
            }
            let local_ty = self
                .inferred_local_types
                .get(&local.id)
                .unwrap_or(&local.ty);
            out.push_str("        ");
            out.push_str(&self.local_value_type(local_ty));
            out.push(' ');
            out.push_str(&self.local_name(local));
            out.push_str(" = ");
            out.push_str(&java_default_value(local_ty));
            out.push_str(";\n");
        }

        if !self.function.blocks.is_empty() {
            out.push_str("        int ");
            out.push_str(&self.control_var);
            out.push_str(" = ");
            out.push_str(&self.function.entry.0.to_string());
            out.push_str(";\n");
            out.push_str("        while (true) {\n");
            out.push_str("            switch (");
            out.push_str(&self.control_var);
            out.push_str(") {\n");
            for block in &self.function.blocks {
                self.emit_block(&mut out, block)?;
            }
            out.push_str("                default:\n");
            out.push_str(
                "                    throw new IllegalStateException(\"unknown Lume block \" + ",
            );
            out.push_str(&self.control_var);
            out.push_str(");\n");
            out.push_str("            }\n");
            out.push_str("        }\n");
        }

        out.push_str("    }\n");
        Some(out)
    }

    fn emit_block(&self, out: &mut String, block: &ir::BasicBlock) -> Option<()> {
        out.push_str("                case ");
        out.push_str(&block.id.0.to_string());
        out.push_str(":\n");
        out.push_str("                {\n");
        for statement in &block.statements {
            self.emit_statement(out, statement)?;
        }
        self.emit_terminator(out, &block.terminator)?;
        out.push_str("                }\n");
        if !terminator_exits_case(&block.terminator.kind) {
            out.push_str("                break;\n");
        }
        Some(())
    }

    fn emit_statement(&self, out: &mut String, statement: &ir::Statement) -> Option<()> {
        match &statement.kind {
            ir::StatementKind::Assign { target, value } => {
                let target_ty = self.place_type(target);
                let value_ty = self.rvalue_type(value);
                let closure_capture_initializers = match value {
                    ir::RValue::Closure { function, captures } => {
                        Some(self.emit_closure_capture_snapshots(out, *function, captures)?)
                    }
                    _ => None,
                };
                let anonymous_object_capture_initializers = match value {
                    ir::RValue::AnonymousObject {
                        fields, methods, ..
                    } => Some(self.emit_anonymous_object_capture_snapshots(out, fields, methods)?),
                    _ => None,
                };
                if matches!(
                    value,
                    ir::RValue::Use(ir::Operand::Const(ir::Constant::Unit))
                ) && !matches!(target, ir::Place::Index { .. })
                    && target_ty.as_ref().is_some_and(|ty| !is_java_void_type(ty))
                {
                    let target_ty = target_ty.as_ref()?;
                    out.push_str("                    ");
                    out.push_str(&self.emit_place(target)?);
                    out.push_str(" = (");
                    out.push_str(&self.names.value_type(target_ty));
                    out.push_str(") ((Object) lume.core.LumeUnit.INSTANCE);\n");
                    return Some(());
                }
                if target_ty.as_ref().is_some_and(is_java_void_type)
                    && rvalue_can_be_java_statement(value)
                    && !matches!(target, ir::Place::Index { .. })
                {
                    out.push_str("                    ");
                    out.push_str(&self.emit_rvalue(value)?);
                    out.push_str(";\n");
                    out.push_str("                    ");
                    out.push_str(&self.emit_place(target)?);
                    out.push_str(" = lume.core.LumeUnit.INSTANCE;\n");
                    return Some(());
                }
                let mut value_expr = match value {
                    ir::RValue::Closure { function, captures } => self.emit_closure(
                        *function,
                        captures,
                        closure_capture_initializers.as_deref(),
                    )?,
                    ir::RValue::Record(fields) => {
                        let target_ty = target_ty.as_ref()?;
                        self.emit_record_as_named_construct(target_ty, fields)?
                    }
                    ir::RValue::AnonymousObject {
                        ty,
                        fields,
                        methods,
                    } => {
                        let (field_values, method_captures) =
                            anonymous_object_capture_initializers.as_ref()?;
                        self.emit_anonymous_object(
                            ty,
                            fields,
                            methods,
                            Some(field_values),
                            Some(method_captures),
                        )?
                    }
                    _ => self.emit_rvalue(value)?,
                };
                if let Some(target_ty) = target_ty {
                    value_expr = self.coerce_to_target_type(value_expr, value_ty, &target_ty);
                }
                if let ir::Place::Index { base, index } = target {
                    out.push_str("                    ");
                    out.push_str(&self.emit_index_assignment(base, index, &value_expr)?);
                    out.push_str(";\n");
                    return Some(());
                }
                out.push_str("                    ");
                out.push_str(&self.emit_place(target)?);
                out.push_str(" = ");
                out.push_str(&value_expr);
                out.push_str(";\n");
                Some(())
            }
            ir::StatementKind::Eval { value } => {
                if !rvalue_can_be_java_statement(value) {
                    return Some(());
                }
                out.push_str("                    ");
                out.push_str(&self.emit_rvalue(value)?);
                out.push_str(";\n");
                Some(())
            }
            ir::StatementKind::Defer { .. } => self.unsupported("defer statement"),
        }
    }

    fn emit_terminator(&self, out: &mut String, terminator: &ir::Terminator) -> Option<()> {
        match &terminator.kind {
            ir::TerminatorKind::Goto(target) => {
                out.push_str("                    ");
                out.push_str(&self.control_var);
                out.push_str(" = ");
                out.push_str(&target.0.to_string());
                out.push_str(";\n");
                Some(())
            }
            ir::TerminatorKind::Branch {
                condition,
                then_block,
                else_block,
            } => {
                let condition_ty = ir::Type::Bool;
                let condition_expr = self.coerce_to_target_type(
                    self.emit_operand(condition)?,
                    self.operand_type(condition),
                    &condition_ty,
                );
                out.push_str("                    if (");
                out.push_str(&condition_expr);
                out.push_str(") {\n");
                out.push_str("                        ");
                out.push_str(&self.control_var);
                out.push_str(" = ");
                out.push_str(&then_block.0.to_string());
                out.push_str(";\n");
                out.push_str("                    } else {\n");
                out.push_str("                        ");
                out.push_str(&self.control_var);
                out.push_str(" = ");
                out.push_str(&else_block.0.to_string());
                out.push_str(";\n");
                out.push_str("                    }\n");
                Some(())
            }
            ir::TerminatorKind::Switch {
                scrutinee,
                arms,
                default,
            } => {
                out.push_str("                    Object __switch = ");
                out.push_str(&self.emit_operand(scrutinee)?);
                out.push_str(";\n");
                for arm in arms {
                    out.push_str("                    if (java.util.Objects.equals(__switch, ");
                    out.push_str(&self.emit_switch_value(&arm.value)?);
                    out.push_str(")) {\n");
                    out.push_str("                        ");
                    out.push_str(&self.control_var);
                    out.push_str(" = ");
                    out.push_str(&arm.target.0.to_string());
                    out.push_str(";\n");
                    out.push_str("                        break;\n");
                    out.push_str("                    }\n");
                }
                out.push_str("                    ");
                out.push_str(&self.control_var);
                out.push_str(" = ");
                out.push_str(&default.0.to_string());
                out.push_str(";\n");
                Some(())
            }
            ir::TerminatorKind::Return(value) => {
                if self.constructor_body || is_java_void_type(&self.function.return_ty) {
                    out.push_str("                    return;\n");
                } else {
                    out.push_str("                    return ");
                    if let Some(value) = value {
                        let value_expr = self.coerce_to_target_type(
                            self.emit_operand(value)?,
                            self.operand_type(value),
                            &self.function.return_ty,
                        );
                        out.push_str(&value_expr);
                    } else {
                        out.push_str(&java_default_value(&self.function.return_ty));
                    }
                    out.push_str(";\n");
                }
                Some(())
            }
            ir::TerminatorKind::Unreachable => {
                out.push_str("                    throw new IllegalStateException(\"entered unreachable Lume block\");\n");
                Some(())
            }
        }
    }

    fn emit_switch_value(&self, value: &ir::SwitchValue) -> Option<String> {
        match value {
            ir::SwitchValue::Bool(value) => Some(value.to_string()),
            ir::SwitchValue::Int(value) => Some(format!("{value}L")),
            ir::SwitchValue::String(value) => Some(java_string_literal(value)),
            ir::SwitchValue::EnumCase(_) => self.unsupported("enum-case switch value"),
        }
    }

    fn emit_rvalue(&self, value: &ir::RValue) -> Option<String> {
        match value {
            ir::RValue::Use(operand) => self.emit_operand(operand),
            ir::RValue::Unary { op, operand } => {
                let op = match op {
                    ir::UnaryOp::Neg => "-",
                    ir::UnaryOp::Not => "!",
                };
                Some(format!("({op}{})", self.emit_operand(operand)?))
            }
            ir::RValue::Binary { op, left, right } => self.emit_binary(*op, left, right),
            ir::RValue::Call { callee, args, .. } => self.emit_call(callee, args),
            ir::RValue::Tuple(items) => self.emit_tuple(items),
            ir::RValue::List(items) => {
                let args = self.emit_operands(items)?;
                Some(format!("lume.core.LumeVector.of({})", args.join(", ")))
            }
            ir::RValue::AnonymousInterface {
                interfaces,
                methods,
            } => self.emit_anonymous_interface(interfaces, methods),
            ir::RValue::AnonymousObject {
                ty,
                fields,
                methods,
            } => self.emit_anonymous_object(ty, fields, methods, None, None),
            ir::RValue::Construct { ty, fields } => self.emit_construct(ty, fields),
            ir::RValue::Variant {
                enum_name,
                case_name,
                fields,
            } => self.emit_variant(enum_name, case_name, fields),
            ir::RValue::Field { base, name } if name == "runtimeType" => {
                self.emit_runtime_type_for_operand(base)
            }
            ir::RValue::Field { base, name } => {
                let base_expr = self.emit_operand(base)?;
                match self.operand_type(base) {
                    Some(ir::Type::Tuple(_)) => {
                        let accessor = tuple_accessor_name(name)?;
                        Some(format!("{base_expr}.{accessor}()"))
                    }
                    Some(ir::Type::Named {
                        name: ref type_name,
                        ..
                    }) if is_core_accessor_backed_type(type_name)
                        || self.type_def(type_name).is_some_and(|ty| {
                            ty.kind == TypeKind::Record
                                || ((ty.kind == TypeKind::Interface
                                    || is_anonymous_object_type(ty))
                                    && ty.fields.iter().any(|field| field.name == *name))
                        }) =>
                    {
                        Some(format!("{base_expr}.{}()", java_member_name(name)))
                    }
                    _ => Some(format!("{base_expr}.{}", java_member_name(name))),
                }
            }
            ir::RValue::TypeOf { ty } => Some(type_value_expr(ty, self.names)),
            ir::RValue::Cast { operand, ty } => self
                .emit_operand(operand)
                .map(|expr| self.unchecked_reference_cast(expr, ty)),
            ir::RValue::NamedValue { path } => self.emit_named_runtime_value(path),
            ir::RValue::Record(fields) => self.unsupported(&format!(
                "anonymous shape literal {{{}}}",
                fields
                    .iter()
                    .map(|field| field.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
            ir::RValue::RecordSpread(_) => self.unsupported("anonymous shape spread"),
            ir::RValue::RecordUpdate { .. } => self.unsupported("shape update"),
            ir::RValue::Index { base, index } => self.emit_index(base, index),
            ir::RValue::TypeTest { operand, ty } => self.emit_type_test(operand, ty),
            ir::RValue::Closure { function, captures } => {
                self.emit_closure(*function, captures, None)
            }
        }
    }

    fn emit_runtime_type_for_operand(&self, operand: &ir::Operand) -> Option<String> {
        self.operand_type(operand)
            .map(|ty| {
                if is_named_builtin(&ty, "Any") {
                    format!(
                        "lume.core.LumeRuntime.runtimeTypeOf({})",
                        self.emit_operand(operand)
                            .unwrap_or_else(|| "null".to_string())
                    )
                } else {
                    type_value_expr(&ty, self.names)
                }
            })
            .or_else(|| match operand {
                ir::Operand::Const(ir::Constant::Bool(_)) => {
                    Some("lume.core.LumeType.primitive(\"Bool\")".to_string())
                }
                ir::Operand::Const(ir::Constant::Int(_)) => {
                    Some("lume.core.LumeType.primitive(\"Int\")".to_string())
                }
                ir::Operand::Const(ir::Constant::Float(_)) => {
                    Some("lume.core.LumeType.primitive(\"Float\")".to_string())
                }
                ir::Operand::Const(ir::Constant::String(_)) => {
                    Some("lume.core.LumeType.primitive(\"Str\")".to_string())
                }
                ir::Operand::Const(ir::Constant::Unit) => {
                    Some("lume.core.LumeType.primitive(\"Unit\")".to_string())
                }
                ir::Operand::Const(ir::Constant::List(_)) => {
                    Some("lume.core.LumeType.primitive(\"Vector\")".to_string())
                }
                _ => None,
            })
    }

    fn emit_type_test(&self, operand: &ir::Operand, ty: &ir::Type) -> Option<String> {
        let value = self.emit_operand(operand)?;
        if matches!(ty, ir::Type::Never) {
            return Some("false".to_string());
        }
        if matches!(ty, ir::Type::Unknown) || is_named_builtin(ty, "Any") {
            return Some(format!("{value} != null"));
        }
        if matches!(ty, ir::Type::Record(_)) {
            return self.unsupported("anonymous shape type test");
        }

        let erased = match ty {
            ir::Type::Named { name, .. } => ir::Type::Named {
                name: name.clone(),
                args: Vec::new(),
            },
            ir::Type::Tuple(items) => ir::Type::Tuple(vec![ir::Type::Unknown; items.len()]),
            ir::Type::Function { params, .. } => ir::Type::Function {
                params: vec![ir::Type::Unknown; params.len()],
                ret: Box::new(ir::Type::Unknown),
            },
            other => other.clone(),
        };
        let java_type = self.names.value_type(&erased);
        let raw_java_type = java_type.split('<').next().unwrap_or(&java_type);
        Some(format!("{value} instanceof {raw_java_type}"))
    }

    fn emit_index(&self, base: &ir::Operand, index: &ir::Operand) -> Option<String> {
        let base_ty = self.operand_type(base)?;
        let base_expr = self.emit_operand(base)?;
        let index_expr = self.emit_operand(index)?;

        match base_ty {
            ir::Type::Named { ref name, ref args }
                if (name == "Vector" || name == "Array") && args.len() == 1 =>
            {
                let indexed =
                    format!("lume.core.LumeRuntime.indexValue({base_expr}, {index_expr})");
                Some(self.coerce_to_target_type(indexed, Some(ir::Type::Unknown), &args[0]))
            }
            ir::Type::Named { ref name, ref args } if name == "Map" && args.len() == 2 => {
                Some(format!("{base_expr}.get({index_expr})"))
            }
            _ => self.unsupported("index expression"),
        }
    }

    fn emit_index_assignment(
        &self,
        base: &ir::Operand,
        index: &ir::Operand,
        value: &str,
    ) -> Option<String> {
        let base_ty = self.operand_type(base)?;
        let base_expr = self.emit_operand(base)?;
        let index_expr = self.emit_operand(index)?;
        match base_ty {
            ir::Type::Named { ref name, ref args }
                if matches!(name.as_str(), "Array" | "Vector") && args.len() == 1 =>
            {
                Some(format!("{base_expr}.set({index_expr}, {value})"))
            }
            ir::Type::Named { ref name, ref args } if name == "Map" && args.len() == 2 => {
                Some(format!("{base_expr}.set({index_expr}, {value})"))
            }
            _ => self.unsupported("indexed assignment target"),
        }
    }

    fn emit_binary(
        &self,
        op: ir::BinaryOp,
        left: &ir::Operand,
        right: &ir::Operand,
    ) -> Option<String> {
        let left = self.emit_operand(left)?;
        let right = self.emit_operand(right)?;
        match op {
            ir::BinaryOp::Eq => Some(format!("java.util.Objects.equals({left}, {right})")),
            ir::BinaryOp::NotEq => Some(format!("!java.util.Objects.equals({left}, {right})")),
            _ => {
                let op = match op {
                    ir::BinaryOp::Add => "+",
                    ir::BinaryOp::Sub => "-",
                    ir::BinaryOp::Mul => "*",
                    ir::BinaryOp::Div => "/",
                    ir::BinaryOp::Mod => "%",
                    ir::BinaryOp::Less => "<",
                    ir::BinaryOp::LessEq => "<=",
                    ir::BinaryOp::Greater => ">",
                    ir::BinaryOp::GreaterEq => ">=",
                    ir::BinaryOp::And => "&&",
                    ir::BinaryOp::Or => "||",
                    ir::BinaryOp::Eq | ir::BinaryOp::NotEq => unreachable!(),
                };
                Some(format!("({left} {op} {right})"))
            }
        }
    }

    fn emit_call(&self, callee: &ir::Callee, args: &[ir::Operand]) -> Option<String> {
        match callee {
            ir::Callee::Direct(id) => {
                let target = self.bundle.ir.function(*id)?;
                let args = self.emit_operands_for_function(args, target)?;
                match target.kind {
                    ir::FunctionKind::TopLevel => {
                        let name = java_member_name(&target.name);
                        if matches!(self.function.kind, ir::FunctionKind::TopLevel) {
                            Some(format!("{name}({})", args.join(", ")))
                        } else {
                            Some(format!(
                                "{}.{}({})",
                                self.module_class,
                                name,
                                args.join(", ")
                            ))
                        }
                    }
                    ir::FunctionKind::Method { owner } => {
                        let owner_ty = self.bundle.ir.types.get(owner.0)?;
                        let name = java_member_name(&target.name);
                        if owner_ty.kind == TypeKind::Object {
                            Some(format!(
                                "{}.INSTANCE.{}({})",
                                self.names.named_type(&owner_ty.name),
                                name,
                                args.join(", ")
                            ))
                        } else if matches!(
                            self.function.kind,
                            ir::FunctionKind::Method { owner: current_owner } if current_owner == owner
                        ) {
                            Some(format!("this.{}({})", name, args.join(", ")))
                        } else {
                            self.unsupported("direct call to non-top-level function")
                        }
                    }
                    _ => self.unsupported("direct call to non-top-level function"),
                }
            }
            ir::Callee::Method { receiver, method } => match method.as_str() {
                "toStr" if args.is_empty() => self.emit_to_string_call(receiver),
                "equals" if args.len() == 1 => {
                    let args = self.emit_operands(args)?;
                    let other = args.first()?;
                    let receiver = self.emit_operand(receiver)?;
                    Some(format!("java.util.Objects.equals({receiver}, {other})"))
                }
                "isSuccess" | "isSet" | "isDefined" if args.is_empty() => {
                    let receiver = self.emit_operand(receiver)?;
                    Some(format!(
                        "lume.core.LumeRuntime.extractSuccessIsSet({receiver})"
                    ))
                }
                _ => {
                    let params = self
                        .method_param_specs_for_receiver(receiver, method, args)
                        .unwrap_or_default();
                    let args = self.emit_operands_for_param_specs(args, &params)?;
                    let mut receiver_expr = self.emit_operand(receiver)?;
                    if let Some(ir::Type::TypeParam(param)) = self.operand_type(receiver)
                        && let Some(bound) = self.generic_bound_for_type_param(&param)
                    {
                        receiver_expr =
                            format!("(({}) {})", self.names.value_type(&bound), receiver_expr);
                    }
                    Some(format!(
                        "{}.{}({})",
                        receiver_expr,
                        java_member_name(method),
                        args.join(", ")
                    ))
                }
            },
            ir::Callee::Intrinsic(intrinsic) => {
                let args = self.emit_operands(args)?;
                self.emit_intrinsic(intrinsic, &args)
            }
            ir::Callee::Named { path } => self.emit_named_runtime_call(path, args),
            ir::Callee::Indirect(callee) => self.emit_indirect_call(callee, args),
        }
    }

    fn emit_indirect_call(&self, callee: &ir::Operand, args: &[ir::Operand]) -> Option<String> {
        let params = match self.operand_type(callee) {
            Some(ir::Type::Function { params, .. }) => params,
            _ => Vec::new(),
        };
        let args = self.emit_operands_for_params(args, &params)?;
        let callee = self.emit_operand(callee)?;
        if args.len() > MAX_JAVA_FUNCTION_ARITY {
            return self.unsupported("function call with more than 12 arguments");
        }
        emit_functional_call(&callee, &args)
    }

    fn emit_to_string_call(&self, receiver: &ir::Operand) -> Option<String> {
        match receiver {
            ir::Operand::Const(ir::Constant::Unit) => {
                Some("lume.core.LumeUnit.INSTANCE.toString()".to_string())
            }
            ir::Operand::Const(ir::Constant::Bool(value)) => {
                Some(format!("Boolean.toString({value})"))
            }
            ir::Operand::Const(ir::Constant::Int(value)) => {
                Some(format!("Long.toString({value}L)"))
            }
            ir::Operand::Const(ir::Constant::Float(value)) => {
                Some(format!("Double.toString({})", java_float_literal(*value)))
            }
            ir::Operand::Const(ir::Constant::String(value)) => Some(format!(
                "{}.toString()",
                java_string_literal(&decode_lume_string_literal(value))
            )),
            _ => Some(format!("{}.toString()", self.emit_operand(receiver)?)),
        }
    }

    fn emit_named_runtime_call(&self, path: &[String], operands: &[ir::Operand]) -> Option<String> {
        match path {
            [case] if core_enum_case_owner(case).is_some() => {
                self.emit_core_enum_case_call(case, operands)
            }
            [owner, case]
                if core_enum_case_owner(case).is_some_and(|expected| expected == owner) =>
            {
                self.emit_core_enum_case_call(case, operands)
            }
            [owner, case] if self.enum_case(owner, case).is_some() => {
                self.emit_enum_case_call(owner, case, operands)
            }
            [owner, case, method] if self.enum_case(owner, case).is_some() => {
                let args = self.emit_operands(operands)?;
                let receiver = self.emit_enum_case_call(owner, case, &[])?;
                Some(format!(
                    "{}.{}({})",
                    receiver,
                    java_member_name(method),
                    args.join(", ")
                ))
            }
            [owner] if owner == "Vector" => {
                let args = self.emit_operands(operands)?;
                Some(format!("lume.core.LumeVector.of({})", args.join(", ")))
            }
            [owner] if owner == "LinkedList" => {
                let args = self.emit_operands(operands)?;
                Some(format!("lume.core.LumeLinkedList.of({})", args.join(", ")))
            }
            [owner] if owner == "Map" => {
                let args = self.emit_operands(operands)?;
                if args.is_empty() {
                    Some("lume.core.LumeMap.empty()".to_string())
                } else {
                    Some(format!("lume.core.LumeMap.fromParts({})", args.join(", ")))
                }
            }
            [owner] if owner == "Set" && operands.is_empty() => {
                Some("lume.core.LumeSet.empty()".to_string())
            }
            [owner] if owner == "Range" && operands.len() == 2 => {
                let args = self.emit_operands(operands)?;
                Some(format!("new lume.core.Range({})", args.join(", ")))
            }
            [owner, method] if owner == "Int" && method == "parse" => {
                let args = self.emit_operands(operands)?;
                Some(format!(
                    "lume.core.LumeRuntime.parseInt({})",
                    args.join(", ")
                ))
            }
            [owner, method] if owner == "Float" && method == "parse" => {
                let args = self.emit_operands(operands)?;
                Some(format!(
                    "lume.core.LumeRuntime.parseFloat({})",
                    args.join(", ")
                ))
            }
            [owner] if self.names.is_java_type(owner) => {
                let params = self
                    .constructor_param_specs(owner, operands)
                    .unwrap_or_default();
                let args = self.emit_operands_for_param_specs(operands, &params)?;
                Some(format!(
                    "new {}{}({})",
                    self.names.named_type(owner),
                    self.names.java_constructor_type_args(owner),
                    args.join(", ")
                ))
            }
            [owner] if self.is_lume_constructible_type(owner) => {
                let params = self
                    .constructor_param_specs(owner, operands)
                    .unwrap_or_default();
                let args = self.emit_operands_for_param_specs(operands, &params)?;
                Some(format!(
                    "new {}({})",
                    self.names.named_type(owner),
                    args.join(", ")
                ))
            }
            [owner, method] if self.names.is_java_single_type(owner) => {
                let params = self
                    .external_method_param_specs(owner, method, operands)
                    .or_else(|| self.type_method_param_specs(owner, method, operands))
                    .unwrap_or_default();
                let args = self.emit_operands_for_param_specs(operands, &params)?;
                Some(format!(
                    "{}.INSTANCE.{}({})",
                    self.names.named_type(owner),
                    java_member_name(method),
                    args.join(", ")
                ))
            }
            [owner, method] if self.names.is_java_type(owner) => {
                let params = self
                    .external_method_param_specs(owner, method, operands)
                    .or_else(|| self.type_method_param_specs(owner, method, operands))
                    .unwrap_or_default();
                let args = self.emit_operands_for_param_specs(operands, &params)?;
                Some(format!(
                    "{}.{}({})",
                    self.names.named_type(owner),
                    java_member_name(method),
                    args.join(", ")
                ))
            }
            [owner, method] if self.is_lume_single_type(owner) => {
                let params = self
                    .type_method_param_specs(owner, method, operands)
                    .unwrap_or_default();
                let args = self.emit_operands_for_param_specs(operands, &params)?;
                Some(format!(
                    "{}.INSTANCE.{}({})",
                    self.names.named_type(owner),
                    java_member_name(method),
                    args.join(", ")
                ))
            }
            [owner, method] if owner == "Array" => {
                let args = self.emit_operands(operands)?;
                let target = match method.as_str() {
                    "ofInt" | "ofFloat" | "ofBool" | "ofStr" | "ofRune" | "fill" => {
                        format!("lume.core.LumeArray.{}", java_member_name(method))
                    }
                    _ => return None,
                };
                Some(format!("{target}({})", args.join(", ")))
            }
            _ => self.unsupported(&format!("named runtime call {}", path.join("."))),
        }
    }

    fn emit_named_runtime_value(&self, path: &[String]) -> Option<String> {
        match path {
            [case] if core_enum_case_owner(case).is_some() => {
                self.emit_core_enum_case_call(case, &[])
            }
            [owner, case]
                if core_enum_case_owner(case).is_some_and(|expected| expected == owner) =>
            {
                self.emit_core_enum_case_call(case, &[])
            }
            [owner, case] if self.enum_case(owner, case).is_some() => {
                self.emit_enum_case_call(owner, case, &[])
            }
            _ => self.unsupported("named runtime value"),
        }
    }

    fn emit_core_enum_case_call(&self, case: &str, operands: &[ir::Operand]) -> Option<String> {
        let owner = core_enum_case_owner(case)?;
        let args = self.emit_operands(operands)?;
        Some(format!(
            "new lume.core.{}.{}<>({})",
            java_type_name(owner),
            java_type_name(case),
            args.join(", ")
        ))
    }

    fn emit_enum_case_call(
        &self,
        enum_name: &str,
        case_name: &str,
        operands: &[ir::Operand],
    ) -> Option<String> {
        let args = self.emit_operands(operands)?;
        Some(format!(
            "new {}.{}{}({})",
            self.names.named_type(enum_name),
            java_type_name(case_name),
            self.lume_constructor_type_args(enum_name),
            args.join(", ")
        ))
    }

    fn enum_case(&self, enum_name: &str, case_name: &str) -> Option<&ir::EnumCase> {
        self.type_def(enum_name)?
            .enum_cases
            .iter()
            .find(|case| case.name == case_name)
    }

    fn is_lume_constructible_type(&self, name: &str) -> bool {
        self.bundle.ir.types.iter().any(|ty| {
            ty.name == name
                && matches!(ty.kind, TypeKind::Class | TypeKind::Record | TypeKind::Enum)
        })
    }

    fn lume_constructor_type_args(&self, name: &str) -> &'static str {
        if self
            .type_def(name)
            .is_some_and(|ty| !ty.type_params.is_empty())
        {
            "<>"
        } else {
            ""
        }
    }

    fn is_lume_single_type(&self, name: &str) -> bool {
        self.type_def(name)
            .is_some_and(|ty| ty.kind == TypeKind::Object)
    }

    fn emit_intrinsic(&self, intrinsic: &ir::Intrinsic, args: &[String]) -> Option<String> {
        match intrinsic {
            ir::Intrinsic::Print => {
                Some(format!("lume.core.LumeRuntime.print({})", args.join(", ")))
            }
            ir::Intrinsic::Println => Some(format!(
                "lume.core.LumeRuntime.println({})",
                args.join(", ")
            )),
            ir::Intrinsic::Printf => {
                Some(format!("lume.core.LumeRuntime.printf({})", args.join(", ")))
            }
            ir::Intrinsic::Panic => Some(format!(
                "lume.core.LumePanic.panic({})",
                args.first()
                    .cloned()
                    .unwrap_or_else(|| java_string_literal("panic"))
            )),
            ir::Intrinsic::Assert => {
                let condition = args.first()?;
                let message = args
                    .get(1)
                    .cloned()
                    .unwrap_or_else(|| java_string_literal("assertion failed"));
                Some(format!(
                    "lume.core.LumeRuntime.assertTrue({condition}, {message})"
                ))
            }
            ir::Intrinsic::Ensure => {
                if args.len() != 2 {
                    return None;
                }
                Some(format!(
                    "lume.core.LumeRuntime.ensure({}, {})",
                    args[0], args[1]
                ))
            }
            ir::Intrinsic::Identity => {
                if args.len() != 1 {
                    return None;
                }
                Some(args[0].clone())
            }
            ir::Intrinsic::ExtractSuccessIsSet => {
                if args.len() != 1 {
                    return None;
                }
                Some(format!(
                    "lume.core.LumeRuntime.extractSuccessIsSet({})",
                    args[0]
                ))
            }
            ir::Intrinsic::ExtractSuccessValue => {
                if args.len() != 1 {
                    return None;
                }
                Some(format!(
                    "lume.core.LumeRuntime.probeSuccessValue({})",
                    args[0]
                ))
            }
            ir::Intrinsic::UnsafeExtractSuccessValue => {
                if args.len() != 1 {
                    return None;
                }
                Some(format!(
                    "lume.core.LumeRuntime.extractSuccessValue({})",
                    args[0]
                ))
            }
            ir::Intrinsic::ListAppend => {
                if args.len() != 2 {
                    return None;
                }
                Some(format!("{}.add({})", args[0], args[1]))
            }
            ir::Intrinsic::ListExtend => {
                if args.len() != 2 {
                    return None;
                }
                Some(format!("{}.addAll({})", args[0], args[1]))
            }
            ir::Intrinsic::ListLen => {
                if args.len() != 1 {
                    return None;
                }
                Some(format!("lume.core.LumeRuntime.listLen({})", args[0]))
            }
            ir::Intrinsic::ListGet => {
                if args.len() != 2 {
                    return None;
                }
                Some(format!(
                    "lume.core.LumeRuntime.listGet({}, {})",
                    args[0], args[1]
                ))
            }
            ir::Intrinsic::ListSlice => {
                if args.len() != 2 {
                    return None;
                }
                Some(format!(
                    "lume.core.LumeRuntime.listSlice({}, {})",
                    args[0], args[1]
                ))
            }
            ir::Intrinsic::IterInit => {
                if args.len() != 1 {
                    return None;
                }
                Some(format!("lume.core.LumeRuntime.iterInit({})", args[0]))
            }
            ir::Intrinsic::IterHasNext => {
                if args.len() != 1 {
                    return None;
                }
                Some(format!("lume.core.LumeRuntime.iterHasNext({})", args[0]))
            }
            ir::Intrinsic::IterNext => {
                if args.len() != 1 {
                    return None;
                }
                Some(format!("lume.core.LumeRuntime.iterNext({})", args[0]))
            }
            ir::Intrinsic::VariantIs(case_name) => {
                if args.len() != 1 {
                    return None;
                }
                Some(format!(
                    "lume.core.LumeRuntime.variantIs({}, {})",
                    args[0],
                    java_string_literal(case_name)
                ))
            }
            ir::Intrinsic::VariantField(field_name) => {
                if args.len() != 1 {
                    return None;
                }
                Some(format!(
                    "lume.core.LumeRuntime.variantField({}, {})",
                    args[0],
                    java_string_literal(field_name)
                ))
            }
        }
    }

    fn emit_tuple(&self, items: &[ir::Operand]) -> Option<String> {
        if !(2..=8).contains(&items.len()) {
            return self.unsupported("tuple arity outside Java Tuple2..Tuple8");
        }
        let args = self.emit_operands(items)?;
        Some(format!(
            "new lume.core.Tuple{}<>({})",
            items.len(),
            args.join(", ")
        ))
    }

    fn emit_construct(&self, ty: &ir::Type, fields: &[ir::NamedOperand]) -> Option<String> {
        let ir::Type::Named { name, .. } = ty else {
            return self.unsupported("non-named construction");
        };
        let operands = fields
            .iter()
            .map(|field| field.value.clone())
            .collect::<Vec<_>>();
        let params = self
            .constructor_param_specs(name, &operands)
            .unwrap_or_default();
        let args = self.emit_operands_for_param_specs(&operands, &params)?;
        Some(format!(
            "new {}({})",
            self.names.named_type(name),
            args.join(", ")
        ))
    }

    fn emit_record_as_named_construct(
        &self,
        ty: &ir::Type,
        fields: &[ir::NamedOperand],
    ) -> Option<String> {
        let ir::Type::Named { name, args } = ty else {
            return self.unsupported(&format!(
                "anonymous shape literal {{{}}}",
                fields
                    .iter()
                    .map(|field| field.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        };
        let type_def = self.type_def(name)?;
        if !matches!(type_def.kind, TypeKind::Class | TypeKind::Record) {
            return None;
        }
        let subst = type_def
            .type_params
            .iter()
            .cloned()
            .zip(args.iter().cloned())
            .collect::<HashMap<_, _>>();
        let mut constructor_args = Vec::new();
        for field in &type_def.fields {
            let field_ty = substitute_java_emit_type(&field.ty, &subst);
            if let Some(value) = fields.iter().find(|value| value.name == field.name) {
                constructor_args.push(self.emit_operand_for_param_spec(
                    &value.value,
                    &JavaParamSpec {
                        ty: field_ty,
                        variadic: false,
                        lazy: false,
                        default: None,
                        coercion: None,
                    },
                )?);
            } else if let Some(initializer) = &field.initializer {
                constructor_args.push(java_constant(initializer));
            } else if field.has_initializer {
                constructor_args.push(java_default_value(&field_ty));
            } else {
                return None;
            }
        }
        Some(format!(
            "new {}({})",
            self.names.named_type(name),
            constructor_args.join(", ")
        ))
    }

    fn emit_variant(
        &self,
        enum_name: &str,
        case_name: &str,
        fields: &[ir::NamedOperand],
    ) -> Option<String> {
        let args = fields
            .iter()
            .map(|field| self.emit_operand(&field.value))
            .collect::<Option<Vec<_>>>()?;
        let owner = if matches!(enum_name, "Option" | "Result" | "Either") {
            format!("lume.core.{}", java_type_name(enum_name))
        } else {
            self.names.named_type(enum_name)
        };
        Some(format!(
            "new {}.{}{}({})",
            owner,
            java_type_name(case_name),
            self.lume_constructor_type_args(enum_name),
            args.join(", ")
        ))
    }

    fn emit_closure(
        &self,
        function_id: ir::FunctionId,
        captures: &[ir::Operand],
        capture_initializers: Option<&[String]>,
    ) -> Option<String> {
        let function = self.bundle.ir.function(function_id)?;
        let local_prefix = format!("lambda{}_", function_id.0);
        let param_locals = function
            .params
            .iter()
            .filter_map(|param| function.locals.get(param.0))
            .collect::<Vec<_>>();
        let target_ty = ir::Type::Function {
            params: param_locals.iter().map(|local| local.ty.clone()).collect(),
            ret: Box::new(function.return_ty.clone()),
        };
        let target_java_ty = self.names.value_type(&target_ty);
        let return_java_ty = self.names.value_type(&function.return_ty);
        if param_locals.len() > MAX_JAVA_FUNCTION_ARITY {
            return self.unsupported("lambda with more than 12 parameters");
        }
        let method_name = if param_locals.is_empty() {
            "get"
        } else {
            "apply"
        };
        let method_params = param_locals
            .iter()
            .map(|param| {
                format!(
                    "{} {local_prefix}{}",
                    self.local_value_type(&param.ty),
                    java_local_name(param)
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        let capture_overrides =
            self.closure_capture_field_overrides(function_id, function, captures)?;
        let body = FunctionEmitter::new(self.bundle, function, self.names)
            .with_capture_overrides(capture_overrides)
            .with_control_var(format!("__block_lambda_{}", function_id.0))
            .with_local_prefix(local_prefix)
            .emit_body()?;

        let mut out = String::new();
        out.push_str("new ");
        out.push_str(&target_java_ty);
        out.push_str("() {\n");
        self.push_closure_capture_fields(
            &mut out,
            function_id,
            function,
            captures,
            capture_initializers,
        )?;
        out.push_str("        @Override\n");
        out.push_str("        public ");
        out.push_str(&return_java_ty);
        out.push(' ');
        out.push_str(method_name);
        out.push('(');
        out.push_str(&method_params);
        out.push(')');
        out.push_str(&body);
        out.push_str("    }");
        Some(out)
    }

    fn closure_capture_field_overrides(
        &self,
        function_id: ir::FunctionId,
        function: &ir::Function,
        captures: &[ir::Operand],
    ) -> Option<HashMap<ir::LocalId, String>> {
        let capture_locals = function
            .locals
            .iter()
            .filter(|local| matches!(local.kind, ir::LocalKind::Capture))
            .collect::<Vec<_>>();
        if capture_locals.len() != captures.len() {
            return None;
        }
        capture_locals
            .iter()
            .enumerate()
            .map(|(index, local)| {
                Some((
                    local.id,
                    format!(
                        "__capture_lambda_{}_{}_{}",
                        function_id.0, local.id.0, index
                    ),
                ))
            })
            .collect()
    }

    fn emit_closure_capture_snapshots(
        &self,
        out: &mut String,
        function_id: ir::FunctionId,
        captures: &[ir::Operand],
    ) -> Option<Vec<String>> {
        let function = self.bundle.ir.function(function_id)?;
        let capture_locals = function
            .locals
            .iter()
            .filter(|local| matches!(local.kind, ir::LocalKind::Capture))
            .collect::<Vec<_>>();
        if capture_locals.len() != captures.len() {
            return None;
        }

        let mut snapshot_names = Vec::new();
        for (index, (local, capture)) in capture_locals.iter().zip(captures).enumerate() {
            let snapshot_name = format!(
                "__capture_value_lambda_{}_{}_{}",
                function_id.0, local.id.0, index
            );
            out.push_str("                    final ");
            out.push_str(&self.local_value_type(&local.ty));
            out.push(' ');
            out.push_str(&snapshot_name);
            out.push_str(" = ");
            out.push_str(&self.emit_capture_initializer(capture)?);
            out.push_str(";\n");
            snapshot_names.push(snapshot_name);
        }
        Some(snapshot_names)
    }

    fn push_closure_capture_fields(
        &self,
        out: &mut String,
        function_id: ir::FunctionId,
        function: &ir::Function,
        captures: &[ir::Operand],
        capture_initializers: Option<&[String]>,
    ) -> Option<()> {
        let capture_locals = function
            .locals
            .iter()
            .filter(|local| matches!(local.kind, ir::LocalKind::Capture))
            .collect::<Vec<_>>();
        if capture_locals.len() != captures.len() {
            return None;
        }

        for (index, (local, capture)) in capture_locals.iter().zip(captures).enumerate() {
            let field_name = format!(
                "__capture_lambda_{}_{}_{}",
                function_id.0, local.id.0, index
            );
            out.push_str("        private final ");
            out.push_str(&self.local_value_type(&local.ty));
            out.push(' ');
            out.push_str(&field_name);
            out.push_str(" = ");
            let initializer = capture_initializers
                .and_then(|values| values.get(index))
                .cloned()
                .unwrap_or(self.emit_capture_initializer(capture)?);
            out.push_str(&initializer);
            out.push_str(";\n");
        }
        if !capture_locals.is_empty() {
            out.push('\n');
        }
        Some(())
    }

    fn emit_anonymous_interface(
        &self,
        interfaces: &[ir::Type],
        methods: &[ir::AnonymousInterfaceMethod],
    ) -> Option<String> {
        let target = interfaces.first()?;
        let ir::Type::Named { name, .. } = target else {
            return None;
        };
        if interfaces.len() != 1 {
            return None;
        }

        let mut out = String::new();
        out.push_str("new ");
        out.push_str(&self.names.named_type(name));
        out.push_str("() {\n");

        for method in methods {
            let function = self.bundle.ir.function(method.function)?;
            let capture_overrides =
                self.push_anonymous_interface_capture_fields(&mut out, method, function, None)?;

            out.push_str("        @Override\n");
            out.push_str("        public ");
            push_function_signature_named(&mut out, function, self.names, &method.name);
            out.push_str(
                &FunctionEmitter::new(self.bundle, function, self.names)
                    .with_capture_overrides(capture_overrides)
                    .emit_body()?,
            );
        }

        out.push_str("    }");
        Some(out)
    }

    fn emit_anonymous_object(
        &self,
        ty: &ir::Type,
        fields: &[ir::NamedOperand],
        methods: &[ir::AnonymousInterfaceMethod],
        field_initializers: Option<&[String]>,
        method_capture_initializers: Option<&[Vec<String>]>,
    ) -> Option<String> {
        let ir::Type::Named { name, .. } = ty else {
            return None;
        };
        let type_def = self.type_def(name)?;
        let mut out = format!("new {}() {{\n", self.names.named_type(name));

        for (field_index, field) in fields.iter().enumerate() {
            let field_def = type_def
                .fields
                .iter()
                .find(|item| item.name == field.name)?;
            let java_field = format!("__field_{}", java_member_name(&field.name));
            let value = field_initializers
                .and_then(|values| values.get(field_index).cloned())
                .unwrap_or(self.coerce_to_target_type(
                    self.emit_operand(&field.value)?,
                    self.operand_type(&field.value),
                    &field_def.ty,
                ));
            out.push_str("        private final ");
            out.push_str(&self.names.value_type(&field_def.ty));
            out.push(' ');
            out.push_str(&java_field);
            out.push_str(" = ");
            out.push_str(&value);
            out.push_str(";\n\n        @Override\n        public ");
            out.push_str(&self.names.return_type(&field_def.ty));
            out.push(' ');
            out.push_str(&java_member_name(&field.name));
            out.push_str("() { return ");
            out.push_str(&java_field);
            out.push_str("; }\n");
        }

        for (method_index, method) in methods.iter().enumerate() {
            let function = self.bundle.ir.function(method.function)?;
            let capture_overrides = self.push_anonymous_interface_capture_fields(
                &mut out,
                method,
                function,
                method_capture_initializers
                    .and_then(|values| values.get(method_index))
                    .map(Vec::as_slice),
            )?;
            out.push_str("        @Override\n        public ");
            push_function_signature_named(&mut out, function, self.names, &method.name);
            out.push_str(
                &FunctionEmitter::new(self.bundle, function, self.names)
                    .with_capture_overrides(capture_overrides)
                    .emit_body()?,
            );
        }

        out.push_str("    }");
        Some(out)
    }

    fn emit_anonymous_object_capture_snapshots(
        &self,
        out: &mut String,
        fields: &[ir::NamedOperand],
        methods: &[ir::AnonymousInterfaceMethod],
    ) -> Option<(Vec<String>, Vec<Vec<String>>)> {
        let mut field_names = Vec::new();
        for (index, field) in fields.iter().enumerate() {
            let name = format!("__object_field_value_{}_{}", self.function.id.0, index);
            let ty = self.operand_type(&field.value).unwrap_or(ir::Type::Unknown);
            out.push_str("                    final ");
            out.push_str(&self.local_value_type(&ty));
            out.push(' ');
            out.push_str(&name);
            out.push_str(" = ");
            out.push_str(&self.emit_operand(&field.value)?);
            out.push_str(";\n");
            field_names.push(name);
        }

        let mut method_names = Vec::new();
        for (method_index, method) in methods.iter().enumerate() {
            let function = self.bundle.ir.function(method.function)?;
            let capture_locals = function
                .locals
                .iter()
                .filter(|local| {
                    matches!(local.kind, ir::LocalKind::Capture)
                        && !(matches!(function.kind, ir::FunctionKind::Method { .. })
                            && local.name == "this")
                })
                .collect::<Vec<_>>();
            if capture_locals.len() != method.captures.len() {
                return None;
            }
            let mut names = Vec::new();
            for (capture_index, (local, capture)) in
                capture_locals.iter().zip(&method.captures).enumerate()
            {
                let name = format!(
                    "__object_method_capture_{}_{}_{}",
                    self.function.id.0, method_index, capture_index
                );
                out.push_str("                    final ");
                out.push_str(&self.local_value_type(&local.ty));
                out.push(' ');
                out.push_str(&name);
                out.push_str(" = ");
                out.push_str(&self.emit_capture_initializer(capture)?);
                out.push_str(";\n");
                names.push(name);
            }
            method_names.push(names);
        }
        Some((field_names, method_names))
    }

    fn push_anonymous_interface_capture_fields(
        &self,
        out: &mut String,
        method: &ir::AnonymousInterfaceMethod,
        function: &ir::Function,
        capture_initializers: Option<&[String]>,
    ) -> Option<HashMap<ir::LocalId, String>> {
        let capture_locals = function
            .locals
            .iter()
            .filter(|local| {
                matches!(local.kind, ir::LocalKind::Capture)
                    && !(matches!(function.kind, ir::FunctionKind::Method { .. })
                        && local.name == "this")
            })
            .collect::<Vec<_>>();
        if capture_locals.len() != method.captures.len() {
            return None;
        }

        let mut overrides = HashMap::new();
        for (index, (local, capture)) in capture_locals.iter().zip(&method.captures).enumerate() {
            let field_name = format!(
                "__capture_{}_{}_{}",
                java_member_name(&method.name),
                local.id.0,
                index
            );
            out.push_str("        private final ");
            out.push_str(&self.local_value_type(&local.ty));
            out.push(' ');
            out.push_str(&field_name);
            out.push_str(" = ");
            if let Some(initializer) = capture_initializers.and_then(|values| values.get(index)) {
                out.push_str(initializer);
            } else {
                out.push_str(&self.emit_capture_initializer(capture)?);
            }
            out.push_str(";\n");
            overrides.insert(local.id, field_name);
        }
        if !capture_locals.is_empty() {
            out.push('\n');
        }
        Some(overrides)
    }

    fn emit_capture_initializer(&self, capture: &ir::Operand) -> Option<String> {
        let local = match capture {
            ir::Operand::Copy(place) | ir::Operand::Move(place) => match place.as_ref() {
                ir::Place::Local(id) => self.function.locals.get(id.0),
                _ => None,
            },
            _ => None,
        };
        if local.is_some_and(|local| local.name == "this") {
            if let ir::FunctionKind::Method { owner } = self.function.kind {
                let owner = self.bundle.ir.types.get(owner.0)?;
                return Some(format!("{}.this", java_type_name(&owner.name)));
            }
        }
        self.emit_operand(capture)
    }

    fn emit_operands(&self, operands: &[ir::Operand]) -> Option<Vec<String>> {
        operands
            .iter()
            .map(|operand| self.emit_operand(operand))
            .collect()
    }

    fn emit_operands_for_params(
        &self,
        operands: &[ir::Operand],
        params: &[ir::Type],
    ) -> Option<Vec<String>> {
        self.emit_operands_for_param_specs(operands, &param_specs_from_types(params.to_vec()))
    }

    fn emit_operands_for_function(
        &self,
        operands: &[ir::Operand],
        function: &ir::Function,
    ) -> Option<Vec<String>> {
        self.emit_operands_for_param_specs(operands, &function_param_specs(function))
    }

    fn emit_operands_for_param_specs(
        &self,
        operands: &[ir::Operand],
        params: &[JavaParamSpec],
    ) -> Option<Vec<String>> {
        let Some(variadic_index) = params.iter().position(|param| param.variadic) else {
            return operands
                .iter()
                .enumerate()
                .map(|(index, operand)| {
                    Some(match params.get(index) {
                        Some(target) => self.emit_operand_for_param_spec(operand, target)?,
                        None => self.emit_operand(operand)?,
                    })
                })
                .collect();
        };

        if operands.len() < variadic_index {
            return None;
        }

        let mut args = Vec::new();
        for (operand, target) in operands.iter().take(variadic_index).zip(params.iter()) {
            args.push(self.emit_operand_for_param_spec(operand, target)?);
        }

        let variadic_target = params.get(variadic_index)?;
        if operands.len() == variadic_index {
            args.push(
                variadic_target
                    .default
                    .as_ref()
                    .map(java_constant)
                    .unwrap_or_else(|| "lume.core.LumeVector.of()".to_string()),
            );
            return Some(args);
        }
        if operands.len() == params.len() {
            let operand = operands.get(variadic_index)?;
            if self.operand_type_is_variadic_list(operand, &variadic_target.ty) {
                args.push(self.emit_operand_for_param_spec(operand, variadic_target)?);
                return Some(args);
            }
        }

        let element_ty = variadic_element_type(&variadic_target.ty);
        let items = operands
            .iter()
            .skip(variadic_index)
            .map(|operand| {
                let expr = self.emit_operand(operand)?;
                Some(match element_ty {
                    Some(target_ty) => {
                        self.coerce_to_target_type(expr, self.operand_type(operand), target_ty)
                    }
                    None => expr,
                })
            })
            .collect::<Option<Vec<_>>>()?;
        args.push(format!("lume.core.LumeVector.of({})", items.join(", ")));
        Some(args)
    }

    fn emit_operand_for_param_spec(
        &self,
        operand: &ir::Operand,
        target: &JavaParamSpec,
    ) -> Option<String> {
        if target.lazy && self.operand_is_zero_arg_function(operand) {
            return self.emit_operand(operand);
        }

        let expr = self.emit_operand(operand)?;
        let target_ty = if target.lazy {
            lazy_param_value_type(&target.ty)
        } else {
            &target.ty
        };
        let expr = self.coerce_to_target_type(expr, self.operand_type(operand), target_ty);
        let expr = coerce_to_java_primitive(expr, target.coercion);
        if target.lazy {
            Some(format!("() -> {expr}"))
        } else {
            Some(expr)
        }
    }

    fn emit_operand(&self, operand: &ir::Operand) -> Option<String> {
        match operand {
            ir::Operand::Copy(place) | ir::Operand::Move(place) => self.emit_place(place),
            ir::Operand::Const(constant) => Some(java_constant(constant)),
        }
    }

    fn operand_is_zero_arg_function(&self, operand: &ir::Operand) -> bool {
        matches!(
            self.operand_type(operand),
            Some(ir::Type::Function { params, .. }) if params.is_empty()
        )
    }

    fn emit_place(&self, place: &ir::Place) -> Option<String> {
        match place {
            ir::Place::Local(id) => self
                .function
                .locals
                .get(id.0)
                .map(|local| self.local_reference(local)),
            ir::Place::Global(id) => self
                .bundle
                .ir
                .globals
                .get(id.0)
                .map(|global| java_member_name(&global.name)),
            ir::Place::Field { base, name } => {
                let base_expr = self.emit_operand(base)?;
                match self.operand_type(base) {
                    Some(ir::Type::Tuple(_)) => {
                        let accessor = tuple_accessor_name(name)?;
                        Some(format!("{base_expr}.{accessor}()"))
                    }
                    Some(ir::Type::Named {
                        name: ref type_name,
                        ..
                    }) if is_core_accessor_backed_type(type_name)
                        || self.bundle.ir.types.iter().any(|ty| {
                            ty.name == *type_name
                                && (ty.kind == TypeKind::Interface || is_anonymous_object_type(ty))
                                && ty.fields.iter().any(|field| field.name == *name)
                        }) =>
                    {
                        Some(format!("{base_expr}.{}()", java_member_name(name)))
                    }
                    _ => Some(format!("{base_expr}.{}", java_member_name(name))),
                }
            }
            ir::Place::Index { .. } => self.unsupported("indexed assignment target"),
        }
    }

    fn unsupported<T>(&self, reason: &str) -> Option<T> {
        if std::env::var_os("LUME_JAVA_DEBUG_STUBS").is_some() {
            eprintln!(
                "java backend cannot emit '{}': {reason}",
                self.function.name
            );
        }
        None
    }

    fn place_type(&self, place: &ir::Place) -> Option<ir::Type> {
        match place {
            ir::Place::Local(id) => self
                .inferred_local_types
                .get(id)
                .cloned()
                .or_else(|| self.function.locals.get(id.0).map(|local| local.ty.clone())),
            ir::Place::Global(id) => self
                .bundle
                .ir
                .globals
                .get(id.0)
                .map(|global| global.ty.clone()),
            ir::Place::Field { base, name } => {
                let base_ty = self.operand_type(base)?;
                self.field_type(&base_ty, name)
            }
            ir::Place::Index { base, .. } => {
                let base_ty = self.operand_type(base)?;
                self.index_assignment_type(&base_ty)
            }
        }
    }

    fn operand_type(&self, operand: &ir::Operand) -> Option<ir::Type> {
        match operand {
            ir::Operand::Copy(place) | ir::Operand::Move(place) => self.place_type(place),
            ir::Operand::Const(value) => Some(constant_type(value)),
        }
    }

    fn operand_type_is_variadic_list(&self, operand: &ir::Operand, target_ty: &ir::Type) -> bool {
        let Some(source_ty) = self.operand_type(operand) else {
            return false;
        };
        matches!(
            (&source_ty, target_ty),
            (
                ir::Type::Named {
                    name: source_name,
                    args: source_args,
                },
                ir::Type::Named {
                    name: target_name,
                    args: target_args,
                },
            ) if source_name == "Vector"
                && target_name == "Vector"
                && source_args.len() == target_args.len()
        )
    }

    fn field_type(&self, ty: &ir::Type, field_name: &str) -> Option<ir::Type> {
        match ty {
            ir::Type::Named { name, .. } => self
                .type_def(name)?
                .fields
                .iter()
                .find(|field| field.name == field_name)
                .map(|field| field.ty.clone()),
            ir::Type::Record(fields) => fields
                .iter()
                .find(|field| field.name == field_name)
                .map(|field| field.ty.clone()),
            ir::Type::Tuple(items) => tuple_field_index(field_name)
                .and_then(|index| items.get(index))
                .cloned(),
            _ => None,
        }
    }

    fn index_assignment_type(&self, ty: &ir::Type) -> Option<ir::Type> {
        match ty {
            ir::Type::Named { name, args }
                if (name == "Array" || name == "Vector") && args.len() == 1 =>
            {
                args.first().cloned()
            }
            ir::Type::Named { name, args } if name == "Map" && args.len() == 2 => {
                args.get(1).cloned()
            }
            ir::Type::Unknown => Some(ir::Type::Unknown),
            _ => None,
        }
    }

    fn constructor_param_specs(
        &self,
        type_name: &str,
        operands: &[ir::Operand],
    ) -> Option<Vec<JavaParamSpec>> {
        let ty = self.type_def(type_name)?;
        if let Some(function) = self.function_named_for_operands(ty, "new", operands) {
            return Some(function_param_specs(function));
        }
        if ty.methods.iter().any(|function_id| {
            self.bundle
                .ir
                .function(*function_id)
                .is_some_and(|f| f.name == "new")
        }) {
            return None;
        }
        let params = ty
            .fields
            .iter()
            .filter(|field| field.visibility != Visibility::Hidden)
            .map(|field| JavaParamSpec {
                ty: field.ty.clone(),
                variadic: false,
                lazy: false,
                default: None,
                coercion: None,
            })
            .collect::<Vec<_>>();
        if param_specs_accept_arg_len(&params, operands.len()) {
            Some(params)
        } else {
            None
        }
    }

    fn method_param_specs_for_receiver(
        &self,
        receiver: &ir::Operand,
        method: &str,
        operands: &[ir::Operand],
    ) -> Option<Vec<JavaParamSpec>> {
        let receiver_ty = self.operand_type(receiver)?;
        if method == "transactionally" && operands.len() == 1 {
            if let ir::Type::Named { name, .. } = &receiver_ty {
                if name == "Database" {
                    return self.operand_type(&operands[0]).map(|ty| {
                        vec![JavaParamSpec {
                            ty,
                            variadic: false,
                            lazy: false,
                            default: None,
                            coercion: None,
                        }]
                    });
                }
            }
        }
        if let Some(params) = builtin_method_param_specs(&receiver_ty, method, operands.len()) {
            return Some(params);
        }
        let ir::Type::Named { name, args } = receiver_ty else {
            return None;
        };
        let params = if self.names.is_java_type(&name) {
            self.external_method_param_specs(&name, method, operands)
                .or_else(|| self.type_method_param_specs(&name, method, operands))?
        } else {
            self.type_method_param_specs(&name, method, operands)?
        };
        Some(
            params
                .into_iter()
                .map(|param| JavaParamSpec {
                    ty: self.substitute_receiver_type_args(&name, &args, &param.ty),
                    variadic: param.variadic,
                    lazy: param.lazy,
                    default: param.default.clone(),
                    coercion: param.coercion,
                })
                .collect(),
        )
    }

    fn type_method_param_specs(
        &self,
        type_name: &str,
        method: &str,
        operands: &[ir::Operand],
    ) -> Option<Vec<JavaParamSpec>> {
        let ty = self.type_def(type_name)?;
        self.function_named_for_operands(ty, method, operands)
            .map(function_param_specs)
    }

    fn method_return_type_for_receiver(
        &self,
        receiver: &ir::Operand,
        method: &str,
        arg_len: usize,
    ) -> Option<ir::Type> {
        let receiver_ty = self.operand_type(receiver)?;
        if let Some(ret) = builtin_method_return_type(&receiver_ty, method, arg_len) {
            return Some(ret);
        }
        let ir::Type::Named { name, args } = receiver_ty else {
            return None;
        };
        let ret = self.type_method_return_type(&name, method, arg_len)?;
        Some(self.substitute_receiver_type_args(&name, &args, &ret))
    }

    fn type_method_return_type(
        &self,
        type_name: &str,
        method: &str,
        arg_len: usize,
    ) -> Option<ir::Type> {
        let ty = self.type_def(type_name)?;
        self.function_named_for_arg_len(ty, method, arg_len)
            .map(|function| function.return_ty.clone())
    }

    fn named_runtime_call_return_type(&self, path: &[String], arg_len: usize) -> Option<ir::Type> {
        match path {
            [case] if core_enum_case_owner(case).is_some() => self.core_enum_case_type(case),
            [owner, case]
                if core_enum_case_owner(case).is_some_and(|expected| expected == owner) =>
            {
                self.core_enum_case_type(case)
            }
            [owner, case] if self.enum_case(owner, case).is_some() => Some(ir::Type::Named {
                name: owner.clone(),
                args: Vec::new(),
            }),
            [owner, case, method] if self.enum_case(owner, case).is_some() => {
                self.type_method_return_type(owner, method, arg_len)
            }
            [owner] if owner == "Vector" => Some(ir::Type::Named {
                name: "Vector".to_string(),
                args: vec![ir::Type::Unknown],
            }),
            [owner] if owner == "Map" => Some(ir::Type::Named {
                name: "Map".to_string(),
                args: vec![ir::Type::Unknown, ir::Type::Unknown],
            }),
            [owner] if owner == "Set" => Some(ir::Type::Named {
                name: "Set".to_string(),
                args: vec![ir::Type::Unknown],
            }),
            [owner] if owner == "Range" && arg_len == 2 => Some(ir::Type::Named {
                name: "Range".to_string(),
                args: Vec::new(),
            }),
            [owner, method] if owner == "Int" && method == "parse" && arg_len == 1 => {
                Some(ir::Type::Named {
                    name: "Option".to_string(),
                    args: vec![ir::Type::Int],
                })
            }
            [owner, method] if owner == "Float" && method == "parse" && arg_len == 1 => {
                Some(ir::Type::Named {
                    name: "Option".to_string(),
                    args: vec![ir::Type::Float],
                })
            }
            [owner] if self.names.is_java_type(owner) || self.is_lume_constructible_type(owner) => {
                Some(ir::Type::Named {
                    name: owner.clone(),
                    args: Vec::new(),
                })
            }
            [owner, method]
                if self.names.is_java_single_type(owner) || self.is_lume_single_type(owner) =>
            {
                self.type_method_return_type(owner, method, arg_len)
                    .or_else(|| self.names.java_method_return_type(owner, method, arg_len))
            }
            [owner, method] if self.names.is_java_type(owner) => self
                .type_method_return_type(owner, method, arg_len)
                .or_else(|| self.names.java_method_return_type(owner, method, arg_len)),
            _ => None,
        }
    }

    fn core_enum_case_type(&self, case: &str) -> Option<ir::Type> {
        core_enum_case_owner(case).map(|owner| ir::Type::Named {
            name: owner.to_string(),
            args: vec![ir::Type::Unknown],
        })
    }

    fn substitute_receiver_type_args(
        &self,
        type_name: &str,
        args: &[ir::Type],
        ty: &ir::Type,
    ) -> ir::Type {
        let Some(type_def) = self.type_def(type_name) else {
            return ty.clone();
        };
        if args.is_empty() {
            return ty.clone();
        }
        if type_def.type_params.is_empty() {
            return match ty {
                ir::Type::TypeParam(_) if args.len() == 1 => args[0].clone(),
                _ => ty.clone(),
            };
        }
        let subst = type_def
            .type_params
            .iter()
            .cloned()
            .zip(args.iter().cloned())
            .collect::<HashMap<_, _>>();
        substitute_java_emit_type(ty, &subst)
    }

    fn type_def(&self, name: &str) -> Option<&ir::TypeDef> {
        self.bundle.ir.types.iter().find(|ty| ty.name == name)
    }

    fn functions_named<'b>(
        &'b self,
        ty: &'b ir::TypeDef,
        name: &'b str,
    ) -> impl Iterator<Item = &'b ir::Function> + 'b {
        ty.methods.iter().filter_map(move |id| {
            let function = self.bundle.ir.function(*id)?;
            (function.name == name).then_some(function)
        })
    }

    fn function_named_for_arg_len<'b>(
        &'b self,
        ty: &'b ir::TypeDef,
        name: &'b str,
        arg_len: usize,
    ) -> Option<&'b ir::Function> {
        let mut variadic_candidate = None;
        for function in self.functions_named(ty, name) {
            if function.params.len() == arg_len
                && !function.param_variadic.iter().any(|variadic| *variadic)
            {
                return Some(function);
            }
            if function_accepts_arg_len(function, arg_len) && variadic_candidate.is_none() {
                variadic_candidate = Some(function);
            }
        }
        variadic_candidate
    }

    fn function_named_for_operands<'b>(
        &'b self,
        ty: &'b ir::TypeDef,
        name: &'b str,
        operands: &[ir::Operand],
    ) -> Option<&'b ir::Function> {
        let mut compatible_variadic = None;
        let mut fallback_exact = None;
        let mut fallback_variadic = None;

        for function in self.functions_named(ty, name) {
            if !function_accepts_arg_len(function, operands.len()) {
                continue;
            }

            let is_variadic = function.param_variadic.iter().any(|variadic| *variadic);
            let compatible = self.operands_match_function(function, operands);

            if !is_variadic && function.params.len() == operands.len() {
                if compatible {
                    return Some(function);
                }
                fallback_exact.get_or_insert(function);
            } else if is_variadic {
                if compatible {
                    compatible_variadic.get_or_insert(function);
                }
                fallback_variadic.get_or_insert(function);
            }
        }

        compatible_variadic.or(fallback_exact).or(fallback_variadic)
    }

    fn operands_match_function(&self, function: &ir::Function, operands: &[ir::Operand]) -> bool {
        self.param_specs_match_operands(&function_param_specs(function), operands)
    }

    fn external_method_param_specs(
        &self,
        owner: &str,
        method: &str,
        operands: &[ir::Operand],
    ) -> Option<Vec<JavaParamSpec>> {
        let candidates = self.names.java_method_param_candidates(owner, method)?;
        self.param_specs_for_operands(candidates, operands)
    }

    fn param_specs_for_operands(
        &self,
        candidates: &[Vec<JavaParamSpec>],
        operands: &[ir::Operand],
    ) -> Option<Vec<JavaParamSpec>> {
        let mut compatible_variadic = None;
        let mut fallback_exact = None;
        let mut fallback_variadic = None;

        for params in candidates {
            if !param_specs_accept_arg_len(params, operands.len()) {
                continue;
            }
            let is_variadic = params.iter().any(|param| param.variadic);
            let compatible = self.param_specs_match_operands(params, operands);

            if !is_variadic && params.len() == operands.len() {
                if compatible {
                    return Some(params.clone());
                }
                fallback_exact.get_or_insert_with(|| params.clone());
            } else if is_variadic {
                if compatible {
                    compatible_variadic.get_or_insert_with(|| params.clone());
                }
                fallback_variadic.get_or_insert_with(|| params.clone());
            }
        }

        compatible_variadic.or(fallback_exact).or(fallback_variadic)
    }

    fn param_specs_match_operands(
        &self,
        params: &[JavaParamSpec],
        operands: &[ir::Operand],
    ) -> bool {
        let Some(variadic_index) = params.iter().position(|param| param.variadic) else {
            return params.len() == operands.len()
                && operands.iter().zip(params.iter()).all(|(operand, param)| {
                    let target_ty = if param.lazy {
                        lazy_param_value_type(&param.ty)
                    } else {
                        &param.ty
                    };
                    self.operand_matches_target(operand, target_ty)
                });
        };

        if operands.len() < variadic_index {
            return false;
        }
        if !operands
            .iter()
            .take(variadic_index)
            .zip(params.iter())
            .all(|(operand, param)| {
                let target_ty = if param.lazy {
                    lazy_param_value_type(&param.ty)
                } else {
                    &param.ty
                };
                self.operand_matches_target(operand, target_ty)
            })
        {
            return false;
        }

        let Some(variadic_target) = params.get(variadic_index) else {
            return false;
        };
        if operands.len() == params.len()
            && operands.get(variadic_index).is_some_and(|operand| {
                self.operand_type_is_variadic_list(operand, &variadic_target.ty)
            })
        {
            return true;
        }

        let Some(element_ty) = variadic_element_type(&variadic_target.ty) else {
            return true;
        };
        operands
            .iter()
            .skip(variadic_index)
            .all(|operand| self.operand_matches_target(operand, element_ty))
    }

    fn operand_matches_target(&self, operand: &ir::Operand, target_ty: &ir::Type) -> bool {
        if matches!(target_ty, ir::Type::Unknown | ir::Type::Never)
            || is_named_builtin(target_ty, "Any")
        {
            return true;
        }
        let Some(source_ty) = self.operand_type(operand) else {
            return !matches!(target_ty, ir::Type::Named { name, .. } if name == "Vector");
        };
        match target_ty {
            ir::Type::Named { name, .. } if name == "Vector" => {
                matches!(source_ty, ir::Type::Named { name, .. } if name == "Vector")
            }
            ir::Type::Function { .. } => {
                matches!(source_ty, ir::Type::Function { .. } | ir::Type::Unknown)
            }
            _ => true,
        }
    }

    fn rvalue_type(&self, value: &ir::RValue) -> Option<ir::Type> {
        match value {
            ir::RValue::Use(operand) => self.operand_type(operand),
            ir::RValue::Unary { op, operand } => match op {
                ir::UnaryOp::Neg => self.operand_type(operand),
                ir::UnaryOp::Not => Some(ir::Type::Bool),
            },
            ir::RValue::Call { callee, args, .. } => match callee {
                ir::Callee::Direct(id) => self
                    .bundle
                    .ir
                    .function(*id)
                    .map(|function| function.return_ty.clone()),
                ir::Callee::Intrinsic(ir::Intrinsic::Ensure) => Some(ir::Type::Named {
                    name: "Result".to_string(),
                    args: vec![ir::Type::Unit, ir::Type::Unknown],
                }),
                ir::Callee::Intrinsic(
                    ir::Intrinsic::Print
                    | ir::Intrinsic::Println
                    | ir::Intrinsic::Printf
                    | ir::Intrinsic::Assert,
                ) => Some(ir::Type::Unit),
                ir::Callee::Intrinsic(ir::Intrinsic::Identity) => {
                    args.first().and_then(|arg| self.operand_type(arg))
                }
                ir::Callee::Intrinsic(ir::Intrinsic::ListAppend)
                | ir::Callee::Intrinsic(ir::Intrinsic::ListExtend) => {
                    args.first().and_then(|arg| self.operand_type(arg))
                }
                ir::Callee::Intrinsic(ir::Intrinsic::ListLen) => Some(ir::Type::Int),
                ir::Callee::Intrinsic(ir::Intrinsic::ListGet) => {
                    args.first().and_then(|arg| match self.operand_type(arg)? {
                        ir::Type::Named { name, args } if name == "Vector" && args.len() == 1 => {
                            args.into_iter().next()
                        }
                        _ => Some(ir::Type::Unknown),
                    })
                }
                ir::Callee::Intrinsic(ir::Intrinsic::ListSlice) => {
                    args.first().and_then(|arg| self.operand_type(arg))
                }
                ir::Callee::Intrinsic(ir::Intrinsic::IterInit) => args
                    .first()
                    .and_then(|arg| self.operand_type(arg))
                    .and_then(|ty| iterable_item_type(&ty))
                    .map(|item| ir::Type::Named {
                        name: "Iterator".to_string(),
                        args: vec![item],
                    }),
                ir::Callee::Intrinsic(ir::Intrinsic::IterNext) => args
                    .first()
                    .and_then(|arg| self.operand_type(arg))
                    .and_then(|ty| iterable_item_type(&ty)),
                ir::Callee::Intrinsic(ir::Intrinsic::ExtractSuccessValue)
                | ir::Callee::Intrinsic(ir::Intrinsic::UnsafeExtractSuccessValue)
                | ir::Callee::Intrinsic(ir::Intrinsic::VariantField(_)) => Some(ir::Type::Unknown),
                ir::Callee::Intrinsic(ir::Intrinsic::ExtractSuccessIsSet)
                | ir::Callee::Intrinsic(ir::Intrinsic::VariantIs(_))
                | ir::Callee::Intrinsic(ir::Intrinsic::IterHasNext) => Some(ir::Type::Bool),
                ir::Callee::Method { receiver, method } => {
                    self.method_return_type_for_receiver(receiver, method, args.len())
                }
                ir::Callee::Named { path } => self.named_runtime_call_return_type(path, args.len()),
                _ => None,
            },
            ir::RValue::Closure { function, .. } => {
                let function = self.bundle.ir.function(*function)?;
                Some(ir::Type::Function {
                    params: function
                        .params
                        .iter()
                        .filter_map(|param| function.locals.get(param.0))
                        .map(|local| local.ty.clone())
                        .collect(),
                    ret: Box::new(function.return_ty.clone()),
                })
            }
            ir::RValue::Construct { ty, .. } => Some(ty.clone()),
            ir::RValue::Binary { op, left, right } => self.binary_value_type(*op, left, right),
            ir::RValue::List(items) => {
                let element_ty = items
                    .iter()
                    .filter_map(|item| self.operand_type(item))
                    .find(|ty| !matches!(ty, ir::Type::Unknown))
                    .unwrap_or(ir::Type::Unknown);
                Some(ir::Type::Named {
                    name: "Vector".to_string(),
                    args: vec![element_ty],
                })
            }
            ir::RValue::AnonymousInterface { interfaces, .. } if interfaces.len() == 1 => {
                interfaces.first().cloned()
            }
            ir::RValue::AnonymousObject { ty, .. } => Some(ty.clone()),
            ir::RValue::Cast { ty, .. } => Some(ty.clone()),
            ir::RValue::Field { base, name } => {
                let base_ty = self.operand_type(base)?;
                self.field_type(&base_ty, name)
            }
            ir::RValue::Index { base, .. } => {
                let base_ty = self.operand_type(base)?;
                self.index_result_type(&base_ty)
            }
            ir::RValue::TypeOf { ty } => Some(runtime_ir_type(ty.clone())),
            ir::RValue::TypeTest { .. } => Some(ir::Type::Bool),
            _ => None,
        }
    }

    fn index_result_type(&self, ty: &ir::Type) -> Option<ir::Type> {
        match ty {
            ir::Type::Named { name, args }
                if (name == "Array" || name == "Vector") && args.len() == 1 =>
            {
                args.first().cloned()
            }
            ir::Type::Named { name, args } if name == "Map" && args.len() == 2 => {
                Some(ir::Type::Named {
                    name: "Option".to_string(),
                    args: vec![args[1].clone()],
                })
            }
            ir::Type::Unknown => Some(ir::Type::Unknown),
            _ => None,
        }
    }

    fn binary_value_type(
        &self,
        op: ir::BinaryOp,
        left: &ir::Operand,
        right: &ir::Operand,
    ) -> Option<ir::Type> {
        match op {
            ir::BinaryOp::Eq
            | ir::BinaryOp::NotEq
            | ir::BinaryOp::Less
            | ir::BinaryOp::LessEq
            | ir::BinaryOp::Greater
            | ir::BinaryOp::GreaterEq
            | ir::BinaryOp::And
            | ir::BinaryOp::Or => Some(ir::Type::Bool),
            ir::BinaryOp::Add => {
                let left_ty = self.operand_type(left);
                let right_ty = self.operand_type(right);
                if left_ty.as_ref().is_some_and(type_is_str)
                    || right_ty.as_ref().is_some_and(type_is_str)
                {
                    return Some(ir::Type::Str);
                }
                if left_ty.as_ref().is_some_and(type_is_float_like)
                    || right_ty.as_ref().is_some_and(type_is_float_like)
                {
                    return Some(ir::Type::Float);
                }
                left_ty.or(right_ty)
            }
            ir::BinaryOp::Sub | ir::BinaryOp::Mul | ir::BinaryOp::Div | ir::BinaryOp::Mod => {
                let left_ty = self.operand_type(left);
                let right_ty = self.operand_type(right);
                if left_ty.as_ref().is_some_and(type_is_float_like)
                    || right_ty.as_ref().is_some_and(type_is_float_like)
                {
                    return Some(ir::Type::Float);
                }
                left_ty.or(right_ty)
            }
        }
    }

    fn coerce_to_target_type(
        &self,
        expr: String,
        source_ty: Option<ir::Type>,
        target_ty: &ir::Type,
    ) -> String {
        if is_java_void_type(target_ty) {
            return "lume.core.LumeUnit.INSTANCE".to_string();
        }
        if matches!(target_ty, ir::Type::Function { .. })
            && source_ty.as_ref().is_some_and(|source| {
                matches!(source, ir::Type::Function { .. } | ir::Type::Unknown)
            })
        {
            return format!("(({}) ({expr}))", self.names.value_type(target_ty));
        }
        if source_ty.as_ref().is_some_and(|source| source != target_ty)
            && self.target_type_needs_reference_cast(target_ty)
        {
            return self.unchecked_reference_cast(expr, target_ty);
        }
        if source_ty.as_ref().is_some_and(|source| {
            java_type_contains_unknown(source) || self.is_unbound_named_type(source)
        }) && self.target_type_needs_reference_cast(target_ty)
        {
            return self.unchecked_reference_cast(expr, target_ty);
        }
        if source_ty.as_ref().is_some_and(|source| {
            java_type_contains_type_param(source) || self.is_unbound_named_type(source)
        }) && !java_type_contains_type_param(target_ty)
        {
            return self.unchecked_reference_cast(expr, target_ty);
        }
        if source_ty.is_none() && self.target_type_needs_reference_cast(target_ty) {
            return self.unchecked_reference_cast(expr, target_ty);
        }
        if !matches!(source_ty, Some(ir::Type::Unknown)) || matches!(target_ty, ir::Type::Unknown) {
            return expr;
        }
        if !self.target_type_can_be_unchecked_cast(target_ty) {
            return expr;
        }
        self.unchecked_reference_cast(expr, target_ty)
    }

    fn target_type_needs_reference_cast(&self, ty: &ir::Type) -> bool {
        if is_java_void_type(ty) {
            return false;
        }
        if self.type_param_is_unbound(ty) {
            return false;
        }
        if self.is_unbound_named_type(ty) {
            return false;
        }
        if java_type_contains_type_param(ty) && !self.type_params_are_bound(ty) {
            return false;
        }
        java_type_needs_reference_cast(ty)
    }

    fn target_type_can_be_unchecked_cast(&self, ty: &ir::Type) -> bool {
        if self.type_param_is_unbound(ty) {
            return false;
        }
        if self.is_unbound_named_type(ty) {
            return false;
        }
        if java_type_contains_type_param(ty) && !self.type_params_are_bound(ty) {
            return false;
        }
        !matches!(ty, ir::Type::Unknown | ir::Type::Never | ir::Type::Unit)
    }

    fn unchecked_reference_cast(&self, expr: String, target_ty: &ir::Type) -> String {
        if self.type_param_is_unbound(target_ty) || self.is_unbound_named_type(target_ty) {
            return expr;
        }
        format!("(({}) ((Object) {expr}))", self.names.value_type(target_ty))
    }

    fn local_value_type(&self, ty: &ir::Type) -> String {
        if java_type_contains_type_param(ty) && !self.type_params_are_bound(ty) {
            return "Object".to_string();
        }
        if self.is_unbound_named_type(ty) {
            return "Object".to_string();
        }
        self.names.value_type(ty)
    }

    fn type_param_is_unbound(&self, ty: &ir::Type) -> bool {
        matches!(ty, ir::Type::TypeParam(name) if !self.type_param_is_bound(name))
    }

    fn type_has_unbound_type_params(&self, ty: &ir::Type) -> bool {
        java_type_contains_type_param(ty) && !self.type_params_are_bound(ty)
    }

    fn is_unbound_named_type(&self, ty: &ir::Type) -> bool {
        let ir::Type::Named { name, args } = ty else {
            return false;
        };
        args.is_empty()
            && !self.type_param_is_bound(name)
            && java_named_builtin_value(name).is_none()
            && !is_builtin_container(name)
            && !self.names.is_java_type(name)
            && !self.bundle.ir.types.iter().any(|ty| ty.name == *name)
    }

    fn type_param_is_bound(&self, name: &str) -> bool {
        self.function.type_params.iter().any(|param| param == name)
            || match self.function.kind {
                ir::FunctionKind::Method { owner } => self
                    .bundle
                    .ir
                    .types
                    .get(owner.0)
                    .is_some_and(|ty| ty.type_params.iter().any(|param| param == name)),
                _ => false,
            }
    }

    fn generic_bound_for_type_param(&self, name: &str) -> Option<ir::Type> {
        let owner_conditions = match self.function.kind {
            ir::FunctionKind::Method { owner } => self
                .bundle
                .ir
                .types
                .get(owner.0)
                .map(|ty| ty.generic_conditions.as_slice())
                .unwrap_or(&[]),
            _ => &[],
        };
        self.function
            .generic_conditions
            .iter()
            .chain(owner_conditions.iter())
            .find_map(|condition| match condition {
                ir::GenericCondition::Bound {
                    subject: ir::Type::TypeParam(subject),
                    bound,
                } if subject == name => Some(bound.clone()),
                _ => None,
            })
    }

    fn type_params_are_bound(&self, ty: &ir::Type) -> bool {
        let mut bound = self.function.type_params.clone();
        if let ir::FunctionKind::Method { owner } = self.function.kind
            && let Some(owner) = self.bundle.ir.types.get(owner.0)
        {
            bound.extend(owner.type_params.iter().cloned());
        }
        java_type_params_are_bound(ty, &bound)
    }

    fn local_reference(&self, local: &ir::Local) -> String {
        if let Some(name) = self.capture_overrides.get(&local.id) {
            return name.clone();
        }
        if matches!(local.kind, ir::LocalKind::Capture) && local.name == "this" {
            "this".to_string()
        } else {
            self.local_name(local)
        }
    }

    fn local_name(&self, local: &ir::Local) -> String {
        format!("{}{}", self.local_prefix, java_local_name(local))
    }

    fn local_is_declared_elsewhere(&self, local: &ir::Local) -> bool {
        matches!(local.kind, ir::LocalKind::Param)
            || self.capture_overrides.contains_key(&local.id)
            || (matches!(local.kind, ir::LocalKind::Capture) && local.name == "this")
    }
}

fn rvalue_can_be_java_statement(value: &ir::RValue) -> bool {
    matches!(
        value,
        ir::RValue::Call { .. } | ir::RValue::Construct { .. } | ir::RValue::Variant { .. }
    )
}

fn operand_local_id(operand: &ir::Operand) -> Option<ir::LocalId> {
    match operand {
        ir::Operand::Copy(place) | ir::Operand::Move(place) => match place.as_ref() {
            ir::Place::Local(id) => Some(*id),
            _ => None,
        },
        ir::Operand::Const(_) => None,
    }
}

fn terminator_exits_case(kind: &ir::TerminatorKind) -> bool {
    matches!(
        kind,
        ir::TerminatorKind::Return(_) | ir::TerminatorKind::Unreachable
    )
}

fn module_class_name(bundle: &BackendBundle) -> String {
    format!("{}Module", module_base_name(bundle))
}

fn runner_class_name(bundle: &BackendBundle) -> String {
    format!("{}Main", module_base_name(bundle))
}

fn module_base_name(bundle: &BackendBundle) -> String {
    let raw = bundle
        .ir
        .module
        .as_deref()
        .and_then(|module| module.split('/').next_back())
        .filter(|segment| !segment.is_empty())
        .map(str::to_string)
        .or_else(|| {
            bundle
                .root_path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| "Main".to_string());
    java_type_name(&raw)
}

struct JavaNames {
    java_types: HashMap<String, String>,
    java_type_kinds: HashMap<String, TypeKind>,
    java_type_param_counts: HashMap<String, usize>,
    java_method_params: HashMap<(String, String), Vec<Vec<JavaParamSpec>>>,
    java_method_returns: HashMap<(String, String, usize), ir::Type>,
}

impl JavaNames {
    fn from_external_classes(external_classes: &HashMap<String, JavaExternalClass>) -> Self {
        let java_types = external_classes
            .iter()
            .map(|(name, class)| (name.clone(), class.qualified_name.clone()))
            .collect();
        let java_type_kinds = external_classes
            .iter()
            .map(|(name, class)| (name.clone(), class.kind))
            .collect();
        let java_type_param_counts = external_classes
            .iter()
            .map(|(name, class)| (name.clone(), class.type_params.len()))
            .collect();
        let mut java_method_params: HashMap<(String, String), Vec<Vec<JavaParamSpec>>> =
            HashMap::new();
        for (owner, class) in external_classes {
            for method in &class.methods {
                let params = method
                    .params
                    .iter()
                    .map(|param| JavaParamSpec {
                        ty: param
                            .ty
                            .as_ref()
                            .map(type_ref_to_ir)
                            .unwrap_or(ir::Type::Unknown),
                        variadic: param.variadic,
                        lazy: false,
                        default: None,
                        coercion: param.coercion,
                    })
                    .collect::<Vec<_>>();
                java_method_params
                    .entry((owner.clone(), method.name.clone()))
                    .or_default()
                    .push(params);
            }
        }
        let java_method_returns = external_classes
            .iter()
            .flat_map(|(owner, class)| {
                class.methods.iter().filter_map(move |method| {
                    let ret = method.return_type.as_ref().map(type_ref_to_ir)?;
                    Some((
                        (owner.clone(), method.name.clone(), method.params.len()),
                        ret,
                    ))
                })
            })
            .collect();
        Self {
            java_types,
            java_type_kinds,
            java_type_param_counts,
            java_method_params,
            java_method_returns,
        }
    }

    fn return_type(&self, ty: &ir::Type) -> String {
        match ty {
            ir::Type::Unit => "void".to_string(),
            ir::Type::Named { name, args } if name == "Unit" && args.is_empty() => {
                "void".to_string()
            }
            _ => self.value_type(ty),
        }
    }

    fn value_type(&self, ty: &ir::Type) -> String {
        match ty {
            ir::Type::Unknown => "Object".to_string(),
            ir::Type::Never => "lume.core.LumePanic".to_string(),
            ir::Type::Unit => "lume.core.LumeUnit".to_string(),
            ir::Type::Bool => "Boolean".to_string(),
            ir::Type::Int => "Long".to_string(),
            ir::Type::Float => "Double".to_string(),
            ir::Type::Str => "String".to_string(),
            ir::Type::Function { params, ret } => self.function_type(params, ret),
            ir::Type::Named { name, .. } if is_reflection_type(name) => {
                "lume.core.LumeType".to_string()
            }
            ir::Type::Named { name, args } if args.is_empty() => java_named_builtin_value(name)
                .or_else(|| self.java_types.get(name).cloned())
                .unwrap_or_else(|| java_type_name(name)),
            ir::Type::Named { name, args } if is_builtin_container(name) => {
                self.builtin_container(name, args)
            }
            ir::Type::Named { name, args } => {
                let args = args
                    .iter()
                    .map(|arg| self.value_type(arg))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{}<{args}>", self.named_type(name))
            }
            ir::Type::Tuple(items) => self.tuple_type(items),
            ir::Type::Record(_) => "Object".to_string(),
            ir::Type::TypeParam(name) => java_type_name(name),
        }
    }

    fn annotation_type(&self, ty: &ir::Type) -> String {
        match ty {
            ir::Type::Bool => "boolean".to_string(),
            ir::Type::Int => "long".to_string(),
            ir::Type::Float => "double".to_string(),
            ir::Type::Str => "String".to_string(),
            ir::Type::Named { name, args } if args.is_empty() => {
                java_named_builtin_annotation(name)
                    .or_else(|| self.java_types.get(name).cloned())
                    .unwrap_or_else(|| java_type_name(name))
            }
            ir::Type::Named { name, args } if name == "Vector" && args.len() == 1 => {
                format!("{}[]", self.annotation_type(&args[0]))
            }
            _ => "String".to_string(),
        }
    }

    fn named_type(&self, name: &str) -> String {
        self.java_types
            .get(name)
            .cloned()
            .unwrap_or_else(|| java_type_name(name))
    }

    fn is_java_type(&self, name: &str) -> bool {
        self.java_types.contains_key(name)
    }

    fn is_java_single_type(&self, name: &str) -> bool {
        self.java_type_kinds
            .get(name)
            .is_some_and(|kind| *kind == TypeKind::Object)
    }

    fn java_constructor_type_args(&self, name: &str) -> &'static str {
        if self
            .java_type_param_counts
            .get(name)
            .is_some_and(|count| *count > 0)
        {
            "<>"
        } else {
            ""
        }
    }

    fn java_method_return_type(
        &self,
        owner: &str,
        method: &str,
        arg_len: usize,
    ) -> Option<ir::Type> {
        self.java_method_returns
            .get(&(owner.to_string(), method.to_string(), arg_len))
            .cloned()
    }

    fn java_method_param_candidates(
        &self,
        owner: &str,
        method: &str,
    ) -> Option<&[Vec<JavaParamSpec>]> {
        self.java_method_params
            .get(&(owner.to_string(), method.to_string()))
            .map(Vec::as_slice)
    }

    fn builtin_container(&self, name: &str, args: &[ir::Type]) -> String {
        match name {
            "Array" if args.len() == 1 => {
                format!("lume.core.LumeArray<{}>", self.value_type(&args[0]))
            }
            "Either" if args.len() == 2 => format!(
                "lume.core.Either<{}, {}>",
                self.value_type(&args[0]),
                self.value_type(&args[1])
            ),
            "Vector" if args.len() == 1 => {
                format!("lume.core.LumeVector<{}>", self.value_type(&args[0]))
            }
            "LinkedList" if args.len() == 1 => {
                format!("lume.core.LumeLinkedList<{}>", self.value_type(&args[0]))
            }
            "Iterator" if args.len() == 1 => {
                format!("lume.core.LumeIterator<{}>", self.value_type(&args[0]))
            }
            "Map" if args.len() == 2 => format!(
                "lume.core.LumeMap<{}, {}>",
                self.value_type(&args[0]),
                self.value_type(&args[1])
            ),
            "Option" if args.len() == 1 => {
                format!("lume.core.Option<{}>", self.value_type(&args[0]))
            }
            "Result" if args.len() == 2 => format!(
                "lume.core.Result<{}, {}>",
                self.value_type(&args[0]),
                self.value_type(&args[1])
            ),
            "Set" if args.len() == 1 => {
                format!("lume.core.LumeSet<{}>", self.value_type(&args[0]))
            }
            _ => "Object".to_string(),
        }
    }

    fn tuple_type(&self, items: &[ir::Type]) -> String {
        if !(2..=8).contains(&items.len()) {
            return "Object".to_string();
        }
        let args = items
            .iter()
            .map(|item| self.value_type(item))
            .collect::<Vec<_>>()
            .join(", ");
        format!("lume.core.Tuple{}<{args}>", items.len())
    }

    fn function_type(&self, params: &[ir::Type], ret: &ir::Type) -> String {
        match params.len() {
            0 => format!("java.util.function.Supplier<{}>", self.value_type(ret)),
            1 => format!(
                "java.util.function.Function<{}, {}>",
                self.value_type(&params[0]),
                self.value_type(ret)
            ),
            2 => format!(
                "java.util.function.BiFunction<{}, {}, {}>",
                self.value_type(&params[0]),
                self.value_type(&params[1]),
                self.value_type(ret)
            ),
            3..=MAX_JAVA_FUNCTION_ARITY => {
                let args = params
                    .iter()
                    .map(|param| self.value_type(param))
                    .chain(std::iter::once(self.value_type(ret)))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("lume.core.Function{}<{}>", params.len(), args)
            }
            _ => "Object".to_string(),
        }
    }
}

fn java_named_builtin_value(name: &str) -> Option<String> {
    match name {
        "Any" => Some("Object".to_string()),
        "Unit" => Some("lume.core.LumeUnit".to_string()),
        "Bool" => Some("Boolean".to_string()),
        "Int" => Some("Long".to_string()),
        "Float" => Some("Double".to_string()),
        "Str" => Some("String".to_string()),
        "Rune" => Some("Integer".to_string()),
        "Type" | "ClassType" | "ShapeType" | "EnumType" | "InterfaceType" | "ObjectType"
        | "AnnotationType" => Some("lume.core.LumeType".to_string()),
        "TypeKind" => Some("lume.core.LumeTypeKind".to_string()),
        "AnnotationValue" => Some("lume.core.LumeAnnotation".to_string()),
        "Field" => Some("lume.core.LumeField".to_string()),
        "Method" => Some("lume.core.LumeMethod".to_string()),
        "Param" => Some("lume.core.LumeParam".to_string()),
        "EnumCase" => Some("lume.core.LumeEnumCase".to_string()),
        "ReflectionError" => Some("lume.core.ReflectionError".to_string()),
        "InvalidIndex" => Some("lume.core.InvalidIndex".to_string()),
        _ => None,
    }
}

fn is_core_accessor_backed_type(name: &str) -> bool {
    matches!(name, "InvalidIndex" | "ReflectionError")
}

fn is_reflection_type(name: &str) -> bool {
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

fn runtime_ir_type(represented: ir::Type) -> ir::Type {
    ir::Type::Named {
        name: "Type".to_string(),
        args: vec![match represented {
            ir::Type::Unknown | ir::Type::Never => ir::Type::named("Any"),
            other => other,
        }],
    }
}

fn java_named_builtin_annotation(name: &str) -> Option<String> {
    match name {
        "Bool" => Some("boolean".to_string()),
        "Int" => Some("long".to_string()),
        "Float" => Some("double".to_string()),
        "Str" => Some("String".to_string()),
        "Rune" => Some("int".to_string()),
        _ => None,
    }
}

fn is_builtin_container(name: &str) -> bool {
    matches!(
        name,
        "Array"
            | "Either"
            | "Iterator"
            | "Vector"
            | "LinkedList"
            | "Map"
            | "Option"
            | "Result"
            | "Set"
    )
}

fn core_enum_case_owner(case: &str) -> Option<&'static str> {
    match case {
        "Some" | "None" => Some("Option"),
        "Ok" | "Err" => Some("Result"),
        "Left" | "Right" => Some("Either"),
        _ => None,
    }
}

fn lazy_core_member_call_name(name: &str) -> bool {
    matches!(name, "getOr" | "orElse" | "toResult" | "toEither")
}

fn builtin_method_param_types(
    receiver: &ir::Type,
    method: &str,
    arg_len: usize,
) -> Option<Vec<ir::Type>> {
    match receiver {
        ir::Type::Named { name, args }
            if matches!(name.as_str(), "Vector" | "LinkedList" | "Array" | "Set")
                && args.len() == 1 =>
        {
            match (method, arg_len) {
                ("add", 1) => Some(vec![args[0].clone()]),
                ("addAll", 1) => Some(vec![receiver.clone()]),
                ("at", 1) => Some(vec![ir::Type::Int]),
                ("setAt", 2) => Some(vec![ir::Type::Int, args[0].clone()]),
                ("removeAt", 1) if matches!(name.as_str(), "Vector" | "LinkedList") => {
                    Some(vec![ir::Type::Int])
                }
                ("insertAt", 2) if matches!(name.as_str(), "Vector" | "LinkedList") => {
                    Some(vec![ir::Type::Int, args[0].clone()])
                }
                _ => None,
            }
        }
        ir::Type::Named { name, args } if name == "Map" && args.len() == 2 => {
            match (method, arg_len) {
                ("get" | "remove", 1) => Some(vec![args[0].clone()]),
                ("put", 2) => Some(vec![args[0].clone(), args[1].clone()]),
                _ => None,
            }
        }
        _ => None,
    }
}

fn builtin_method_return_type(
    receiver: &ir::Type,
    method: &str,
    arg_len: usize,
) -> Option<ir::Type> {
    match receiver {
        ir::Type::Named { name, args }
            if matches!(name.as_str(), "Vector" | "LinkedList")
                && args.len() == 1
                && method == "zipWithIndex"
                && arg_len == 0 =>
        {
            Some(ir::Type::list(ir::Type::Tuple(vec![
                args[0].clone(),
                ir::Type::Int,
            ])))
        }
        _ => None,
    }
}

fn iterable_item_type(ty: &ir::Type) -> Option<ir::Type> {
    match ty {
        ir::Type::Named { name, args }
            if matches!(
                name.as_str(),
                "Array" | "Iterator" | "Vector" | "LinkedList" | "Option" | "Set"
            ) && args.len() == 1 =>
        {
            args.first().cloned()
        }
        ir::Type::Named { name, args } if name == "Range" && args.is_empty() => Some(ir::Type::Int),
        _ => None,
    }
}

fn builtin_method_param_specs(
    receiver: &ir::Type,
    method: &str,
    arg_len: usize,
) -> Option<Vec<JavaParamSpec>> {
    match receiver {
        ir::Type::Named { name, args } if name == "Option" && args.len() == 1 => {
            match (method, arg_len) {
                ("getOr", 1) => Some(vec![java_param_spec(args[0].clone(), true)]),
                ("orElse", 1) => Some(vec![java_param_spec(receiver.clone(), true)]),
                ("toResult" | "toEither", 1) => {
                    Some(vec![java_param_spec(ir::Type::Unknown, true)])
                }
                _ => None,
            }
        }
        ir::Type::Named { name, args } if name == "Result" && args.len() == 2 => {
            match (method, arg_len) {
                ("getOr", 1) => Some(vec![java_param_spec(args[0].clone(), true)]),
                ("orElse", 1) => Some(vec![java_param_spec(receiver.clone(), true)]),
                _ => None,
            }
        }
        ir::Type::Named { name, args } if name == "Either" && args.len() == 2 => {
            match (method, arg_len) {
                ("getOr", 1) => Some(vec![java_param_spec(args[1].clone(), true)]),
                ("orElse", 1) => Some(vec![java_param_spec(receiver.clone(), true)]),
                _ => None,
            }
        }
        _ => builtin_method_param_types(receiver, method, arg_len).map(param_specs_from_types),
    }
}

fn java_type_params(params: &[String]) -> String {
    if params.is_empty() {
        String::new()
    } else {
        format!(
            "<{}>",
            params
                .iter()
                .map(|param| java_type_name(param))
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}

fn java_type_args(params: &[String]) -> String {
    if params.is_empty() {
        String::new()
    } else {
        format!(
            "<{}>",
            params
                .iter()
                .map(|param| java_type_name(param))
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}

fn java_type_name(name: &str) -> String {
    sanitize_identifier(name, IdentifierStyle::Type)
}

fn coerce_to_java_primitive(expr: String, coercion: Option<JavaPrimitiveCoercion>) -> String {
    match coercion {
        Some(JavaPrimitiveCoercion::Byte) => format!("((Number) ({expr})).byteValue()"),
        Some(JavaPrimitiveCoercion::Short) => format!("((Number) ({expr})).shortValue()"),
        Some(JavaPrimitiveCoercion::Int) => format!("((Number) ({expr})).intValue()"),
        Some(JavaPrimitiveCoercion::Float) => format!("((Number) ({expr})).floatValue()"),
        None => expr,
    }
}

fn java_member_name(name: &str) -> String {
    sanitize_identifier(name, IdentifierStyle::Member)
}

fn tuple_field_index(name: &str) -> Option<usize> {
    let index = name.strip_prefix('_')?.parse::<usize>().ok()?;
    (1..=8).contains(&index).then_some(index - 1)
}

fn tuple_accessor_name(name: &str) -> Option<&'static str> {
    match tuple_field_index(name)? {
        0 => Some("first"),
        1 => Some("second"),
        2 => Some("third"),
        3 => Some("fourth"),
        4 => Some("fifth"),
        5 => Some("sixth"),
        6 => Some("seventh"),
        7 => Some("eighth"),
        _ => None,
    }
}

fn java_local_name(local: &ir::Local) -> String {
    format!("{}_{}", java_member_name(&local.name), local.id.0)
}

fn java_default_value(ty: &ir::Type) -> String {
    if is_java_void_type(ty) {
        return "lume.core.LumeUnit.INSTANCE".to_string();
    }
    if type_is_named_or_primitive(ty, "Bool", |ty| matches!(ty, ir::Type::Bool)) {
        "false".to_string()
    } else if type_is_named_or_primitive(ty, "Int", |ty| matches!(ty, ir::Type::Int)) {
        "0L".to_string()
    } else if type_is_named_or_primitive(ty, "Float", |ty| matches!(ty, ir::Type::Float)) {
        "0.0".to_string()
    } else if type_is_named_or_primitive(ty, "Str", |ty| matches!(ty, ir::Type::Str)) {
        java_string_literal("")
    } else {
        "null".to_string()
    }
}

fn type_is_named_or_primitive(
    ty: &ir::Type,
    name: &str,
    primitive: impl FnOnce(&ir::Type) -> bool,
) -> bool {
    primitive(ty)
        || matches!(ty, ir::Type::Named { name: ty_name, args } if ty_name == name && args.is_empty())
}

fn is_java_void_type(ty: &ir::Type) -> bool {
    matches!(ty, ir::Type::Unit)
        || matches!(ty, ir::Type::Named { name, args } if name == "Unit" && args.is_empty())
}

fn is_named_builtin(ty: &ir::Type, expected: &str) -> bool {
    matches!(ty, ir::Type::Named { name, args } if name == expected && args.is_empty())
}

fn type_is_str(ty: &ir::Type) -> bool {
    type_is_named_or_primitive(ty, "Str", |ty| matches!(ty, ir::Type::Str))
}

fn type_is_float_like(ty: &ir::Type) -> bool {
    type_is_named_or_primitive(ty, "Float", |ty| matches!(ty, ir::Type::Float))
}

fn java_type_contains_type_param(ty: &ir::Type) -> bool {
    match ty {
        ir::Type::TypeParam(_) => true,
        ir::Type::Named { args, .. } | ir::Type::Tuple(args) => {
            args.iter().any(java_type_contains_type_param)
        }
        ir::Type::Record(fields) => fields
            .iter()
            .any(|field| java_type_contains_type_param(&field.ty)),
        ir::Type::Function { params, ret } => {
            params.iter().any(java_type_contains_type_param) || java_type_contains_type_param(ret)
        }
        ir::Type::Unknown
        | ir::Type::Never
        | ir::Type::Unit
        | ir::Type::Bool
        | ir::Type::Int
        | ir::Type::Float
        | ir::Type::Str => false,
    }
}

fn java_type_contains_unknown(ty: &ir::Type) -> bool {
    match ty {
        ir::Type::Unknown => true,
        ir::Type::Named { args, .. } | ir::Type::Tuple(args) => {
            args.iter().any(java_type_contains_unknown)
        }
        ir::Type::Record(fields) => fields
            .iter()
            .any(|field| java_type_contains_unknown(&field.ty)),
        ir::Type::Function { params, ret } => {
            params.iter().any(java_type_contains_unknown) || java_type_contains_unknown(ret)
        }
        ir::Type::TypeParam(_)
        | ir::Type::Never
        | ir::Type::Unit
        | ir::Type::Bool
        | ir::Type::Int
        | ir::Type::Float
        | ir::Type::Str => false,
    }
}

fn java_type_needs_reference_cast(ty: &ir::Type) -> bool {
    match ty {
        ir::Type::Named { name, args } => {
            !args.is_empty()
                || (java_named_builtin_value(name).is_none() && !is_reflection_type(name))
        }
        ir::Type::TypeParam(_)
        | ir::Type::Tuple(_)
        | ir::Type::Record(_)
        | ir::Type::Function { .. } => true,
        ir::Type::Unknown
        | ir::Type::Never
        | ir::Type::Unit
        | ir::Type::Bool
        | ir::Type::Int
        | ir::Type::Float
        | ir::Type::Str => false,
    }
}

fn java_type_params_are_bound(ty: &ir::Type, bound: &[String]) -> bool {
    match ty {
        ir::Type::TypeParam(name) => bound.iter().any(|param| param == name),
        ir::Type::Named { args, .. } | ir::Type::Tuple(args) => args
            .iter()
            .all(|arg| java_type_params_are_bound(arg, bound)),
        ir::Type::Record(fields) => fields
            .iter()
            .all(|field| java_type_params_are_bound(&field.ty, bound)),
        ir::Type::Function { params, ret } => {
            params
                .iter()
                .all(|param| java_type_params_are_bound(param, bound))
                && java_type_params_are_bound(ret, bound)
        }
        ir::Type::Unknown
        | ir::Type::Never
        | ir::Type::Unit
        | ir::Type::Bool
        | ir::Type::Int
        | ir::Type::Float
        | ir::Type::Str => true,
    }
}

fn substitute_java_emit_type(ty: &ir::Type, subst: &HashMap<String, ir::Type>) -> ir::Type {
    match ty {
        ir::Type::TypeParam(name) => subst.get(name).cloned().unwrap_or_else(|| ty.clone()),
        ir::Type::Named { name, args } => ir::Type::Named {
            name: name.clone(),
            args: args
                .iter()
                .map(|arg| substitute_java_emit_type(arg, subst))
                .collect(),
        },
        ir::Type::Tuple(items) => ir::Type::Tuple(
            items
                .iter()
                .map(|item| substitute_java_emit_type(item, subst))
                .collect(),
        ),
        ir::Type::Record(fields) => ir::Type::Record(
            fields
                .iter()
                .map(|field| ir::NamedType {
                    name: field.name.clone(),
                    ty: substitute_java_emit_type(&field.ty, subst),
                })
                .collect(),
        ),
        ir::Type::Function { params, ret } => ir::Type::Function {
            params: params
                .iter()
                .map(|param| substitute_java_emit_type(param, subst))
                .collect(),
            ret: Box::new(substitute_java_emit_type(ret, subst)),
        },
        _ => ty.clone(),
    }
}

fn java_constant(constant: &ir::Constant) -> String {
    match constant {
        ir::Constant::Unit => "lume.core.LumeUnit.INSTANCE".to_string(),
        ir::Constant::Bool(value) => value.to_string(),
        ir::Constant::Int(value) => format!("{value}L"),
        ir::Constant::Float(value) => java_float_literal(*value),
        ir::Constant::String(value) => java_string_literal(&decode_lume_string_literal(value)),
        ir::Constant::List(items) => {
            let items = items
                .iter()
                .map(java_constant)
                .collect::<Vec<_>>()
                .join(", ");
            format!("lume.core.LumeVector.of({items})")
        }
    }
}

fn constant_type(constant: &ir::Constant) -> ir::Type {
    match constant {
        ir::Constant::Unit => ir::Type::Unit,
        ir::Constant::Bool(_) => ir::Type::Bool,
        ir::Constant::Int(_) => ir::Type::Int,
        ir::Constant::Float(_) => ir::Type::Float,
        ir::Constant::String(_) => ir::Type::Str,
        ir::Constant::List(_) => ir::Type::Unknown,
    }
}

fn type_ref_to_ir(reference: &TypeRef) -> ir::Type {
    match reference {
        TypeRef::Wildcard { .. } => ir::Type::Unknown,
        TypeRef::Named { name, args, .. } if name == "Never" && args.is_empty() => ir::Type::Never,
        TypeRef::Named { name, args, .. } => ir::Type::Named {
            name: name.clone(),
            args: args.iter().map(type_ref_to_ir).collect(),
        },
        TypeRef::Tuple { fields, .. } => ir::Type::Tuple(
            fields
                .iter()
                .map(|field| type_ref_to_ir(&field.ty))
                .collect(),
        ),
        TypeRef::Record { fields, .. } => ir::Type::Record(
            fields
                .iter()
                .map(|field| ir::NamedType {
                    name: field.name.clone(),
                    ty: type_ref_to_ir(&field.ty),
                })
                .collect(),
        ),
        TypeRef::Function { params, ret, .. } => ir::Type::Function {
            params: params.iter().map(type_ref_to_ir).collect(),
            ret: Box::new(type_ref_to_ir(ret)),
        },
    }
}

fn java_float_literal(value: f64) -> String {
    let mut rendered = value.to_string();
    if !rendered.contains('.') && !rendered.contains('e') && !rendered.contains('E') {
        rendered.push_str(".0");
    }
    rendered
}

fn java_string_literal(value: &str) -> String {
    let mut out = String::from("\"");
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch if ch.is_control() => out.push_str(&format!("\\u{:04x}", ch as u32)),
            ch => out.push(ch),
        }
    }
    out.push('"');
    out
}

fn decode_lume_string_literal(raw: &str) -> String {
    let (is_raw, quoted) = raw
        .strip_prefix("raw")
        .map_or((false, raw), |quoted| (true, quoted));
    let body = if quoted.starts_with("\"\"\"") && quoted.ends_with("\"\"\"") && quoted.len() >= 6 {
        &quoted[3..quoted.len() - 3]
    } else {
        quoted
            .strip_prefix('"')
            .and_then(|value| value.strip_suffix('"'))
            .unwrap_or(quoted)
    };
    if is_raw {
        body.to_string()
    } else {
        decode_lume_string_contents(body)
    }
}

fn decode_lume_string_contents(body: &str) -> String {
    let mut out = String::new();
    let mut chars = body.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some('\\') => out.push('\\'),
            Some('"') => out.push('"'),
            Some('0') => out.push('\0'),
            Some('$') => out.push('$'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

fn sanitize_identifier(name: &str, style: IdentifierStyle) -> String {
    let mut pieces = name
        .split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_')
        .filter(|piece| !piece.is_empty());
    let mut out = String::new();

    match style {
        IdentifierStyle::Type => {
            for piece in pieces {
                let mut chars = piece.chars();
                if let Some(first) = chars.next() {
                    out.push(first.to_ascii_uppercase());
                    out.extend(chars);
                }
            }
        }
        IdentifierStyle::Member => {
            if let Some(first_piece) = pieces.next() {
                out.push_str(first_piece);
            }
            for piece in pieces {
                let mut chars = piece.chars();
                if let Some(first) = chars.next() {
                    out.push(first.to_ascii_uppercase());
                    out.extend(chars);
                }
            }
        }
    }

    if out.is_empty() {
        out.push('_');
    }
    if out
        .chars()
        .next()
        .is_some_and(|first| !first.is_ascii_alphabetic() && first != '_')
    {
        out.insert(0, '_');
    }
    if is_java_reserved(&out) {
        out.push('_');
    }
    out
}

#[derive(Debug, Clone, Copy)]
enum IdentifierStyle {
    Type,
    Member,
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

struct JavaPackage {
    name: Option<String>,
    relative_dir: PathBuf,
}

impl JavaPackage {
    fn from_module(module: Option<&str>) -> Self {
        let segments = module
            .into_iter()
            .flat_map(|module| module.split('/'))
            .filter(|segment| !segment.is_empty())
            .map(sanitize_package_segment)
            .collect::<Vec<_>>();
        let name = (!segments.is_empty()).then(|| segments.join("."));
        let relative_dir = segments.iter().fold(PathBuf::new(), |path, segment| {
            path.join(Path::new(segment))
        });
        Self { name, relative_dir }
    }

    fn relative_file(&self, file_name: &str) -> PathBuf {
        self.relative_dir.join(file_name)
    }
}

fn sanitize_package_segment(segment: &str) -> String {
    let mut out = segment
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' {
                ch.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect::<String>();
    if out.is_empty() {
        out.push('_');
    }
    if out
        .chars()
        .next()
        .is_some_and(|first| !first.is_ascii_alphabetic() && first != '_')
    {
        out.insert(0, '_');
    }
    if is_java_reserved(&out) {
        out.push('_');
    }
    out
}
